use super::*;
use axum::routing::get;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rates {
    input: f64,
    cached_input: f64,
    output: f64,
}
impl Rates {
    fn valid(&self) -> bool {
        [self.input, self.cached_input, self.output]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0 && *v <= 1_000_000.0)
    }
}
pub(super) fn routes() -> Router<Arc<App>> {
    Router::new().route("/api/settings/usage-pricing/{agent}", get(read).post(save))
}
fn key(app: &App, agent: &str) -> ApiResult<String> {
    if !app.store.board_agents()?.iter().any(|a| a.id == agent) {
        return Err(rejected(StatusCode::NOT_FOUND, "Agent not found"));
    }
    Ok(format!("usage-pricing:{agent}"))
}
async fn read(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(agent): Path<String>,
) -> ApiResult<Json<Value>> {
    app.authorize(&headers)?;
    let key = key(&app, &agent)?;
    let value: Option<String> = app
        .store
        .0
        .lock()
        .unwrap()
        .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(anyhow::Error::from)?;
    Ok(Json(
        value
            .map(|v| serde_json::from_str::<Value>(&v))
            .transpose()
            .map_err(anyhow::Error::from)?
            .unwrap_or(Value::Null),
    ))
}
async fn save(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(agent): Path<String>,
    Json(rates): Json<Rates>,
) -> ApiResult<Json<Value>> {
    app.authorize(&headers)?;
    app.csrf(&headers)?;
    let key = key(&app, &agent)?;
    if !rates.valid() {
        return Err(rejected(
            StatusCode::BAD_REQUEST,
            "Rates must be finite, non-negative USD amounts per million tokens",
        ));
    }
    let value = json!({"input":rates.input,"cachedInput":rates.cached_input,"output":rates.output,"updatedAt":crate::now()});
    app.store.0.lock().unwrap().execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value.to_string()]).map_err(anyhow::Error::from)?;
    Ok(Json(value))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rates_reject_negative_nonfinite_and_unbounded_values() {
        for value in [-1.0, f64::NAN, f64::INFINITY, 1_000_001.0] {
            assert!(
                !Rates {
                    input: value,
                    cached_input: 0.0,
                    output: 1.0
                }
                .valid()
            );
        }
        assert!(
            Rates {
                input: 0.0,
                cached_input: 0.0,
                output: 1.0
            }
            .valid()
        );
    }
}
