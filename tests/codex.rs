#[path = "../src/test_support.rs"]
mod test_support;
use agentmanager::{codex::Codex, worker};
use serde_json::json;
use std::{os::unix::fs::PermissionsExt, time::Duration};

#[tokio::test]
async fn discovery_spans_cli_and_desktop_folders_and_keeps_empty_managed_sessions() {
    let dir = test_support::tempdir();
    let binary = dir.path().join("codex-test");
    std::fs::write(&binary, include_str!("fixtures/codex.py")).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let saved = json!({"cli-old":{"id":"cli-old","name":"CLI project","source":"cli","locked":true,"cwd":"/projects/one","preview":"Earlier work","updatedAt":20,"turns":[{"id":"old-turn","items":[]}]},"desktop-old":{"id":"desktop-old","name":"Desktop project","source":"vscode","cwd":"/unrelated/project","preview":"Other work","updatedAt":10,"turns":[{"id":"desktop-turn","items":[]}]}});
    std::fs::write(
        binary.with_extension("state.json"),
        serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    let codex = Codex::spawn(binary.to_str().unwrap()).await.unwrap();
    let empty = codex
        .call("thread/start", json!({"cwd":dir.path()}))
        .await
        .unwrap();
    let empty_id = empty["thread"]["id"].as_str().unwrap();
    let first = worker::dispatch(
        codex.clone(),
        "manager/list",
        json!({"limit":1,"cwd":"/projects/one"}),
    )
    .await
    .unwrap();
    assert!(
        first["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == empty_id)
    );
    assert!(
        first["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "cli-old")
    );
    let second = worker::dispatch(
        codex.clone(),
        "manager/list",
        json!({"limit":1,"cursor":first["nextCursor"]}),
    )
    .await
    .unwrap();
    assert_eq!(second["data"][0]["cwd"], "/unrelated/project");
    assert!(second["nextCursor"].is_null());
    assert_eq!(
        codex
            .call("thread/read", json!({"threadId":"cli-old"}))
            .await
            .unwrap()["thread"]["name"],
        "CLI project"
    );
    assert_eq!(
        codex
            .call(
                "thread/turns/list",
                json!({"threadId":"cli-old","itemsView":"full"})
            )
            .await
            .unwrap()["data"][0]["id"],
        "old-turn"
    );
    assert!(
        codex
            .call(
                "thread/resume",
                json!({"threadId":"cli-old","excludeTurns":true})
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("active writer")
    );
    assert_eq!(
        codex
            .call(
                "thread/resume",
                json!({"threadId":"desktop-old","excludeTurns":true})
            )
            .await
            .unwrap()["thread"]["id"],
        "desktop-old"
    );
}

#[tokio::test]
async fn approvals_survive_without_subscribers_and_can_only_be_answered_once() {
    let dir = test_support::tempdir();
    let binary = dir.path().join("codex-test");
    std::fs::write(&binary, include_str!("fixtures/codex.py")).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let codex = Codex::spawn(binary.to_str().unwrap()).await.unwrap();
    let models = codex.call("model/list", json!({})).await.unwrap();
    assert_eq!(models["data"][0]["model"], "test-model");
    assert_eq!(
        worker::dispatch(codex.clone(), "manager/status", json!({}))
            .await
            .unwrap()["workerVersion"],
        env!("CARGO_PKG_VERSION")
    );
    codex
        .call(
            "turn/start",
            json!({"threadId":"thread-1","input":[{"type":"text","text":"test"}]}),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while codex.status()["requests"].as_array().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(codex.status()["active"]["thread-1"], "turn-1");
    let a = codex.respond(json!({"requestId":100,"result":{"decision":"accept"}}));
    let b = codex.respond(json!({"requestId":100,"result":{"decision":"decline"}}));
    let (a, b) = tokio::join!(a, b);
    assert!(a.is_ok() ^ b.is_ok());
    tokio::time::timeout(Duration::from_secs(3), async {
        while !codex.status()["active"].as_object().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(codex.status()["requests"].as_array().unwrap().is_empty());
    assert!(
        worker::dispatch(
            codex.clone(),
            "command/exec",
            json!({"command":"touch /not-executed"})
        )
        .await
        .is_err()
    );
    let folder = dir.path().join("nested/new-folder");
    worker::dispatch(codex.clone(), "fs/mkdir", json!({"path":folder}))
        .await
        .unwrap();
    let listing = worker::dispatch(codex, "fs/list", json!({"path":folder.parent()}))
        .await
        .unwrap();
    assert_eq!(listing["entries"][0]["name"], "new-folder");
}

#[tokio::test]
#[ignore = "requires an installed, configured Codex CLI; read-only, no inference"]
async fn installed_codex_handshake_and_models() {
    let codex = Codex::spawn("codex").await.unwrap();
    let models = codex.call("model/list", json!({})).await.unwrap();
    assert!(models["data"].is_array());
}

#[tokio::test]
#[ignore = "requires installed Codex; exercises session lifecycle in a temporary Codex home, no inference"]
async fn installed_codex_session_lifecycle() {
    let dir = test_support::tempdir();
    let binary = dir.path().join("isolated-codex");
    std::fs::write(&binary,"#!/usr/bin/env python3\nimport os,sys\nos.environ['CODEX_HOME']=sys.argv[0]+'.home'\nos.makedirs(os.environ['CODEX_HOME'],exist_ok=True)\nos.execvp('codex',['codex',*sys.argv[1:]])\n").unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let codex = Codex::spawn(binary.to_str().unwrap()).await.unwrap();
    let started=codex.call("thread/start",json!({"cwd":dir.path(),"sandbox":"read-only","approvalPolicy":"on-request","config":{"model_reasoning_effort":"medium"}})).await.unwrap();
    let id = started["thread"]["id"].as_str().unwrap();
    codex
        .call(
            "thread/name/set",
            json!({"threadId":id,"name":"Lifecycle smoke test"}),
        )
        .await
        .unwrap();
    let resumed = codex
        .call("thread/resume", json!({"threadId":id,"excludeTurns":true}))
        .await
        .unwrap();
    assert_eq!(resumed["thread"]["name"], "Lifecycle smoke test");
    let list = codex
        .list_sessions(json!({"cwd":"/some-other-folder"}))
        .await
        .unwrap();
    assert!(
        list["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == id)
    );
    let history = codex
        .call(
            "thread/turns/list",
            json!({"threadId":id,"itemsView":"full","sortDirection":"desc","limit":20}),
        )
        .await
        .unwrap();
    assert!(history["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn agent_persona_delivery_pause_and_journal_survive_worker_restart() {
    let dir = test_support::tempdir();
    let binary = dir.path().join("codex-test");
    std::fs::write(&binary, include_str!("fixtures/codex.py")).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let catalog = dir.path().join("sessions.json");
    let codex = Codex::spawn_managed(binary.to_str().unwrap(), Some(catalog.clone()))
        .await
        .unwrap();
    let existing=codex.call("thread/start",json!({"cwd":dir.path(),"developerInstructions":"<persona>Remember conversations across channels.</persona>"})).await.unwrap();
    let thread = existing["thread"]["id"].as_str().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let token = agentmanager::security::token();
    let registered = worker::dispatch(
        codex.clone(),
        "manager/agent/register",
        json!({"id":id,"thread":thread,"host":"http://127.0.0.1:8080","token":token}),
    )
    .await
    .unwrap();
    assert_eq!(registered["thread"], thread);
    assert_eq!(
        registered["persona"],
        "<persona>Remember conversations across channels.</persona>"
    );
    let batch = json!({"id":id,"notifications":[{"id":1,"event":{"topic":1,"body":"First"}},{"id":2,"event":{"topic":2,"body":"Second"}}]});
    worker::dispatch(codex.clone(), "manager/agent/deliver", batch.clone())
        .await
        .unwrap();
    worker::dispatch(codex.clone(), "manager/agent/deliver", batch.clone())
        .await
        .unwrap();
    let calls = || {
        std::fs::read_to_string(binary.with_extension("calls.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        calls()
            .iter()
            .filter(|c| c["method"] == "turn/start")
            .count(),
        1
    );
    worker::dispatch(
        codex.clone(),
        "manager/agent/configure",
        json!({"id":id,"persona":"Edited persona","paused":false}),
    )
    .await
    .unwrap();
    worker::dispatch(
        codex.clone(),
        "manager/agent/deliver",
        json!({"id":id,"notifications":[{"id":3,"event":{"topic":1,"body":"Latest advice"}}]}),
    )
    .await
    .unwrap();
    let steer = calls()
        .into_iter()
        .find(|c| c["method"] == "turn/steer")
        .unwrap();
    assert!(
        steer["params"]["input"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Latest advice")
    );
    assert!(!steer.to_string().contains("Edited persona"));
    worker::dispatch(
        codex.clone(),
        "manager/agent/configure",
        json!({"id":id,"paused":true}),
    )
    .await
    .unwrap();
    assert!(
        worker::dispatch(
            codex.clone(),
            "manager/agent/deliver",
            json!({"id":id,"notifications":[{"id":4}]})
        )
        .await
        .is_err()
    );
    assert!(
        worker::dispatch(
            codex.clone(),
            "turn/start",
            json!({"threadId":thread,"input":[]})
        )
        .await
        .is_err()
    );
    tokio::time::sleep(Duration::from_millis(30)).await;
    let reopened = Codex::spawn_managed(binary.to_str().unwrap(), Some(catalog))
        .await
        .unwrap();
    assert!(reopened.agents.paused(thread));
    worker::dispatch(
        reopened.clone(),
        "manager/agent/configure",
        json!({"id":id,"paused":false}),
    )
    .await
    .unwrap();
    worker::dispatch(
        reopened.clone(),
        "manager/agent/deliver",
        json!({"id":id,"notifications":[{"id":1},{"id":2},{"id":3},{"id":4}]}),
    )
    .await
    .unwrap();
    let starts = calls()
        .into_iter()
        .filter(|c| c["method"] == "turn/start")
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 2);
    let text = starts[1]["params"]["input"][0]["text"].as_str().unwrap();
    assert!(text.contains("interrupted"));
    assert!(text.contains("\"id\":4"));
    assert!(!text.contains("\"id\":1"));
    let resumed = calls()
        .into_iter()
        .rfind(|c| c["method"] == "thread/resume")
        .unwrap();
    assert_eq!(resumed["params"]["developerInstructions"], "Edited persona");
    assert_eq!(resumed["params"]["threadId"], thread);
    assert!(!registered.to_string().contains(&token));
}

#[tokio::test]
async fn missing_agent_session_is_an_error_and_never_allocates_a_replacement() {
    let dir = test_support::tempdir();
    let binary = dir.path().join("codex-test");
    std::fs::write(&binary, include_str!("fixtures/codex.py")).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let catalog = dir.path().join("sessions.json");
    let codex = Codex::spawn_managed(binary.to_str().unwrap(), Some(catalog.clone()))
        .await
        .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let result=worker::dispatch(codex,"manager/agent/register",json!({"id":id,"cwd":dir.path(),"host":"http://127.0.0.1:8080","token":"test-token","persona":"Sage"})).await.unwrap();
    let data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(catalog.with_extension("agents.json")).unwrap())
            .unwrap();
    assert_eq!(data[&id]["thread"], result["thread"]);
    // A fresh runtime with its original binding but no rollout must fail to resume.
    std::fs::write(binary.with_extension("state.json"), "{}").unwrap();
    let reopened = Codex::spawn_managed(binary.to_str().unwrap(), Some(catalog))
        .await
        .unwrap();
    assert!(
        worker::dispatch(
            reopened,
            "manager/agent/deliver",
            json!({"id":id,"notifications":[{"id":10}]})
        )
        .await
        .is_err()
    );
    let calls = std::fs::read_to_string(binary.with_extension("calls.jsonl")).unwrap();
    assert_eq!(
        calls
            .lines()
            .filter(
                |l| serde_json::from_str::<serde_json::Value>(l).unwrap()["method"]
                    == "thread/start"
            )
            .count(),
        1
    );
}

#[tokio::test]
#[ignore = "requires installed Codex; isolated home, MCP discovery and instruction import only, no inference"]
async fn installed_codex_agent_configuration() {
    let dir = test_support::tempdir();
    let binary = dir.path().join("isolated-codex");
    std::fs::write(&binary,"#!/usr/bin/env python3\nimport os,sys\nos.environ['CODEX_HOME']=sys.argv[0]+'.home'\nos.makedirs(os.environ['CODEX_HOME'],exist_ok=True)\nos.execvp('codex',['codex',*sys.argv[1:]])\n").unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let codex = Codex::spawn(binary.to_str().unwrap()).await.unwrap();
    let persona = "<persona>Remember the shared board.</persona>";
    let started = codex
        .call(
            "thread/start",
            json!({"cwd":dir.path(),"developerInstructions":persona,"sandbox":"read-only"}),
        )
        .await
        .unwrap();
    let id = started["thread"]["id"].as_str().unwrap();
    codex.call("thread/inject_items",json!({"threadId":id,"items":[{"type":"message","role":"user","content":[{"type":"input_text","text":"Context-only test; no inference or response requested."}]}]})).await.unwrap();
    // Use an empty catalogue to exercise import from the actual rollout format.
    let reader = Codex::spawn(binary.to_str().unwrap()).await.unwrap();
    let imported =
        agentmanager::agent_runtime::session_instructions(&reader, id, &started["thread"]).unwrap();
    assert_eq!(imported, persona);
    let credential = dir.path().join("mcp.json");
    agentmanager::security::private_write(
        &credential,
        &serde_json::to_vec(
            &json!({"host":"http://127.0.0.1:9","token":"isolated-test-token","cwd":dir.path()}),
        )
        .unwrap(),
    )
    .unwrap();
    codex
        .call("thread/unsubscribe", json!({"threadId":id}))
        .await
        .unwrap();
    let resumed=codex.call("thread/resume",json!({"threadId":id,"excludeTurns":true,"developerInstructions":"Updated persona","config":{"mcp_servers":{"agentmanager_board":{"command":env!("CARGO_BIN_EXE_worker"),"args":["mcp","--credential-file",credential],"required":true}}}})).await.unwrap();
    assert_eq!(resumed["thread"]["id"], id);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let status = codex
            .call("mcpServerStatus/list", json!({"threadId":id}))
            .await
            .unwrap();
        if let Some(server) = status["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "agentmanager_board")
            && server["tools"].as_object().is_some_and(|t| {
                t.contains_key("create_reminder")
                    && t.contains_key("search")
                    && t.contains_key("post_reply")
            })
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Board MCP tools unavailable: {status}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Context-only empty threads are retained by the creating worker's catalogue;
    // Codex intentionally omits them from discovery by another process.
    let search = worker::dispatch(
        codex,
        "manager/search",
        json!({"query":"Context-only test"}),
    )
    .await
    .unwrap();
    assert!(
        search["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["thread"] == id),
        "{search}"
    );
}

#[tokio::test]
async fn session_search_reads_all_folders_without_resuming_or_starting_a_turn() {
    let dir = test_support::tempdir();
    let binary = dir.path().join("codex-search");
    std::fs::write(&binary, include_str!("fixtures/codex.py")).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let saved = json!({
        "old-cli":{"id":"old-cli","name":"Earlier CLI","source":"cli","cwd":"/projects/one","preview":"Old project","updatedAt":20,"turns":[{"id":"t1","items":[{"id":"u1","type":"userMessage","content":[{"type":"text","text":"copper reactor design"}]},{"id":"a1","type":"agentMessage","text":"Cooling uses copper pipes"}]}]},
        "desktop":{"id":"desktop","archived":true,"name":"Desktop conversation","source":"vscode","cwd":"/unrelated/root","preview":"Other work","updatedAt":10,"turns":[{"id":"t2","items":[{"id":"a2","type":"agentMessage","text":"A copper valve"}]}]}
    });
    std::fs::write(
        binary.with_extension("state.json"),
        serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    let codex = Codex::spawn(binary.to_str().unwrap()).await.unwrap();
    let result = worker::dispatch(codex.clone(), "manager/search", json!({"query":"copper"}))
        .await
        .unwrap();
    assert_eq!(result["data"].as_array().unwrap().len(), 3, "{result}");
    assert!(
        result["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["cwd"] == "/unrelated/root")
    );
    let calls = std::fs::read_to_string(binary.with_extension("calls.jsonl")).unwrap();
    assert!(!calls.contains("thread/resume") && !calls.contains("turn/start"));
    assert!(
        worker::dispatch(codex, "manager/search", json!({"query":"x"}))
            .await
            .is_err()
    );
}
