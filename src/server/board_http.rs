use super::*;
use crate::board::{Actor, Agent};
use axum::routing::post;

pub(super) fn routes() -> Router<Arc<App>> {
    let mut routes = Router::new()
        .route("/api/board/openapi.json", get(openapi))
        .route("/api/avatars/{id}", get(avatar))
        .route("/api/attachments/{id}", get(attachment));
    for op in crate::board_api::operations() {
        routes = routes.route(
            &op.path(),
            post(operation)
                .layer::<_, Infallible>(axum::Extension(op))
                .layer::<_, Infallible>(DefaultBodyLimit::max(29 * 1024 * 1024)),
        );
    }
    routes
}
fn domain(error: anyhow::Error) -> ApiError {
    tracing::debug!(%error,"Board request rejected");
    let msg = error.to_string();
    let text = if msg.contains("UNIQUE constraint") {
        "That name or session is already registered".to_owned()
    } else if msg.contains("FOREIGN KEY constraint") {
        "Referenced person, channel or worker does not exist".to_owned()
    } else if msg.contains("sqlite") || msg.contains("database") {
        "Board database operation failed".to_owned()
    } else {
        msg
    };
    ApiError(StatusCode::BAD_REQUEST, text)
}
fn actor(app: &App, headers: &HeaderMap, write: bool) -> ApiResult<Actor> {
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        if headers.contains_key(header::ORIGIN) {
            return Err(rejected(
                StatusCode::FORBIDDEN,
                "Agent credentials cannot be used by browser requests",
            ));
        }
        let token = value
            .to_str()
            .ok()
            .and_then(|s| s.strip_prefix("Bearer "))
            .ok_or_else(|| rejected(StatusCode::UNAUTHORIZED, "Invalid agent credential"))?;
        return app
            .store
            .board_agent_auth(token)?
            .ok_or_else(|| rejected(StatusCode::UNAUTHORIZED, "Invalid agent credential"));
    }
    let username = app.authorize(headers)?;
    if write {
        app.csrf(headers)?;
    }
    app.store.board_human(&username).map_err(domain)
}
fn lock(app: &App, id: &str) -> Arc<tokio::sync::Mutex<()>> {
    app.agent_locks
        .lock()
        .unwrap()
        .entry(id.to_owned())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}
