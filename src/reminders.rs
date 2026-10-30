//! Private schedules deliver through the same durable inbox as board activity.
use crate::{board::Actor, store::Store};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};

pub fn migrate(db: &Connection) -> Result<()> {
    // Older events always belonged to a topic. Personal reminders may have none.
    let required: bool = db.query_row(
        "SELECT \"notnull\" FROM pragma_table_info('board_events') WHERE name='topic'",
        [],
        |r| r.get(0),
    )?;
    if required {
        db.execute_batch("PRAGMA foreign_keys=OFF; BEGIN IMMEDIATE;")?;
        let result = db.execute_batch("
CREATE TABLE board_events_new(id INTEGER PRIMARY KEY AUTOINCREMENT,topic INTEGER REFERENCES board_topics(id),actor TEXT NOT NULL REFERENCES board_actors(id),kind TEXT NOT NULL,payload TEXT NOT NULL,created_at INTEGER NOT NULL);
INSERT INTO board_events_new SELECT * FROM board_events;
DROP TABLE board_events;
ALTER TABLE board_events_new RENAME TO board_events;
COMMIT;");
        if result.is_err() {
            let _ = db.execute_batch("ROLLBACK;");
        }
        db.execute_batch("PRAGMA foreign_keys=ON;")?;
        result?;
    }
    db.execute_batch("
CREATE TABLE IF NOT EXISTS board_reminders(id INTEGER PRIMARY KEY AUTOINCREMENT,owner TEXT NOT NULL REFERENCES board_actors(id),message TEXT NOT NULL,topic INTEGER REFERENCES board_topics(id),due_at INTEGER NOT NULL,repeat_seconds INTEGER,state TEXT NOT NULL DEFAULT 'active' CHECK(state IN ('active','completed','cancelled')),created_at INTEGER NOT NULL,last_fired_at INTEGER);
CREATE INDEX IF NOT EXISTS reminder_due ON board_reminders(state,due_at);
CREATE INDEX IF NOT EXISTS reminder_owner ON board_reminders(owner,id);")?;
    Ok(())
}
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":r.get::<_,i64>(0)?,"message":r.get::<_,String>(1)?,"topic":r.get::<_,Option<i64>>(2)?,"dueAt":r.get::<_,i64>(3)?,"repeatSeconds":r.get::<_,Option<i64>>(4)?,"state":r.get::<_,String>(5)?,"createdAt":r.get::<_,i64>(6)?,"lastFiredAt":r.get::<_,Option<i64>>(7)?}),
    )
}
const SELECT: &str = "SELECT id,message,topic,due_at,repeat_seconds,state,created_at,last_fired_at FROM board_reminders";
pub fn operation(tx: &Transaction<'_>, actor: &Actor, op: &str, p: &Value) -> Result<Value> {
    let current = crate::now();
    let id = p["id"].as_i64().unwrap_or(0);
    if op == "read_reminder" {
        return tx
            .query_row(
                &format!("{SELECT} WHERE id=?1 AND owner=?2"),
                params![id, actor.id],
                row,
            )
            .optional()?
            .context("Reminder not found");
    }
    if op == "list_reminders" {
        let before = p["before"].as_i64().unwrap_or(i64::MAX);
        let history = p["history"].as_bool().unwrap_or(false);
        let data = tx.prepare(&format!("{SELECT} WHERE owner=?1 AND id<?2 AND (?3 OR state='active') ORDER BY id DESC LIMIT 51"))?.query_map(params![actor.id,before,history],row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        return Ok(
            json!({"data":data.iter().take(50).collect::<Vec<_>>(),"nextCursor":if data.len()>50 {Some(data[49]["id"].as_i64().unwrap())}else{None}}),
        );
    }
    if op == "cancel_reminder" {
        ensure!(tx.execute("UPDATE board_reminders SET state='cancelled' WHERE id=?1 AND owner=?2 AND state='active'",params![id,actor.id])?==1,"Active reminder not found");
    } else {
        let message = p["message"].as_str().context("message is required")?;
        ensure!(
            !message.trim().is_empty() && message.len() <= 20000,
            "Message must contain 1–20000 bytes"
        );
        let due = p["dueAt"]
            .as_i64()
            .context("dueAt must be a UTC Unix timestamp in seconds")?;
        ensure!(
            due > current && due <= current + 315360000,
            "Choose a future time within ten years"
        );
        let repeat = if p["repeatSeconds"].is_null() {
            None
        } else {
            let value = p["repeatSeconds"]
                .as_i64()
                .context("repeatSeconds must be an integer")?;
            ensure!(
                (1..=31536000).contains(&value),
                "Repeat interval must be 1 second to 365 days"
            );
            Some(value)
        };
        let topic = if p["topic"].is_null() {
            None
        } else {
            let value = p["topic"].as_i64().context("Invalid topic")?;
            ensure!(
                tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM board_topics WHERE id=?1)",
                    [value],
                    |r| r.get::<_, bool>(0)
                )?,
                "Topic not found"
            );
            Some(value)
        };
        if op == "create_reminder" {
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM board_reminders WHERE owner=?1 AND state='active'",
                [&actor.id],
                |r| r.get(0),
            )?;
            ensure!(count < 1000, "At most 1000 active reminders per person");
            tx.execute("INSERT INTO board_reminders(owner,message,topic,due_at,repeat_seconds,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![actor.id,message,topic,due,repeat,current])?;
        } else {
            ensure!(tx.execute("UPDATE board_reminders SET message=?1,topic=?2,due_at=?3,repeat_seconds=?4 WHERE id=?5 AND owner=?6 AND state='active'",params![message,topic,due,repeat,id,actor.id])?==1,"Active reminder not found");
        }
    }
    let id = if op == "create_reminder" {
        tx.last_insert_rowid()
    } else {
        id
    };
    Ok(tx.query_row(
        &format!("{SELECT} WHERE id=?1 AND owner=?2"),
        params![id, actor.id],
        row,
    )?)
}
impl Store {
    pub fn fire_reminders(&self, current: i64) -> Result<usize> {
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        let due=tx.prepare("SELECT r.id,r.owner,r.message,r.topic,r.due_at,r.repeat_seconds,a.handle,a.name,a.kind FROM board_reminders r JOIN board_actors a ON a.id=r.owner WHERE r.state='active' AND r.due_at<=?1 AND (a.kind='agent' AND EXISTS(SELECT 1 FROM board_agents g JOIN workers w ON w.id=g.worker WHERE g.id=a.id) OR a.kind='human' AND EXISTS(SELECT 1 FROM users u WHERE a.id='human:'||u.username)) ORDER BY r.due_at,r.id LIMIT 100")?.query_map([current],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<i64>>(3)?,r.get::<_,i64>(4)?,r.get::<_,Option<i64>>(5)?,Actor{id:r.get(1)?,handle:r.get(6)?,name:r.get(7)?,kind:r.get(8)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, owner, message, topic, due, repeat, actor) in &due {
            let linked=topic.map(|id|tx.query_row("SELECT t.title,t.channel,c.name FROM board_topics t JOIN board_channels c ON c.id=t.channel WHERE t.id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?))).optional()).transpose()?.flatten();
            let (title, channel, channel_name) = match linked {
                Some((title, c, n)) => (title, Some(c), Some(n)),
                None => ("Personal reminder".into(), None, None),
            };
            let payload = json!({"kind":"reminder","reminder":id,"dueAt":due,"topic":topic,"channel":channel,"channelName":channel_name,"title":title,"body":message,"author":actor,"attachments":[]});
            tx.execute("INSERT INTO board_events(topic,actor,kind,payload,created_at) VALUES(?1,?2,'reminder',?3,?4)",params![topic,owner,payload.to_string(),current])?;
            let event = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO board_notifications(recipient,event) VALUES(?1,?2)",
                params![owner, event],
            )?;
            if let Some(interval) = repeat {
                let next = due + ((current - due) / interval + 1) * interval;
                tx.execute(
                    "UPDATE board_reminders SET due_at=?1,last_fired_at=?2 WHERE id=?3",
                    params![next, current, id],
                )?;
            } else {
                tx.execute(
                    "UPDATE board_reminders SET state='completed',last_fired_at=?1 WHERE id=?2",
                    params![current, id],
                )?;
            }
        }
        tx.commit()?;
        Ok(due.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Store, Actor, Actor) {
        let dir = crate::test_support::tempdir();
        let s = Store::open(dir.path()).unwrap();
        s.set_user_password("alice", "hash").unwrap();
        s.set_user_password("bob", "hash").unwrap();
        let a = s.board_human("alice").unwrap();
        let b = s.board_human("bob").unwrap();
        (dir, s, a, b)
    }
    #[test]
    fn private_reminders_are_owned_validated_idempotent_and_never_publish_mentions() {
        let (_dir, s, a, b) = setup();
        let due = crate::now() + 10;
        let input = json!({"message":"Private @bob follow-up","dueAt":due,"repeatSeconds":10,"requestId":"same"});
        let r = s.board(&a, "create_reminder", &input).unwrap();
        assert_eq!(s.board(&a, "create_reminder", &input).unwrap(), r);
        assert!(
            s.board(&b, "read_reminder", &json!({"id":r["id"]}))
                .is_err()
        );
        assert!(
            s.board(&b, "cancel_reminder", &json!({"id":r["id"]}))
                .is_err()
        );
        assert!(
            s.board(
                &b,
                "update_reminder",
                &json!({"id":r["id"],"message":"stolen","dueAt":due})
            )
            .is_err()
        );
        for p in [
            json!({"message":"","dueAt":due}),
            json!({"message":"x","dueAt":0}),
            json!({"message":"x","dueAt":due,"repeatSeconds":0}),
            json!({"message":"x","dueAt":due,"repeatSeconds":1.5}),
            json!({"message":"x","dueAt":due,"topic":999}),
        ] {
            assert!(s.board(&a, "create_reminder", &p).is_err(), "{p}");
        }
        assert_eq!(s.fire_reminders(due).unwrap(), 1);
        let notes = s.board_pending(&a.id).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0]["event"]["kind"], "reminder");
        assert!(notes[0]["event"]["topic"].is_null());
        assert!(s.board_pending(&b.id).unwrap().is_empty());
        assert_eq!(
            s.board(&b, "list_topics", &json!({})).unwrap()["data"],
            json!([])
        );
        assert!(
            s.board(&b, "read_notification", &json!({"id":notes[0]["id"]}))
                .is_err()
        );
        s.board(&a, "cancel_reminder", &json!({"id":r["id"]}))
            .unwrap();
        assert_eq!(s.fire_reminders(due + 100).unwrap(), 0);
        assert_eq!(s.board_pending(&a.id).unwrap().len(), 1);
    }
    #[test]
    fn restart_coalesces_missed_repeats_and_delivery_is_atomic() {
        let (dir, s, a, _) = setup();
        let due = crate::now() + 10;
        let r = s
            .board(
                &a,
                "create_reminder",
                &json!({"message":"Repeat","dueAt":due,"repeatSeconds":10}),
            )
            .unwrap();
        drop(s);
        let s = Store::open(dir.path()).unwrap();
        s.0.lock().unwrap().execute_batch("CREATE TRIGGER fail_reminder BEFORE UPDATE ON board_reminders BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        assert!(s.fire_reminders(due + 35).is_err());
        assert!(s.board_pending(&a.id).unwrap().is_empty());
        s.0.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_reminder;")
            .unwrap();
        assert_eq!(s.fire_reminders(due + 35).unwrap(), 1);
        assert_eq!(
            s.board(&a, "read_reminder", &json!({"id":r["id"]}))
                .unwrap()["dueAt"],
            due + 40
        );
        assert_eq!(s.fire_reminders(due + 35).unwrap(), 0);
        assert_eq!(s.fire_reminders(due + 40).unwrap(), 1);
        assert_eq!(s.board_pending(&a.id).unwrap().len(), 2);
        s.remove_user("alice").unwrap();
        assert_eq!(s.fire_reminders(due + 100).unwrap(), 0);
    }
    #[test]
    fn migration_preserves_topic_events_notification_ids_and_foreign_keys() {
        let (dir, s, a, b) = setup();
        let t = s
            .board(
                &a,
                "create_topic",
                &json!({"channel":1,"body":"Hello @bob"}),
            )
            .unwrap();
        let old = s.board_pending(&b.id).unwrap();
        {
            let db = s.0.lock().unwrap();
            db.execute_batch("PRAGMA foreign_keys=OFF;BEGIN;
CREATE TABLE legacy_events(id INTEGER PRIMARY KEY AUTOINCREMENT,topic INTEGER NOT NULL REFERENCES board_topics(id),actor TEXT NOT NULL REFERENCES board_actors(id),kind TEXT NOT NULL,payload TEXT NOT NULL,created_at INTEGER NOT NULL);
INSERT INTO legacy_events SELECT * FROM board_events;
DROP TABLE board_events;ALTER TABLE legacy_events RENAME TO board_events;COMMIT;PRAGMA foreign_keys=ON;").unwrap();
        }
        drop(s);
        let s = Store::open(dir.path()).unwrap();
        assert_eq!(s.board_pending(&b.id).unwrap(), old);
        let due = crate::now() + 1;
        s.board(
            &b,
            "create_reminder",
            &json!({"message":"Context","dueAt":due,"topic":t["id"]}),
        )
        .unwrap();
        s.fire_reminders(due).unwrap();
        assert_eq!(s.board_pending(&b.id).unwrap().len(), 2);
        assert!(
            s.0.lock()
                .unwrap()
                .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
                .optional()
                .unwrap()
                .is_none()
        );
    }
}
