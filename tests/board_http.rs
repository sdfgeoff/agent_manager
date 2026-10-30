#[path = "../src/test_support.rs"]
mod test_support;
use agentmanager::{
    board::Agent,
    security,
    server::{App, router},
    store::Store,
};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Duration};
struct Fixture {
    dir: tempfile::TempDir,
    url: String,
    client: Client,
    cookie: String,
    app: Arc<App>,
    task: tokio::task::JoinHandle<()>,
    worker: Option<std::process::Child>,
    worker_id: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(w) = &mut self.worker {
            let _ = w.kill();
            let _ = w.wait();
        }
        self.app.shutdown();
        self.task.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let dir = test_support::tempdir();
        let store = Store::open(&dir.path().join("server")).unwrap();
        store
            .set_password(&security::hash_password("board test password").unwrap())
            .unwrap();
        let key = store.enrollment().unwrap();
        let (worker_id, _) = store.enroll(&key, "unused").unwrap();
        store.revoke(&worker_id).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = App::new(store, &url).unwrap();
        let routes = router(app.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, routes).await.unwrap();
        });
        let client = Client::builder()
            .timeout(Duration::from_secs(8))
            .build()
            .unwrap();
        let r = client
            .post(format!("{url}/api/login"))
            .header("Origin", &url)
            .header("X-Agentmanager", "1")
            .json(&json!({"username":"admin","password":"board test password"}))
            .send()
            .await
            .unwrap();
        let cookie = r.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        Self {
            dir,
            url,
            client,
            cookie,
            app,
            task,
            worker: None,
            worker_id: String::new(),
        }
    }
    async fn op(&self, op: &str, p: Value) -> Value {
        let r = self
            .client
            .post(format!("{}/api/board/{op}", self.url))
            .header("Cookie", &self.cookie)
            .header("Origin", &self.url)
            .header("X-Agentmanager", "1")
            .json(&p)
            .send()
            .await
            .unwrap();
        let status = r.status();
        let value = r.json::<Value>().await.unwrap();
        assert_eq!(status, StatusCode::OK, "{op}: {value}");
        value
    }
    async fn start_worker(&mut self) {
        let binary = self.dir.path().join("codex-test");
        std::fs::write(&binary, include_str!("fixtures/codex.py")).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let credential = self.dir.path().join("worker.json");
        let key = if credential.exists() {
            None
        } else {
            Some(self.app.store.enrollment().unwrap())
        };
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_worker"));
        cmd.args([
            "--host",
            &self.url,
            "--credential-file",
            credential.to_str().unwrap(),
            "--codex",
            binary.to_str().unwrap(),
            "--name",
            "board-test-worker",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
        if let Some(key) = key {
            cmd.env("AGENTMANAGER_KEY", key);
        }
        self.worker = Some(cmd.spawn().unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        loop {
            if let Ok(bytes) = std::fs::read(&credential) {
                let v: Value = serde_json::from_slice(&bytes).unwrap();
                self.worker_id = v["id"].as_str().unwrap().into();
                let list = self
                    .client
                    .get(format!("{}/api/workers", self.url))
                    .header("Cookie", &self.cookie)
                    .send()
                    .await
                    .unwrap()
                    .json::<Value>()
                    .await
                    .unwrap();
                if list
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|w| w["id"] == self.worker_id && w["workerVersion"].is_string())
                {
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "worker did not connect"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
    fn calls(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("codex-test.calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}
#[tokio::test]
async fn authenticated_board_and_agent_credentials_are_separate_from_host_administration() {
    let f = Fixture::new().await;
    let human = f.app.store.board_human("admin").unwrap();
    let key = f.app.store.enrollment().unwrap();
    let (worker, _) = f.app.store.enroll(&key, "worker").unwrap();
    let token = security::token();
    let agent = Agent {
        id: uuid::Uuid::new_v4().to_string(),
        handle: "sage".into(),
        name: "Sage".into(),
        worker,
        thread: "existing".into(),
        persona: "Private config".into(),
        settings: json!({}),
        paused: false,
        error: None,
    };
    f.app.store.board_add_agent(&agent, &token).unwrap();
    let url = format!("{}/api/board/create_topic", f.url);
    assert_eq!(
        f.client
            .post(&url)
            .json(&json!({"channel":1,"body":"No auth"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.client
            .post(&url)
            .header("Cookie", &f.cookie)
            .json(&json!({"channel":1,"body":"No CSRF"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let published = f
        .client
        .post(&url)
        .bearer_auth(&token)
        .json(&json!({"channel":1,"body":"Agent contribution"}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(published["author"], agent.id);
    assert_eq!(
        f.client
            .post(&url)
            .bearer_auth(&token)
            .header("Origin", &f.url)
            .json(&json!({"channel":1,"body":"Browser bearer"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    for op in ["create_agent", "update_agent"] {
        assert_eq!(
            f.client
                .post(format!("{}/api/board/{op}", f.url))
                .bearer_auth(&token)
                .json(&json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        f.client
            .get(format!("{}/api/workers", f.url))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let file = f
        .app
        .store
        .board(
            &human,
            "upload_attachment",
            &json!({"name":"picture.html","mime":"text/html","data":"PGgxPkhlbGxvPC9oMT4="}),
        )
        .unwrap();
    let url = format!("{}/api/attachments/{}", f.url, file["id"].as_str().unwrap());
    assert_eq!(
        f.client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    f.op(
        "post_reply",
        json!({"topic":published["id"],"body":"Evidence","attachments":[file["id"]]}),
    )
    .await;
    let r = f.client.get(&url).bearer_auth(&token).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["content-type"], "application/octet-stream");
    assert!(
        r.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment;")
    );
    assert_eq!(
        f.client.get(&url).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn offline_notifications_batch_steer_pause_and_explicit_publication_work_end_to_end() {
    let mut f = Fixture::new().await;
    f.start_worker().await;
    let request = uuid::Uuid::new_v4().to_string();
    let p = json!({"requestId":request,"handle":"sage","name":"Sage","worker":f.worker_id,"cwd":f.dir.path(),"persona":"Remember across channels","settings":{"sandbox":"read-only"}});
    let a = f.op("create_agent", p.clone()).await;
    assert_eq!(f.op("create_agent", p).await["id"], a["id"]);
    let id = a["id"].as_str().unwrap();
    f.op(
        "set_subscription",
        json!({"actor":id,"kind":"channel","target":1,"subscribed":true}),
    )
    .await;
    f.op("update_agent", json!({"id":id,"paused":true})).await;
    let topic = f
        .op(
            "create_topic",
            json!({"channel":1,"body":"First notification"}),
        )
        .await;
    f.op(
        "post_reply",
        json!({"topic":topic["id"],"body":"@sage second notification"}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(f.calls().iter().all(|c| c["method"] != "turn/start"));
    assert_eq!(f.app.store.board_pending(id).unwrap().len(), 2);
    f.op("update_agent", json!({"id":id,"paused":false})).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(7);
    while f.app.store.board_pending(id).unwrap().len() == 2 {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let starts = f
        .calls()
        .into_iter()
        .filter(|c| c["method"] == "turn/start")
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 1);
    let text = starts[0]["params"]["input"][0]["text"].as_str().unwrap();
    assert!(text.contains("First notification") && text.contains("second notification"));
    assert!(!text.contains("Remember across channels"));
    f.op(
        "post_reply",
        json!({"topic":topic["id"],"body":"@sage latest advice"}),
    )
    .await;
    let deadline = std::time::Instant::now() + Duration::from_secs(7);
    while !f.calls().iter().any(|c| c["method"] == "turn/steer") {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        f.op("read_topic", json!({"topic":topic["id"]})).await["replies"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "No automatic Codex replies should be published"
    );
    f.op("update_agent", json!({"id":id,"paused":true})).await;
    let mut w = f.worker.take().unwrap();
    w.kill().unwrap();
    w.wait().unwrap();
    f.start_worker().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        f.calls()
            .iter()
            .filter(|c| c["method"] == "thread/start")
            .count(),
        1,
        "Restart must not allocate a new session"
    );
    let credential = f
        .dir
        .path()
        .join("worker.sessions.agents.agent-credentials")
        .join(format!("{id}.json"));
    assert!(credential.exists());
    // Real stdio MCP transport, using only its private test credential and the shared HTTP API.
    use std::io::{BufRead, Write};
    let mut mcp = std::process::Command::new(env!("CARGO_BIN_EXE_worker"))
        .args(["mcp", "--credential-file", credential.to_str().unwrap()])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = mcp.stdin.take().unwrap();
    let output = mcp.stdout.take().unwrap();
    let url = f.url.clone();
    let response=tokio::task::spawn_blocking(move||{writeln!(stdin,"{}",json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"post_reply","arguments":{"topic":topic["id"],"body":"Explicit MCP post"}}})).unwrap();stdin.flush().unwrap();let mut line=String::new();std::io::BufReader::new(output).read_line(&mut line).unwrap();line}).await.unwrap();
    assert!(response.contains("\"isError\":false"), "{url}: {response}");
    mcp.kill().unwrap();
    mcp.wait().unwrap();
}

#[tokio::test]
async fn scheduled_reminders_use_private_mcp_api_timer_and_paused_agent_delivery() {
    let mut f = Fixture::new().await;
    f.start_worker().await;
    let a=f.op("create_agent",json!({"handle":"reminder-agent","name":"Reminder agent","worker":f.worker_id,"cwd":f.dir.path(),"persona":"Private persona","settings":{}})).await;
    let id = a["id"].as_str().unwrap();
    f.op("update_agent", json!({"id":id,"paused":true})).await;
    let credential = f
        .dir
        .path()
        .join("worker.sessions.agents.agent-credentials")
        .join(format!("{id}.json"));
    let due = agentmanager::now() + 2;
    // Use the actual MCP stdio adapter, not a direct database insert.
    use std::io::{BufRead, Write};
    let mut mcp = std::process::Command::new(env!("CARGO_BIN_EXE_worker"))
        .args(["mcp", "--credential-file", credential.to_str().unwrap()])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = mcp.stdin.take().unwrap();
    let output = mcp.stdout.take().unwrap();
    let response=tokio::task::spawn_blocking(move||{
        writeln!(input,"{}",json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_reminder","arguments":{"message":"Private timed follow-up @admin","dueAt":due}}})).unwrap();
        input.flush().unwrap();let mut line=String::new();std::io::BufReader::new(output).read_line(&mut line).unwrap();line
    }).await.unwrap();
    mcp.kill().unwrap();
    mcp.wait().unwrap();
    assert!(response.contains("\"isError\":false"), "{response}");
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while f.app.store.board_pending(id).unwrap().is_empty() {
        assert!(std::time::Instant::now() < deadline, "Timer did not fire");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(f.calls().iter().all(|c| c["method"] != "turn/start"));
    let human = f.app.store.board_human("admin").unwrap();
    assert!(
        f.app.store.board_pending(&human.id).unwrap().is_empty(),
        "Reminder mentions must not notify others"
    );
    let activity = f.op("agent_activity", json!({})).await;
    assert_eq!(activity["agents"][0]["queued"], 1);
    assert_eq!(activity["agents"][0]["paused"], true);
    let pending = f.app.store.board_pending(id).unwrap();
    let agent = f.app.store.board_agent(id).unwrap();
    let actor = agentmanager::board::Actor {
        id: agent.id,
        handle: agent.handle,
        name: agent.name,
        kind: "agent".into(),
    };
    f.app
        .store
        .board(&actor, "mark_read", &json!({"id":pending[0]["id"]}))
        .unwrap();
    assert_eq!(
        f.app.store.board_pending(id).unwrap().len(),
        1,
        "Read state must not acknowledge delivery"
    );
    f.op("update_agent", json!({"id":id,"paused":false})).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(7);
    while !f.app.store.board_pending(id).unwrap().is_empty() {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let starts = f
        .calls()
        .into_iter()
        .filter(|c| c["method"] == "turn/start")
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 1);
    let input = starts[0]["params"]["input"][0]["text"].as_str().unwrap();
    assert!(input.contains("Private timed follow-up") && !input.contains("Private persona"));
    assert_eq!(f.op("list_topics", json!({})).await["data"], json!([]));
}
