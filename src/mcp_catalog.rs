//! Generic discovery of explicitly advertised JSON-POST tools in the server's OpenAPI document.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::collections::HashSet;
use url::Url;

#[derive(Clone, Debug, PartialEq)]
pub enum Adapter {
    Upload,
    Download,
}
#[derive(Clone, Debug)]
pub struct Tool {
    pub definition: Value,
    pub url: Url,
    pub adapter: Option<Adapter>,
}
#[derive(Default)]
pub struct Catalog {
    pub tools: Vec<Tool>,
    pub instructions: String,
    etag: Option<String>,
}
impl Catalog {
    pub async fn refresh(
        &mut self,
        client: &reqwest::Client,
        host: &Url,
        token: &str,
    ) -> Result<()> {
        let mut request = client
            .get(host.join("/api/board/openapi.json")?)
            .bearer_auth(token)
            .timeout(std::time::Duration::from_secs(10));
        if let Some(etag) = &self.etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        let mut reply = request.send().await?;
        if reply.status() == reqwest::StatusCode::NOT_MODIFIED {
            ensure!(self.etag.is_some(), "Unexpected empty discovery cache");
            return Ok(());
        }
        if matches!(reply.status().as_u16(), 401 | 403) {
            self.tools.clear();
            self.etag = None;
            self.instructions.clear();
            bail!("Agent credential was rejected during tool discovery");
        }
        if !reply.status().is_success() {
            bail!(
                "Tool discovery returned HTTP {}; the server must offer /api/board/openapi.json",
                reply.status()
            );
        }
        let etag = reply
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut bytes = Vec::new();
        while let Some(chunk) = reply.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 4 * 1024 * 1024,
                "Tool catalogue exceeds 4 MB"
            );
            bytes.extend(chunk);
        }
        let document: Value = serde_json::from_slice(&bytes).context("Invalid OpenAPI document")?;
        let tools = parse(&document, host)?;
        self.instructions = document["x-mcp-instructions"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        self.tools = tools;
        self.etag = etag;
        Ok(())
    }
    pub fn definitions(&self) -> Vec<Value> {
        self.tools.iter().map(|t| t.definition.clone()).collect()
    }
    pub fn find(&self, name: &str) -> Option<Tool> {
        self.tools
            .iter()
            .find(|t| t.definition["name"] == name)
            .cloned()
    }
}
pub fn parse(document: &Value, host: &Url) -> Result<Vec<Tool>> {
    ensure!(
        document["openapi"]
            .as_str()
            .is_some_and(|v| v.starts_with("3.1.")),
        "OpenAPI 3.1 is required"
    );
    let paths = document["paths"]
        .as_object()
        .context("OpenAPI paths are missing")?;
    let mut tools = Vec::new();
    let mut names = HashSet::new();
    for (path, item) in paths {
        let op = &item["post"];
        let Some(meta) = op.get("x-mcp").filter(|v| v.is_object()) else {
            continue;
        };
        ensure!(tools.len() < 256, "Too many MCP tools");
        // This bridge never follows document servers, external refs, arbitrary HTTP
        // destinations, query strings or path templates with credentials attached.
        ensure!(
            path.starts_with("/api/board/")
                && path.split('/').skip(3).all(|s| !s.is_empty()
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))),
            "Unsafe tool endpoint"
        );
        let url = host.join(path)?;
        ensure!(
            url.origin() == host.origin()
                && url.path() == path
                && url.query().is_none()
                && url.fragment().is_none(),
            "Unsafe tool endpoint"
        );
        let name = op["operationId"]
            .as_str()
            .context("Missing tool operationId")?;
        ensure!(
            !name.is_empty()
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
                && names.insert(name.to_owned()),
            "Invalid or duplicate tool name"
        );
        let adapter = match meta["adapter"].as_str() {
            None => None,
            Some("workspace-upload-v1") => Some(Adapter::Upload),
            Some("workspace-download-v1") => Some(Adapter::Download),
            Some(_) => continue, // Future local adapters need local code; never forward them blindly.
        };
        let mut input = meta
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| op["requestBody"]["content"]["application/json"]["schema"].clone());
        ensure!(
            input["type"] == "object",
            "Tool input must be an object schema"
        );
        // Internal HTTP idempotency keys are generated by the bridge.
        if let Some(properties) = input["properties"].as_object_mut() {
            properties.remove("requestId");
        }
        if let Some(required) = input["required"].as_array_mut() {
            required.retain(|v| v != "requestId");
        }
        tools.push(Tool{url,adapter,definition:json!({"name":name,"description":op["description"].as_str().unwrap_or(""),"inputSchema":input,"annotations":meta["annotations"].as_object().cloned().unwrap_or_default()})});
    }
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogue_selects_only_explicit_tools_and_uses_server_paths_and_schemas() {
        let host = Url::parse("https://example.com").unwrap();
        let mut doc = json!({"openapi":"3.1.1","paths":{
            "/api/board/future_endpoint":{"post":{"operationId":"future_tool","description":"New server feature","requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"newField":{"type":"boolean"},"requestId":{"type":"string"}}}}}},"x-mcp":{}}},
            "/api/board/admin":{"post":{"operationId":"admin","requestBody":{}}},
            "/api/board/new_local":{"post":{"operationId":"new_local","x-mcp":{"inputSchema":{"type":"object"},"adapter":"unknown-v2"}}}
        }});
        let tools = parse(&doc, &host).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(
            tools[0].url.as_str(),
            "https://example.com/api/board/future_endpoint"
        );
        assert_eq!(
            tools[0].definition["inputSchema"]["properties"]["newField"]["type"],
            "boolean"
        );
        assert!(
            tools[0].definition["inputSchema"]["properties"]
                .get("requestId")
                .is_none()
        );
        doc["paths"]["/api/board/future_endpoint"]["post"]["operationId"] = json!("another_tool");
        assert_eq!(
            parse(&doc, &host).unwrap()[0].definition["name"],
            "another_tool"
        );
        for path in [
            "https://evil.example/api/board/x",
            "//evil.example/api/board/x",
            "/api/board/../settings",
            "/api/board/x?query=1",
            "/api/board/%2e%2e",
            "/api/board/{id}",
        ] {
            let bad = json!({"openapi":"3.1.1","paths":{path:{"post":{"operationId":"bad","x-mcp":{"inputSchema":{"type":"object"}}}}}});
            assert!(parse(&bad, &host).is_err(), "{path}");
        }
    }
}
