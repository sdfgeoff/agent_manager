//! Ephemeral presence: no last-seen history, private commands or credentials in responses.
use super::*;
use std::collections::HashSet;

const HUMAN_TTL: Duration = Duration::from_secs(65);
const WORKER_TTL: Duration = Duration::from_secs(35);

#[derive(Default)]
pub(super) struct Presence {
    humans: HashMap<(String, String), Human>,
    workers: HashMap<String, Worker>,
}
struct Human {
    username: String,
    active: bool,
    seen: Instant,
}
struct Worker {
    status: Value,
    seen: Instant,
    revision: u64,
}
impl Presence {
    pub(super) fn forget_worker(&mut self, id: &str) {
        self.workers.remove(id);
    }
    pub(super) fn observe(&mut self, event: &Value) -> bool {
        if event["type"] == "workers_changed" || event["type"] == "board_changed" {
            return true;
        }
        if event["type"] != "codex" {
            return false;
        }
        let Some(id) = event["worker"].as_str() else {
            return false;
        };
        let message = &event["message"];
        let method = message["method"].as_str().unwrap_or("");
        let relevant = matches!(
            method,
            "manager/connected"
                | "manager/resync"
                | "manager/codexExited"
                | "manager/requestsChanged"
                | "turn/started"
                | "turn/completed"
                | "item/completed"
        ) || message.get("id").is_some();
        if !relevant {
            return false;
        }
        let worker = self.workers.entry(id.into()).or_insert(Worker {
            status: Value::Null,
            seen: Instant::now(),
            revision: 0,
        });
        worker.revision = worker.revision.wrapping_add(1);
        match method {
            "manager/connected" => {
                worker.status = summary(&message["params"]);
                worker.seen = Instant::now();
            }
            "manager/codexExited" => {
                worker.status = json!({"alive":false});
            }
            "turn/started" => {
                if let (Some(thread), Some(turn)) = (
                    message["params"]["threadId"].as_str(),
                    message["params"]["turn"]["id"].as_str(),
                ) && worker.status["active"].is_object()
                {
                    worker.status["active"][thread] = json!(turn);
                }
            }
            "turn/completed" => {
                if let Some(thread) = message["params"]["threadId"].as_str() {
                    let completed = &message["params"]["turn"]["id"];
                    if (completed.is_null() || worker.status["active"][thread] == *completed)
                        && let Some(active) = worker.status["active"].as_object_mut()
                    {
                        active.remove(thread);
                    }
                }
            }
            "manager/resync" => worker.status = Value::Null,
            _ => {
                if message.get("id").is_some()
                    && let Some(thread) = message["params"]["threadId"].as_str()
                    && let Some(waiting) = worker.status["waiting"].as_array_mut()
                    && !waiting.iter().any(|v| v == thread)
                {
                    waiting.push(json!(thread));
                }
            }
        }
        true
    }
}
fn summary(status: &Value) -> Value {
    if !status["alive"].is_boolean()
        || !status["active"].is_object()
        || !status["requests"].is_array()
    {
        return Value::Null;
    }
    let waiting = status["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["params"]["threadId"].as_str())
        .collect::<HashSet<_>>();
    json!({"alive":status["alive"],"active":status["active"],"waiting":waiting})
}
fn agent_state(
    paused: bool,
    online: bool,
    error: bool,
    worker: Option<&Worker>,
    thread: &str,
) -> &'static str {
    if paused {
        return "paused";
    }
    if !online {
        return "offline";
    }
    if error {
        return "error";
    }
    let Some(worker) = worker.filter(|w| w.seen.elapsed() < WORKER_TTL) else {
        return "unknown";
    };
    if worker.status["alive"] == false {
        return "error";
    }
    if worker.status["alive"] != true {
        return "unknown";
    }
    if worker.status["waiting"]
        .as_array()
        .is_some_and(|v| v.iter().any(|v| v == thread))
    {
        return "waiting";
    }
    if !worker.status["active"][thread].is_null() {
        "working"
    } else {
        "idle"
    }
}

