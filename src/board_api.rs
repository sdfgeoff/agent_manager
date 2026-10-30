//! Typed board endpoint registration: routing, validation, OpenAPI and MCP share this registry.
#![allow(dead_code)] // Request/response fields are consumed by Serde and schema derivation.
use crate::board::{Actor, Agent};
use anyhow::{Result, ensure};
use schemars::{JsonSchema, generate::SchemaSettings};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

pub const GUIDANCE: &str = "You participate in a shared board with humans and agents. Use board tools to read topics and explicitly publish posts. Ordinary final responses stay private. Notifications are context, not an obligation to reply. All channels are visible. Subscribe to channels for new topics and topics for later activity. Mentions, posts and assignments subscribe participants. You have one persistent session across channels. Fetch attachments explicitly; notifications contain metadata only. Keep durable personal notes in your workspace. Private reminders let you schedule follow-ups for yourself, including recurring fixed intervals; they do not publish posts. Do not disclose credentials. Do not create automatic reply loops; contribute when useful.";

#[derive(Deserialize, JsonSchema)]
pub struct Empty {}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Request<T> {
    #[serde(flatten)]
    body: T,
    /// Optional idempotency key. Reuse only for a retry of identical arguments.
    request_id: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
pub struct Id {
    pub id: i64,
}
#[derive(Deserialize, JsonSchema)]
pub struct AgentId {
    pub id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Discussion,
    Working,
    Done,
    Archived,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionKind {
    Channel,
    Topic,
}
#[derive(Deserialize, JsonSchema)]
pub struct Search {
    pub query: String,
    pub offset: Option<u64>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateReminder {
    pub message: String,
    /// Future UTC Unix timestamp in seconds.
    pub due_at: i64,
    /// Fixed elapsed interval, 1–31536000 seconds; null means once.
    pub repeat_seconds: Option<i64>,
    pub topic: Option<i64>,
}
#[derive(Deserialize, JsonSchema)]
pub struct UpdateReminder {
    pub id: i64,
    #[serde(flatten)]
    pub reminder: CreateReminder,
}
#[derive(Deserialize, JsonSchema)]
pub struct ListReminders {
    pub history: Option<bool>,
    pub before: Option<i64>,
}
#[derive(Deserialize, JsonSchema)]
pub struct CreateChannel {
    pub name: String,
    pub description: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
pub struct ListTopics {
    pub channel: Option<i64>,
    pub status: Option<Status>,
    pub search: Option<String>,
    pub before: Option<i64>,
}
#[derive(Deserialize, JsonSchema)]
pub struct ReadTopic {
    pub topic: i64,
    pub before: Option<i64>,
    pub reply: Option<i64>,
}
#[derive(Deserialize, JsonSchema)]
pub struct CreateTopic {
    pub channel: i64,
    pub title: Option<String>,
    pub body: String,
    pub attachments: Option<Vec<String>>,
}
#[derive(Deserialize, JsonSchema)]
pub struct PostReply {
    pub topic: i64,
    pub body: String,
    pub attachments: Option<Vec<String>>,
}
#[derive(Deserialize, JsonSchema)]
pub struct UpdateTopic {
    pub topic: i64,
    pub title: Option<String>,
    pub status: Option<Status>,
    /// Omit to retain the assignee; null explicitly unassigns.
    pub assignee: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
pub struct SetSubscription {
    pub kind: SubscriptionKind,
    pub target: i64,
    pub subscribed: bool,
    /// Humans may additionally manage another participant's subscriptions.
    pub actor: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
pub struct ListNotifications {
    pub before: Option<i64>,
}
#[derive(Deserialize, JsonSchema)]
pub struct UploadAttachment {
    pub name: String,
    pub mime: String,
    pub data: String,
}
#[derive(Deserialize, JsonSchema)]
pub struct ReadAttachment {
    pub id: String,
}
#[derive(Deserialize, JsonSchema)]
pub struct SetAvatar {
    /// Defaults to yourself. Humans may manage any active participant; agents only themselves.
    pub actor: Option<String>,
    /// Base64 PNG, at most 256 KiB and 512×512 pixels. Null removes the avatar.
    pub data: Option<String>,
}
/// Worker-local upload view. The adapter reads the file inside its workspace.
#[derive(Deserialize, JsonSchema)]
pub struct UploadFile {
    pub path: String,
    pub mime: Option<String>,
}
/// Worker-local download view. Saving never overwrites an existing destination.
#[derive(Deserialize, JsonSchema)]
pub struct DownloadFile {
    pub id: String,
    pub path: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
pub struct CreateAgent {
    pub handle: String,
    pub name: String,
    pub worker: String,
    pub thread: Option<String>,
    pub cwd: Option<String>,
    pub persona: Option<String>,
    pub settings: Option<Value>,
}
#[derive(Deserialize, JsonSchema)]
pub struct UpdateAgent {
    pub id: String,
    pub name: Option<String>,
    pub persona: Option<String>,
    pub paused: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct OkReply {
    pub ok: bool,
}
#[derive(Deserialize, JsonSchema)]
pub struct SubscriptionReply {
    pub subscribed: bool,
}
#[derive(Deserialize, JsonSchema)]
pub struct ReplyCreated {
    pub id: i64,
    pub topic: i64,
}
#[derive(Deserialize, JsonSchema)]
pub struct Channel {
    pub id: i64,
    pub name: String,
    pub description: String,
}
#[derive(Deserialize, JsonSchema)]
pub struct SubscribedChannel {
    #[serde(flatten)]
    pub channel: Channel,
    pub subscribed: bool,
}
#[derive(Deserialize, JsonSchema)]
pub struct Person {
    #[serde(flatten)]
    pub actor: Actor,
    pub active: bool,
    #[serde(rename = "avatarUrl")]
    pub avatar_url: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub me: Actor,
    pub people: Vec<Person>,
    pub channels: Vec<SubscribedChannel>,
    pub statuses: Vec<Status>,
    pub server_time: i64,
}
#[derive(Deserialize, JsonSchema)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    pub id: i64,
    pub author: String,
    pub body: String,
    pub created_at: i64,
    pub attachments: Vec<Attachment>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Topic {
    pub id: i64,
    pub channel: i64,
    pub title: String,
    pub body: String,
    pub status: Status,
    pub assignee: Option<String>,
    pub author: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub channel_name: String,
    pub next_cursor: Option<i64>,
    pub subscribed: Option<bool>,
    pub attachments: Option<Vec<Attachment>>,
    pub replies: Option<Vec<Reply>>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Topics {
    pub data: Vec<Topic>,
    pub next_cursor: Option<i64>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub id: i64,
    pub message: String,
    pub topic: Option<i64>,
    pub due_at: i64,
    pub repeat_seconds: Option<i64>,
    pub state: String,
    pub created_at: i64,
    pub last_fired_at: Option<i64>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Reminders {
    pub data: Vec<Reminder>,
    pub next_cursor: Option<i64>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub id: i64,
    /// Event payload; fields depend on the event's kind (including private reminders).
    pub event: Value,
    pub created_at: i64,
    pub read: bool,
}
#[derive(Deserialize, JsonSchema)]
pub struct Notifications {
    pub data: Vec<Notification>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub id: i64,
    pub kind: String,
    pub topic: Option<i64>,
    pub title: String,
    pub body: String,
    pub channel: Option<i64>,
    pub channel_name: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub data: Vec<SearchHit>,
    pub next_offset: Option<i64>,
}
#[derive(Deserialize, JsonSchema)]
pub struct AttachmentData {
    #[serde(flatten)]
    pub attachment: Attachment,
    pub data: String,
}
#[derive(Deserialize, JsonSchema)]
pub struct OnlineAgent {
    #[serde(flatten)]
    pub agent: Agent,
    pub online: bool,
}
#[derive(Deserialize, JsonSchema)]
pub struct AgentUpdated {
    pub agent: Agent,
    pub synchronized: bool,
}
#[derive(Deserialize, JsonSchema)]
pub struct ChannelSubscription {
    pub target: i64,
    pub subscribed: bool,
}
#[derive(Deserialize, JsonSchema)]
pub struct AgentSubscriptions {
    pub data: Vec<ChannelSubscription>,
}
#[derive(Deserialize, JsonSchema)]
pub struct AgentActivityEntry {
    pub id: String,
    pub worker: String,
    pub thread: String,
    pub paused: bool,
    pub error: Option<String>,
    pub queued: i64,
}
#[derive(Deserialize, JsonSchema)]
pub struct AssignedTopic {
    pub id: i64,
    pub title: String,
    pub channel: i64,
    pub status: Status,
    pub assignee: String,
}
#[derive(Deserialize, JsonSchema)]
pub struct Activity {
    pub agents: Vec<AgentActivityEntry>,
    pub topics: Vec<AssignedTopic>,
}
#[derive(Deserialize, JsonSchema)]
pub struct ApiError {
    pub error: String,
}

#[derive(Clone)]
pub struct Operation {
    pub name: &'static str,
    pub description: &'static str,
    pub readonly: bool,
    pub human_only: bool,
    pub adapter: Option<&'static str>,
    input: fn() -> Value,
    output: fn() -> Value,
    tool_input: fn() -> Value,
    pub validate: fn(&Value) -> Result<()>,
    pub validate_response: fn(&Value) -> Result<()>,
}
fn schema<T: JsonSchema>() -> Value {
    let settings = SchemaSettings::draft2020_12().with(|s| s.inline_subschemas = true);
    serde_json::to_value(settings.into_generator().into_root_schema_for::<T>()).unwrap()
}
fn validate<T: DeserializeOwned>(v: &Value) -> Result<()> {
    ensure!(v.is_object(), "Request must be an object");
    serde_json::from_value::<Request<T>>(v.clone())?;
    Ok(())
}
fn validate_response<T: DeserializeOwned>(v: &Value) -> Result<()> {
    serde_json::from_value::<T>(v.clone())?;
    Ok(())
}
fn endpoint<I: DeserializeOwned + JsonSchema, O: DeserializeOwned + JsonSchema>(
    name: &'static str,
    description: &'static str,
    readonly: bool,
    human_only: bool,
) -> Operation {
    Operation {
        name,
        description,
        readonly,
        human_only,
        adapter: None,
        input: schema::<Request<I>>,
        output: schema::<O>,
        tool_input: schema::<I>,
        validate: validate::<I>,
        validate_response: validate_response::<O>,
    }
}
impl Operation {
    pub fn path(&self) -> String {
        format!("/api/board/{}", self.name)
    }
    fn local_view<T: JsonSchema>(mut self, adapter: &'static str) -> Self {
        self.adapter = Some(adapter);
        self.tool_input = schema::<T>;
        self
    }
    pub fn openapi(&self) -> Value {
        let mut result = json!({
            "operationId":self.name,"description":self.description,
            "security":[{"agentBearer":[]},{"browserSession":[]}],
            "requestBody":{"required":true,"content":{"application/json":{"schema":(self.input)()}}},
            "responses":{"200":{"description":"Successful board operation","content":{"application/json":{"schema":(self.output)()}}},
                "400":{"description":"Invalid arguments or domain operation","content":{"application/json":{"schema":schema::<ApiError>()}}},
                "401":{"description":"Sign in or present an agent credential"},
                "403":{"description":"Forbidden for this actor or invalid browser Origin/header"}}
        });
        if !self.human_only {
            result["x-mcp"] = json!({"inputSchema":(self.tool_input)(),"adapter":self.adapter,"annotations":{"readOnlyHint":self.readonly && self.adapter.is_none(),"destructiveHint":!self.readonly,"openWorldHint":false}});
        } else {
            result["security"] = json!([{"browserSession":[]}]);
        }
        result
    }
}

/// Adding a typed registration creates its HTTP route, schema and (unless human-only) MCP tool.
pub fn operations() -> Vec<Operation> {
    vec![
        endpoint::<Search,SearchResults>("search","Search shared topics, replies, published attachment names and your own reminders. Private schedules of others are never included.",true,false),
        endpoint::<CreateReminder,Reminder>("create_reminder","Schedule a PRIVATE reminder for yourself. dueAt is a future UTC Unix timestamp in SECONDS. Optional repeatSeconds is a fixed elapsed interval (1–31536000 seconds), not a calendar rule; missed repeats coalesce. Optional topic links context without posting or subscribing anyone. Fires through your inbox and wakes/steers your same session unless paused.",false,false),
        endpoint::<ListReminders,Reminders>("list_reminders","List your PRIVATE upcoming reminders. history=true includes completed/cancelled schedules; before paginates.",true,false),
        endpoint::<Id,Reminder>("read_reminder","Read a reminder you own, including its full message.",true,false),
        endpoint::<UpdateReminder,Reminder>("update_reminder","Replace an active reminder you own. Same fields and timestamp units as create_reminder; omitted repeatSeconds/topic clear those fields.",false,false),
        endpoint::<Id,Reminder>("cancel_reminder","Cancel your own active reminder. Previously delivered inbox items remain.",false,false),
        endpoint::<Empty,Overview>("overview","Discover your identity, people, channels, subscriptions and serverTime (current UTC Unix seconds for scheduling).",true,false),
        endpoint::<SetAvatar,OkReply>("set_avatar","Set or remove a shared profile avatar. Agents may update themselves; humans may update any active participant. Supply base64 PNG up to 256 KiB and 512×512 pixels, or null to remove.",false,false),
        endpoint::<CreateChannel,Channel>("create_channel","Create a shared channel.",false,false),
        endpoint::<ListTopics,Topics>("list_topics","List/search topics, newest first, with pagination. Status: discussion, working, done, archived.",true,false),
        endpoint::<ReadTopic,Topic>("read_topic","Read a topic, opening attachments and latest replies. Use before for older replies.",true,false),
        endpoint::<CreateTopic,Topic>("create_topic","Publish a topic in Discussion. Only explicit tool calls publish content.",false,false),
        endpoint::<PostReply,ReplyCreated>("post_reply","Publish a reply. Subscribes you; @mentions notify and resubscribe recipients.",false,false),
        endpoint::<UpdateTopic,Topic>("update_topic","Edit title/status/assignee. assignee is an actor id or null to unassign. Anyone may participate.",false,false),
        endpoint::<SetSubscription,SubscriptionReply>("set_subscription","Subscribe or unsubscribe yourself from a channel (new topics) or topic (activity).",false,false),
        endpoint::<ListNotifications,Notifications>("list_notifications","Read your inbox; does not acknowledge runtime delivery.",true,false),
        endpoint::<Id,OkReply>("mark_read","Mark your own notification read without changing delivery or subscriptions.",false,false),
        endpoint::<UploadAttachment,Attachment>("upload_attachment","Upload a local workspace file privately. Attach the returned id to create_topic/post_reply to publish.",false,false).local_view::<UploadFile>("workspace-upload-v1"),
        endpoint::<ReadAttachment,AttachmentData>("read_attachment","Fetch a published attachment; images are returned as image content. Optionally save inside your workspace without overwriting.",true,false).local_view::<DownloadFile>("workspace-download-v1"),
        endpoint::<Id,Notification>("read_notification","Read a full notification you own, including its untruncated event message.",true,false),
        endpoint::<Empty,Vec<OnlineAgent>>("list_agents","List registered agents and their configuration.",true,true),
        endpoint::<CreateAgent,Agent>("create_agent","Register an existing session or create a persistent agent.",false,true),
        endpoint::<UpdateAgent,AgentUpdated>("update_agent","Edit an agent persona or pause/resume it.",false,true),
        endpoint::<AgentId,AgentSubscriptions>("agent_subscriptions","Read an agent's channel subscriptions.",true,true),
        endpoint::<Empty,Activity>("agent_activity","Read agent activity and assigned topics.",true,true),
    ]
}
pub fn document(human: bool, cookie_name: &str) -> Value {
    let paths = operations()
        .into_iter()
        .filter(|op| human || !op.human_only)
        .map(|op| (op.path(), json!({"post":op.openapi()})))
        .collect::<serde_json::Map<String, Value>>();
    json!({"openapi":"3.1.1","jsonSchemaDialect":"https://json-schema.org/draft/2020-12/schema",
        "info":{"title":"Agent Manager Board API","version":env!("CARGO_PKG_VERSION"),
            "description":"Authenticated board operations. Browser POSTs additionally require the configured Origin and X-Agentmanager: 1. Account, deployment and worker-control APIs are intentionally outside this document."},
        "paths":paths,"components":{"securitySchemes":{"agentBearer":{"type":"http","scheme":"bearer"},"browserSession":{"type":"apiKey","in":"cookie","name":cookie_name}}},
        "x-mcp-instructions":GUIDANCE})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_operations_share_types_access_policy_and_self_contained_schemas() {
        let doc = document(false, "__Host-am");
        let paths = doc["paths"].as_object().unwrap();
        for op in operations() {
            assert_eq!(paths.contains_key(&op.path()), !op.human_only);
            let api = op.openapi();
            assert_eq!(api["operationId"], op.name);
            assert!(
                !api.to_string().contains("\"$ref\""),
                "Schemas must not contain dangling embedded references"
            );
        }
        let sub = operations()
            .into_iter()
            .find(|o| o.name == "set_subscription")
            .unwrap();
        assert!((sub.validate)(&json!({"kind":"topic","target":1,"subscribed":true,"requestId":"retry","futureOptionalField":"ignored"})).is_ok());
        assert!((sub.validate)(&json!({"kind":"unknown","target":1,"subscribed":true})).is_err());
        let topic = operations()
            .into_iter()
            .find(|o| o.name == "read_topic")
            .unwrap();
        assert_eq!(
            topic.openapi()["x-mcp"]["inputSchema"]["properties"]["reply"]["type"],
            json!(["integer", "null"])
        );
        let upload = operations()
            .into_iter()
            .find(|o| o.name == "upload_attachment")
            .unwrap()
            .openapi();
        assert!(
            upload["requestBody"]["content"]["application/json"]["schema"]["properties"]
                .get("data")
                .is_some()
        );
        assert!(
            upload["x-mcp"]["inputSchema"]["properties"]
                .get("path")
                .is_some()
        );
        assert!(
            upload["x-mcp"]["inputSchema"]["properties"]
                .get("data")
                .is_none()
        );
    }
}
