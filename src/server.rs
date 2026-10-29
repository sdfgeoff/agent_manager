use crate::{Wire, now, security, store::Store};
use anyhow::Result;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, Path, Request, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{
        Html, IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{delete, get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    convert::Infallible,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_stream::wrappers::BroadcastStream;

type ApiResult<T> = std::result::Result<T, ApiError>;
pub struct ApiError(StatusCode, String);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        tracing::error!(%error,"request failed");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error".into(),
        )
    }
}
fn rejected(status: StatusCode, text: &str) -> ApiError {
    ApiError(status, text.into())
}

#[derive(Clone)]
struct Session {
    expires: i64,
    username: String,
    version: String,
}
struct Peer {
    generation: String,
    sender: mpsc::Sender<Wire>,
    stop: watch::Sender<bool>,
    version: Option<String>,
}
struct Pending {
    worker: String,
    generation: String,
    sender: oneshot::Sender<Wire>,
}

pub struct App {
    pub store: Store,
    origin: String,
    secure: bool,
    dummy_password: String,
    sessions: Mutex<HashMap<String, Session>>,
    peers: Mutex<HashMap<String, Peer>>,
    pending: Mutex<HashMap<String, Pending>>,
    events: broadcast::Sender<Value>,
    attempts: Mutex<VecDeque<Instant>>,
    login_lock: tokio::sync::Semaphore,
    deployment: Option<crate::webhook::Deployment>,
}

impl App {
    pub fn shutdown(&self) {
        self.sessions.lock().unwrap().clear();
        for peer in self.peers.lock().unwrap().values() {
            let _ = peer.stop.send(true);
        }
    }
    pub fn new(store: Store, public_url: &str) -> Result<Arc<Self>> {
        Self::with_deployment(store, public_url, None)
    }
    pub fn with_deployment(
        store: Store,
        public_url: &str,
        deployment: Option<crate::webhook::Deployment>,
    ) -> Result<Arc<Self>> {
        let url = security::origin(public_url)?;
        if store.users()?.is_empty() {
            anyhow::bail!(
                "Create an account first: server --data-dir <DIR> set-password --username <NAME>"
            );
        }
        let dummy_password = security::hash_password(&security::token())?;
        Ok(Arc::new(Self {
            store,
            origin: url.origin().ascii_serialization(),
            secure: url.scheme() == "https",
            dummy_password,
            sessions: Mutex::new(HashMap::new()),
            peers: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            events: broadcast::channel(2048).0,
            attempts: Mutex::new(VecDeque::new()),
            login_lock: tokio::sync::Semaphore::new(2),
            deployment,
        }))
    }
    fn valid_session(&self, session: &Session) -> bool {
        session.expires > now()
            && self
                .store
                .user_version(&session.username)
                .ok()
                .flatten()
                .as_deref()
                == Some(session.version.as_str())
    }
    fn authorize(&self, headers: &HeaderMap) -> ApiResult<String> {
        let key = cookie(headers)
            .ok_or_else(|| rejected(StatusCode::UNAUTHORIZED, "Sign in required"))?;
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|_, s| s.expires > now());
        if let Some(session) = sessions.get(&security::digest(key))
            && self.valid_session(session)
        {
            Ok(session.username.clone())
        } else {
            Err(rejected(StatusCode::UNAUTHORIZED, "Sign in required"))
        }
    }
    fn csrf(&self, headers: &HeaderMap) -> ApiResult<()> {
        if headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(self.origin.as_str()) {
            return Err(rejected(StatusCode::FORBIDDEN, "Origin rejected"));
        }
        if headers.get("x-agentmanager").and_then(|v| v.to_str().ok()) != Some("1") {
            return Err(rejected(StatusCode::FORBIDDEN, "Missing request header"));
        }
        Ok(())
    }
    fn emit(&self, value: Value) {
        let _ = self.events.send(value);
    }
    fn cookie(&self, token: &str, age: u64) -> String {
        format!(
            "{}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={age}{}",
            if self.secure { "__Host-am" } else { "am" },
            if self.secure { "; Secure" } else { "" }
        )
    }
    fn limit_login(&self) -> ApiResult<()> {
        let mut attempts = self.attempts.lock().unwrap();
        attempts.retain(|t| t.elapsed() < Duration::from_secs(60));
        if attempts.len() >= 10 {
            return Err(rejected(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many attempts; wait a minute",
            ));
        }
        attempts.push_back(Instant::now());
        Ok(())
    }
}

