use crate::{now, security};
use anyhow::Result;
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf, sync::Mutex};

/// Worker-owned session metadata. Codex hides empty threads from thread/list,
/// so the UI must retain the ids returned by thread/start independently.
pub struct Catalog {
    path: Option<PathBuf>,
    threads: Mutex<HashMap<String, Value>>,
}
impl Catalog {
    pub fn open(path: Option<PathBuf>) -> Result<Self> {
        let threads = match path.as_ref() {
            Some(path) if path.exists() => serde_json::from_slice(&std::fs::read(path)?)?,
            _ => HashMap::new(),
        };
        Ok(Self {
            path,
            threads: Mutex::new(threads),
        })
    }
    pub fn record(&self, method: &str, params: &Value, result: &Value) -> Result<()> {
        let mut threads = self.threads.lock().unwrap();
        if let Some(id) = result["thread"]["id"].as_str() {
            let mut metadata = result["thread"].clone();
            if let Some(object) = metadata.as_object_mut() {
                object.remove("turns");
            }
            // A start response may omit a name which was previously set.
            if metadata["name"].is_null()
                && let Some(previous) = threads.get(id)
            {
                metadata["name"] = previous["name"].clone();
            }
            threads.insert(id.into(), metadata);
        } else if let Some(id) = params["threadId"].as_str()
            && let Some(thread) = threads.get_mut(id)
        {
            match method {
                "thread/name/set" | "thread/name/updated" => {
                    thread["name"] = params["name"].clone()
                }
                "turn/start" => {
                    thread["updatedAt"] = json!(now());
                    if thread["preview"].as_str().unwrap_or_default().is_empty()
                        && let Some(text) = params["input"][0]["text"].as_str()
                    {
                        thread["preview"] = json!(text.chars().take(200).collect::<String>());
                    }
                }
                "thread/archive" => {
                    threads.remove(id);
                }
                _ => return Ok(()),
            }
        } else {
            return Ok(());
        }
        if let Some(path) = &self.path {
            security::private_write(path, &serde_json::to_vec(&*threads)?)?;
        }
        Ok(())
    }
    pub fn page(&self, params: &Value) -> Value {
        let threads = self.threads.lock().unwrap();
        let mut entries = threads.values().collect::<Vec<_>>();
        entries.sort_by(|a, b| {
            b["updatedAt"]
                .as_i64()
                .cmp(&a["updatedAt"].as_i64())
                .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
        });
        let offset = params["cursor"]
            .as_str()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0)
            .min(entries.len());
        let limit = params["limit"].as_u64().unwrap_or(50).clamp(1, 100) as usize;
        let end = offset.saturating_add(limit).min(entries.len());
        json!({"data":entries[offset..end],"nextCursor":if end<entries.len() {Some(end.to_string())} else {None}})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_sessions_and_names_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        let catalog = Catalog::open(Some(path.clone())).unwrap();
        catalog
            .record(
                "thread/start",
                &json!({}),
                &json!({"thread":{"id":"test","cwd":"/tmp","turns":[],"updatedAt":now()}}),
            )
            .unwrap();
        catalog
            .record(
                "thread/name/set",
                &json!({"threadId":"test","name":"Empty session"}),
                &json!({}),
            )
            .unwrap();
        let catalog = Catalog::open(Some(path)).unwrap();
        let page = catalog.page(&json!({}));
        assert_eq!(page["data"][0]["name"], "Empty session");
        assert!(page["data"][0].get("turns").is_none());
    }
}
