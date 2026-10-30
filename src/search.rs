//! Board search and a private worker-local full-text session index.
use crate::{board::Actor, codex::Codex};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::{collections::HashSet, io::BufRead, path::Path, sync::Arc};

pub fn board_search(tx: &Transaction<'_>, actor: &Actor, p: &Value) -> Result<Value> {
    let q = p["query"].as_str().unwrap_or("").trim();
    ensure!(
        (2..=200).contains(&q.chars().count()),
        "Search needs 2–200 characters"
    );
    let offset = p["offset"].as_u64().unwrap_or(0).min(100000) as i64;
    let data=tx.prepare("SELECT kind,id,topic,title,body,channel,channel_name FROM (
SELECT 'topic' kind,t.id id,t.id topic,t.title title,t.body body,t.channel channel,c.name channel_name FROM board_topics t JOIN board_channels c ON c.id=t.channel
UNION ALL SELECT 'reply',r.id,t.id,t.title,r.body,t.channel,c.name FROM board_replies r JOIN board_topics t ON t.id=r.topic JOIN board_channels c ON c.id=t.channel
UNION ALL SELECT 'attachment',a.rowid,t.id,t.title,a.name,t.channel,c.name FROM board_attachments a JOIN board_topics t ON t.id=a.topic JOIN board_channels c ON c.id=t.channel
UNION ALL SELECT 'reminder',r.id,r.topic,'Personal reminder',r.message,t.channel,c.name FROM board_reminders r LEFT JOIN board_topics t ON t.id=r.topic LEFT JOIN board_channels c ON c.id=t.channel WHERE r.owner=?1
) WHERE instr(lower(title||' '||body),lower(?2))>0 ORDER BY kind,id DESC LIMIT 51 OFFSET ?3")?.query_map(params![actor.id,q,offset],|r|Ok(json!({"kind":r.get::<_,String>(0)?,"id":r.get::<_,i64>(1)?,"topic":r.get::<_,Option<i64>>(2)?,"title":r.get::<_,String>(3)?,"body":r.get::<_,String>(4)?,"channel":r.get::<_,Option<i64>>(5)?,"channelName":r.get::<_,Option<String>>(6)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut results = data.into_iter().take(51).collect::<Vec<_>>();
    let more = results.len() > 50;
    results.truncate(50);
    for row in &mut results {
        row["body"] = json!(excerpt(row["body"].as_str().unwrap_or(""), q, 360));
    }
    Ok(json!({"data":results,"nextOffset":if more {Some(offset+50)}else{None}}))
}
fn excerpt(text: &str, query: &str, length: usize) -> String {
    let lower = text.to_lowercase();
    let byte = lower.find(&query.to_lowercase()).unwrap_or(0);
    // Lowercasing can change UTF-8 lengths, so use a bounded character offset.
    let start = lower[..byte].chars().count().saturating_sub(80);
    let out = text.chars().skip(start).take(length).collect::<String>();
    format!(
        "{}{out}{}",
        if start > 0 { "… " } else { "" },
        if text.chars().count() > start + length {
            " …"
        } else {
            ""
        }
    )
}
pub struct Index {
    db: Connection,
    refreshed: Option<std::time::Instant>,
    unavailable: Vec<Value>,
    incomplete: bool,
}
impl Index {
    pub fn open(path: Option<&Path>) -> Result<Self> {
        let db = match path {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                // The index contains private session messages.
                if !path.exists() {
                    crate::security::private_write(path, &[])?;
                }
                Connection::open(path)?
            }
            None => Connection::open_in_memory()?,
        };
        db.execute_batch("PRAGMA journal_mode=DELETE;
CREATE TABLE IF NOT EXISTS files(thread TEXT PRIMARY KEY,title TEXT NOT NULL,cwd TEXT NOT NULL,stamp TEXT NOT NULL);
CREATE VIRTUAL TABLE IF NOT EXISTS messages USING fts5(thread UNINDEXED,role UNINDEXED,body,tokenize='unicode61');")?;
        Ok(Self {
            db,
            refreshed: None,
            unavailable: Vec::new(),
            incomplete: false,
        })
    }
    fn record(&mut self, t: &Value, fallback: Option<&Value>) -> Result<bool> {
        let id = t["id"].as_str().context("Session id missing")?;
        let title = t["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or(t["preview"].as_str())
            .unwrap_or(id);
        let cwd = t["cwd"].as_str().unwrap_or("");
        let path = t["path"].as_str().map(Path::new);
        let metadata = path.and_then(|p| std::fs::metadata(p).ok());
        let stamp = format!(
            "{:?}:{}:{}:{}:{}:{}",
            metadata.as_ref().and_then(|m| m.modified().ok()),
            metadata.as_ref().map(|m| m.len()).unwrap_or(0),
            t["updatedAt"],
            t["name"],
            t["preview"],
            t["cwd"]
        );
        let old = self
            .db
            .query_row("SELECT stamp FROM files WHERE thread=?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .optional()?;
        if old.as_deref() == Some(&stamp) && fallback.is_none() {
            self.db.execute(
                "UPDATE files SET title=?1,cwd=?2 WHERE thread=?3",
                params![title, cwd, id],
            )?;
            return Ok(true);
        }
        let mut messages = Vec::new();
        let mut readable = false;
        if let Some(path) = path
            && let Ok(file) = std::fs::File::open(path)
        {
            readable = true;
            for line in std::io::BufReader::new(file).lines() {
                let line = line?;
                let Ok(v) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if v["type"] == "response_item" && v["payload"]["type"] == "message" {
                    collect_message(&v["payload"], &mut messages);
                }
            }
        } else if let Some(thread) = fallback {
            readable = true;
            for turn in thread["turns"].as_array().into_iter().flatten() {
                for item in turn["items"].as_array().into_iter().flatten() {
                    collect_message(item, &mut messages);
                }
            }
        }
        if !readable {
            return Ok(false);
        }
        let tx = self.db.transaction()?;
        tx.execute("DELETE FROM messages WHERE thread=?1", [id])?;
        tx.execute(
            "INSERT INTO messages(thread,role,body) VALUES(?1,'session',?2)",
            params![
                id,
                format!("{title}\n{cwd}\n{}", t["preview"].as_str().unwrap_or(""))
            ],
        )?;
        for (role, text) in messages {
            tx.execute(
                "INSERT INTO messages(thread,role,body) VALUES(?1,?2,?3)",
                params![id, role, text],
            )?;
        }
        tx.execute("INSERT INTO files VALUES(?1,?2,?3,?4) ON CONFLICT(thread) DO UPDATE SET title=excluded.title,cwd=excluded.cwd,stamp=excluded.stamp",params![id,title,cwd,stamp])?;
        tx.commit()?;
        Ok(true)
    }
    fn query(&self, p: &Value) -> Result<Value> {
        let q = p["query"].as_str().unwrap_or("").trim();
        ensure!(
            (2..=200).contains(&q.chars().count()),
            "Search needs 2–200 characters"
        );
        let query = q
            .split_whitespace()
            .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        let offset = p["offset"].as_u64().unwrap_or(0).min(100000) as i64;
        let mut data=self.db.prepare("SELECT m.rowid,m.thread,m.role,snippet(messages,2,'','',' … ',40),f.title,f.cwd FROM messages m JOIN files f ON f.thread=m.thread WHERE messages MATCH ?1 ORDER BY rank,m.rowid LIMIT 51 OFFSET ?2")?.query_map(params![query,offset],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"thread":r.get::<_,String>(1)?,"role":r.get::<_,String>(2)?,"body":r.get::<_,String>(3)?,"title":r.get::<_,String>(4)?,"cwd":r.get::<_,String>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let more = data.len() > 50;
        data.truncate(50);
        Ok(json!({"data":data,"nextOffset":if more{Some(offset+50)}else{None}}))
    }
}
fn collect_message(item: &Value, out: &mut Vec<(String, String)>) {
    let role = item["role"]
        .as_str()
        .or_else(|| match item["type"].as_str() {
            Some("userMessage") => Some("user"),
            Some("agentMessage") => Some("assistant"),
            _ => None,
        });
    let Some(role @ ("user" | "assistant")) = role else {
        return;
    };
    let mut text = item["text"].as_str().unwrap_or("").to_owned();
    for c in item["content"].as_array().into_iter().flatten() {
        if let Some(t) = c["text"].as_str() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(t);
        }
    }
    if !text.is_empty() {
        out.push((role.into(), text));
    }
}
pub async fn sessions(codex: Arc<Codex>, p: Value) -> Result<Value> {
    // Validate before any discovery or disk work.
    let query = p["query"].as_str().unwrap_or("").trim();
    ensure!(
        (2..=200).contains(&query.chars().count()),
        "Search needs 2–200 characters"
    );
    let _gate = codex.search_gate.lock().await;
    let refresh = p["offset"].as_u64().unwrap_or(0) == 0
        && codex
            .search
            .lock()
            .unwrap()
            .refreshed
            .is_none_or(|t| t.elapsed().as_secs() >= 5);
    let started = std::time::Instant::now();
    let mut unavailable = Vec::new();
    let mut incomplete = false;
    if refresh {
        let mut seen = HashSet::new();
        let mut listed = Vec::new();
        for archived in [false, true] {
            let mut cursor = Value::Null;
            for page in 0..1000 {
                let result = codex
                    .list_sessions(json!({"limit":100,"cursor":cursor,"archived":archived}))
                    .await?;
                for t in result["data"].as_array().context("Invalid session list")? {
                    if let Some(id) = t["id"].as_str()
                        && seen.insert(id.to_owned())
                    {
                        listed.push(t.clone());
                    }
                }
                cursor = result["nextCursor"].clone();
                if cursor.is_null() {
                    break;
                }
                if page == 999 {
                    incomplete = true;
                }
            }
        }
        for t in &listed {
            if started.elapsed().as_secs() >= 20 {
                incomplete = true;
                break;
            }
            let index = codex.search.clone();
            let metadata = t.clone();
            let indexed = match tokio::task::spawn_blocking(move || {
                index.lock().unwrap().record(&metadata, None)
            })
            .await?
            {
                Ok(value) => value,
                Err(_) => {
                    unavailable.push(t["id"].clone());
                    continue;
                }
            };
            if !indexed {
                let mut turns = Vec::new();
                let mut cursor = Value::Null;
                let outcome=async {
                    loop {
                        let page=codex.call("thread/turns/list",json!({"threadId":t["id"],"cursor":cursor,"itemsView":"full","sortDirection":"desc","limit":100})).await?;
                        turns.extend(page["data"].as_array().context("Invalid session history")?.iter().cloned());
                        cursor=page["nextCursor"].clone();
                        if cursor.is_null(){break;}
                    }
                    Ok::<_,anyhow::Error>(())
                }.await;
                if outcome.is_ok() {
                    let index = codex.search.clone();
                    let metadata = t.clone();
                    let indexed = tokio::task::spawn_blocking(move || {
                        index
                            .lock()
                            .unwrap()
                            .record(&metadata, Some(&json!({"turns":turns})))
                    })
                    .await?;
                    if indexed.is_err() {
                        unavailable.push(t["id"].clone());
                    }
                } else {
                    unavailable.push(t["id"].clone());
                }
            }
        }
        let mut index = codex.search.lock().unwrap();
        if !incomplete {
            let old = index
                .db
                .prepare("SELECT thread FROM files")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let tx = index.db.transaction()?;
            for id in old.into_iter().filter(|id| !seen.contains(id)) {
                tx.execute("DELETE FROM messages WHERE thread=?1", [&id])?;
                tx.execute("DELETE FROM files WHERE thread=?1", [&id])?;
            }
            tx.commit()?;
        }
        index.refreshed = if incomplete {
            None
        } else {
            Some(std::time::Instant::now())
        };
        index.unavailable = unavailable;
        index.incomplete = incomplete;
    }
    let index = codex.search.lock().unwrap();
    let mut result = index.query(&p)?;
    result["unavailable"] = json!(index.unavailable);
    result["incomplete"] = json!(index.incomplete);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn board_search_includes_replies_and_published_names_but_only_own_reminders() {
        let d = crate::test_support::tempdir();
        let s = crate::store::Store::open(d.path()).unwrap();
        s.set_user_password("alice", "hash").unwrap();
        s.set_user_password("bob", "hash").unwrap();
        let a = s.board_human("alice").unwrap();
        let b = s.board_human("bob").unwrap();
        let t = s
            .board(
                &a,
                "create_topic",
                &json!({"channel":1,"title":"Engine design","body":"Opening"}),
            )
            .unwrap();
        s.board(
            &a,
            "post_reply",
            &json!({"topic":t["id"],"body":"copper reactor reply"}),
        )
        .unwrap();
        let file = s
            .board(
                &a,
                "upload_attachment",
                &json!({"name":"copper-report.txt","mime":"text/plain","data":"dGVzdA=="}),
            )
            .unwrap();
        let search = |actor: &Actor| {
            s.board(actor, "search", &json!({"query":"copper"}))
                .unwrap()
        };
        assert_eq!(search(&a)["data"].as_array().unwrap().len(), 1);
        s.board(
            &a,
            "post_reply",
            &json!({"topic":t["id"],"body":"Evidence","attachments":[file["id"]]}),
        )
        .unwrap();
        s.board(
            &a,
            "create_reminder",
            &json!({"message":"copper private","dueAt":crate::now()+60}),
        )
        .unwrap();
        assert_eq!(search(&a)["data"].as_array().unwrap().len(), 3);
        assert_eq!(search(&b)["data"].as_array().unwrap().len(), 2);
        for i in 0..55 {
            s.board(
                &a,
                "post_reply",
                &json!({"topic":t["id"],"body":format!("Needle {i}")}),
            )
            .unwrap();
        }
        let first = s.board(&a, "search", &json!({"query":"Needle"})).unwrap();
        assert_eq!(first["data"].as_array().unwrap().len(), 50);
        assert_eq!(
            s.board(
                &a,
                "search",
                &json!({"query":"Needle","offset":first["nextOffset"]})
            )
            .unwrap()["data"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
    }
    #[test]
    fn worker_index_reads_real_rollout_shape_ignores_instructions_and_updates_after_append() {
        use std::io::Write;
        let d = crate::test_support::tempdir();
        let file = d.path().join("rollout.jsonl");
        let db = d.path().join("search.sqlite3");
        std::fs::write(&file,concat!(
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"developer\",\"content\":[{\"text\":\"secretpersona\"}]}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Design the copper reactor\"}]}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Copper cooling proposal\"}]}}\n")).unwrap();
        let mut idx = Index::open(Some(&db)).unwrap();
        let t =
            json!({"id":"session","name":"Design","cwd":"/another/root","path":file,"updatedAt":1});
        assert!(idx.record(&t, None).unwrap());
        let hits = idx.query(&json!({"query":"copper"})).unwrap();
        assert_eq!(hits["data"].as_array().unwrap().len(), 2);
        assert_eq!(hits["data"][0]["cwd"], "/another/root");
        assert!(
            idx.query(&json!({"query":"secretpersona"})).unwrap()["data"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let count: i64 = idx
            .db
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        idx.record(&t, None).unwrap();
        assert_eq!(
            idx.db
                .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            count
        );
        writeln!(std::fs::OpenOptions::new().append(true).open(&file).unwrap(),"{}",json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"New appended mercury"}]}})).unwrap();
        idx.record(&t, None).unwrap();
        drop(idx);
        let idx = Index::open(Some(&db)).unwrap();
        assert_eq!(
            idx.query(&json!({"query":"mercury"})).unwrap()["data"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(db).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
