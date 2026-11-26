//! In-memory login protection. IPs behind a NAT share a budget; usernames do not
//! lock one another out across networks. Restarting the server clears cooldowns.
use axum::http::HeaderMap;
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const RETENTION: Duration = Duration::from_secs(15 * 60);
const MAX_SOURCES: usize = 4096;

#[derive(Default)]
pub(super) struct LoginThrottle(Mutex<HashMap<IpAddr, Source>>);
struct Source {
    failures: u32,
    ready: Instant,
    last: Instant,
    running: bool,
}
pub(super) struct Attempt {
    throttle: Arc<LoginThrottle>,
    ip: IpAddr,
    finished: bool,
}
impl LoginThrottle {
    pub(super) fn begin(self: &Arc<Self>, ip: IpAddr, now: Instant) -> Result<Attempt, u64> {
        let mut sources = self.0.lock().unwrap();
        sources.retain(|_, s| s.running || now.duration_since(s.last) < RETENTION);
        if !sources.contains_key(&ip) && sources.len() >= MAX_SOURCES {
            // Never evict a penalized source to make room for attacker-chosen IPs.
            return Err(60);
        }
        let source = sources.entry(ip).or_insert(Source {
            failures: 0,
            ready: now,
            last: now,
            running: false,
        });
        if source.running {
            return Err(1);
        }
        if source.ready > now {
            return Err(source.ready.duration_since(now).as_secs_f64().ceil() as u64);
        }
        source.running = true;
        source.last = now;
        Ok(Attempt {
            throttle: self.clone(),
            ip,
            finished: false,
        })
    }
}
impl Attempt {
    pub(super) fn busy(mut self) {
        let mut sources = self.throttle.0.lock().unwrap();
        let source = sources.get_mut(&self.ip).expect("active source retained");
        source.running = false;
        self.finished = true;
    }
    pub(super) fn finish(mut self, success: bool, now: Instant) -> u64 {
        let delay = self.record(success, now);
        self.finished = true;
        delay
    }
    fn record(&self, success: bool, now: Instant) -> u64 {
        let mut sources = self.throttle.0.lock().unwrap();
        if success {
            sources.remove(&self.ip);
            return 0;
        }
        let source = sources.get_mut(&self.ip).expect("active source retained");
        source.failures = (source.failures + 1).min(7);
        let seconds = (1u64 << (source.failures - 1)).min(60);
        source.ready = now + Duration::from_secs(seconds);
        source.last = now;
        source.running = false;
        seconds
    }
}
impl Drop for Attempt {
    fn drop(&mut self) {
        // Disconnecting a request must not bypass failure accounting.
        if !self.finished {
            self.record(false, Instant::now());
        }
    }
}

pub(super) fn client_ip(peer: Option<SocketAddr>, headers: &HeaderMap) -> IpAddr {
    let Some(peer) = peer else {
        return Ipv4Addr::LOCALHOST.into();
    };
    // The server binds only loopback. Caddy overwrites/appends X-Forwarded-For;
    // use the rightmost hop, never a client-supplied prefix. Ignore all forwarded
    // headers from non-loopback peers (including in alternate library hosts).
    let forwarded = peer
        .ip()
        .is_loopback()
        .then(|| {
            headers
                .get_all("x-forwarded-for")
                .iter()
                .next_back()?
                .to_str()
                .ok()?
                .rsplit(',')
                .next()?
                .trim()
                .parse::<IpAddr>()
                .ok()
        })
        .flatten();
    match forwarded.unwrap_or(peer.ip()) {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ip)),
        ip => ip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exponential_delays_are_bounded_and_do_not_extend_on_rejected_retries() {
        let throttle = Arc::new(LoginThrottle::default());
        let ip = "192.0.2.1".parse().unwrap();
        let mut now = Instant::now();
        for delay in [1, 2, 4, 8, 16, 32, 60, 60] {
            let attempt = throttle.begin(ip, now).ok().unwrap();
            assert_eq!(throttle.begin(ip, now).err(), Some(1));
            assert_eq!(attempt.finish(false, now), delay);
            assert_eq!(throttle.begin(ip, now).err(), Some(delay));
            now += Duration::from_secs(delay);
        }
        throttle.begin(ip, now).ok().unwrap().finish(true, now);
        assert_eq!(throttle.begin(ip, now).ok().unwrap().finish(false, now), 1);
        now += RETENTION;
        assert_eq!(throttle.begin(ip, now).ok().unwrap().finish(false, now), 1);
    }
    #[test]
    fn sources_are_independent_and_cancelled_attempts_count() {
        let throttle = Arc::new(LoginThrottle::default());
        let now = Instant::now();
        let ip = "192.0.2.1".parse().unwrap();
        drop(throttle.begin(ip, now).ok().unwrap());
        assert_eq!(throttle.begin(ip, Instant::now()).err(), Some(1));
        throttle
            .begin("192.0.2.2".parse().unwrap(), Instant::now())
            .ok()
            .unwrap()
            .finish(true, Instant::now());
    }
    #[test]
    fn busy_verifier_does_not_reset_failure_history() {
        let throttle = Arc::new(LoginThrottle::default());
        let ip = "192.0.2.1".parse().unwrap();
        let now = Instant::now();
        throttle.begin(ip, now).ok().unwrap().finish(false, now);
        let later = now + Duration::from_secs(1);
        throttle.begin(ip, later).ok().unwrap().busy();
        assert_eq!(
            throttle.begin(ip, later).ok().unwrap().finish(false, later),
            2
        );
    }

    #[test]
    fn bounded_storage_preserves_active_penalties() {
        let throttle = Arc::new(LoginThrottle::default());
        let now = Instant::now();
        for n in 0..MAX_SOURCES {
            throttle
                .begin(Ipv4Addr::from(n as u32).into(), now)
                .ok()
                .unwrap()
                .finish(false, now);
        }
        let other = "192.0.2.1".parse().unwrap();
        assert_eq!(throttle.begin(other, now).err(), Some(60));
        assert_eq!(throttle.begin(Ipv4Addr::from(0).into(), now).err(), Some(1));
        throttle
            .begin(other, now + RETENTION)
            .ok()
            .unwrap()
            .finish(true, now + RETENTION);
    }
    #[test]
    fn only_loopback_proxy_and_rightmost_forwarded_address_are_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "198.51.100.1, 192.0.2.2".parse().unwrap(),
        );
        assert_eq!(
            client_ip(Some("127.0.0.1:80".parse().unwrap()), &headers).to_string(),
            "192.0.2.2"
        );
        assert_eq!(
            client_ip(Some("203.0.113.1:80".parse().unwrap()), &headers).to_string(),
            "203.0.113.1"
        );
        headers.insert("x-forwarded-for", "192.0.2.1, invalid".parse().unwrap());
        assert_eq!(
            client_ip(Some("127.0.0.1:80".parse().unwrap()), &headers).to_string(),
            "127.0.0.1"
        );
        headers.insert("x-forwarded-for", "::ffff:192.0.2.1".parse().unwrap());
        assert_eq!(
            client_ip(Some("127.0.0.1:80".parse().unwrap()), &headers).to_string(),
            "192.0.2.1"
        );
    }
}
