//! Durable, per-browser Web Push delivery. Notification payloads contain no post text.
use crate::{now, security, store::Store};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use web_push_native::{
    Auth, WebPushBuilder,
    jwt_simple::algorithms::{ECDSAP256PublicKeyLike, ES256KeyPair},
    p256::PublicKey,
};

pub fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS push_subscriptions(id TEXT PRIMARY KEY,username TEXT NOT NULL REFERENCES users(username) ON DELETE CASCADE,session TEXT NOT NULL,data TEXT NOT NULL,generation TEXT NOT NULL,cursor INTEGER NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,next_try INTEGER NOT NULL DEFAULT 0,last_error TEXT,last_sent INTEGER);CREATE INDEX IF NOT EXISTS push_user ON push_subscriptions(username);")?;
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub endpoint: String,
    pub keys: Keys,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Keys {
    pub p256dh: String,
    pub auth: String,
}
impl Subscription {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.endpoint.len() <= 4096, "Push endpoint is too long");
        let url = url::Url::parse(&self.endpoint)?;
        let host = url.host_str().unwrap_or("");
        let known = matches!(
            host,
            "fcm.googleapis.com"
                | "android.googleapis.com"
                | "web.push.apple.com"
                | "updates.push.services.mozilla.com"
        ) || host.ends_with(".push.services.mozilla.com")
            || host.ends_with(".push.apple.com")
            || host.ends_with(".notify.windows.com");
        ensure!(
            url.scheme() == "https"
                && known
                && url.port_or_known_default() == Some(443)
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none(),
            "Unsupported push service endpoint"
        );
        let key = URL_SAFE_NO_PAD.decode(&self.keys.p256dh)?;
        PublicKey::from_sec1_bytes(&key).context("Invalid browser push key")?;
        ensure!(
            URL_SAFE_NO_PAD.decode(&self.keys.auth)?.len() == 16,
            "Invalid browser push authentication key"
        );
        Ok(())
    }
}
pub struct Push {
    key: ES256KeyPair,
    pub public_key: String,
    contact: String,
    client: reqwest::Client,
}
pub struct PendingPush {
    pub id: String,
    pub data: Subscription,
    pub cursor: i64,
    pub generation: String,
    pub attempts: u32,
    pub newest: i64,
}
impl Push {
    pub fn new(store: &Store, origin: &str) -> Result<Self> {
        let generated = ES256KeyPair::generate();
        let encoded = URL_SAFE_NO_PAD.encode(generated.to_bytes());
        let stored = {
            let db = store.0.lock().unwrap();
            db.execute(
                "INSERT OR IGNORE INTO settings(key,value) VALUES('vapid_private',?1)",
                [encoded],
            )?;
            db.query_row(
                "SELECT value FROM settings WHERE key='vapid_private'",
                [],
                |r| r.get::<_, String>(0),
            )?
        };
        let key = ES256KeyPair::from_bytes(&URL_SAFE_NO_PAD.decode(stored)?)?;
        let public_key =
            URL_SAFE_NO_PAD.encode(key.public_key().public_key().to_bytes_uncompressed());
        Ok(Self {
            key,
            public_key,
            contact: origin.to_owned(),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(20))
                .build()?,
        })
    }
    pub fn request(
        &self,
        subscription: &Subscription,
        payload: &Value,
    ) -> Result<reqwest::Request> {
        subscription.validate()?;
        let public =
            PublicKey::from_sec1_bytes(&URL_SAFE_NO_PAD.decode(&subscription.keys.p256dh)?)?;
        let auth = URL_SAFE_NO_PAD.decode(&subscription.keys.auth)?;
        let request = WebPushBuilder::new(
            subscription.endpoint.parse()?,
            public,
            Auth::clone_from_slice(&auth),
        )
        .with_valid_duration(Duration::from_secs(86400))
        .with_vapid(&self.key, &self.contact)
        .build(payload.to_string())?;
        Ok(request.try_into()?)
    }
    pub async fn send(&self, p: &PendingPush) -> Result<u16> {
        let payload = json!({"title":"Agent Manager","body":"New activity in your inbox","tag":"agent-manager-inbox","url":format!("/#/inbox?notification={}",p.newest)});
        let request = self.request(&p.data, &payload)?;
        Ok(self.client.execute(request).await?.status().as_u16())
    }
}
impl Store {
    pub fn push_subscribe(&self, user: &str, session: &str, data: &Subscription) -> Result<()> {
        data.validate()?;
        let id = security::digest(&data.endpoint);
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM push_subscriptions WHERE username=?1",
            [user],
            |r| r.get(0),
        )?;
        ensure!(
            count < 20
                || tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM push_subscriptions WHERE id=?1)",
                    [&id],
                    |r| r.get::<_, bool>(0)
                )?,
            "At most 20 notification devices per account"
        );
        let cursor: i64 = tx.query_row(
            "SELECT COALESCE(MAX(id),0) FROM board_notifications WHERE recipient=?1",
            [format!("human:{user}")],
            |r| r.get(0),
        )?;
        tx.execute("INSERT INTO push_subscriptions(id,username,session,data,cursor,generation) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET username=excluded.username,session=excluded.session,data=excluded.data,generation=excluded.generation,cursor=CASE WHEN push_subscriptions.username=excluded.username THEN push_subscriptions.cursor ELSE excluded.cursor END,next_try=0,last_error=NULL",params![id,user,session,serde_json::to_string(data)?,cursor,security::token()])?;
        tx.commit()?;
        Ok(())
    }
    pub fn push_status(&self, user: &str, endpoint: &str) -> Result<Value> {
        let row = self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT last_error,last_sent FROM push_subscriptions WHERE username=?1 AND id=?2",
                params![user, security::digest(endpoint)],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<i64>>(1)?)),
            )
            .optional()?;
        Ok(match row {
            Some((error, sent)) => json!({"enabled":true,"error":error,"lastSent":sent}),
            None => json!({"enabled":false}),
        })
    }
    pub fn push_unsubscribe(&self, user: &str, endpoint: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "DELETE FROM push_subscriptions WHERE username=?1 AND id=?2",
            params![user, security::digest(endpoint)],
        )?;
        Ok(())
    }
    pub fn push_logout(&self, session: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .execute("DELETE FROM push_subscriptions WHERE session=?1", [session])?;
        Ok(())
    }
    pub fn push_pending(&self) -> Result<Vec<PendingPush>> {
        let db = self.0.lock().unwrap();
        let records=db.prepare("SELECT id,username,data,cursor,attempts,generation FROM push_subscriptions WHERE next_try<=?1 ORDER BY COALESCE(last_sent,0),id")?.query_map([now()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,u32>(4)?,r.get::<_,String>(5)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut pending = Vec::new();
        for (id, user, data, cursor, attempts, generation) in records {
            let newest:Option<i64>=db.query_row("SELECT MAX(id) FROM board_notifications WHERE recipient=?1 AND id>?2 AND read_at IS NULL",params![format!("human:{user}"),cursor],|r|r.get(0))?;
            if let Some(newest) = newest {
                pending.push(PendingPush {
                    id,
                    data: serde_json::from_str(&data)?,
                    cursor,
                    attempts,
                    newest,
                    generation,
                });
                if pending.len() >= 100 {
                    break;
                }
            }
        }
        Ok(pending)
    }
    pub fn push_result(&self, p: &PendingPush, status: Option<u16>) -> Result<()> {
        let db = self.0.lock().unwrap();
        match status {
            Some(200..=299) => {
                db.execute("UPDATE push_subscriptions SET cursor=MAX(cursor,?1),attempts=0,next_try=0,last_error=NULL,last_sent=?2 WHERE id=?3 AND cursor=?4 AND generation=?5",params![p.newest,now(),p.id,p.cursor,p.generation])?;
            }
            Some(404 | 410) => {
                db.execute(
                    "DELETE FROM push_subscriptions WHERE id=?1 AND generation=?2",
                    params![p.id, p.generation],
                )?;
            }
            _ => {
                let delay = (15i64 * 2i64.pow(p.attempts.min(10))).min(86400);
                let message = match status {
                    Some(n) => format!("Push service returned HTTP {n}; retry scheduled"),
                    None => "Push service could not be reached; retry scheduled".to_owned(),
                };
                db.execute("UPDATE push_subscriptions SET attempts=attempts+1,next_try=?1,last_error=?2 WHERE id=?3 AND cursor=?4 AND generation=?5",params![now()+delay,message,p.id,p.cursor,p.generation])?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use web_push_native::p256::{
        SecretKey,
        elliptic_curve::{rand_core::OsRng, sec1::ToEncodedPoint},
    };
    fn subscription(label: &str) -> (Subscription, SecretKey) {
        let key = SecretKey::random(&mut OsRng);
        let s = Subscription {
            endpoint: format!("https://fcm.googleapis.com/wp/{label}"),
            keys: Keys {
                p256dh: URL_SAFE_NO_PAD.encode(key.public_key().to_encoded_point(false).as_bytes()),
                auth: URL_SAFE_NO_PAD.encode([7u8; 16]),
            },
        };
        (s, key)
    }
    #[test]
    fn push_request_uses_persisted_vapid_key_and_encrypts_payload() {
        let dir = crate::test_support::tempdir();
        let store = Store::open(dir.path()).unwrap();
        let push = Push::new(&store, "https://agents.example.com").unwrap();
        let other = Push::new(&store, "https://agents.example.com").unwrap();
        assert_eq!(push.public_key, other.public_key);
        let (s, secret) = subscription("encrypted");
        let payload = json!({"title":"Agent Manager","body":"New activity in your inbox","url":"/#/inbox?notification=42"});
        let request = push.request(&s, &payload).unwrap();
        assert_eq!(request.headers()["content-encoding"], "aes128gcm");
        assert!(
            request.headers()["authorization"]
                .to_str()
                .unwrap()
                .contains(&format!("k={}", push.public_key))
        );
        let bytes = request.body().unwrap().as_bytes().unwrap();
        assert!(!String::from_utf8_lossy(bytes).contains("New activity"));
        let decoded =
            web_push_native::decrypt(bytes.to_vec(), &secret, &Auth::clone_from_slice(&[7u8; 16]))
                .unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&decoded).unwrap(), payload);
    }
    #[test]
    fn delivery_retries_restarts_expiry_and_logout_are_independent_of_inbox_read_state() {
        let dir = crate::test_support::tempdir();
        let store = Store::open(dir.path()).unwrap();
        store.set_user_password("alice", "hash").unwrap();
        store.set_user_password("bob", "hash").unwrap();
        let alice = store.board_human("alice").unwrap();
        let bob = store.board_human("bob").unwrap();
        let (s, _) = subscription("device");
        store
            .push_subscribe("alice", "browser-session", &s)
            .unwrap();
        store
            .board(
                &alice,
                "set_subscription",
                &json!({"kind":"channel","target":1,"subscribed":true}),
            )
            .unwrap();
        let t = store
            .board(
                &bob,
                "create_topic",
                &json!({"channel":1,"body":"Private discussion text"}),
            )
            .unwrap();
        let pending = store.push_pending().unwrap();
        assert_eq!(pending.len(), 1);
        store.push_result(&pending[0], Some(503)).unwrap();
        assert!(store.push_pending().unwrap().is_empty());
        assert!(store.push_status("alice", &s.endpoint).unwrap()["error"].is_string());
        // An explicit re-enable clears retry delay without losing the pending cursor.
        store
            .push_subscribe("alice", "browser-session", &s)
            .unwrap();
        let p = store.push_pending().unwrap().remove(0);
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.push_pending().unwrap().len(), 1);
        store.push_result(&p, Some(201)).unwrap();
        assert!(store.push_pending().unwrap().is_empty());
        assert!(
            !store.board_pending(&alice.id).unwrap().is_empty(),
            "Push does not ACK worker delivery or human read state"
        );
        store
            .board(
                &bob,
                "post_reply",
                &json!({"topic":t["id"],"body":"@alice new event"}),
            )
            .unwrap();
        let p = store.push_pending().unwrap().remove(0);
        // A late response to an old registration cannot delete a freshly enabled one.
        store
            .push_subscribe("alice", "browser-session", &s)
            .unwrap();
        store.push_result(&p, Some(410)).unwrap();
        assert_eq!(
            store.push_status("alice", &s.endpoint).unwrap()["enabled"],
            true
        );
        let p = store.push_pending().unwrap().remove(0);
        store.push_result(&p, Some(410)).unwrap();
        assert_eq!(
            store.push_status("alice", &s.endpoint).unwrap()["enabled"],
            false
        );
        store
            .push_subscribe("alice", "browser-session", &s)
            .unwrap();
        store.push_logout("different-session").unwrap();
        assert_eq!(
            store.push_status("alice", &s.endpoint).unwrap()["enabled"],
            true
        );
        store.push_logout("browser-session").unwrap();
        assert_eq!(
            store.push_status("alice", &s.endpoint).unwrap()["enabled"],
            false
        );
    }
    #[test]
    fn invalid_endpoints_keys_and_cross_account_unsubscribe_are_rejected() {
        let dir = crate::test_support::tempdir();
        let store = Store::open(dir.path()).unwrap();
        store.set_user_password("alice", "hash").unwrap();
        store.set_user_password("bob", "hash").unwrap();
        let (mut s, _) = subscription("safe");
        for endpoint in [
            "http://fcm.googleapis.com/wp/a",
            "https://127.0.0.1/a",
            "https://fcm.googleapis.com.evil.invalid/a",
            "https://user@fcm.googleapis.com/a",
            "https://fcm.googleapis.com:8443/a",
        ] {
            s.endpoint = endpoint.into();
            assert!(s.validate().is_err());
        }
        s.endpoint = "https://fcm.googleapis.com/wp/safe".into();
        s.keys.auth = URL_SAFE_NO_PAD.encode([0u8; 15]);
        assert!(s.validate().is_err());
        s.keys.auth = URL_SAFE_NO_PAD.encode([0u8; 16]);
        store.push_subscribe("alice", "one", &s).unwrap();
        store.push_unsubscribe("bob", &s.endpoint).unwrap();
        assert_eq!(
            store.push_status("alice", &s.endpoint).unwrap()["enabled"],
            true
        );
        store.set_user_password("alice", "new hash").unwrap();
        assert_eq!(
            store.push_status("alice", &s.endpoint).unwrap()["enabled"],
            false
        );
    }
}
