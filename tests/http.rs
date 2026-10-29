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
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store
            .set_password(&security::hash_password("test password long enough").unwrap())
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = App::new(store, &url).unwrap();
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
        let reply = self
            .client
            .post(format!("{}/api/login", self.url))
            .header("Origin", &self.url)
            .header("X-Agentmanager", "1")
            .json(&json!({"password":"test password long enough"}))
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
        .json(&json!({"password":"test password long enough"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let wrong = server
        .client
        .post(format!("{}/api/login", server.url))
        .header("Origin", &server.url)
        .header("X-Agentmanager", "1")
        .json(&json!({"password":"wrong"}))
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
