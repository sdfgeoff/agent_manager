use crate::{Wire, codex::Codex, security};
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Semaphore, broadcast, mpsc};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

#[derive(Serialize, Deserialize)]
pub struct Credential {
    pub host: String,
    pub id: String,
    pub credential: String,
}

pub async fn credential(
    host: Option<&str>,
    key: Option<&str>,
    name: &str,
    path: &Path,
) -> Result<Credential> {
    if let Some(key) = key {
        let host = host.context("--host is required for first registration")?;
        let url = security::origin(host)?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .build()?;
        let reply = client
            .post(url.join("api/enroll")?)
            .json(&json!({"key":key,"name":name}))
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        let credential = Credential {
            host: url.origin().ascii_serialization(),
            id: reply["id"].as_str().context("Missing worker id")?.into(),
            credential: reply["credential"]
                .as_str()
                .context("Missing worker credential")?
                .into(),
        };
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            // A custom credential path may live in an existing shared directory.
            // Only the credential itself needs mode 0600; never chmod its parent.
            std::fs::create_dir_all(parent)?;
        }
        security::private_write(path, &serde_json::to_vec(&credential)?)?;
        tracing::info!(worker=%credential.id,"registered worker; saved credential (key is no longer needed)");
        Ok(credential)
    } else {
        let saved: Credential = serde_json::from_slice(
            &std::fs::read(path)
                .context("No saved credential; register with --host URL --key KEY")?,
        )?;
        security::origin(&saved.host)?;
        if let Some(host) = host
            && security::origin(host)?.origin() != security::origin(&saved.host)?.origin()
        {
            bail!("Saved credential belongs to a different server; use another --credential-file");
        }
        Ok(saved)
    }
}

pub async fn run(credential: Credential, binary: String, catalog_path: PathBuf) -> Result<()> {
    let mut codex = Codex::spawn_managed(&binary, Some(catalog_path.clone())).await?;
    let mut delay = 1;
    loop {
        let started = std::time::Instant::now();
        if !codex.alive() {
            tracing::warn!("Codex exited; restarting it. Interrupted turns are not rerun.");
            match Codex::spawn_managed(&binary, Some(catalog_path.clone())).await {
                Ok(new) => {
                    codex = new;
                    delay = 1;
                }
                Err(error) => {
                    tracing::error!(%error,"Cannot restart Codex; retrying in 30 seconds");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    continue;
                }
            }
        }
        match connection(&credential, codex.clone()).await {
            Ok(()) => {}
            Err(error) => tracing::warn!(%error,"server connection lost; Codex continues locally"),
        }
        if started.elapsed() > Duration::from_secs(30) {
            delay = 1;
        }
        tracing::info!(seconds = delay, "reconnecting");
        tokio::time::sleep(Duration::from_secs(delay)).await;
        delay = (delay * 2).min(30);
    }
}

async fn connection(credential: &Credential, codex: Arc<Codex>) -> Result<()> {
    let mut url = security::origin(&credential.host)?.join("api/worker")?;
    url.set_scheme(if credential.host.starts_with("https:") {
        "wss"
    } else {
        "ws"
    })
    .map_err(|_| anyhow::anyhow!("Invalid WebSocket URL"))?;
    let mut request = url.as_str().into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", credential.credential).parse()?,
    );
    let mut events = codex.events.subscribe();
    let (socket, _) = connect_async(request).await?;
    tracing::info!("connected to server");
    let (mut sink, mut stream) = socket.split();
    let (tx, mut responses) = mpsc::channel::<Wire>(128);
    let permits = Arc::new(Semaphore::new(32));
    let mut ping = tokio::time::interval(Duration::from_secs(20));
    let mut last_seen = std::time::Instant::now();
    let hello = Wire::Event {
        message: json!({"method":"manager/connected","params":status(&codex)}),
    };
    sink.send(Message::Text(serde_json::to_string(&hello)?.into()))
        .await?;
    loop {
        let outgoing = tokio::select! {
            _=ping.tick()=>{
                if last_seen.elapsed()>Duration::from_secs(65) {bail!("Server heartbeat timed out");}
                Some(Message::Ping(Vec::new().into()))
            },
            response=responses.recv()=>response.map(|r|Message::Text(serde_json::to_string(&r).unwrap().into())),
            event=events.recv()=>match event {
                Ok(message)=>Some(Message::Text(serde_json::to_string(&Wire::Event {message})?.into())),
                Err(broadcast::error::RecvError::Lagged(_))=>Some(Message::Text(serde_json::to_string(&Wire::Event {message:json!({"method":"manager/resync","params":{}})})?.into())),
                Err(_)=>bail!("Codex event stream closed"),
            },
            incoming=stream.next()=>{
                last_seen=std::time::Instant::now();
                match incoming {
                    Some(Ok(Message::Text(text)))=>{
                        if let Wire::Request {id,method,params}=serde_json::from_str(&text)? {
                            let permit=permits.clone().try_acquire_owned();
                            let tx=tx.clone();let codex=codex.clone();
                            tokio::spawn(async move {
                                let result=match permit {Ok(_permit)=>dispatch(codex,&method,params).await,Err(_)=>Err(anyhow::anyhow!("Worker is busy"))};
                                let (result,error)=match result {Ok(v)=>(Some(v),None),Err(e)=>(None,Some(format!("{e:#}")))};
                                let _=tx.send(Wire::Response {id,result,error}).await;
                            });
                        }
                        None
                    }
                    Some(Ok(Message::Ping(data)))=>Some(Message::Pong(data)),
                    Some(Ok(Message::Pong(_)))=>None,
                    _=>bail!("Server closed the connection"),
                }
            }
        };
        if let Some(message) = outgoing {
            tokio::time::timeout(Duration::from_secs(20), sink.send(message)).await??;
        }
        if !codex.alive() {
            bail!("Codex process exited; reconnecting after restart");
        }
    }
}

