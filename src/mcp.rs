//! Private stdio MCP adapter: tools call the same authenticated board API as browsers.
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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
    let mut catalogue = crate::mcp_catalog::Catalog::default();
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
            "initialize" => match catalogue.refresh(&client, &host, token).await {
                Ok(()) => {
                    json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"agentmanager-board","version":env!("CARGO_PKG_VERSION")},"instructions":catalogue.instructions}})
                }
                Err(error) => {
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":error.to_string()}})
                }
            },
            "ping" => json!({"jsonrpc":"2.0","id":id,"result":{}}),
            "tools/list" => match catalogue.refresh(&client, &host, token).await {
                Ok(()) => {
                    json!({"jsonrpc":"2.0","id":id,"result":{"tools":catalogue.definitions()}})
                }
                Err(error) => {
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":error.to_string()}})
                }
            },
            "tools/call" => {
                let outcome = async {
                    let op = request["params"]["name"]
                        .as_str()
                        .context("Missing tool name")?;
                    // MCP clients may call a newly advertised tool without first
                    // asking tools/list again. Unknown names trigger one refresh.
                    if catalogue.find(op).is_none() {
                        catalogue.refresh(&client, &host, token).await?;
                    }
                    let tool = catalogue.find(op).context("Unknown tool")?;
                    let mut args = request["params"]["arguments"].clone();
                    if args.is_null() {
                        args = json!({});
                    }
                    if !args.is_object() {
                        bail!("Arguments must be an object");
                    }
                    args.as_object_mut().unwrap().remove("requestId");
                    if tool.definition["annotations"]["readOnlyHint"] != true {
                        args["requestId"] = json!(uuid::Uuid::new_v4().to_string());
                    }
                    if tool.adapter == Some(crate::mcp_catalog::Adapter::Upload) {
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
                    let destination = if tool.adapter == Some(crate::mcp_catalog::Adapter::Download)
                    {
                        args.as_object_mut()
                            .unwrap()
                            .remove("path")
                            .and_then(|v| v.as_str().map(str::to_owned))
                    } else {
                        None
                    };
                    let reply = client
                        .post(tool.url)
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
                    if tool.adapter == Some(crate::mcp_catalog::Adapter::Download) {
                        let data = result["data"]
                            .as_str()
                            .context("Missing file data")?
                            .to_owned();
                        if let Some(path) = destination.as_deref() {
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
                            use std::os::unix::fs::OpenOptionsExt;
                            let decoded = STANDARD.decode(&data)?;
                            let mut f = std::fs::OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .mode(0o600)
                                .open(&dest)?;
                            f.write_all(&decoded)?;
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
