//! Native Codex goals stay authoritative; only agent-pause restoration is journaled here.
use crate::codex::Codex;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Serialize, Deserialize)]
pub struct Identity {
    fingerprint: String,
}
impl Identity {
    pub fn from_goal(goal: &Value) -> Self {
        Self {
            fingerprint: crate::security::digest(
                &serde_json::json!([goal["objective"], goal["createdAt"], goal["tokenBudget"]])
                    .to_string(),
            ),
        }
    }
    pub fn matches(&self, goal: &Value) -> bool {
        goal["createdAt"].is_i64() && self.fingerprint == Self::from_goal(goal).fingerprint
    }
}
pub fn unsupported(error: &anyhow::Error) -> bool {
    let text = error.to_string().to_lowercase();
    text.contains("-32601")
        || text.contains("unsupported method")
        || text.contains("not supported")
        || text.contains("unknown test method")
        || (text.contains("goal") && (text.contains("disabled") || text.contains("not enabled")))
}
pub async fn read_optional(codex: &Arc<Codex>, thread: &str) -> Result<Option<Value>> {
    match codex
        .call("thread/goal/get", serde_json::json!({"threadId":thread}))
        .await
    {
        Ok(value) => Ok(value.get("goal").filter(|g| !g.is_null()).cloned()),
        Err(error) if unsupported(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn validate_change(params: &Value) -> Result<()> {
    if let Some(value) = params.get("objective").filter(|v| !v.is_null()) {
        ensure!(
            value
                .as_str()
                .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 4000),
            "Goal objective must contain 1–4000 characters"
        );
    }
    if let Some(value) = params.get("tokenBudget").filter(|v| !v.is_null()) {
        ensure!(
            value.as_i64().is_some_and(|v| v > 0),
            "Goal token budget must be a positive integer or null"
        );
    }
    if let Some(value) = params.get("status").filter(|v| !v.is_null()) {
        ensure!(
            matches!(
                value.as_str(),
                Some(
                    "active" | "paused" | "blocked" | "usageLimited" | "budgetLimited" | "complete"
                )
            ),
            "Invalid goal status"
        );
    }
    Ok(())
}
