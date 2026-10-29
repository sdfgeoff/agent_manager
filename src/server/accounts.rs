use super::*;
use axum::routing::{get, post};

pub(super) fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/settings/users", get(users).post(create_user))
        .route("/api/settings/users/{username}", delete(remove_user))
        .route("/api/settings/password", post(password))
}
async fn users(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    app.authorize(&headers)?;
    Ok(Json(json!(app.store.users()?)))
}
#[derive(Deserialize)]
struct NewUser {
    username: String,
    password: String,
}
async fn hash(app: &App, password: String) -> ApiResult<String> {
    app.limit_login()?;
    let _permit = app.login_lock.try_acquire().map_err(|_| {
        rejected(
            StatusCode::TOO_MANY_REQUESTS,
            "Password operations are busy; try again",
        )
    })?;
    tokio::task::spawn_blocking(move || security::hash_password(&password))
        .await
        .map_err(|_| rejected(StatusCode::INTERNAL_SERVER_ERROR, "Password hashing failed"))?
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))
}
async fn create_user(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<NewUser>,
) -> ApiResult<Json<Value>> {
    app.authorize(&headers)?;
    app.csrf(&headers)?;
    let username = security::username(&body.username)
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    let password_hash = hash(&app, body.password).await?;
    app.store
        .create_user_password(&username, &password_hash)
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    app.emit(json!({"type":"board_changed"}));
    Ok(Json(json!({"username":username})))
}
async fn remove_user(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> ApiResult<Json<Value>> {
    let me = app.authorize(&headers)?;
    app.csrf(&headers)?;
    app.store
        .remove_user(&username)
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    app.emit(json!({"type":"board_changed"}));
    Ok(Json(json!({"ok":true,"signedOut":me==username})))
}
#[derive(Deserialize)]
struct Password {
    current_password: String,
    new_password: String,
}
async fn password(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<Password>,
) -> ApiResult<Response> {
    let username = app.authorize(&headers)?;
    app.csrf(&headers)?;
    app.limit_login()?;
    let _permit = app.login_lock.try_acquire().map_err(|_| {
        rejected(
            StatusCode::TOO_MANY_REQUESTS,
            "Password operations are busy; try again",
        )
    })?;
    let user = app
        .store
        .user(&username)?
        .ok_or_else(|| rejected(StatusCode::UNAUTHORIZED, "Sign in required"))?;
    let hash = user.password_hash;
    let new_hash = tokio::task::spawn_blocking(move || {
        if !security::verify_password(&body.current_password, &hash) {
            anyhow::bail!("Current password is incorrect");
        }
        security::hash_password(&body.new_password)
    })
    .await
    .map_err(|_| {
        rejected(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Password operation failed",
        )
    })?
    .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    let version = app
        .store
        .change_user_password(&username, &user.version, &new_hash)
        .map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
    if let Some(key) = cookie(&headers) {
        app.sessions.lock().unwrap().remove(&security::digest(key));
    }
    let token = security::token();
    app.sessions.lock().unwrap().insert(
        security::digest(&token),
        Session {
            expires: now() + 604800,
            username,
            version,
        },
    );
    Ok((
        [(header::SET_COOKIE, app.cookie(&token, 604800))],
        Json(json!({"ok":true,"otherSessionsSignedOut":true})),
    )
        .into_response())
}
