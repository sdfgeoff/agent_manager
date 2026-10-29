#[path = "../src/test_support.rs"]
mod test_support;
use agentmanager::{
    Wire, security,
    server::{App, router},
    store::Store,
};
use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

struct Server {
    url: String,
    client: Client,
    app: Arc<App>,
    _dir: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn new() -> Self {
        Self::with_deployment(None).await
    }
    async fn with_deployment(deployment: Option<agentmanager::webhook::Deployment>) -> Self {
        let dir = test_support::tempdir();
        let store = Store::open(dir.path()).unwrap();
        store
            .set_password(&security::hash_password("test password long enough").unwrap())
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = App::with_deployment(store, &url, deployment).unwrap();
        let routes = router(app.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, routes).await.unwrap();
        });
        Self {
            url,
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            app,
            _dir: dir,
            task,
        }
    }
    async fn login(&self) -> String {
        self.login_as("admin", "test password long enough").await
    }
    async fn login_as(&self, username: &str, password: &str) -> String {
        let reply = self
            .client
            .post(format!("{}/api/login", self.url))
            .header("Origin", &self.url)
            .header("X-Agentmanager", "1")
            .json(&json!({"username":username,"password":password}))
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), StatusCode::OK);
        reply.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .into()
    }
    async fn worker(&self) -> (String, String) {
        let key = self.app.store.enrollment().unwrap();
        self.app.store.enroll(&key, "test worker").unwrap()
    }
}

