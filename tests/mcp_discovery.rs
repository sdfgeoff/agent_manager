#[path = "../src/test_support.rs"]
mod test_support;
use agentmanager::{board_api, security};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
struct Api {
    future: AtomicBool,
    revoked: AtomicBool,
    cached: AtomicUsize,
    calls: Mutex<Vec<Value>>,
}
fn auth(state: &Api, headers: &HeaderMap) -> bool {
    !state.revoked.load(Ordering::SeqCst)
        && headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            == Some("Bearer isolated-discovery-token")
}
async fn document(State(s): State<Arc<Api>>, headers: HeaderMap) -> Response {
    if !auth(&s, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut doc = board_api::document(false, "am");
    // The alias demonstrates that the bridge chooses its adapter from metadata,
    // not from a hardcoded tool name.
    doc["paths"]["/api/board/upload_attachment"]["post"]["operationId"] =
        json!("send_workspace_file");
    doc["paths"]["/api/board/read_attachment"]["post"]["operationId"] =
        json!("fetch_workspace_file");
    if s.future.load(Ordering::SeqCst) {
        #[derive(schemars::JsonSchema)]
        #[allow(dead_code)]
        struct FutureArguments {
            new_flag: bool,
        }
        doc["paths"]["/api/board/future_endpoint"] = json!({"post":{
            "operationId":"server_future_tool","description":"Only this server knows the new tool",
            "requestBody":{"content":{"application/json":{"schema":schemars::schema_for!(FutureArguments)}}},
            "x-mcp":{}
        }});
    }
    let etag = format!("\"{}\"", security::digest(&doc.to_string()));
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(etag.as_str())
    {
        s.cached.fetch_add(1, Ordering::SeqCst);
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response();
    }
    ([(header::ETAG, etag)], Json(doc)).into_response()
}
async fn future(State(s): State<Arc<Api>>, headers: HeaderMap, Json(v): Json<Value>) -> Response {
    if !auth(&s, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Credential revoked"})),
        )
            .into_response();
    }
    s.calls.lock().unwrap().push(v.clone());
    Json(json!({"newServerResult":v["new_flag"],"requestId":v["requestId"]})).into_response()
}
async fn upload(State(s): State<Arc<Api>>, headers: HeaderMap, Json(v): Json<Value>) -> Response {
    assert!(auth(&s, &headers));
    let data = STANDARD.decode(v["data"].as_str().unwrap()).unwrap();
    s.calls.lock().unwrap().push(v.clone());
    Json(json!({"id":"file","name":v["name"],"mime":v["mime"],"size":data.len()})).into_response()
}
async fn download(State(s): State<Arc<Api>>, headers: HeaderMap, Json(v): Json<Value>) -> Response {
    assert!(auth(&s, &headers));
    assert!(
        v.get("path").is_none(),
        "Local destination paths must not be sent to the server"
    );
    s.calls.lock().unwrap().push(v);
    let data = include_bytes!("../web/public/icon-192.png");
    Json(json!({"id":"image","name":"picture.png","mime":"image/png","size":data.len(),"data":STANDARD.encode(data)})).into_response()
}
struct Fixture {
    dir: tempfile::TempDir,
    state: Arc<Api>,
    task: tokio::task::JoinHandle<()>,
    child: tokio::process::Child,
    input: tokio::process::ChildStdin,
    output: Lines<BufReader<tokio::process::ChildStdout>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        self.task.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let dir = test_support::tempdir();
        std::fs::create_dir(dir.path().join("workspace")).unwrap();
        let state = Arc::new(Api {
            future: AtomicBool::new(false),
            revoked: AtomicBool::new(false),
            cached: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
        });
        let routes = Router::new()
            .route("/api/board/openapi.json", get(document))
            .route("/api/board/future_endpoint", post(future))
            .route("/api/board/upload_attachment", post(upload))
            .route("/api/board/read_attachment", post(download))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
        let credential = dir.path().join("mcp.json");
        security::private_write(&credential,&serde_json::to_vec(&json!({"host":host,"token":"isolated-discovery-token","cwd":dir.path().join("workspace")})).unwrap()).unwrap();
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_worker"))
            .args(["mcp", "--credential-file", credential.to_str().unwrap()])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap()).lines();
        Self {
            dir,
            state,
            task,
            child,
            input,
            output,
        }
    }
    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.input
            .write_all(
                format!(
                    "{}\n",
                    json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        self.input.flush().await.unwrap();
        let line = tokio::time::timeout(Duration::from_secs(10), self.output.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }
    async fn call(&mut self, name: &str, args: Value) -> Value {
        self.request("tools/call", json!({"name":name,"arguments":args}))
            .await
    }
}
#[tokio::test]
async fn same_mcp_binary_discovers_new_server_tools_and_forwards_new_arguments() {
    let mut f = Fixture::new().await;
    let init = f.request("initialize", json!({})).await;
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("shared board")
    );
    let tools = f.request("tools/list", json!({})).await;
    assert!(
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["name"] != "server_future_tool")
    );
    assert!(
        f.state.cached.load(Ordering::SeqCst) > 0,
        "Subsequent discovery should use ETags"
    );
    f.state.future.store(true, Ordering::SeqCst);
    let tools = f.request("tools/list", json!({})).await;
    let tool = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "server_future_tool")
        .unwrap();
    assert_eq!(
        tool["inputSchema"]["properties"]["new_flag"]["type"],
        "boolean"
    );
    let reply = f.call("server_future_tool", json!({"new_flag":true})).await;
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let result: Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(result["newServerResult"], true);
    assert!(uuid::Uuid::parse_str(result["requestId"].as_str().unwrap()).is_ok());
    f.state.revoked.store(true, Ordering::SeqCst);
    assert_eq!(
        f.request("tools/list", json!({})).await["error"]["code"],
        -32000
    );
    assert_eq!(
        f.call("server_future_tool", json!({"new_flag":false}))
            .await["result"]["isError"],
        true
    );
    assert_eq!(f.state.calls.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn discovered_attachment_adapters_keep_workspace_checks_images_and_no_overwrites() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let mut f = Fixture::new().await;
    let workspace = f.dir.path().join("workspace");
    std::fs::write(workspace.join("note.txt"), "Workspace fixture").unwrap();
    let outside = f.dir.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "Do not upload this").unwrap();
    symlink(outside.join("secret.txt"), workspace.join("escape.txt")).unwrap();
    symlink(&outside, workspace.join("escape-dir")).unwrap();
    let up = f
        .call(
            "send_workspace_file",
            json!({"path":"note.txt","mime":"text/plain"}),
        )
        .await;
    assert_eq!(up["result"]["isError"], false, "{up}");
    assert_eq!(f.state.calls.lock().unwrap()[0]["name"], "note.txt");
    let before = f.state.calls.lock().unwrap().len();
    for path in [workspace.join("escape.txt"), outside.join("secret.txt")] {
        assert_eq!(
            f.call("send_workspace_file", json!({"path":path})).await["result"]["isError"],
            true
        );
    }
    assert_eq!(f.state.calls.lock().unwrap().len(), before);
    let read = f
        .call(
            "fetch_workspace_file",
            json!({"id":"image","path":"picture.png"}),
        )
        .await;
    assert_eq!(read["result"]["isError"], false, "{read}");
    assert_eq!(read["result"]["content"][1]["type"], "image");
    assert_eq!(
        std::fs::read(workspace.join("picture.png")).unwrap(),
        include_bytes!("../web/public/icon-192.png")
    );
    assert_eq!(
        std::fs::metadata(workspace.join("picture.png"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        f.call(
            "fetch_workspace_file",
            json!({"id":"image","path":"picture.png"})
        )
        .await["result"]["isError"],
        true
    );
    assert_eq!(
        f.call(
            "fetch_workspace_file",
            json!({"id":"image","path":"escape-dir/escaped.png"})
        )
        .await["result"]["isError"],
        true
    );
    assert!(!outside.join("escaped.png").exists());
}