fn cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|v| {
            v.trim()
                .strip_prefix("__Host-am=")
                .or_else(|| v.trim().strip_prefix("am="))
        })
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route(
            "/healthz",
            get(|| async { Json(json!({"version":env!("CARGO_PKG_VERSION")})) }),
        )
        .route(
            "/",
            get(|| async { Html(include_str!("../web/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../web/style.css"),
                )
            }),
        )
        .route("/api/login", post(login))
        .route("/api/deploy/webhook", post(deployment_webhook))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/workers", get(workers))
        .route("/api/workers/{id}", delete(revoke))
        .route("/api/workers/{id}/rpc", post(rpc))
        .route("/api/enrollment", post(enrollment))
        .route("/api/enroll", post(enroll))
        .route("/api/worker", get(worker_upgrade))
        .route("/api/events", get(events))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn(security_headers))
        .with_state(app)
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    for (key, value) in [
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cache-control", "no-store"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=()",
        ),
    ] {
        response
            .headers_mut()
            .insert(key, HeaderValue::from_static(value));
    }
    response
}

async fn deployment_webhook(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let deployment = app
        .deployment
        .as_ref()
        .ok_or_else(|| rejected(StatusCode::NOT_FOUND, "Deployment webhooks are not enabled"))?;
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|s| s.to_str().ok())
        .unwrap_or_default();
    if !deployment.verify(signature, &body) {
        return Err(rejected(
            StatusCode::UNAUTHORIZED,
            "Invalid webhook signature",
        ));
    }
    let event = headers.get("x-github-event").and_then(|s| s.to_str().ok());
    if event == Some("ping") {
        return Ok(Json(json!({"ok":true})).into_response());
    }
    let payload: Value = serde_json::from_slice(&body)
        .map_err(|_| rejected(StatusCode::BAD_REQUEST, "Invalid webhook payload"))?;
    if payload["repository"]["full_name"].as_str() != Some(&deployment.repository) {
        return Err(rejected(
            StatusCode::FORBIDDEN,
            "Unexpected release repository",
        ));
    }
    if event != Some("release")
        || payload["action"] != "published"
        || payload["release"]["draft"] == true
        || payload["release"]["prerelease"] == true
    {
        return Ok(Json(json!({"ignored":true})).into_response());
    }
    if crate::updater::version(payload["release"]["tag_name"].as_str().unwrap_or_default()).is_err()
    {
        return Err(rejected(
            StatusCode::BAD_REQUEST,
            "Expected a stable version tag",
        ));
    }
    match deployment.notify.try_send(()) {
        Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {
            Ok((StatusCode::ACCEPTED, Json(json!({"queued":true}))).into_response())
        }
        Err(_) => Err(rejected(
            StatusCode::SERVICE_UNAVAILABLE,
            "Deployment listener is unavailable",
        )),
    }
}

