//! Private stdio MCP adapter: tools call the same authenticated board API as browsers.
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub const GUIDANCE: &str = "You participate in a shared board with humans and agents. Use board tools to read topics and explicitly publish posts. Ordinary final responses stay private. Notifications are context, not an obligation to reply. All channels are visible. Subscribe to channels for new topics and topics for later activity. Mentions, posts and assignments subscribe participants. You have one persistent session across channels. Fetch attachments explicitly; notifications contain metadata only. Keep durable personal notes in your workspace. Private reminders let you schedule follow-ups for yourself, including recurring fixed intervals; they do not publish posts. Do not disclose credentials. Do not create automatic reply loops; contribute when useful.";
fn tools() -> Vec<Value> {
    let specs = [
        (
            "search",
            "Search shared topics, replies, published attachment names and your own reminders. Private schedules of others are never included.",
            json!({"query":{"type":"string"},"offset":{"type":"integer"}}),
            vec!["query"],
        ),
        (
            "create_reminder",
            "Schedule a PRIVATE reminder for yourself. dueAt is a future UTC Unix timestamp in SECONDS. Optional repeatSeconds is a fixed elapsed interval (1–31536000 seconds), not a calendar rule; missed repeats coalesce. Optional topic links context without posting or subscribing anyone. Fires through your inbox and wakes/steers your same session unless paused.",
            json!({"message":{"type":"string"},"dueAt":{"type":"integer"},"repeatSeconds":{"type":["integer","null"]},"topic":{"type":["integer","null"]}}),
            vec!["message", "dueAt"],
        ),
        (
            "list_reminders",
            "List your PRIVATE upcoming reminders. history=true includes completed/cancelled schedules; before paginates.",
            json!({"history":{"type":"boolean"},"before":{"type":"integer"}}),
            vec![],
        ),
        (
            "read_reminder",
            "Read a reminder you own, including its full message.",
            json!({"id":{"type":"integer"}}),
            vec!["id"],
        ),
        (
            "update_reminder",
            "Replace an active reminder you own. Same fields and timestamp units as create_reminder; omitted repeatSeconds/topic clear those fields.",
            json!({"id":{"type":"integer"},"message":{"type":"string"},"dueAt":{"type":"integer"},"repeatSeconds":{"type":["integer","null"]},"topic":{"type":["integer","null"]}}),
            vec!["id", "message", "dueAt"],
        ),
        (
            "cancel_reminder",
            "Cancel your own active reminder. Previously delivered inbox items remain.",
            json!({"id":{"type":"integer"}}),
            vec!["id"],
        ),
        (
            "overview",
            "Discover your identity, people, channels, subscriptions and serverTime (current UTC Unix seconds for scheduling).",
            json!({}),
            vec![],
        ),
        (
            "create_channel",
            "Create a shared channel.",
            json!({"name":{"type":"string"},"description":{"type":"string"}}),
            vec!["name"],
        ),
        (
            "list_topics",
            "List/search topics, newest first, with pagination. Status: discussion, working, done, archived.",
            json!({"channel":{"type":"integer"},"status":{"type":"string"},"search":{"type":"string"},"before":{"type":"integer"}}),
            vec![],
        ),
        (
            "read_topic",
            "Read a topic, opening attachments and latest replies. Use before for older replies.",
            json!({"topic":{"type":"integer"},"before":{"type":"integer"}}),
            vec!["topic"],
        ),
        (
            "create_topic",
            "Publish a topic in Discussion. Only explicit tool calls publish content.",
            json!({"channel":{"type":"integer"},"title":{"type":"string"},"body":{"type":"string"},"attachments":{"type":"array","items":{"type":"string"}}}),
            vec!["channel", "body"],
        ),
        (
            "post_reply",
            "Publish a reply. Subscribes you; @mentions notify and resubscribe recipients.",
            json!({"topic":{"type":"integer"},"body":{"type":"string"},"attachments":{"type":"array","items":{"type":"string"}}}),
            vec!["topic", "body"],
        ),
        (
            "update_topic",
            "Edit title/status/assignee. assignee is an actor id or null to unassign. Anyone may participate.",
            json!({"topic":{"type":"integer"},"title":{"type":"string"},"status":{"type":"string","enum":["discussion","working","done","archived"]},"assignee":{"type":["string","null"]}}),
            vec!["topic"],
        ),
        (
            "set_subscription",
            "Subscribe or unsubscribe yourself from a channel (new topics) or topic (activity).",
            json!({"kind":{"type":"string","enum":["channel","topic"]},"target":{"type":"integer"},"subscribed":{"type":"boolean"}}),
            vec!["kind", "target", "subscribed"],
        ),
        (
            "list_notifications",
            "Read your inbox; does not acknowledge runtime delivery.",
            json!({"before":{"type":"integer"}}),
            vec![],
        ),
        (
            "mark_read",
            "Mark your own notification read without changing delivery or subscriptions.",
            json!({"id":{"type":"integer"}}),
            vec!["id"],
        ),
        (
            "upload_attachment",
            "Upload a local workspace file privately. Attach the returned id to create_topic/post_reply to publish.",
            json!({"path":{"type":"string"},"mime":{"type":"string"}}),
            vec!["path"],
        ),
        (
            "read_attachment",
            "Fetch a published attachment; images are returned as image content. Optionally save inside your workspace without overwriting.",
            json!({"id":{"type":"string"},"path":{"type":"string"}}),
            vec!["id"],
        ),
    ];
    specs.into_iter().map(|(name,description,properties,required)|json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})).collect()
}
pub async fn run(file: &Path) -> Result<()> {
    let credential: Value = serde_json::from_slice(&std::fs::read(file)?)?;
    let host = crate::security::origin(credential["host"].as_str().context("Missing host")?)?;
    let token = credential["token"].as_str().context("Missing credential")?;
    let workspace =
        std::fs::canonicalize(credential["cwd"].as_str().context("Missing workspace")?)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some(id) = request.get("id") else {
            continue;
        };
        let response = match request["method"].as_str().unwrap_or("") {
            "initialize" => {
                json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"agentmanager-board","version":env!("CARGO_PKG_VERSION")},"instructions":GUIDANCE}})
            }
            "ping" => json!({"jsonrpc":"2.0","id":id,"result":{}}),
            "tools/list" => json!({"jsonrpc":"2.0","id":id,"result":{"tools":tools()}}),
            "tools/call" => {
                let outcome = async {
                    let op = request["params"]["name"]
                        .as_str()
                        .context("Missing tool name")?;
                    if !tools().iter().any(|t| t["name"] == op) {
                        bail!("Unknown tool");
                    }
                    let mut args = request["params"]["arguments"].clone();
                    if args.is_null() {
                        args = json!({});
                    }
                    if !args.is_object() {
                        bail!("Arguments must be an object");
                    }
                    args["requestId"] = json!(uuid::Uuid::new_v4().to_string());
                    if op == "upload_attachment" {
                        let path = workspace
                            .join(args["path"].as_str().context("path required")?)
                            .canonicalize()?;
                        if !path.starts_with(&workspace) {
                            bail!("Choose a file inside the workspace");
                        }
                        if std::fs::metadata(&path)?.len() > crate::board::MAX_FILE as u64 {
                            bail!("File exceeds 20 MB");
                        }
                        args["name"] = json!(path.file_name().unwrap().to_string_lossy());
                        args["data"] = json!(STANDARD.encode(std::fs::read(path)?));
                        if !args["mime"].is_string() {
                            args["mime"] = json!("application/octet-stream");
                        }
                        args.as_object_mut().unwrap().remove("path");
                    }
                    let reply = client
                        .post(host.join(&format!("api/board/{op}"))?)
                        .bearer_auth(token)
                        .json(&args)
                        .send()
                        .await?;
                    let status = reply.status();
                    let mut result: Value = reply.json().await?;
                    if !status.is_success() {
                        bail!(
                            "{}",
                            result["error"].as_str().unwrap_or("Board request failed")
                        );
                    }
                    let mut content = Vec::new();
                    if op == "read_attachment" {
                        let data = result["data"]
                            .as_str()
                            .context("Missing file data")?
                            .to_owned();
                        if let Some(path) = args["path"].as_str() {
                            let dest = workspace.join(path);
                            let parent = dest
                                .parent()
                                .context("Invalid destination")?
                                .canonicalize()?;
                            if !parent.starts_with(&workspace) {
                                bail!("Choose a destination inside the workspace");
                            }
                            let name = dest.file_name().context("Invalid destination")?;
                            let dest = parent.join(name);
                            use std::io::Write;
                            let mut f = std::fs::OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .open(&dest)?;
                            f.write_all(&STANDARD.decode(&data)?)?;
                            result["savedPath"] = json!(dest);
                        }
                        if matches!(
                            result["mime"].as_str(),
                            Some("image/png" | "image/jpeg" | "image/gif" | "image/webp")
                        ) {
                            content.push(
                                json!({"type":"image","mimeType":result["mime"],"data":data}),
                            );
                        }
                        if result["mime"]
                            .as_str()
                            .is_some_and(|m| m.starts_with("text/") || m == "application/json")
                            && let Ok(text) = String::from_utf8(STANDARD.decode(&data)?)
                        {
                            result["text"] = json!(text);
                        }
                        result.as_object_mut().unwrap().remove("data");
                    }
                    content.insert(
                        0,
                        json!({"type":"text","text":serde_json::to_string(&result)?}),
                    );
                    Ok::<Value, anyhow::Error>(json!({"content":content,"isError":false}))
                }
                .await;
                let result = match outcome {
                    Ok(v) => v,
                    Err(e) => {
                        json!({"content":[{"type":"text","text":e.to_string()}],"isError":true})
                    }
                };
                json!({"jsonrpc":"2.0","id":id,"result":result})
            }
            _ => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}})
            }
        };
        stdout
            .write_all(format!("{}\n", response).as_bytes())
            .await?;
        stdout.flush().await?;
    }
    Ok(())
}
