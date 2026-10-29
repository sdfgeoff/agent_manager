use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::{broadcast, mpsc, oneshot},
};

type Reply = std::result::Result<Value, String>;
pub struct Codex {
    pub catalog: crate::catalog::Catalog,
    writer: mpsc::Sender<Value>,
    pending: Mutex<HashMap<String, oneshot::Sender<Reply>>>,
    requests: Mutex<HashMap<String, Value>>,
    active: Mutex<HashMap<String, String>>,
    pub events: broadcast::Sender<Value>,
    alive: std::sync::atomic::AtomicBool,
}

impl Codex {
    pub async fn spawn(binary: &str) -> Result<Arc<Self>> {
        Self::spawn_managed(binary, None).await
    }

    pub async fn spawn_managed(
        binary: &str,
        catalog_path: Option<std::path::PathBuf>,
    ) -> Result<Arc<Self>> {
        let catalog = crate::catalog::Catalog::open(catalog_path)?;
        let mut child = Command::new(binary)
            .args(["app-server", "--listen", "stdio://"])
            .env_remove("AGENTMANAGER_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context(
                "Cannot start Codex; install Codex CLI and sign in with `codex login` first",
            )?;
        let mut stdin = child.stdin.take().unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let (tx, mut rx) = mpsc::channel::<Value>(128);
        let codex = Arc::new(Self {
            catalog,
            writer: tx,
            pending: Mutex::new(HashMap::new()),
            requests: Mutex::new(HashMap::new()),
            active: Mutex::new(HashMap::new()),
            events: broadcast::channel(2048).0,
            alive: std::sync::atomic::AtomicBool::new(true),
        });
        let state = codex.clone();
        tokio::spawn(async move {
            let write = async move {
                while let Some(value) = rx.recv().await {
                    let mut line = serde_json::to_vec(&value)?;
                    line.push(b'\n');
                    stdin.write_all(&line).await?;
                    stdin.flush().await?;
                }
                Ok::<_, anyhow::Error>(())
            };
            let read = {
                let state = state.clone();
                async move {
                    while let Some(line) = lines.next_line().await? {
                        if let Ok(message) = serde_json::from_str::<Value>(&line) {
                            state.incoming(message);
                        }
                    }
                    Ok::<_, anyhow::Error>(())
                }
            };
            tokio::select! {
                result=write=>{if let Err(error)=result {tracing::error!(%error,"Codex input closed");}},
                result=read=>{if let Err(error)=result {tracing::error!(%error,"Codex output closed");}},
                result=child.wait()=>{tracing::warn!(?result,"Codex process exited");},
            }
            state
                .alive
                .store(false, std::sync::atomic::Ordering::Release);
            for (_, tx) in state.pending.lock().unwrap().drain() {
                let _ = tx.send(Err("Codex process exited; restart the worker".into()));
            }
            state.requests.lock().unwrap().clear();
            state.active.lock().unwrap().clear();
            let _ = state
                .events
                .send(json!({"method":"manager/codexExited","params":{}}));
        });
        codex.call("initialize",json!({"clientInfo":{"name":"agentmanager","title":"Agent Manager","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
        codex.writer.send(json!({"method":"initialized"})).await?;
        Ok(codex)
    }

    fn incoming(&self, message: Value) {
        if message.get("method").is_none() {
            if let Some(id) = message.get("id").and_then(Value::as_str)
                && let Some(tx) = self.pending.lock().unwrap().remove(id)
            {
                let reply = if let Some(error) = message.get("error") {
                    Err(error.to_string())
                } else {
                    Ok(message.get("result").cloned().unwrap_or(Value::Null))
                };
                let _ = tx.send(reply);
            }
            return;
        }
        if let Some(id) = message.get("id") {
            self.requests
                .lock()
                .unwrap()
                .insert(id.to_string(), message.clone());
        }
        let params = &message["params"];
        if message["method"] == "thread/name/updated"
            && let Err(error) = self
                .catalog
                .record("thread/name/updated", params, &json!({}))
        {
            tracing::error!(%error,"Cannot update session catalog");
        }
        match message["method"].as_str().unwrap_or_default() {
            "turn/started" => {
                if let (Some(thread), Some(turn)) =
                    (params["threadId"].as_str(), params["turn"]["id"].as_str())
                {
                    self.active
                        .lock()
                        .unwrap()
                        .insert(thread.into(), turn.into());
                }
            }
            "turn/completed" => {
                if let Some(thread) = params["threadId"].as_str() {
                    self.active.lock().unwrap().remove(thread);
                    self.requests
                        .lock()
                        .unwrap()
                        .retain(|_, v| v["params"]["threadId"].as_str() != Some(thread));
                }
            }
            "item/completed" => {
                if let Some(item) = params["item"]["id"].as_str() {
                    self.requests
                        .lock()
                        .unwrap()
                        .retain(|_, v| v["params"]["itemId"].as_str() != Some(item));
                }
            }
            _ => {}
        }
        let _ = self.events.send(message);
    }

    pub fn alive(&self) -> bool {
        self.alive.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn status(&self) -> Value {
        json!({"alive":self.alive(),"requests":self.requests.lock().unwrap().values().collect::<Vec<_>>(),"active":*self.active.lock().unwrap()})
    }
    pub async fn call(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        if !self.alive() {
            bail!("Codex process exited; restart the worker");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        let _guard = CallGuard {
            state: self.clone(),
            id: id.clone(),
        };
        self.writer
            .send(json!({"id":id,"method":method,"params":params}))
            .await?;
        let value = tokio::time::timeout(Duration::from_secs(80), rx)
            .await
            .context("Codex request timed out; check the session before retrying")??;
        let value = value.map_err(|e| anyhow::anyhow!(e))?;
        self.catalog.record(method, &params, &value)?;
        Ok(value)
    }
    pub async fn respond(&self, params: Value) -> Result<Value> {
        let id = params.get("requestId").context("requestId is required")?;
        let result = params.get("result").context("result is required")?;
        let request = self
            .requests
            .lock()
            .unwrap()
            .remove(&id.to_string())
            .context("Request has already been answered or is no longer pending")?;
        if let Err(error) = self.writer.send(json!({"id":id,"result":result})).await {
            self.requests
                .lock()
                .unwrap()
                .insert(id.to_string(), request);
            return Err(error.into());
        }
        let _ = self
            .events
            .send(json!({"method":"manager/requestsChanged","params":{}}));
        Ok(json!({"ok":true}))
    }
}
struct CallGuard {
    state: Arc<Codex>,
    id: String,
}
impl Drop for CallGuard {
    fn drop(&mut self) {
        self.state.pending.lock().unwrap().remove(&self.id);
    }
}

pub fn allowed(method: &str) -> bool {
    matches!(
        method,
        "thread/start"
            | "thread/resume"
            | "thread/list"
            | "thread/read"
            | "thread/turns/list"
            | "thread/items/list"
            | "thread/name/set"
            | "thread/archive"
            | "thread/unarchive"
            | "thread/fork"
            | "turn/start"
            | "turn/steer"
            | "turn/interrupt"
            | "model/list"
            | "account/read"
            | "account/rateLimits/read"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_session_methods_are_exposed() {
        assert!(allowed("turn/start"));
        assert!(!allowed("command/exec"));
        assert!(!allowed("config/batchWrite"));
        assert!(!allowed("account/logout"));
    }
}