#[derive(Deserialize)]
struct Login {
    username: String,
    password: String,
}
async fn login(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<Login>,
) -> ApiResult<Response> {
    app.csrf(&headers)?;
    app.limit_login()?;
    let _permit = app
        .login_lock
        .try_acquire()
        .map_err(|_| rejected(StatusCode::TOO_MANY_REQUESTS, "Sign in busy; try again"))?;
    let user = match security::username(&body.username) {
        Ok(username) => app.store.user(&username)?,
        Err(_) => None,
    };
    let hash = user
        .as_ref()
        .map(|u| u.password_hash.clone())
        .unwrap_or_else(|| app.dummy_password.clone());
    let valid =
        tokio::task::spawn_blocking(move || security::verify_password(&body.password, &hash))
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
    if !valid || user.is_none() {
        return Err(rejected(
            StatusCode::UNAUTHORIZED,
            "Incorrect username or password",
        ));
    }
    let user = user.unwrap();
    if app.store.user_version(&user.username)?.as_deref() != Some(&user.version) {
        return Err(rejected(
            StatusCode::UNAUTHORIZED,
            "Incorrect username or password",
        ));
    }
    let token = security::token();
    {
        let mut sessions = app.sessions.lock().unwrap();
        sessions.retain(|_, s| s.expires > now());
        if sessions.len() >= 100 {
            sessions.clear();
        }
        sessions.insert(
            security::digest(&token),
            Session {
                expires: now() + 604800,
                username: user.username.clone(),
                version: user.version,
            },
        );
    }
    Ok((
        [(header::SET_COOKIE, app.cookie(&token, 604800))],
        Json(json!({"ok":true,"username":user.username})),
    )
        .into_response())
}
async fn logout(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Response> {
    app.csrf(&headers)?;
    if let Some(key) = cookie(&headers) {
        app.sessions.lock().unwrap().remove(&security::digest(key));
    }
    Ok((
        [(header::SET_COOKIE, app.cookie("", 0))],
        Json(json!({"ok":true})),
    )
        .into_response())
}
async fn me(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let username = app.authorize(&headers)?;
    Ok(Json(
        json!({"ok":true,"origin":app.origin,"username":username}),
    ))
}
async fn workers(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    app.authorize(&headers)?;
    let records = app.store.workers()?;
    let peers = app.peers.lock().unwrap();
    Ok(Json(json!(records.into_iter().map(|r|{
        let peer=peers.get(&r.id);
        let version=peer.map(|p|p.version.clone()).unwrap_or(r.version);
        json!({"id":r.id,"name":r.name,"online":peer.is_some(),"createdAt":r.created_at,"workerVersion":version})
    }).collect::<Vec<_>>())))
}
async fn enrollment(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    app.authorize(&headers)?;
    app.csrf(&headers)?;
    Ok(Json(
        json!({"key":app.store.enrollment()?,"expiresIn":600,"host":app.origin}),
    ))
}
#[derive(Deserialize)]
struct Enrollment {
    key: String,
    name: String,
}
async fn enroll(
    State(app): State<Arc<App>>,
    Json(body): Json<Enrollment>,
) -> ApiResult<Json<Value>> {
    app.limit_login()?;
    let (id, credential) = app.store.enroll(&body.key, &body.name).map_err(|_| {
        rejected(
            StatusCode::UNAUTHORIZED,
            "Invalid or expired registration key",
        )
    })?;
    app.emit(json!({"type":"workers_changed"}));
    Ok(Json(json!({"id":id,"credential":credential})))
}
async fn revoke(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    app.authorize(&headers)?;
    app.csrf(&headers)?;
    app.store.revoke(&id)?;
    if let Some(peer) = app.peers.lock().unwrap().remove(&id) {
        let _ = peer.stop.send(true);
    }
    app.emit(json!({"type":"workers_changed"}));
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
struct Rpc {
    method: String,
    #[serde(default = "empty_object")]
    params: Value,
}
fn empty_object() -> Value {
    json!({})
}
async fn rpc(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(worker): Path<String>,
    Json(body): Json<Rpc>,
) -> ApiResult<Json<Value>> {
    app.authorize(&headers)?;
    app.csrf(&headers)?;
    if body.method.len() > 100 {
        return Err(rejected(StatusCode::BAD_REQUEST, "Invalid method"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    {
        let peers = app.peers.lock().unwrap();
        let peer = peers
            .get(&worker)
            .ok_or_else(|| rejected(StatusCode::SERVICE_UNAVAILABLE, "Worker is offline"))?;
        let mut pending = app.pending.lock().unwrap();
        if pending.len() >= 256 {
            return Err(rejected(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many pending requests",
            ));
        }
        pending.insert(
            id.clone(),
            Pending {
                worker: worker.clone(),
                generation: peer.generation.clone(),
                sender: tx,
            },
        );
        if peer
            .sender
            .try_send(Wire::Request {
                id: id.clone(),
                method: body.method,
                params: body.params,
            })
            .is_err()
        {
            pending.remove(&id);
            return Err(rejected(StatusCode::SERVICE_UNAVAILABLE, "Worker is busy"));
        }
    }
    // Drop cleanup also handles a disconnected HTTP client.
    let _cleanup = PendingGuard {
        app: app.clone(),
        id: id.clone(),
    };
    let reply = tokio::time::timeout(Duration::from_secs(90), rx)
        .await
        .map_err(|_| {
            rejected(
                StatusCode::GATEWAY_TIMEOUT,
                "Worker response timed out; check session before retrying",
            )
        })?
        .map_err(|_| {
            rejected(
                StatusCode::SERVICE_UNAVAILABLE,
                "Worker disconnected; check session before retrying",
            )
        })?;
    match reply {
        Wire::Response { result, error, .. } => {
            if let Some(error) = error {
                Err(ApiError(StatusCode::BAD_REQUEST, error))
            } else {
                Ok(Json(result.unwrap_or(Value::Null)))
            }
        }
        _ => Err(rejected(StatusCode::BAD_GATEWAY, "Invalid worker response")),
    }
}
struct PendingGuard {
    app: Arc<App>,
    id: String,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.app.pending.lock().unwrap().remove(&self.id);
    }
}

async fn events(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Response> {
    app.authorize(&headers)?;
    let key = security::digest(cookie(&headers).unwrap());
    let state = app.clone();
    // Periodic auth checks close an already-open stream after logout or expiry.
    let heartbeat =
        tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(Duration::from_secs(15)))
            .map(|_| None);
    let updates = BroadcastStream::new(app.events.subscribe())
        .map(|item| Some(item.unwrap_or_else(|_| json!({"type":"resync"}))));
    let stream = futures_util::stream::select(heartbeat, updates)
        .take_while(move |_| {
            let valid = state
                .sessions
                .lock()
                .unwrap()
                .get(&key)
                .is_some_and(|s| state.valid_session(s));
            std::future::ready(valid)
        })
        .map(|value| {
            Ok::<_, Infallible>(
                Event::default().data(
                    value
                        .unwrap_or_else(|| json!({"type":"heartbeat"}))
                        .to_string(),
                ),
            )
        });
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

async fn worker_upgrade(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    // Browser cookies never authenticate workers; browser origins are rejected.
    if headers.contains_key(header::ORIGIN) {
        return Err(rejected(StatusCode::FORBIDDEN, "Worker endpoint only"));
    }
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| rejected(StatusCode::UNAUTHORIZED, "Worker credential required"))?;
    let id = app
        .store
        .worker_for_token(token)?
        .ok_or_else(|| rejected(StatusCode::UNAUTHORIZED, "Invalid worker credential"))?;
    Ok(ws
        .max_message_size(16 * 1024 * 1024)
        .max_frame_size(16 * 1024 * 1024)
        .on_upgrade(move |socket| worker_socket(app, id, socket)))
}
async fn worker_socket(app: Arc<App>, id: String, socket: WebSocket) {
    let generation = uuid::Uuid::new_v4().to_string();
    let (tx, mut rx) = mpsc::channel(128);
    let (stop, mut stopped) = watch::channel(false);
    {
        let mut peers = app.peers.lock().unwrap();
        // Revocation may have happened between the HTTP handshake and this upgrade.
        if !app.store.worker_exists(&id).unwrap_or(false) {
            return;
        }
        if let Some(old) = peers.insert(
            id.clone(),
            Peer {
                generation: generation.clone(),
                sender: tx,
                stop,
                version: None,
            },
        ) {
            let _ = old.stop.send(true);
        }
    }
    app.emit(json!({"type":"workers_changed"}));
    tracing::info!(worker=%id,"worker connected");
    let (mut sink, mut source) = socket.split();
    loop {
        tokio::select! {
            _=stopped.changed()=>break,
            Some(message)=rx.recv()=>{
                if !matches!(tokio::time::timeout(Duration::from_secs(20),sink.send(Message::Text(serde_json::to_string(&message).unwrap().into()))).await,Ok(Ok(()))) {break;}
            }
            incoming=tokio::time::timeout(Duration::from_secs(65),source.next())=>{
                match incoming {
                    Ok(Some(Ok(Message::Text(text))))=>{
                        if let Ok(message)=serde_json::from_str::<Wire>(&text) {
                            match message {
                                Wire::Response {id:ref request_id,..}=>{
                                    let mut pending=app.pending.lock().unwrap();
                                    if pending.get(request_id).is_some_and(|p|p.worker==id && p.generation==generation)
                                        && let Some(p)=pending.remove(request_id) {let _=p.sender.send(message);
                                    }
                                }
                                Wire::Event {message}=>{
                                    if message["method"]=="manager/connected" {
                                        let version=message["params"]["workerVersion"].as_str().filter(|v|!v.is_empty() && v.len()<=128 && v.bytes().all(|c|c.is_ascii_alphanumeric() || b".+-_".contains(&c)));
                                        if let Some(version)=version {
                                            let mut peers=app.peers.lock().unwrap();
                                            if let Some(peer)=peers.get_mut(&id) && peer.generation==generation {
                                                if let Err(error)=app.store.set_worker_version(&id,version) {tracing::warn!(%error,"Could not persist worker version");}
                                                peer.version=Some(version.into());
                                                app.emit(json!({"type":"workers_changed"}));
                                            }
                                        }
                                    }
                                    app.emit(json!({"type":"codex","worker":id,"message":message}));
                                }
                                _=>{}
                            }
                        }
                    }
                    Ok(Some(Ok(Message::Ping(data))))=>{if sink.send(Message::Pong(data)).await.is_err() {break;}}
                    Ok(Some(Ok(Message::Pong(_))))=>{},
                    _=>break,
                }
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), sink.close()).await;
    {
        let mut peers = app.peers.lock().unwrap();
        if peers.get(&id).is_some_and(|p| p.generation == generation) {
            peers.remove(&id);
        }
    }
    app.pending
        .lock()
        .unwrap()
        .retain(|_, p| p.generation != generation);
    app.emit(json!({"type":"workers_changed"}));
    tracing::info!(worker=%id,"worker disconnected");
}