fn capable(app: &App, worker: &str) -> ApiResult<()> {
    let peers = app.peers.lock().unwrap();
    let peer = peers
        .get(worker)
        .ok_or_else(|| rejected(StatusCode::SERVICE_UNAVAILABLE, "Worker is offline"))?;
    if !peer.capabilities.iter().any(|c| c == "board-agents") {
        return Err(rejected(
            StatusCode::BAD_REQUEST,
            "Upgrade this worker to use board agents; standalone sessions still work",
        ));
    }
    Ok(())
}
// Scoped cleanup also revokes bootstrap authorization on cancellation/timeouts.
struct RegistrationCredential {
    app: Arc<App>,
    id: String,
    token_hash: String,
}
impl Drop for RegistrationCredential {
    fn drop(&mut self) {
        if let Err(error) = self
            .app
            .store
            .board_end_registration(&self.id, &self.token_hash)
        {
            tracing::error!(%error,"Cannot revoke registration discovery credential");
        }
    }
}
async fn openapi(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Response> {
    let bootstrap = if let Some(value) = headers.get(header::AUTHORIZATION) {
        if headers.contains_key(header::ORIGIN) {
            return Err(rejected(
                StatusCode::FORBIDDEN,
                "Agent credentials cannot be used by browser requests",
            ));
        }
        let token = value
            .to_str()
            .ok()
            .and_then(|s| s.strip_prefix("Bearer "))
            .unwrap_or("");
        app.store.board_registration_discovery(token)?
    } else {
        false
    };
    let human = if bootstrap {
        false
    } else {
        actor(&app, &headers, false)?.kind == "human"
    };
    let document = crate::board_api::document(human, if app.secure { "__Host-am" } else { "am" });
    let etag = format!("\"{}\"", security::digest(&document.to_string()));
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(etag.as_str())
    {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response());
    }
    Ok(([(header::ETAG, etag)], Json(document)).into_response())
}
async fn operation(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    axum::Extension(endpoint): axum::Extension<crate::board_api::Operation>,
    Json(p): Json<Value>,
) -> ApiResult<Json<Value>> {
    let who = actor(&app, &headers, true)?;
    if endpoint.human_only && who.kind != "human" {
        return Err(rejected(StatusCode::FORBIDDEN, "Human account required"));
    }
    (endpoint.validate)(&p).map_err(domain)?;
    let op = endpoint.name;
    if !p.is_object() {
        return Err(rejected(
            StatusCode::BAD_REQUEST,
            "Request must be an object",
        ));
    }
    let result = match op {
        "list_agents" => {
            // Credentials and private delivery journals are never included.
            let agents = app.store.board_agents()?;
            let peers = app.peers.lock().unwrap();
            json!(
                agents
                    .into_iter()
                    .map(|a| {
                        let online = peers.contains_key(&a.worker);
                        let mut v = json!(a);
                        v["online"] = json!(online);
                        v
                    })
                    .collect::<Vec<_>>()
            )
        }
        "create_agent" => {
            if who.kind != "human" {
                return Err(rejected(
                    StatusCode::FORBIDDEN,
                    "Agent administration requires a human account",
                ));
            }
            if p["persona"].as_str().is_some_and(|s| s.len() > 100000) {
                return Err(rejected(StatusCode::BAD_REQUEST, "Persona is too long"));
            }
            let _creation = lock(&app, "create-agent").lock_owned().await;
            if let Some(request) = p["requestId"].as_str()
                && let Ok(existing) = app.store.board_agent(request)
            {
                if p["handle"].as_str() != Some(existing.handle.as_str())
                    || p["worker"].as_str() != Some(existing.worker.as_str())
                {
                    return Err(rejected(
                        StatusCode::BAD_REQUEST,
                        "Request id reused with different input",
                    ));
                }
                return Ok(Json(json!(existing)));
            }
            // A resumed failed conversion can retain its original worker-side
            // identity even when the reopened dialog has a new request id.
            if let (Some(thread), Some(worker), Some(handle)) = (
                p["thread"].as_str(),
                p["worker"].as_str(),
                p["handle"].as_str(),
            ) && let Some(existing) = app.store.board_agents()?.into_iter().find(|a| {
                a.worker == worker && a.thread == thread && a.handle == handle.trim().to_lowercase()
            }) {
                return Ok(Json(json!(existing)));
            }
            let handle = p["handle"].as_str().unwrap_or("").trim().to_lowercase();
            if !(2..=40).contains(&handle.len())
                || !handle
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
            {
                return Err(rejected(
                    StatusCode::BAD_REQUEST,
                    "Use 2–40 lowercase letters, digits, underscores or hyphens for the handle",
                ));
            }
            app.store.board_human(&who.handle)?;
            let overview = app
                .store
                .board(&who, "overview", &json!({}))
                .map_err(domain)?;
            if overview["people"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["handle"] == handle)
            {
                return Err(rejected(
                    StatusCode::BAD_REQUEST,
                    "Handle is already in use",
                ));
            }
            let name = p["name"]
                .as_str()
                .filter(|s| !s.trim().is_empty() && s.len() <= 100)
                .ok_or_else(|| {
                    rejected(
                        StatusCode::BAD_REQUEST,
                        "Name is required (at most 100 characters)",
                    )
                })?;
            let worker = p["worker"]
                .as_str()
                .ok_or_else(|| rejected(StatusCode::BAD_REQUEST, "Choose a worker"))?
                .to_owned();
            capable(&app, &worker)?;
            if let Some(thread) = p["thread"].as_str()
                && app
                    .store
                    .board_agents()?
                    .iter()
                    .any(|a| a.worker == worker && a.thread == thread)
            {
                return Err(rejected(
                    StatusCode::BAD_REQUEST,
                    "Session is already registered",
                ));
            }
            let id = p["requestId"]
                .as_str()
                .filter(|s| uuid::Uuid::parse_str(s).is_ok())
                .map(str::to_owned)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            if let Ok(a) = app.store.board_agent(&id) {
                return Ok(Json(json!(a)));
            }
            let token = security::token();
            let id = app
                .store
                .board_begin_registration(
                    &id,
                    &worker,
                    p["thread"].as_str().filter(|s| !s.is_empty()),
                    &token,
                )
                .map_err(domain)?;
            let _credential = RegistrationCredential {
                app: app.clone(),
                id: id.clone(),
                token_hash: security::digest(&token),
            };
            let _guard = lock(&app, &id).lock_owned().await;
            let mut config = p.clone();
            config["id"] = json!(id);
            config["token"] = json!(token);
            config["host"] = json!(app.origin);
            let created = worker_call(
                app.clone(),
                worker.clone(),
                "manager/agent/register".into(),
                config,
            )
            .await?;
            let a = Agent {
                id,
                handle,
                name: name.to_owned(),
                worker,
                thread: created["thread"]
                    .as_str()
                    .ok_or_else(|| {
                        rejected(StatusCode::BAD_GATEWAY, "Invalid worker agent response")
                    })?
                    .into(),
                persona: created["persona"].as_str().unwrap_or("").into(),
                settings: created["settings"].clone(),
                paused: false,
                error: None,
            };
            app.store.board_add_agent(&a, &token).map_err(domain)?;
            json!(a)
        }
        "update_agent" => {
            if who.kind != "human" {
                return Err(rejected(
                    StatusCode::FORBIDDEN,
                    "Agent administration requires a human account",
                ));
            }
            let id = p["id"]
                .as_str()
                .ok_or_else(|| rejected(StatusCode::BAD_REQUEST, "Agent id required"))?;
            let _guard = lock(&app, id).lock_owned().await;
            let old = app.store.board_agent(id).map_err(domain)?;
            let name = p["name"].as_str().unwrap_or(&old.name);
            let persona = p["persona"].as_str().unwrap_or(&old.persona);
            let paused = p["paused"].as_bool().unwrap_or(old.paused);
            app.store
                .board_edit_agent(id, name, persona, paused)
                .map_err(domain)?;
            let config = json!({"id":id,"persona":persona,"paused":paused});
            let synchronized = match worker_call(
                app.clone(),
                old.worker.clone(),
                "manager/agent/configure".into(),
                config,
            )
            .await
            {
                Ok(_) => true,
                Err(e) => {
                    app.store.board_agent_error(id, Some(&e.1))?;
                    false
                }
            };
            json!({"agent":app.store.board_agent(id)?,"synchronized":synchronized})
        }
        _ => app.store.board(&who, op, &p).map_err(domain)?,
    };
    (endpoint.validate_response)(&result).map_err(|error| {
        tracing::error!(operation=op,%error,"Invalid board response");
        rejected(StatusCode::INTERNAL_SERVER_ERROR, "Invalid board response")
    })?;
    if !endpoint.readonly {
        app.emit(json!({"type":"board_changed"}));
        app.board_wake.notify_one();
    }
    Ok(Json(result))
}
async fn avatar(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    actor(&app, &headers, false)?;
    let (mime, bytes) = app
        .store
        .board_avatar_image(&id)?
        .ok_or_else(|| rejected(StatusCode::NOT_FOUND, "Avatar not found"))?;
    Ok((
        [
            (header::CONTENT_TYPE, mime.as_str()),
            (header::CACHE_CONTROL, "private, no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response())
}
async fn attachment(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let who = actor(&app, &headers, false)?;
    let (name, mime, bytes) = app.store.board_attachment(&who, &id).map_err(domain)?;
    let inline = matches!(
        mime.as_str(),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    );
    let safe = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || " ._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(if inline {
            &mime
        } else {
            "application/octet-stream"
        })
        .unwrap(),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "{}; filename=\"{}\"",
            if inline { "inline" } else { "attachment" },
            safe
        ))
        .unwrap(),
    );
    Ok(response)
}