pub async fn dispatch(codex: Arc<Codex>, method: &str, params: Value) -> Result<Value> {
    if method.starts_with("manager/agent/") {
        return codex.agents.dispatch(&codex, method, params).await;
    }
    if method == "manager/instructions" {
        let id = params["threadId"].as_str().context("Session id required")?;
        let data = codex.call("thread/read", params.clone()).await?;
        return Ok(
            json!({"persona":crate::agent_runtime::session_instructions(&codex,id,&data["thread"])?}),
        );
    }
    let thread = params["threadId"].as_str().unwrap_or("");
    if codex.agents.bound(thread)
        && matches!(
            method,
            "thread/resume" | "turn/start" | "turn/steer" | "thread/archive"
        )
    {
        if method == "thread/archive" {
            bail!("Registered agent sessions cannot be archived independently");
        }
        let _guard = codex.agents.thread_gate(thread).lock_owned().await;
        codex.agents.prepare_manual(&codex, thread).await?;
        let params = if method == "thread/resume" {
            codex.agents.manual_resume(thread, params.clone())?
        } else {
            params
        };
        return codex.call(method, params).await;
    }
    match method {
        "manager/search" => crate::search::sessions(codex, params).await,
        "manager/list" => codex.list_sessions(params).await,
        "manager/status" => Ok(status(&codex)),
        "manager/respond" => codex.respond(params).await,
        "fs/list" => list_dir(params).await,
        "fs/mkdir" => {
            let path = absolute_path(&params)?;
            tokio::fs::create_dir_all(&path).await?;
            Ok(json!({"path":path}))
        }
        method if crate::codex::allowed(method) => codex.call(method, params).await,
        _ => bail!("Unsupported method: {method}"),
    }
}
fn status(codex: &Codex) -> Value {
    let mut status = codex.status();
    status["capabilities"] = json!(["board-agents", "session-search"]);
    status["workerVersion"] = json!(env!("CARGO_PKG_VERSION"));
    status
}
pub(crate) fn absolute_path(params: &Value) -> Result<PathBuf> {
    let path = params["path"].as_str().unwrap_or("~");
    let home = std::env::var("HOME").context("HOME is not set")?;
    let path = if path == "~" {
        PathBuf::from(&home)
    } else if let Some(rest) = path.strip_prefix("~/") {
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(path)
    };
    if !path.is_absolute() {
        bail!("Choose an absolute path or a path beginning with ~/");
    }
    Ok(path)
}
async fn list_dir(params: Value) -> Result<Value> {
    let path = tokio::fs::canonicalize(absolute_path(&params)?).await?;
    let mut dir = tokio::fs::read_dir(&path).await?;
    let mut entries = Vec::new();
    let mut truncated = false;
    let mut seen = 0;
    while let Some(entry) = dir.next_entry().await? {
        seen += 1;
        if seen > 20000 || entries.len() >= 2000 {
            truncated = true;
            break;
        }
        if tokio::fs::metadata(entry.path())
            .await
            .is_ok_and(|m| m.is_dir())
        {
            entries.push(json!({"name":entry.file_name().to_string_lossy(),"path":entry.path()}));
        }
    }
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(json!({"path":path,"parent":path.parent(),"entries":entries,"truncated":truncated}))
}