pub(super) fn routes() -> Router<Arc<App>> {
    Router::new().route("/api/presence", get(read).post(heartbeat))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Heartbeat {
    client: String,
    active: bool,
}
async fn heartbeat(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<Heartbeat>,
) -> ApiResult<Json<Value>> {
    let username = app.authorize(&headers)?;
    app.csrf(&headers)?;
    if uuid::Uuid::parse_str(&body.client).is_err() {
        return Err(rejected(StatusCode::BAD_REQUEST, "Invalid presence client"));
    }
    let key = security::digest(cookie(&headers).unwrap());
    {
        let mut presence = app.presence.lock().unwrap();
        presence.humans.retain(|_, h| h.seen.elapsed() < HUMAN_TTL);
        if !presence
            .humans
            .contains_key(&(key.clone(), body.client.clone()))
            && (presence.humans.len() >= 4096
                || presence
                    .humans
                    .keys()
                    .filter(|(session, _)| session == &key)
                    .count()
                    >= 32)
        {
            return Err(rejected(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many presence clients",
            ));
        }
        presence.humans.insert(
            (key, body.client),
            Human {
                username,
                active: body.active,
                seen: Instant::now(),
            },
        );
    }
    // State changes are small and independent of board metadata/inbox refreshes.
    let _ = app.events.send(json!({"type":"presence_changed"}));
    Ok(Json(json!({"ok":true})))
}
async fn read(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    app.authorize(&headers)?;
    let agents = app.store.board_agents()?;
    let sessions = app.sessions.lock().unwrap();
    let peers = app.peers.lock().unwrap();
    let mut presence = app.presence.lock().unwrap();
    presence.humans.retain(|(key, _), h| {
        h.seen.elapsed() < HUMAN_TTL && sessions.get(key).is_some_and(|s| app.valid_session(s))
    });
    let mut actors = serde_json::Map::new();
    for human in presence.humans.values() {
        let id = format!("human:{}", human.username);
        if human.active || !actors.contains_key(&id) {
            actors.insert(id, json!(if human.active { "active" } else { "away" }));
        }
    }
    for agent in agents {
        actors.insert(
            agent.id,
            json!(agent_state(
                agent.paused,
                peers.contains_key(&agent.worker),
                agent.error.is_some(),
                presence.workers.get(&agent.worker),
                &agent.thread
            )),
        );
    }
    Ok(Json(json!({"actors":actors})))
}

pub(super) fn start(app: &Arc<App>) {
    let weak = Arc::downgrade(app);
    tokio::spawn(async move {
        loop {
            let Some(app) = weak.upgrade() else { break };
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(10)) => {},
                _ = app.presence_wake.notified() => {},
            }
            // Coalesce lifecycle bursts, never poll on token deltas.
            tokio::time::sleep(Duration::from_millis(150)).await;
            let agent_workers = app
                .store
                .board_agents()
                .unwrap_or_default()
                .into_iter()
                .map(|a| a.worker)
                .collect::<HashSet<_>>();
            let workers = app
                .peers
                .lock()
                .unwrap()
                .iter()
                .filter(|(id, _)| agent_workers.contains(*id))
                .map(|(id, p)| (id.clone(), p.generation.clone()))
                .collect::<Vec<_>>();
            futures_util::stream::iter(workers)
                .for_each_concurrent(4, |(worker, generation)| {
                    let app = app.clone();
                    async move {
                        let revision = app
                            .presence
                            .lock()
                            .unwrap()
                            .workers
                            .get(&worker)
                            .map(|w| w.revision);
                        let status = tokio::time::timeout(
                            Duration::from_secs(5),
                            worker_call(
                                app.clone(),
                                worker.clone(),
                                "manager/status".into(),
                                json!({}),
                            ),
                        )
                        .await;
                        let peers = app.peers.lock().unwrap();
                        if peers
                            .get(&worker)
                            .is_none_or(|p| p.generation != generation)
                        {
                            return;
                        }
                        let mut presence = app.presence.lock().unwrap();
                        if presence.workers.get(&worker).map(|w| w.revision) != revision {
                            return;
                        }
                        let status = match status {
                            Ok(Ok(s)) => summary(&s),
                            _ => Value::Null,
                        };
                        presence.workers.insert(
                            worker,
                            Worker {
                                status,
                                seen: Instant::now(),
                                revision: revision.unwrap_or(0),
                            },
                        );
                    }
                })
                .await;
            let connected = app
                .peers
                .lock()
                .unwrap()
                .keys()
                .cloned()
                .collect::<HashSet<_>>();
            app.presence
                .lock()
                .unwrap()
                .workers
                .retain(|id, _| connected.contains(id));
            let _ = app.events.send(json!({"type":"presence_changed"}));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_presence_tracks_lifecycle_and_never_claims_stale_workers_are_ready() {
        let mut presence = Presence::default();
        let event = |method: &str, params: Value| json!({"type":"codex","worker":"w","message":{"method":method,"params":params}});
        presence.observe(&event(
            "manager/connected",
            json!({"alive":true,"active":{},"requests":[]}),
        ));
        let state = |presence: &Presence| {
            agent_state(false, true, false, presence.workers.get("w"), "session")
        };
        assert_eq!(state(&presence), "idle");
        presence.observe(&event(
            "turn/started",
            json!({"threadId":"session","turn":{"id":"new"}}),
        ));
        assert_eq!(state(&presence), "working");
        // A late completion from an older turn must not clear the active one.
        presence.observe(&event(
            "turn/completed",
            json!({"threadId":"session","turn":{"id":"old"}}),
        ));
        assert_eq!(state(&presence), "working");
        assert!(!presence.observe(&event(
            "item/agentMessage/delta",
            json!({"threadId":"session","delta":"text"})
        )));
        presence.observe(&json!({"type":"codex","worker":"w","message":{"id":3,"method":"item/commandExecution/requestApproval","params":{"threadId":"session","command":"private command"}}}));
        assert_eq!(state(&presence), "waiting");
        assert!(
            !presence.workers["w"]
                .status
                .to_string()
                .contains("private command")
        );
        presence.observe(&event("manager/codexExited", json!({})));
        assert_eq!(state(&presence), "error");
        let worker = presence.workers.get_mut("w").unwrap();
        worker.seen = Instant::now() - WORKER_TTL;
        assert_eq!(state(&presence), "unknown");
        assert_eq!(agent_state(false, false, false, None, "session"), "offline");
        assert_eq!(agent_state(true, false, true, None, "session"), "paused");
        assert_eq!(agent_state(false, true, true, None, "session"), "error");
        assert!(summary(&json!({"alive":true})).is_null());
    }

    #[tokio::test]
    async fn expired_human_heartbeats_are_removed_without_waiting_for_a_timer() {
        let dir = crate::test_support::tempdir();
        let store = Store::open(dir.path()).unwrap();
        store
            .set_password(&security::hash_password("presence test password").unwrap())
            .unwrap();
        let app = App::new(store, "http://127.0.0.1").unwrap();
        let token = security::token();
        let key = security::digest(&token);
        app.sessions.lock().unwrap().insert(
            key.clone(),
            Session {
                expires: now() + 60,
                username: "admin".into(),
                version: app.store.user_version("admin").unwrap().unwrap(),
            },
        );
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, format!("am={token}").parse().unwrap());
        app.presence.lock().unwrap().humans.insert(
            (key, "tab".into()),
            Human {
                username: "admin".into(),
                active: true,
                seen: Instant::now() - HUMAN_TTL,
            },
        );
        let result = read(State(app.clone()), headers)
            .await
            .unwrap_or_else(|e| panic!("{}", e.1))
            .0;
        assert!(result["actors"].get("human:admin").is_none());
        assert!(app.presence.lock().unwrap().humans.is_empty());
    }
}