pub(super) fn start(app: &Arc<App>) {
    let weak = Arc::downgrade(app);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3));
        let retries = Arc::new(Mutex::new(HashMap::<String, Instant>::new()));
        let permits = Arc::new(tokio::sync::Semaphore::new(8));
        loop {
            let Some(waiting) = weak.upgrade() else { break };
            tokio::select! {_=tick.tick()=>{},_=waiting.board_wake.notified()=>{}};
            drop(waiting);
            let Some(app) = weak.upgrade() else { break };
            match app.store.fire_reminders(now()) {
                Ok(n) if n > 0 => app.emit(json!({"type":"board_changed"})),
                Ok(_) => {}
                Err(error) => tracing::error!(%error,"Cannot deliver due reminders"),
            }
            let agents = match app.store.board_agents() {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!(%e,"Cannot load agents");
                    continue;
                }
            };
            for agent in agents {
                if retries
                    .lock()
                    .unwrap()
                    .get(&agent.id)
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(15))
                {
                    continue;
                }
                if capable(&app, &agent.worker).is_err() {
                    continue;
                }
                let Ok(guard) = lock(&app, &agent.id).try_lock_owned() else {
                    continue;
                };
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    continue;
                };
                let app = app.clone();
                let retries = retries.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let agent = match app.store.board_agent(&agent.id) {
                        Ok(a) => a,
                        Err(_) => return,
                    };
                    let pending = app.store.board_pending(&agent.id).unwrap_or_default();
                    // Also synchronize pause/persona after reconnect, including idle agents.
                    let config = json!({"id":agent.id,"persona":agent.persona,"paused":agent.paused,"settings":agent.settings});
                    let outcome = async {
                        worker_call(
                            app.clone(),
                            agent.worker.clone(),
                            "manager/agent/configure".into(),
                            config,
                        )
                        .await?;
                        if !agent.paused {
                            worker_call(
                                app.clone(),
                                agent.worker.clone(),
                                "manager/agent/deliver".into(),
                                json!({"id":agent.id,"notifications":pending}),
                            )
                            .await?;
                            let ids = pending
                                .iter()
                                .filter_map(|n| n["id"].as_i64())
                                .collect::<Vec<_>>();
                            app.store.board_ack(&agent.id, &ids)?;
                        }
                        Ok::<_, ApiError>(())
                    }
                    .await;
                    match outcome {
                        Ok(()) => {
                            if agent.error.is_some() {
                                let _ = app.store.board_agent_error(&agent.id, None);
                                app.emit(json!({"type":"board_changed"}));
                            }
                        }
                        Err(error) => {
                            retries
                                .lock()
                                .unwrap()
                                .insert(agent.id.clone(), Instant::now());
                            if agent.error.as_deref() != Some(&error.1) {
                                let _ = app.store.board_agent_error(&agent.id, Some(&error.1));
                                app.emit(json!({"type":"board_changed"}));
                            }
                        }
                    }
                    drop(guard);
                });
            }
        }
    });
}
