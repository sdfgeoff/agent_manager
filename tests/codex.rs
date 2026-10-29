use agentmanager::{codex::Codex, worker};
use serde_json::json;
use std::{os::unix::fs::PermissionsExt, time::Duration};

#[tokio::test]
async fn approvals_survive_without_subscribers_and_can_only_be_answered_once() {
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("codex-test");
    std::fs::write(&binary, include_str!("fixtures/codex.py")).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let codex = Codex::spawn(binary.to_str().unwrap()).await.unwrap();
    let models = codex.call("model/list", json!({})).await.unwrap();
    assert_eq!(models["data"][0]["model"], "test-model");
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
            json!({"command":"touch /tmp/should-not-run"})
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
    let dir = tempfile::tempdir().unwrap();
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
    let history = codex
        .call(
            "thread/turns/list",
            json!({"threadId":id,"itemsView":"full","sortDirection":"desc","limit":20}),
        )
        .await
        .unwrap();
    assert!(history["data"].as_array().unwrap().is_empty());
}
