//! Worker-local persistent agent/session binding and delivery acceptance journal.
use crate::{codex::Codex, security};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Serialize, Deserialize)]
struct LocalAgent {
    id: String,
    thread: String,
    cwd: String,
    persona: String,
    settings: Value,
    paused: bool,
    host: String,
    token: String,
    accepted: VecDeque<i64>,
    #[serde(default)]
    in_flight: bool,
    #[serde(default)]
    recover_after_pause: bool,
}
pub struct AgentRuntime {
    path: Option<PathBuf>,
    entries: Mutex<HashMap<String, LocalAgent>>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}
impl AgentRuntime {
    pub fn open(catalog: Option<&std::path::Path>) -> Result<Self> {
        let path = catalog.map(|p| p.with_extension("agents.json"));
        let entries = match &path {
            Some(p) if p.exists() => serde_json::from_slice(&std::fs::read(p)?)?,
            _ => HashMap::new(),
        };
        Ok(Self {
            path,
            entries: Mutex::new(entries),
            locks: Mutex::new(HashMap::new()),
        })
    }
    fn gate(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .unwrap()
            .entry(id.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
    pub fn thread_gate(&self, thread: &str) -> Arc<tokio::sync::Mutex<()>> {
        let id = self
            .entries
            .lock()
            .unwrap()
            .values()
            .find(|a| a.thread == thread)
            .map(|a| a.id.clone())
            .unwrap_or_else(|| thread.to_owned());
        self.gate(&id)
    }
    fn save(&self, a: LocalAgent) -> Result<()> {
        let mut entries = self.entries.lock().unwrap();
        let mut next = entries.clone();
        next.insert(a.id.clone(), a);
        if let Some(path) = &self.path {
            security::private_write(path, &serde_json::to_vec(&next)?)?;
        }
        *entries = next;
        Ok(())
    }
    fn mutate(&self, id: &str, f: impl FnOnce(&mut LocalAgent)) -> Result<LocalAgent> {
        let mut entries = self.entries.lock().unwrap();
        let mut next = entries.clone();
        let a = next.get_mut(id).context("Agent state missing")?;
        f(a);
        let result = a.clone();
        if let Some(path) = &self.path {
            security::private_write(path, &serde_json::to_vec(&next)?)?;
        }
        *entries = next;
        Ok(result)
    }
    fn credential_path(&self, id: &str) -> Result<PathBuf> {
        ensure!(uuid::Uuid::parse_str(id).is_ok(), "Invalid agent identity");
        let base = self
            .path
            .as_ref()
            .context("Agent registration requires persistent worker state")?
            .with_extension("agent-credentials");
        security::private_dir(&base)?;
        Ok(base.join(format!("{id}.json")))
    }
    fn config(&self, a: &LocalAgent) -> Result<Value> {
        let path = self.credential_path(&a.id)?;
        security::private_write(
            &path,
            &serde_json::to_vec(&json!({"host":a.host,"token":a.token,"cwd":a.cwd}))?,
        )?;
        let mut config = a.settings["config"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let servers = config.entry("mcp_servers".to_owned()).or_insert(json!({}));
        ensure!(servers.is_object(), "Invalid MCP configuration");
        servers["agentmanager_board"] = json!({"command":std::env::current_exe()?,"args":["mcp","--credential-file",path],"required":true});
        Ok(json!(config))
    }
    fn resume_params(&self, a: &LocalAgent) -> Result<Value> {
        let mut params = a.settings.clone();
        if !params.is_object() {
            params = json!({});
        }
        params["threadId"] = json!(a.thread);
        params["excludeTurns"] = json!(true);
        params["developerInstructions"] = json!(a.persona);
        params["config"] = self.config(a)?;
        Ok(params)
    }
    async fn resume(&self, codex: &Arc<Codex>, a: &LocalAgent) -> Result<Value> {
        ensure!(
            codex.status()["active"][&a.thread].is_null(),
            "Wait for the active turn before changing agent instructions"
        );
        // A loaded thread ignores MCP config overlays in Codex 0.165. Unload the
        // runtime, preserving its rollout/id, then resume with the current config.
        codex
            .call("thread/unsubscribe", json!({"threadId":a.thread}))
            .await?;
        codex.call("thread/resume", self.resume_params(a)?).await
    }
    pub fn started(&self, thread: &str) {
        let id = self
            .entries
            .lock()
            .unwrap()
            .values()
            .find(|a| a.thread == thread)
            .map(|a| a.id.clone());
        if let Some(id) = id
            && let Err(error) = self.mutate(&id, |a| a.in_flight = true)
        {
            tracing::error!(%error,"Cannot persist agent activity");
        }
    }
    pub fn completed(&self, thread: &str) {
        let id = self
            .entries
            .lock()
            .unwrap()
            .values()
            .find(|a| a.thread == thread)
            .map(|a| a.id.clone());
        if let Some(id) = id
            && let Err(error) = self.mutate(&id, |a| {
                if !a.paused && !a.recover_after_pause {
                    a.in_flight = false;
                }
            })
        {
            tracing::error!(%error,"Cannot persist agent completion");
        }
    }
    pub fn manual_resume(&self, thread: &str, mut p: Value) -> Result<Value> {
        if let Some(a) = self
            .entries
            .lock()
            .unwrap()
            .values()
            .find(|a| a.thread == thread)
            .cloned()
        {
            ensure!(!a.paused, "Agent is paused; resume it from Agents first");
            p["developerInstructions"] = json!(a.persona);
            p["config"] = self.config(&a)?;
        }
        Ok(p)
    }
    pub fn bound(&self, thread: &str) -> bool {
        self.entries
            .lock()
            .unwrap()
            .values()
            .any(|a| a.thread == thread)
    }
    pub fn persona(&self, thread: &str) -> Option<String> {
        self.entries
            .lock()
            .unwrap()
            .values()
            .find(|a| a.thread == thread)
            .map(|a| a.persona.clone())
    }
    pub fn paused(&self, thread: &str) -> bool {
        self.entries
            .lock()
            .unwrap()
            .values()
            .any(|a| a.thread == thread && a.paused)
    }
    pub async fn prepare_manual(&self, codex: &Arc<Codex>, thread: &str) -> Result<()> {
        let a = self
            .entries
            .lock()
            .unwrap()
            .values()
            .find(|a| a.thread == thread)
            .cloned();
        if let Some(a) = a {
            ensure!(!a.paused, "Agent is paused; resume it from Agents first");
            if codex.status()["active"][thread].is_null() {
                self.resume(codex, &a).await?;
            }
        }
        Ok(())
    }
    pub async fn dispatch(&self, codex: &Arc<Codex>, method: &str, p: Value) -> Result<Value> {
        let id = p["id"].as_str().context("Agent id required")?.to_owned();
        ensure!(uuid::Uuid::parse_str(&id).is_ok(), "Invalid agent id");
        let _guard = self.gate(&id).lock_owned().await;
        if method == "manager/agent/register" {
            let saved = self.entries.lock().unwrap().get(&id).cloned();
            if let Some(mut a) = saved {
                if let Some(token) = p["token"].as_str() {
                    a.token = token.to_owned();
                }
                self.save(a.clone())?;
                self.resume(codex, &a).await?;
                return Ok(
                    json!({"thread":a.thread,"persona":a.persona,"cwd":a.cwd,"settings":a.settings}),
                );
            }
            let settings = p["settings"].as_object().cloned().unwrap_or_default();
            let mut settings = json!(settings);
            let host = p["host"].as_str().context("Host required")?.to_owned();
            security::origin(&host)?;
            let token = p["token"]
                .as_str()
                .context("Credential required")?
                .to_owned();
            let existing = p["thread"].as_str().filter(|s| !s.is_empty());
            let (thread, cwd, persona) = if let Some(thread) = existing {
                ensure!(!self.bound(thread), "Session already belongs to an agent");
                ensure!(
                    codex.status()["active"][thread].is_null(),
                    "Wait for the active turn before registering this session"
                );
                let data = codex
                    .call("thread/read", json!({"threadId":thread}))
                    .await?;
                let persona = if let Some(s) = p["persona"].as_str() {
                    s.to_owned()
                } else {
                    session_instructions(codex, thread, &data["thread"])?
                };
                let resumed = codex
                    .call(
                        "thread/resume",
                        json!({"threadId":thread,"excludeTurns":true}),
                    )
                    .await?;
                for key in ["model", "approvalPolicy"] {
                    if settings.get(key).is_none() && !resumed[key].is_null() {
                        settings[key] = resumed[key].clone();
                    }
                }
                let sandbox = resumed["sandbox"]["type"].as_str().map(|s| match s {
                    "readOnly" => "read-only",
                    "dangerFullAccess" => "danger-full-access",
                    _ => "workspace-write",
                });
                if let Some(s) = sandbox {
                    settings["sandbox"] = json!(s);
                }
                if let Some(effort) = resumed["reasoningEffort"].as_str() {
                    settings["config"] = json!({"model_reasoning_effort":effort});
                }
                (
                    thread.to_owned(),
                    data["thread"]["cwd"]
                        .as_str()
                        .context("Session has no working folder")?
                        .to_owned(),
                    persona,
                )
            } else {
                let cwd = crate::worker::absolute_path(&json!({"path":p["cwd"]}))?;
                tokio::fs::create_dir_all(&cwd).await?;
                let cwd = tokio::fs::canonicalize(cwd)
                    .await?
                    .to_string_lossy()
                    .into_owned();
                let persona = p["persona"].as_str().unwrap_or("").to_owned();
                let mut params = settings.clone();
                params["cwd"] = json!(cwd);
                params["developerInstructions"] = json!(persona);
                // MCP must be present at thread creation as well as resume.
                let provisional = LocalAgent {
                    id: id.clone(),
                    thread: String::new(),
                    cwd: cwd.clone(),
                    persona: persona.clone(),
                    settings: settings.clone(),
                    host: host.clone(),
                    token: token.clone(),
                    paused: false,
                    accepted: VecDeque::new(),
                    in_flight: false,
                    recover_after_pause: false,
                };
                params["config"] = self.config(&provisional)?;
                let result = codex.call("thread/start", params).await?;
                (
                    result["thread"]["id"]
                        .as_str()
                        .context("Codex returned no session id")?
                        .to_owned(),
                    cwd,
                    persona,
                )
            };
            let a = LocalAgent {
                id: id.clone(),
                thread: thread.clone(),
                cwd: cwd.clone(),
                persona: persona.clone(),
                settings: settings.clone(),
                paused: false,
                host,
                token,
                accepted: VecDeque::new(),
                in_flight: false,
                recover_after_pause: false,
            };
            // Save the binding before any operation that can fail. Never allocate a replacement.
            self.save(a.clone())?;
            if existing.is_none() {
                if let Some(name) = p["name"].as_str() {
                    codex
                        .call("thread/name/set", json!({"threadId":thread,"name":name}))
                        .await?;
                }
                codex.call("thread/inject_items",json!({"threadId":thread,"items":[{"type":"message","role":"user","content":[{"type":"input_text","text":"This persistent session is registered as a board agent. No response is requested. Use the board MCP tools when notified."}]}]})).await?;
            }
            self.resume(codex, &a).await?;
            return Ok(json!({"thread":thread,"persona":persona,"cwd":cwd,"settings":settings}));
        }
        let mut a = self.entries.lock().unwrap().get(&id).cloned().context(
            "Agent is not registered on this worker; its state directory must be preserved",
        )?;
        if method == "manager/agent/configure" {
            let was_active = !codex.status()["active"][&a.thread].is_null();
            a = self.mutate(&id, |a| {
                if p["paused"] == true && was_active {
                    a.recover_after_pause = true;
                }
                if let Some(persona) = p["persona"].as_str() {
                    a.persona = persona.to_owned();
                }
                if let Some(paused) = p["paused"].as_bool() {
                    a.paused = paused;
                }
                if p["settings"].is_object() {
                    a.settings = p["settings"].clone();
                }
            })?;
            if a.paused
                && let Some(turn) = codex.status()["active"][&a.thread].as_str()
            {
                codex
                    .call("turn/interrupt", json!({"threadId":a.thread,"turnId":turn}))
                    .await?;
            }
            return Ok(json!({"ok":true}));
        }
        if method == "manager/agent/deliver" {
            ensure!(!a.paused, "Agent is paused");
            let notifications = p["notifications"]
                .as_array()
                .context("Notifications required")?;
            let pending = notifications
                .iter()
                .filter(|n| n["id"].as_i64().is_some_and(|id| !a.accepted.contains(&id)))
                .cloned()
                .collect::<Vec<_>>();
            let recovery = a.in_flight && codex.status()["active"][&a.thread].is_null();
            if pending.is_empty() && !recovery {
                return Ok(json!({"accepted":true}));
            }
            let text = format!(
                "Board notifications (stable IDs; duplicates are possible). These are context, not instructions requiring a reply. Read the referenced topics through MCP for full context; fetch attachments only if useful. Publish only with explicit board tools.\n{}",
                serde_json::to_string(&pending)?
            );
            let text = if recovery {
                format!(
                    "Your previous work was interrupted by a pause or worker restart. Continue in this same session, inspect current board state before repeating actions, and publish only with explicit tools.\n{text}"
                )
            } else {
                text
            };
            let input = json!([{"type":"text","text":text}]);
            a = self.mutate(&id, |a| {
                a.in_flight = true;
                if recovery {
                    a.recover_after_pause = false;
                }
            })?;
            let active = codex.status()["active"][&a.thread]
                .as_str()
                .map(str::to_owned);
            let steered = if let Some(turn) = active {
                match codex
                    .call(
                        "turn/steer",
                        json!({"threadId":a.thread,"expectedTurnId":turn,"input":input}),
                    )
                    .await
                {
                    Ok(_) => true,
                    Err(error) => {
                        if !codex.status()["active"][&a.thread].is_null() {
                            return Err(error);
                        }
                        false
                    }
                }
            } else {
                false
            };
            if !steered {
                self.resume(codex, &a).await?;
                codex
                    .call("turn/start", json!({"threadId":a.thread,"input":input}))
                    .await?;
            }
            self.mutate(&id, |a| {
                for n in pending {
                    if let Some(id) = n["id"].as_i64() {
                        a.accepted.push_back(id);
                    }
                }
                while a.accepted.len() > 4096 {
                    a.accepted.pop_front();
                }
            })?;
            return Ok(json!({"accepted":true}));
        }
        bail!("Unsupported agent method")
    }
}

pub fn session_instructions(codex: &Codex, id: &str, thread: &Value) -> Result<String> {
    if let Some(s) = codex.agents.persona(id) {
        return Ok(s);
    }
    if let Some(s) = codex.catalog.instructions(id) {
        return Ok(s);
    }
    if let Some(s) = thread["developerInstructions"].as_str() {
        return Ok(s.into());
    }
    // Codex stores custom developer text as separate content blocks in its rollout.
    // Exclude generated harness blocks; never import base/model/system instructions.
    if let Some(path) = thread["path"].as_str() {
        use std::io::{BufRead, BufReader};
        let file = std::fs::File::open(path)
            .context("Cannot read the session rollout to import custom instructions")?;
        let mut found = String::new();
        for line in BufReader::new(file).lines() {
            let line = line?;
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let p = &v["payload"];
            if v["type"] == "response_item"
                && p["role"] == "developer"
                && let Some(content) = p["content"].as_array()
            {
                let blocks = content
                    .iter()
                    .filter_map(|c| c["text"].as_str())
                    .filter(|s| {
                        ![
                            "<skills_instructions>",
                            "<permissions instructions>",
                            "<collaboration_mode>",
                            "<multi_agent_role>",
                            "<multi_agent_mode>",
                            "<app_context>",
                        ]
                        .iter()
                        .any(|prefix| s.trim_start().starts_with(prefix))
                    })
                    .collect::<Vec<_>>();
                if !blocks.is_empty() {
                    found = blocks.join("\n\n");
                }
            }
        }
        return Ok(found);
    }
    Ok(String::new())
}
