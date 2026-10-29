pub mod catalog;
pub mod codex;
pub mod security;
pub mod server;
pub mod store;
pub mod updater;
pub mod webhook;
pub mod worker;

#[cfg(test)]
mod test_support;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
// This envelope is deliberately unversioned. Preserve existing field meanings;
// additions must be optional/defaulted so older workers can stay connected.
pub enum Wire {
    Request {
        id: String,
        method: String,
        params: Value,
    },
    Response {
        id: String,
        result: Option<Value>,
        error: Option<String>,
    },
    Event {
        message: Value,
    },
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn tracing_init() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "agentmanager=info".into()),
        )
        .init();
}

pub async fn shutdown_signal() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("Cannot install SIGTERM handler");
    tokio::select! {_=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{}}
}
