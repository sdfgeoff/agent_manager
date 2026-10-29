//! Shared board domain operations. The HTTP and MCP clients use this same contract.
use crate::{now, security, store::Store};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const STATUSES: [&str; 4] = ["discussion", "working", "done", "archived"];
pub const MAX_FILE: usize = 20 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Actor {
    pub id: String,
    pub handle: String,
    pub name: String,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub handle: String,
    pub name: String,
    pub worker: String,
    pub thread: String,
    pub persona: String,
    pub settings: Value,
    pub paused: bool,
    pub error: Option<String>,
}
pub fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS board_actors(id TEXT PRIMARY KEY,handle TEXT NOT NULL UNIQUE,name TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('human','agent')));
CREATE TABLE IF NOT EXISTS board_agents(id TEXT PRIMARY KEY REFERENCES board_actors(id),worker TEXT NOT NULL,thread TEXT NOT NULL,persona TEXT NOT NULL,settings TEXT NOT NULL,paused INTEGER NOT NULL DEFAULT 0,token_hash TEXT NOT NULL UNIQUE,error TEXT,UNIQUE(worker,thread));
CREATE TABLE IF NOT EXISTS board_channels(id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT NOT NULL UNIQUE,description TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS board_topics(id INTEGER PRIMARY KEY AUTOINCREMENT,channel INTEGER NOT NULL REFERENCES board_channels(id),title TEXT NOT NULL,body TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'discussion' CHECK(status IN ('discussion','working','done','archived')),assignee TEXT REFERENCES board_actors(id),author TEXT NOT NULL REFERENCES board_actors(id),created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS board_replies(id INTEGER PRIMARY KEY AUTOINCREMENT,topic INTEGER NOT NULL REFERENCES board_topics(id),author TEXT NOT NULL REFERENCES board_actors(id),body TEXT NOT NULL,created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS board_subscriptions(actor TEXT NOT NULL REFERENCES board_actors(id),kind TEXT NOT NULL CHECK(kind IN ('channel','topic')),target INTEGER NOT NULL,subscribed INTEGER NOT NULL,PRIMARY KEY(actor,kind,target));
CREATE TABLE IF NOT EXISTS board_events(id INTEGER PRIMARY KEY AUTOINCREMENT,topic INTEGER NOT NULL REFERENCES board_topics(id),actor TEXT NOT NULL REFERENCES board_actors(id),kind TEXT NOT NULL,payload TEXT NOT NULL,created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS board_notifications(id INTEGER PRIMARY KEY AUTOINCREMENT,recipient TEXT NOT NULL REFERENCES board_actors(id),event INTEGER NOT NULL REFERENCES board_events(id),read_at INTEGER,delivered_at INTEGER,UNIQUE(recipient,event));
CREATE INDEX IF NOT EXISTS board_notification_delivery ON board_notifications(recipient,delivered_at,id);
CREATE INDEX IF NOT EXISTS board_topic_channel ON board_topics(channel,id);
CREATE INDEX IF NOT EXISTS board_reply_topic ON board_replies(topic,id);
CREATE TABLE IF NOT EXISTS board_attachments(id TEXT PRIMARY KEY,owner TEXT NOT NULL REFERENCES board_actors(id),name TEXT NOT NULL,mime TEXT NOT NULL,size INTEGER NOT NULL,topic INTEGER REFERENCES board_topics(id),reply INTEGER REFERENCES board_replies(id));
CREATE TABLE IF NOT EXISTS board_requests(actor TEXT NOT NULL,request TEXT NOT NULL,operation TEXT NOT NULL,input TEXT NOT NULL,result TEXT NOT NULL,PRIMARY KEY(actor,request));
INSERT OR IGNORE INTO board_channels(name,description) VALUES('general','Shared conversations and work');
COMMIT;")?;
    Ok(())
}
fn actor_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Actor> {
    Ok(Actor {
        id: r.get(0)?,
        handle: r.get(1)?,
        name: r.get(2)?,
        kind: r.get(3)?,
    })
}
fn agent_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Agent> {
    let raw: String = r.get(6)?;
    Ok(Agent {
        id: r.get(0)?,
        handle: r.get(1)?,
        name: r.get(2)?,
        worker: r.get(3)?,
        thread: r.get(4)?,
        persona: r.get(5)?,
        settings: serde_json::from_str(&raw).unwrap_or(json!({})),
        paused: r.get(7)?,
        error: r.get(8)?,
    })
}
const AGENTS: &str = "SELECT a.id,a.handle,a.name,g.worker,g.thread,g.persona,g.settings,g.paused,g.error FROM board_agents g JOIN board_actors a ON a.id=g.id";
fn string<'a>(p: &'a Value, key: &str, max: usize) -> Result<&'a str> {
    let v = p[key]
        .as_str()
        .with_context(|| format!("{key} is required"))?;
    ensure!(v.len() <= max, "{key} is too long");
    Ok(v)
}
fn number(p: &Value, key: &str) -> Result<i64> {
    p[key]
        .as_i64()
        .filter(|n| *n > 0)
        .with_context(|| format!("{key} must be a positive integer"))
}
fn handle(v: &str) -> Result<String> {
    let v = v.trim().to_lowercase();
    ensure!(
        (2..=40).contains(&v.len())
            && v.bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c)),
        "Use 2–40 lowercase letters, digits, underscores or hyphens"
    );
    Ok(v)
}
fn topic(db: &Connection, id: i64) -> Result<Value> {
    db.query_row("SELECT t.id,t.channel,t.title,t.body,t.status,t.assignee,t.author,t.created_at,t.updated_at,c.name FROM board_topics t JOIN board_channels c ON c.id=t.channel WHERE t.id=?1",[id],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"channel":r.get::<_,i64>(1)?,"title":r.get::<_,String>(2)?,"body":r.get::<_,String>(3)?,"status":r.get::<_,String>(4)?,"assignee":r.get::<_,Option<String>>(5)?,"author":r.get::<_,String>(6)?,"createdAt":r.get::<_,i64>(7)?,"updatedAt":r.get::<_,i64>(8)?,"channelName":r.get::<_,String>(9)?}))).optional()?.context("Topic not found")
}
fn subscribe(tx: &Transaction<'_>, actor: &str, kind: &str, target: i64, yes: bool) -> Result<()> {
    tx.execute("INSERT INTO board_subscriptions VALUES(?1,?2,?3,?4) ON CONFLICT(actor,kind,target) DO UPDATE SET subscribed=excluded.subscribed",params![actor,kind,target,yes])?;
    Ok(())
}
fn attachments(db: &Connection, topic: i64, reply: Option<i64>) -> Result<Vec<Value>> {
    Ok(db.prepare("SELECT id,name,mime,size FROM board_attachments WHERE topic=?1 AND reply IS ?2 ORDER BY rowid")?.query_map(params![topic,reply],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"mime":r.get::<_,String>(2)?,"size":r.get::<_,i64>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)
}
fn attach(
    tx: &Transaction<'_>,
    actor: &str,
    topic: i64,
    reply: Option<i64>,
    p: &Value,
) -> Result<()> {
    if let Some(ids) = p.get("attachments") {
        let ids = ids.as_array().context("attachments must be an array")?;
        ensure!(ids.len() <= 10, "At most 10 attachments per post");
        for id in ids {
            let id = id.as_str().context("Invalid attachment id")?;
            ensure!(tx.execute("UPDATE board_attachments SET topic=?1,reply=?2 WHERE id=?3 AND owner=?4 AND topic IS NULL",params![topic,reply,id,actor])?==1,"Attachment unavailable or already posted");
        }
    }
    Ok(())
}
fn publish(
    tx: &Transaction<'_>,
    actor: &Actor,
    id: i64,
    kind: &str,
    body: &str,
    reply: Option<i64>,
) -> Result<()> {
    let t = topic(tx, id)?;
    let channel = t["channel"].as_i64().unwrap();
    subscribe(tx, &actor.id, "topic", id, true)?;
    let mut recipients = BTreeSet::new();

    // Mentions deliberately resubscribe. Match complete ASCII handles, never substrings.
    for word in body.split(|c: char| !(c.is_ascii_alphanumeric() || "@_.-".contains(c))) {
        if let Some(mut name) = word.strip_prefix('@') {
            loop {
                let mentioned = tx
                    .query_row(
                        "SELECT id FROM board_actors a WHERE handle=?1 AND (kind='agent' OR EXISTS(SELECT 1 FROM users u WHERE a.id='human:'||u.username))",
                        [name.to_lowercase()],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?;
                if let Some(id_actor) = mentioned {
                    subscribe(tx, &id_actor, "topic", id, true)?;
                    break;
                }
                // Try the complete handle first; only trailing sentence punctuation
                // may be removed. Never turn an unknown @bob.example into @bob.
                if let Some(shorter) = name.strip_suffix('.') {
                    name = shorter;
                } else {
                    break;
                }
            }
        }
    }
    for row in tx.prepare("SELECT actor FROM board_subscriptions WHERE subscribed=1 AND ((kind='topic' AND target=?1) OR (kind='channel' AND target=?2 AND ?3='created'))")?.query_map(params![id,channel,kind],|r|r.get::<_,String>(0))?{recipients.insert(row?);}
    recipients.remove(&actor.id);
    let payload = json!({"topic":id,"channel":channel,"channelName":t["channelName"],"title":t["title"],"kind":kind,"author":actor,"body":body,"status":t["status"],"assignee":t["assignee"],"reply":reply,"attachments":attachments(tx,id,reply)?});
    tx.execute(
        "INSERT INTO board_events(topic,actor,kind,payload,created_at) VALUES(?1,?2,?3,?4,?5)",
        params![id, actor.id, kind, payload.to_string(), now()],
    )?;
    let event = tx.last_insert_rowid();
    for recipient in recipients {
        tx.execute(
            "INSERT INTO board_notifications(recipient,event) VALUES(?1,?2)",
            params![recipient, event],
        )?;
    }
    Ok(())
}

impl Store {
    pub fn board_human(&self, username: &str) -> Result<Actor> {
        let db = self.0.lock().unwrap();
        db.execute("INSERT OR IGNORE INTO board_actors SELECT 'human:'||username,username,username,'human' FROM users",[])?;
        Ok(db.query_row(
            "SELECT id,handle,name,kind FROM board_actors WHERE id=?1",
            [format!("human:{username}")],
            actor_row,
        )?)
    }
    pub fn board_agent_auth(&self, token: &str) -> Result<Option<Actor>> {
        Ok(self.0.lock().unwrap().query_row("SELECT a.id,a.handle,a.name,a.kind FROM board_actors a JOIN board_agents g ON a.id=g.id WHERE g.token_hash=?1",[security::digest(token)],actor_row).optional()?)
    }
    pub fn board_agents(&self) -> Result<Vec<Agent>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .prepare(AGENTS)?
            .query_map([], agent_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn board_agent(&self, id: &str) -> Result<Agent> {
        self.board_agents()?
            .into_iter()
            .find(|a| a.id == id)
            .context("Agent not found")
    }
    pub fn board_add_agent(&self, a: &Agent, token: &str) -> Result<()> {
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        let h = handle(&a.handle)?;
        ensure!(
            !a.name.trim().is_empty() && a.name.len() <= 100,
            "Invalid name"
        );
        tx.execute(
            "INSERT INTO board_actors VALUES(?1,?2,?3,'agent')",
            params![a.id, h, a.name],
        )?;
        tx.execute("INSERT INTO board_agents(id,worker,thread,persona,settings,paused,token_hash) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![a.id,a.worker,a.thread,a.persona,a.settings.to_string(),a.paused,security::digest(token)])?;
        tx.commit()?;
        Ok(())
    }
    pub fn board_edit_agent(
        &self,
        id: &str,
        name: &str,
        persona: &str,
        paused: bool,
    ) -> Result<()> {
        ensure!(
            !name.trim().is_empty() && name.len() <= 100 && persona.len() <= 100000,
            "Invalid agent settings"
        );
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        ensure!(
            tx.execute(
                "UPDATE board_actors SET name=?1 WHERE id=?2 AND kind='agent'",
                params![name, id]
            )? == 1,
            "Agent not found"
        );
        tx.execute(
            "UPDATE board_agents SET persona=?1,paused=?2,error=NULL WHERE id=?3",
            params![persona, paused, id],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn board_agent_settings(&self, id: &str, settings: &Value) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE board_agents SET settings=?1 WHERE id=?2",
            params![settings.to_string(), id],
        )?;
        Ok(())
    }
    pub fn board_agent_error(&self, id: &str, error: Option<&str>) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE board_agents SET error=?1 WHERE id=?2",
            params![error, id],
        )?;
        Ok(())
    }
    pub fn board_ack(&self, actor: &str, ids: &[i64]) -> Result<()> {
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        for id in ids {
            tx.execute(
                "UPDATE board_notifications SET delivered_at=?1 WHERE id=?2 AND recipient=?3",
                params![now(), id, actor],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn board_pending(&self, actor: &str) -> Result<Vec<Value>> {
        notifications(&self.0.lock().unwrap(), actor, true, 0)
    }
    pub fn board_attachment(&self, actor: &Actor, id: &str) -> Result<(String, String, Vec<u8>)> {
        let db = self.0.lock().unwrap();
        let (owner, name, mime, published): (String, String, String, bool) = db
            .query_row(
                "SELECT owner,name,mime,topic IS NOT NULL FROM board_attachments WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
            .context("Attachment not found")?;
        ensure!(
            published || owner == actor.id,
            "Attachment is private until posted"
        );
        Ok((
            name,
            mime,
            std::fs::read(self.1.join("attachments").join(id))?,
        ))
    }
    pub fn board(&self, actor: &Actor, op: &str, p: &Value) -> Result<Value> {
        ensure!(p.is_object(), "Request must be an object");
        if op == "upload_attachment" {
            let data = STANDARD.decode(string(p, "data", MAX_FILE * 4 / 3 + 8)?)?;
            ensure!(data.len() <= MAX_FILE, "File exceeds 20 MB");
            let name = string(p, "name", 255)?
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("file");
            ensure!(
                !name.is_empty() && !name.chars().any(char::is_control),
                "Invalid file name"
            );
            let mime = string(p, "mime", 120)?;
            ensure!(
                mime.bytes().all(|c| c.is_ascii_graphic() || c == b' '),
                "Invalid content type"
            );
            let id = uuid::Uuid::new_v4().to_string();
            let dir = self.1.join("attachments");
            security::private_dir(&dir)?;
            security::private_write(&dir.join(&id), &data)?;
            if let Err(e) = self.0.lock().unwrap().execute(
                "INSERT INTO board_attachments(id,owner,name,mime,size) VALUES(?1,?2,?3,?4,?5)",
                params![id, actor.id, name, mime, data.len()],
            ) {
                let _ = std::fs::remove_file(dir.join(&id));
                return Err(e.into());
            }
            return Ok(json!({"id":id,"name":name,"mime":mime,"size":data.len()}));
        }
        if op == "read_attachment" {
            let id = string(p, "id", 100)?;
            let (name, mime, bytes) = self.board_attachment(actor, id)?;
            return Ok(json!({"id":id,"name":name,"mime":mime,"data":STANDARD.encode(bytes)}));
        }
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        let request = p["requestId"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 100);
        if let Some(request)=request && let Some((old_op,input,result))=tx.query_row("SELECT operation,input,result FROM board_requests WHERE actor=?1 AND request=?2",params![actor.id,request],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?{ensure!(old_op==op&&serde_json::from_str::<Value>(&input)?==*p,"Request id reused with different input");return Ok(serde_json::from_str(&result)?);}
        let result = match op {
            "agent_subscriptions" => {
                ensure!(actor.kind == "human", "Human account required");
                let id = string(p, "id", 100)?;
                let data=tx.prepare("SELECT target,subscribed FROM board_subscriptions WHERE actor=?1 AND kind='channel'")?.query_map([id],|r|Ok(json!({"target":r.get::<_,i64>(0)?,"subscribed":r.get::<_,bool>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
                json!({"data":data})
            }
            "overview" => {
                let people = tx
                    .prepare("SELECT a.id,a.handle,a.name,a.kind,(a.kind='agent' OR EXISTS(SELECT 1 FROM users u WHERE a.id='human:'||u.username)) FROM board_actors a ORDER BY kind,name")?
                    .query_map([],|r|{let mut value=json!(actor_row(r)?);value["active"]=json!(r.get::<_,bool>(4)?);Ok(value)})?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let channels=tx.prepare("SELECT c.id,c.name,c.description,COALESCE(s.subscribed,0) FROM board_channels c LEFT JOIN board_subscriptions s ON s.kind='channel' AND s.target=c.id AND s.actor=?1 ORDER BY c.name")?.query_map([&actor.id],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"name":r.get::<_,String>(1)?,"description":r.get::<_,String>(2)?,"subscribed":r.get::<_,bool>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
                json!({"me":actor,"people":people,"channels":channels,"statuses":STATUSES})
            }
            "create_channel" => {
                let name = handle(string(p, "name", 40)?)?;
                let description = p["description"].as_str().unwrap_or("");
                ensure!(description.len() <= 4000, "Description too long");
                tx.execute(
                    "INSERT INTO board_channels(name,description) VALUES(?1,?2)",
                    params![name, description],
                )?;
                json!({"id":tx.last_insert_rowid(),"name":name,"description":description})
            }
            "list_topics" => {
                let channel = p["channel"].as_i64();
                let before = p["before"].as_i64().unwrap_or(i64::MAX);
                let status = p["status"].as_str();
                let search = p["search"].as_str().unwrap_or("");
                ensure!(search.len() <= 200, "Search is too long");
                let ids=tx.prepare("SELECT id FROM board_topics WHERE (?1 IS NULL OR channel=?1) AND id<?2 AND (?3 IS NULL OR status=?3) AND (?4='' OR instr(lower(title||' '||body),lower(?4))>0 OR EXISTS(SELECT 1 FROM board_replies r WHERE r.topic=board_topics.id AND instr(lower(r.body),lower(?4))>0)) ORDER BY id DESC LIMIT 51")?.query_map(params![channel,before,status,search],|r|r.get::<_,i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                let mut data = Vec::new();
                for id in ids.iter().take(50) {
                    data.push(topic(&tx, *id)?);
                }
                json!({"data":data,"nextCursor":if ids.len()>50 {ids.get(49).copied()}else{None}})
            }
            "read_topic" => {
                let id = number(p, "topic")?;
                let mut t = topic(&tx, id)?;
                let before = p["before"].as_i64().unwrap_or(i64::MAX);
                let rows=tx.prepare("SELECT id,author,body,created_at FROM board_replies WHERE topic=?1 AND id<?2 ORDER BY id DESC LIMIT 101")?.query_map(params![id,before],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"author":r.get::<_,String>(1)?,"body":r.get::<_,String>(2)?,"createdAt":r.get::<_,i64>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
                let more = rows.len() > 100;
                let mut rows = rows.into_iter().take(100).collect::<Vec<_>>();
                for r in &mut rows {
                    r["attachments"] = json!(attachments(&tx, id, r["id"].as_i64())?);
                }
                rows.reverse();
                t["nextCursor"] = if more {
                    rows.first().map(|r| r["id"].clone()).unwrap_or(Value::Null)
                } else {
                    Value::Null
                };
                t["replies"] = json!(rows);
                t["attachments"] = json!(attachments(&tx, id, None)?);
                t["subscribed"]=json!(tx.query_row("SELECT subscribed FROM board_subscriptions WHERE actor=?1 AND kind='topic' AND target=?2",params![actor.id,id],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false));
                t
            }
            "create_topic" => {
                let channel = number(p, "channel")?;
                let body = string(p, "body", 200000)?;
                ensure!(
                    !body.trim().is_empty()
                        || p["attachments"].as_array().is_some_and(|a| !a.is_empty()),
                    "Add text or an attachment"
                );
                let title = p["title"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        body.lines()
                            .next()
                            .unwrap_or("Attachment")
                            .chars()
                            .take(100)
                            .collect()
                    });
                ensure!(title.len() <= 500, "Title is too long");
                tx.execute("INSERT INTO board_topics(channel,title,body,author,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?5)",params![channel,title,body,actor.id,now()])?;
                let id = tx.last_insert_rowid();
                attach(&tx, &actor.id, id, None, p)?;
                publish(&tx, actor, id, "created", body, None)?;
                topic(&tx, id)?
            }
            "post_reply" => {
                let id = number(p, "topic")?;
                topic(&tx, id)?;
                let body = string(p, "body", 200000)?;
                ensure!(
                    !body.trim().is_empty()
                        || p["attachments"].as_array().is_some_and(|a| !a.is_empty()),
                    "Add text or an attachment"
                );
                tx.execute(
                    "INSERT INTO board_replies(topic,author,body,created_at) VALUES(?1,?2,?3,?4)",
                    params![id, actor.id, body, now()],
                )?;
                let reply = tx.last_insert_rowid();
                attach(&tx, &actor.id, id, Some(reply), p)?;
                tx.execute(
                    "UPDATE board_topics SET updated_at=?1 WHERE id=?2",
                    params![now(), id],
                )?;
                publish(&tx, actor, id, "reply", body, Some(reply))?;
                json!({"id":reply,"topic":id})
            }
            "update_topic" => {
                let id = number(p, "topic")?;
                let old = topic(&tx, id)?;
                let title = p["title"]
                    .as_str()
                    .unwrap_or(old["title"].as_str().unwrap());
                ensure!(
                    !title.trim().is_empty() && title.len() <= 500,
                    "Invalid title"
                );
                let status = p["status"]
                    .as_str()
                    .unwrap_or(old["status"].as_str().unwrap());
                ensure!(STATUSES.contains(&status), "Invalid status");
                ensure!(
                    p.get("assignee")
                        .is_none_or(|v| v.is_null() || v.is_string()),
                    "Assignee must be an actor id or null"
                );
                let assignee = if p.get("assignee").is_some() {
                    p["assignee"].as_str()
                } else {
                    old["assignee"].as_str()
                };
                if assignee != old["assignee"].as_str()
                    && let Some(a) = assignee
                {
                    ensure!(tx.query_row("SELECT EXISTS(SELECT 1 FROM board_actors a WHERE id=?1 AND (kind='agent' OR EXISTS(SELECT 1 FROM users u WHERE a.id='human:'||u.username)))",[a],|r|r.get::<_,bool>(0))?,"Assignee is not an active participant");
                    subscribe(&tx, a, "topic", id, true)?;
                }
                tx.execute("UPDATE board_topics SET title=?1,status=?2,assignee=?3,updated_at=?4 WHERE id=?5",params![title,status,assignee,now(),id])?;
                publish(&tx, actor, id, "updated", "", None)?;
                topic(&tx, id)?
            }
            "set_subscription" => {
                let kind = string(p, "kind", 20)?;
                let target = number(p, "target")?;
                ensure!(
                    ["topic", "channel"].contains(&kind),
                    "Invalid subscription kind"
                );
                if kind == "topic" {
                    topic(&tx, target)?;
                } else {
                    ensure!(
                        tx.query_row(
                            "SELECT EXISTS(SELECT 1 FROM board_channels WHERE id=?1)",
                            [target],
                            |r| r.get::<_, bool>(0)
                        )?,
                        "Channel not found"
                    );
                }
                let yes = p["subscribed"]
                    .as_bool()
                    .context("subscribed is required")?;
                let recipient = p["actor"].as_str().unwrap_or(&actor.id);
                ensure!(
                    recipient == actor.id || actor.kind == "human",
                    "Only humans may manage another participant's subscriptions"
                );
                subscribe(&tx, recipient, kind, target, yes)?;
                json!({"subscribed":yes})
            }
            "list_notifications" => {
                json!({"data":notifications(&tx,&actor.id,false,p["before"].as_i64().unwrap_or(0))?})
            }
            "mark_read" => {
                let id = number(p, "id")?;
                tx.execute(
                    "UPDATE board_notifications SET read_at=?1 WHERE id=?2 AND recipient=?3",
                    params![now(), id, actor.id],
                )?;
                json!({"ok":true})
            }
            _ => bail!("Unknown board operation"),
        };
        if let Some(request) = request {
            tx.execute(
                "INSERT INTO board_requests VALUES(?1,?2,?3,?4,?5)",
                params![actor.id, request, op, p.to_string(), result.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(result)
    }
}
fn notifications(db: &Connection, actor: &str, pending: bool, before: i64) -> Result<Vec<Value>> {
    let sql = if pending {
        "SELECT n.id,e.payload,e.created_at,n.read_at FROM board_notifications n JOIN board_events e ON e.id=n.event WHERE n.recipient=?1 AND n.delivered_at IS NULL ORDER BY n.id LIMIT 100"
    } else {
        "SELECT n.id,e.payload,e.created_at,n.read_at FROM board_notifications n JOIN board_events e ON e.id=n.event WHERE n.recipient=?1 AND (?2=0 OR n.id<?2) ORDER BY n.id DESC LIMIT 100"
    };
    let mut s = db.prepare(sql)?;
    let row = |r: &rusqlite::Row<'_>| {
        let raw: String = r.get(1)?;
        let mut event = serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null);
        if let Some(body) = event["body"].as_str()
            && body.chars().count() > 2000
        {
            let preview = body.chars().take(2000).collect::<String>();
            event["body"] = json!(preview);
            event["bodyTruncated"] = json!(true);
        }
        Ok(
            json!({"id":r.get::<_,i64>(0)?,"event":event,"createdAt":r.get::<_,i64>(2)?,"read":r.get::<_,Option<i64>>(3)?.is_some()}),
        )
    };
    if pending {
        Ok(s.query_map([actor], row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    } else {
        Ok(s.query_map(params![actor, before], row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Store, Actor, Actor) {
        let dir = crate::test_support::tempdir();
        let store = Store::open(dir.path()).unwrap();
        store
            .set_user_password("alice", "unused-test-hash")
            .unwrap();
        store.set_user_password("bob", "unused-test-hash").unwrap();
        let alice = store.board_human("alice").unwrap();
        let bob = store.board_human("bob").unwrap();
        (dir, store, alice, bob)
    }
    #[test]
    fn subscriptions_mentions_assignments_and_read_state_are_independent() {
        let (_dir, s, a, b) = setup();
        s.board(
            &b,
            "set_subscription",
            &json!({"kind":"channel","target":1,"subscribed":true}),
        )
        .unwrap();
        let t = s
            .board(
                &a,
                "create_topic",
                &json!({"channel":1,"body":"Discuss a plan"}),
            )
            .unwrap();
        let id = t["id"].as_i64().unwrap();
        assert_eq!(t["status"], "discussion");
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 1);
        assert!(s.board_pending(&a.id).unwrap().is_empty());
        s.board(
            &a,
            "post_reply",
            &json!({"topic":id,"body":"Only channel subscribers do not get replies"}),
        )
        .unwrap();
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 1);
        s.board(&a, "post_reply", &json!({"topic":id,"body":"Hello @bob"}))
            .unwrap();
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 2);
        s.board(
            &b,
            "set_subscription",
            &json!({"kind":"topic","target":id,"subscribed":false}),
        )
        .unwrap();
        s.board(
            &a,
            "post_reply",
            &json!({"topic":id,"body":"Hello @bobby is not a mention of bob"}),
        )
        .unwrap();
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 2);
        s.board(&a, "post_reply", &json!({"topic":id,"body":"Again @bob"}))
            .unwrap();
        s.board(
            &a,
            "post_reply",
            &json!({"topic":id,"body":"Subscribed again"}),
        )
        .unwrap();
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 4);
        let note = s.board_pending(&b.id).unwrap()[0]["id"].as_i64().unwrap();
        s.board(&b, "mark_read", &json!({"id":note})).unwrap();
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 4);
        s.board_ack(&b.id, &[note]).unwrap();
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 3);
        let inbox = s.board(&b, "list_notifications", &json!({})).unwrap();
        assert_eq!(inbox["data"].as_array().unwrap().len(), 4);
        s.board(
            &b,
            "set_subscription",
            &json!({"kind":"topic","target":id,"subscribed":false}),
        )
        .unwrap();
        s.board(
            &a,
            "update_topic",
            &json!({"topic":id,"assignee":b.id,"status":"working"}),
        )
        .unwrap();
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 4);
        let t = s.board(&b, "read_topic", &json!({"topic":id})).unwrap();
        assert_eq!(t["subscribed"], true);
        assert_eq!(t["assignee"], b.id);
    }
    #[test]
    fn mutation_rollback_idempotency_and_attachment_privacy_survive_reopen() {
        let (dir, s, a, b) = setup();
        let f=s.board(&a,"upload_attachment",&json!({"name":"notes.txt","mime":"text/plain","data":STANDARD.encode(b"private draft")})).unwrap();
        let fid = f["id"].as_str().unwrap();
        assert!(s.board_attachment(&b, fid).is_err());
        assert_eq!(s.board_attachment(&a, fid).unwrap().2, b"private draft");
        let bad = json!({"channel":1,"body":"@bob","attachments":[fid,"missing"]});
        assert!(s.board(&a, "create_topic", &bad).is_err());
        assert!(s.board_pending(&b.id).unwrap().is_empty());
        assert!(
            s.board(&a, "list_topics", &json!({})).unwrap()["data"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(s.board_attachment(&b, fid).is_err());
        let p = json!({"channel":1,"body":"@bob review this","attachments":[fid],"requestId":"stable-create"});
        let first = s.board(&a, "create_topic", &p).unwrap();
        assert_eq!(s.board(&a, "create_topic", &p).unwrap(), first);
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 1);
        assert_eq!(s.board_attachment(&b, fid).unwrap().2, b"private draft");
        assert!(
            s.board(
                &a,
                "create_topic",
                &json!({"channel":1,"body":"different","requestId":"stable-create"})
            )
            .is_err()
        );
        drop(s);
        let s = Store::open(dir.path()).unwrap();
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 1);
        assert_eq!(s.board(&a, "create_topic", &p).unwrap(), first);
        let page = s
            .board(&b, "read_topic", &json!({"topic":first["id"]}))
            .unwrap();
        assert_eq!(page["attachments"][0]["id"], fid);
    }
    #[test]
    fn agents_share_board_capabilities_but_cannot_manage_another_identity() {
        let (_dir, s, a, b) = setup();
        let key = s.enrollment().unwrap();
        let (worker, _) = s.enroll(&key, "worker").unwrap();
        let token = security::token();
        let id = uuid::Uuid::new_v4().to_string();
        s.board_add_agent(
            &Agent {
                id: id.clone(),
                handle: "sage".into(),
                name: "Sage".into(),
                worker,
                thread: "thread".into(),
                persona: "Be thoughtful".into(),
                settings: json!({}),
                paused: false,
                error: None,
            },
            &token,
        )
        .unwrap();
        let agent = s.board_agent_auth(&token).unwrap().unwrap();
        assert_eq!(agent.kind, "agent");
        assert!(s.board_agent_auth("wrong").unwrap().is_none());
        let t = s
            .board(
                &agent,
                "create_topic",
                &json!({"channel":1,"body":"Hi @alice"}),
            )
            .unwrap();
        assert_eq!(t["author"], id);
        s.board(
            &agent,
            "update_topic",
            &json!({"topic":t["id"],"status":"working","assignee":b.id}),
        )
        .unwrap();
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 1);
        assert!(
            s.board(
                &agent,
                "set_subscription",
                &json!({"actor":a.id,"kind":"channel","target":1,"subscribed":true})
            )
            .is_err()
        );
        s.board(
            &a,
            "set_subscription",
            &json!({"actor":agent.id,"kind":"channel","target":1,"subscribed":true}),
        )
        .unwrap();
        assert!(
            s.board(&agent, "agent_subscriptions", &json!({"id":id}))
                .is_err()
        );
        let public = s.board(&a, "overview", &json!({})).unwrap().to_string();
        assert!(!public.contains(&token));
        assert!(!public.contains("token_hash"));
    }
    #[test]
    fn revoking_a_worker_disables_its_agent_tokens_and_preserves_board_history() {
        let (_dir, s, h, _) = setup();
        let key = s.enrollment().unwrap();
        let (worker, credential) = s.enroll(&key, "worker").unwrap();
        let mut tokens = Vec::new();
        for name in ["sage", "spark"] {
            let token = security::token();
            let id = uuid::Uuid::new_v4().to_string();
            s.board_add_agent(
                &Agent {
                    id,
                    handle: name.into(),
                    name: name.into(),
                    worker: worker.clone(),
                    thread: name.into(),
                    persona: String::new(),
                    settings: json!({}),
                    paused: false,
                    error: None,
                },
                &token,
            )
            .unwrap();
            let a = s.board_agent_auth(&token).unwrap().unwrap();
            s.board(
                &a,
                "create_topic",
                &json!({"channel":1,"body":"Keep this history"}),
            )
            .unwrap();
            tokens.push(token);
        }
        s.revoke(&worker).unwrap();
        assert!(s.worker_for_token(&credential).unwrap().is_none());
        for token in tokens {
            assert!(s.board_agent_auth(&token).unwrap().is_none());
        }
        assert!(s.board_agents().unwrap().iter().all(|a| a.paused));
        assert_eq!(
            s.board(&h, "list_topics", &json!({})).unwrap()["data"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn dotted_mentions_match_complete_handles_before_sentence_punctuation() {
        let (_dir, s, a, b) = setup();
        s.set_user_password("bob.example", "a-test-password")
            .unwrap();
        let dotted = s.board_human("bob.example").unwrap();
        let t = s
            .board(
                &a,
                "create_topic",
                &json!({
                    "channel": 1, "body": "@bob.example. and @bob."
                }),
            )
            .unwrap();
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 1);
        assert_eq!(s.board_pending(&dotted.id).unwrap().len(), 1);
        s.board(
            &b,
            "set_subscription",
            &json!({"kind":"topic","target":t["id"],"subscribed":false}),
        )
        .unwrap();
        s.board(
            &a,
            "post_reply",
            &json!({
                "topic":t["id"], "body":"@bob.unknown and mail@bob.example"
            }),
        )
        .unwrap();
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 1);
    }

    #[test]
    fn removing_a_human_retains_authorship_but_stops_mentions_and_new_assignment() {
        let (_dir, s, a, b) = setup();
        let t = s
            .board(
                &a,
                "create_topic",
                &json!({"channel":1,"body":"Keep my authorship"}),
            )
            .unwrap();
        s.board(
            &b,
            "update_topic",
            &json!({"topic":t["id"],"assignee":a.id}),
        )
        .unwrap();
        let before = s.board_pending(&a.id).unwrap().len();
        s.remove_user("alice").unwrap();
        s.board(
            &b,
            "post_reply",
            &json!({"topic":t["id"],"body":"@alice is no longer an active account"}),
        )
        .unwrap();
        assert_eq!(s.board_pending(&a.id).unwrap().len(), before);
        let overview = s.board(&b, "overview", &json!({})).unwrap();
        assert_eq!(
            overview["people"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == a.id)
                .unwrap()["active"],
            false
        );
        assert_eq!(
            s.board(&b, "read_topic", &json!({"topic":t["id"]}))
                .unwrap()["author"],
            a.id
        );
        s.board(
            &b,
            "update_topic",
            &json!({"topic":t["id"],"assignee":null}),
        )
        .unwrap();
        assert!(
            s.board(
                &b,
                "update_topic",
                &json!({"topic":t["id"],"assignee":a.id})
            )
            .is_err()
        );
    }
    #[test]
    fn topic_and_reply_pagination_does_not_lose_hidden_or_older_work() {
        let (_dir, s, a, _) = setup();
        for i in 0..55 {
            s.board(
                &a,
                "create_topic",
                &json!({"channel":1,"body":format!("Topic {i}")}),
            )
            .unwrap();
        }
        let first = s.board(&a, "list_topics", &json!({"channel":1})).unwrap();
        assert_eq!(first["data"].as_array().unwrap().len(), 50);
        let second = s
            .board(
                &a,
                "list_topics",
                &json!({"channel":1,"before":first["nextCursor"]}),
            )
            .unwrap();
        assert_eq!(second["data"].as_array().unwrap().len(), 5);
        for i in 0..105 {
            s.board(
                &a,
                "post_reply",
                &json!({"topic":1,"body":format!("Reply {i}")}),
            )
            .unwrap();
        }
        let page = s.board(&a, "read_topic", &json!({"topic":1})).unwrap();
        assert_eq!(page["replies"].as_array().unwrap().len(), 100);
        let older = s
            .board(
                &a,
                "read_topic",
                &json!({"topic":1,"before":page["nextCursor"]}),
            )
            .unwrap();
        assert_eq!(older["replies"].as_array().unwrap().len(), 5);
        assert_eq!(
            s.board(&a, "list_topics", &json!({"search":"Reply 104"}))
                .unwrap()["data"][0]["id"],
            1
        );
    }
}
