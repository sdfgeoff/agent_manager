use super::*;
use axum::routing::{get, post};
pub(super) fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/push", get(config).post(subscribe))
        .route("/api/push/status", post(status))
        .route("/api/push/unsubscribe", post(unsubscribe))
}
async fn config(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    app.authorize(&headers)?;
    Ok(Json(json!({"publicKey":app.push.public_key})))
}
async fn subscribe(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(data): Json<crate::push::Subscription>,
) -> ApiResult<Json<Value>> {
    let username = app.authorize(&headers)?;
    app.csrf(&headers)?;
    app.store
        .push_subscribe(
            &username,
            &security::digest(cookie(&headers).unwrap()),
            &data,
        )
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct Endpoint {
    endpoint: String,
}
async fn status(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(data): Json<Endpoint>,
) -> ApiResult<Json<Value>> {
    let username = app.authorize(&headers)?;
    app.csrf(&headers)?;
    Ok(Json(app.store.push_status(&username, &data.endpoint)?))
}
async fn unsubscribe(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(data): Json<Endpoint>,
) -> ApiResult<Json<Value>> {
    let username = app.authorize(&headers)?;
    app.csrf(&headers)?;
    app.store.push_unsubscribe(&username, &data.endpoint)?;
    Ok(Json(json!({"ok":true})))
}
pub(super) fn start(app: &Arc<App>) {
    let weak = Arc::downgrade(app);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3));
        let running = Arc::new(Mutex::new(std::collections::HashSet::new()));
        let permits = Arc::new(tokio::sync::Semaphore::new(8));
        loop {
            interval.tick().await;
            let Some(app) = weak.upgrade() else { break };
            let pending = match app.store.push_pending() {
                Ok(v) => v,
                Err(error) => {
                    tracing::error!(%error,"Cannot read pending browser notifications");
                    continue;
                }
            };
            for p in pending {
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    break;
                };
                if !running.lock().unwrap().insert(p.id.clone()) {
                    continue;
                }
                let app = app.clone();
                let running = running.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let status = app.push.send(&p).await.ok();
                    if let Err(error) = app.store.push_result(&p, status) {
                        tracing::error!(%error,"Cannot record browser notification delivery");
                    }
                    running.lock().unwrap().remove(&p.id);
                });
            }
        }
    });
}