#[tokio::test]
async fn separate_accounts_share_workers_and_password_resets_only_revoke_that_user() {
    let server = Server::new().await;
    let management = Store::open(server._dir.path()).unwrap();
    management
        .set_user_password(
            "alice",
            &security::hash_password("alice test password").unwrap(),
        )
        .unwrap();
    let admin = server.login().await;
    let alice = server.login_as(" ALICE ", "alice test password").await;
    let (worker, _) = server.worker().await;
    for cookie in [&admin, &alice] {
        let response = server
            .client
            .get(format!("{}/api/workers", server.url))
            .header("Cookie", cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.json::<Value>().await.unwrap()[0]["id"], worker);
    }
    let me = server
        .client
        .get(format!("{}/api/me", server.url))
        .header("Cookie", &alice)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(me["username"], "alice");
    management
        .set_user_password(
            "alice",
            &security::hash_password("replacement password").unwrap(),
        )
        .unwrap();
    assert_eq!(
        server
            .client
            .get(format!("{}/api/me", server.url))
            .header("Cookie", &alice)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        server
            .client
            .get(format!("{}/api/me", server.url))
            .header("Cookie", &admin)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let new_alice = server.login_as("alice", "replacement password").await;
    server.app.store.remove_user("alice").unwrap();
    assert_eq!(
        server
            .client
            .get(format!("{}/api/workers", server.url))
            .header("Cookie", new_alice)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(server.app.store.worker_exists(&worker).unwrap());
}

#[tokio::test]
async fn deployment_only_accepts_signed_stable_release_events_for_this_repository() {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let secret = b"test-only-webhook-secret-long-enough";
    let (deployment, mut notifications) =
        agentmanager::webhook::Deployment::new("sdfgeoff/agent_manager".into(), secret.to_vec())
            .unwrap();
    let server = Server::with_deployment(Some(deployment)).await;
    let payload = json!({"action":"published","repository":{"full_name":"sdfgeoff/agent_manager"},"release":{"tag_name":"v0.2.0","draft":false,"prerelease":false}});
    let sign = |body: &[u8]| {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    };
    let body = serde_json::to_vec(&payload).unwrap();
    let endpoint = format!("{}/api/deploy/webhook", server.url);
    let unsigned = server
        .client
        .post(&endpoint)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(unsigned.status(), StatusCode::UNAUTHORIZED);
    let signature = sign(&body);
    let mut other = payload.clone();
    other["repository"]["full_name"] = json!("someone/else");
    let other = serde_json::to_vec(&other).unwrap();
    let forged = server
        .client
        .post(&endpoint)
        .header("X-GitHub-Event", "release")
        .header("X-Hub-Signature-256", &signature)
        .body(other.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status(), StatusCode::UNAUTHORIZED);
    let wrong_repo = server
        .client
        .post(&endpoint)
        .header("X-GitHub-Event", "release")
        .header("X-Hub-Signature-256", sign(&other))
        .body(other)
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_repo.status(), StatusCode::FORBIDDEN);
    assert!(notifications.try_recv().is_err());
    let valid = server
        .client
        .post(&endpoint)
        .header("X-GitHub-Event", "release")
        .header("X-Hub-Signature-256", signature)
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(valid.status(), StatusCode::ACCEPTED);
    notifications.try_recv().unwrap();
    let body = serde_json::to_vec(&json!({"zen":"test ping"})).unwrap();
    assert_eq!(
        server
            .client
            .post(endpoint)
            .header("X-GitHub-Event", "ping")
            .header("X-Hub-Signature-256", sign(&body))
            .body(body)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(notifications.try_recv().is_err());
}

#[tokio::test]
async fn invalid_logins_do_not_reveal_whether_a_username_exists() {
    let server = Server::new().await;
    let mut errors = Vec::new();
    for username in ["admin", "unknown", "bad name"] {
        let response = server
            .client
            .post(format!("{}/api/login", server.url))
            .header("Origin", &server.url)
            .header("X-Agentmanager", "1")
            .json(&json!({"username":username,"password":"an incorrect password"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        errors.push(response.json::<Value>().await.unwrap());
    }
    assert!(errors.windows(2).all(|e| e[0] == e[1]));
}

#[tokio::test]
async fn browser_authentication_csrf_logout_and_headers() {
    let server = Server::new().await;
    let public = server.client.get(&server.url).send().await.unwrap();
    assert_eq!(public.status(), StatusCode::OK);
    assert!(
        public.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    assert_eq!(
        server
            .client
            .get(format!("{}/api/workers", server.url))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let denied = server
        .client
        .post(format!("{}/api/login", server.url))
        .header("Origin", "https://attacker.example")
        .header("X-Agentmanager", "1")
        .json(&json!({"username":"admin","password":"test password long enough"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let wrong = server
        .client
        .post(format!("{}/api/login", server.url))
        .header("Origin", &server.url)
        .header("X-Agentmanager", "1")
        .json(&json!({"username":"admin","password":"wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    let cookie = server.login().await;
    let enrollment = server
        .client
        .post(format!("{}/api/enrollment", server.url))
        .header("Cookie", &cookie)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(enrollment.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        server
            .client
            .get(format!("{}/api/workers", server.url))
            .header("Cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let logout = server
        .client
        .post(format!("{}/api/logout", server.url))
        .header("Cookie", &cookie)
        .header("Origin", &server.url)
        .header("X-Agentmanager", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::OK);
    assert_eq!(
        server
            .client
            .get(format!("{}/api/workers", server.url))
            .header("Cookie", cookie)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn registration_cannot_be_replayed_and_browser_cookie_cannot_authenticate_a_worker() {
    let server = Server::new().await;
    let cookie = server.login().await;
    let reply = server
        .client
        .post(format!("{}/api/enrollment", server.url))
        .header("Cookie", &cookie)
        .header("Origin", &server.url)
        .header("X-Agentmanager", "1")
        .send()
        .await
        .unwrap();
    let key = reply.json::<Value>().await.unwrap()["key"]
        .as_str()
        .unwrap()
        .to_owned();
    let payload = json!({"key":key,"name":"laptop"});
    assert_eq!(
        server
            .client
            .post(format!("{}/api/enroll", server.url))
            .json(&payload)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        server
            .client
            .post(format!("{}/api/enroll", server.url))
            .json(&payload)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let ws = server.url.replacen("http:", "ws:", 1) + "/api/worker";
    let mut request = ws.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("Cookie", cookie.parse().unwrap());
    assert!(connect_async(request).await.is_err());
}

#[tokio::test]
async fn newer_server_accepts_original_worker_packets_without_a_version_handshake() {
    let server = Server::new().await;
    let cookie = server.login().await;
    let (worker, credential) = server.worker().await;
    let mut request = (server.url.replacen("http:", "ws:", 1) + "/api/worker")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {credential}").parse().unwrap(),
    );
    let (mut socket, _) = connect_async(request).await.unwrap();
    // Frozen packet shapes, intentionally independent of the current Wire enum.
    // Legacy workers do not send a protocol version or capabilities handshake.
    socket.send(Message::Text(r#"{"type":"event","message":{"method":"manager/connected","params":{"alive":true,"active":{},"requests":[]}}}"#.into())).await.unwrap();
    for (method, legacy_result) in [
        (
            "manager/status",
            json!({"alive":true,"active":{},"requests":[]}),
        ),
        ("manager/list", json!({"data":[],"nextCursor":null})),
    ] {
        let request = server
            .client
            .post(format!("{}/api/workers/{worker}/rpc", server.url))
            .header("Cookie", &cookie)
            .header("Origin", &server.url)
            .header("X-Agentmanager", "1")
            .json(&json!({"method":method,"params":{}}));
        let http = tokio::spawn(async move { request.send().await.unwrap() });
        let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let packet: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        assert_eq!(packet["type"], "request");
        assert_eq!(packet["method"], method);
        assert_eq!(packet["params"], json!({}));
        // Omitted optional error and additional envelope fields are both accepted.
        let reply = json!({"type":"response","id":packet["id"],"result":legacy_result,"extra_metadata":"ignored"});
        socket
            .send(Message::Text(reply.to_string().into()))
            .await
            .unwrap();
        let response = http.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.json::<Value>().await.unwrap(), legacy_result);
    }
    socket.close(None).await.unwrap();
}

#[tokio::test]
async fn rpc_routes_to_worker_and_revocation_disconnects_it() {
    let server = Server::new().await;
    let cookie = server.login().await;
    let (worker, credential) = server.worker().await;
    let ws = server.url.replacen("http:", "ws:", 1) + "/api/worker";
    let mut request = ws.clone().into_client_request().unwrap();
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {credential}").parse().unwrap(),
    );
    let (mut socket, _) = connect_async(request).await.unwrap();
    let request = server
        .client
        .post(format!("{}/api/workers/{worker}/rpc", server.url))
        .header("Cookie", &cookie)
        .header("Origin", &server.url)
        .header("X-Agentmanager", "1")
        .json(&json!({"method":"manager/status","params":{}}));
    let http = tokio::spawn(async move { request.send().await.unwrap() });
    let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Wire::Request { id, method, .. } =
        serde_json::from_str(message.to_text().unwrap()).unwrap()
    else {
        panic!("expected worker request")
    };
    assert_eq!(method, "manager/status");
    socket
        .send(Message::Text(
            serde_json::to_string(&Wire::Response {
                id,
                result: Some(json!({"alive":true})),
                error: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let response = http.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.json::<Value>().await.unwrap()["alive"], true);
    let revoke = server
        .client
        .delete(format!("{}/api/workers/{worker}", server.url))
        .header("Cookie", &cookie)
        .header("Origin", &server.url)
        .header("X-Agentmanager", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(revoke.status(), StatusCode::OK);
    let closed = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap();
    assert!(matches!(closed, None | Some(Ok(Message::Close(_)))));
    let mut request = ws.into_client_request().unwrap();
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {credential}").parse().unwrap(),
    );
    assert!(connect_async(request).await.is_err());
}

#[tokio::test]
async fn global_authentication_rate_limit_is_enforced() {
    let server = Server::new().await;
    for index in 0..11 {
        let response = server
            .client
            .post(format!("{}/api/enroll", server.url))
            .json(&json!({"key":"invalid","name":"test"}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if index < 10 {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::TOO_MANY_REQUESTS
            }
        );
    }
}
