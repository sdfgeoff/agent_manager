//! Worker-local observed token usage. This is neither billing nor topic attribution.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, sync::Mutex};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Tokens {
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
    reasoning_output_tokens: i64,
    total_tokens: i64,
    #[serde(default)]
    cache_write_input_tokens: i64,
}
impl Tokens {
    fn values(&self) -> [i64; 6] {
        [
            self.input_tokens,
            self.cached_input_tokens,
            self.output_tokens,
            self.reasoning_output_tokens,
            self.total_tokens,
            self.cache_write_input_tokens,
        ]
    }
    fn delta(&self, old: &Self) -> Option<Self> {
        let a = self.values();
        let b = old.values();
        if a.iter().zip(b).any(|(a, b)| *a < b) {
            return None;
        }
        Some(Self {
            input_tokens: a[0] - b[0],
            cached_input_tokens: a[1] - b[1],
            output_tokens: a[2] - b[2],
            reasoning_output_tokens: a[3] - b[3],
            total_tokens: a[4] - b[4],
            cache_write_input_tokens: a[5] - b[5],
        })
    }
}
pub struct Ledger(Mutex<Connection>);
impl Ledger {
    pub fn open(path: Option<&Path>) -> Result<Self> {
        let db = if let Some(path) = path {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if !path.exists() {
                crate::security::private_write(path, b"")?;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            }
            Connection::open(path)?
        } else {
            Connection::open_in_memory()?
        };
        db.execute_batch("PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS usage_threads(thread TEXT PRIMARY KEY, totals TEXT, model TEXT);
            CREATE TABLE IF NOT EXISTS usage_turns(
                seq INTEGER PRIMARY KEY AUTOINCREMENT, thread TEXT NOT NULL, turn TEXT NOT NULL,
                agent TEXT, model TEXT, tokens TEXT NOT NULL, partial INTEGER NOT NULL,
                first_seen INTEGER NOT NULL, updated_at INTEGER NOT NULL, status TEXT NOT NULL,
                UNIQUE(thread,turn));
            CREATE INDEX IF NOT EXISTS usage_agent ON usage_turns(agent,seq);
            CREATE TABLE IF NOT EXISTS usage_daily(
                thread TEXT NOT NULL, turn TEXT NOT NULL, day TEXT NOT NULL,
                agent TEXT, input INTEGER NOT NULL, cached INTEGER NOT NULL,
                output INTEGER NOT NULL, partial INTEGER NOT NULL,
                PRIMARY KEY(thread,turn,day));")?;
        Ok(Self(Mutex::new(db)))
    }
    pub fn configure(&self, thread: &str, fresh: bool, model: Option<&str>) -> Result<()> {
        let db = self.0.lock().unwrap();
        db.execute(
            "INSERT INTO usage_threads(thread,totals,model) VALUES(?1,?2,?3)
            ON CONFLICT(thread) DO UPDATE SET model=COALESCE(excluded.model,usage_threads.model)",
            params![
                thread,
                if fresh {
                    Some(serde_json::to_string(&Tokens::default())?)
                } else {
                    None
                },
                model
            ],
        )?;
        Ok(())
    }
    pub fn record(&self, p: &Value, agent: Option<&str>) -> Result<()> {
        self.record_at(p, agent, crate::now())
    }
    fn record_at(&self, p: &Value, agent: Option<&str>, observed_at: i64) -> Result<()> {
        let thread = p["threadId"].as_str().context("Usage thread missing")?;
        let turn = p["turnId"].as_str().context("Usage turn missing")?;
        let total: Tokens = serde_json::from_value(p["tokenUsage"]["total"].clone())?;
        let last: Tokens = serde_json::from_value(p["tokenUsage"]["last"].clone())?;
        ensure!(
            total
                .values()
                .iter()
                .chain(last.values().iter())
                .all(|v| *v >= 0),
            "Negative token usage"
        );
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT totals FROM usage_threads WHERE thread=?1",
                [thread],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let old = previous
            .map(|s| serde_json::from_str::<Tokens>(&s))
            .transpose()?;
        let (delta, partial) = match old {
            Some(old) => {
                // Ignore duplicate or older snapshots. Never add cumulative totals twice.
                if total.total_tokens <= old.total_tokens {
                    return Ok(());
                }
                match total.delta(&old) {
                    Some(delta) => (delta, false),
                    None => (last, true),
                }
            }
            // An existing session may contain years of usage. Start with only its
            // latest reported request; mark this turn partial rather than import history.
            None => (last, true),
        };
        tx.execute(
            "INSERT INTO usage_threads(thread,totals) VALUES(?1,?2)
            ON CONFLICT(thread) DO UPDATE SET totals=excluded.totals",
            params![thread, serde_json::to_string(&total)?],
        )?;
        let prior: Option<String> = tx
            .query_row(
                "SELECT tokens FROM usage_turns WHERE thread=?1 AND turn=?2",
                params![thread, turn],
                |r| r.get(0),
            )
            .optional()?;
        tx.execute("INSERT INTO usage_daily(thread,turn,day,agent,input,cached,output,partial)
            VALUES(?1,?2,date(?3,'unixepoch'),?4,?5,?6,?7,?8)
            ON CONFLICT(thread,turn,day) DO UPDATE SET
                input=usage_daily.input+excluded.input,cached=usage_daily.cached+excluded.cached,
                output=usage_daily.output+excluded.output,partial=MAX(usage_daily.partial,excluded.partial)",
            params![thread,turn,observed_at,agent,delta.input_tokens,delta.cached_input_tokens,delta.output_tokens,partial])?;
        let mut combined = serde_json::to_value(delta)?;
        if let Some(prior) = prior {
            let prior: Value = serde_json::from_str(&prior)?;
            for (key, value) in combined.as_object_mut().unwrap() {
                *value = json!(
                    value
                        .as_i64()
                        .unwrap()
                        .checked_add(prior[key].as_i64().unwrap_or(0))
                        .context("Token total overflow")?
                );
            }
        }
        tx.execute("INSERT INTO usage_turns(thread,turn,agent,model,tokens,partial,first_seen,updated_at,status)
            VALUES(?1,?2,?3,(SELECT model FROM usage_threads WHERE thread=?1),?4,?5,?6,?6,'observed')
            ON CONFLICT(thread,turn) DO UPDATE SET tokens=excluded.tokens,
                partial=MAX(usage_turns.partial,excluded.partial),updated_at=excluded.updated_at,
                agent=COALESCE(usage_turns.agent,excluded.agent)",
            params![thread,turn,agent,combined.to_string(),partial,observed_at])?;
        tx.commit()?;
        Ok(())
    }
    pub fn completed(&self, p: &Value) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE usage_turns SET status=?3,updated_at=?4 WHERE thread=?1 AND turn=?2",
            params![
                p["threadId"].as_str(),
                p["turn"]["id"].as_str(),
                p["turn"]["status"].as_str().unwrap_or("unknown"),
                crate::now()
            ],
        )?;
        Ok(())
    }
    pub fn daily(&self, p: &Value) -> Result<Value> {
        let agent = p["agentId"].as_str().context("Agent id required")?;
        let db = self.0.lock().unwrap();
        let data = db
            .prepare(
                "SELECT day,SUM(input),SUM(cached),SUM(output),MAX(partial)
            FROM usage_daily WHERE agent=?1 GROUP BY day ORDER BY day DESC LIMIT 366",
            )?
            .query_map([agent], |r| {
                Ok(json!({"day":r.get::<_,String>(0)?,
                "inputTokens":r.get::<_,i64>(1)?,"cachedInputTokens":r.get::<_,i64>(2)?,
                "outputTokens":r.get::<_,i64>(3)?,"partial":r.get::<_,bool>(4)?}))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"data":data,"timezone":"UTC"}))
    }
    pub fn page(&self, p: &Value) -> Result<Value> {
        let db = self.0.lock().unwrap();
        let agent = p["agentId"].as_str();
        let before = p["before"].as_i64().unwrap_or(i64::MAX);
        let rows=db.prepare("SELECT seq,thread,turn,agent,model,tokens,partial,first_seen,updated_at,status
            FROM usage_turns WHERE (?1 IS NULL OR agent=?1) AND seq<?2 ORDER BY seq DESC LIMIT 51")?
            .query_map(params![agent,before],|r|Ok(json!({
                "id":r.get::<_,i64>(0)?,"threadId":r.get::<_,String>(1)?,"turnId":r.get::<_,String>(2)?,
                "agentId":r.get::<_,Option<String>>(3)?,"configuredModel":r.get::<_,Option<String>>(4)?,
                "tokens":serde_json::from_str::<Value>(&r.get::<_,String>(5)?).unwrap_or(Value::Null),
                "partial":r.get::<_,bool>(6)?,"firstSeen":r.get::<_,i64>(7)?,
                "updatedAt":r.get::<_,i64>(8)?,"status":r.get::<_,String>(9)?
            })))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let next = if rows.len() > 50 {
            Some(rows[49]["id"].clone())
        } else {
            None
        };
        let totals = db.query_row(
            "SELECT COUNT(*),COALESCE(SUM(json_extract(tokens,'$.inputTokens')),0),
            COALESCE(SUM(json_extract(tokens,'$.cachedInputTokens')),0),
            COALESCE(SUM(json_extract(tokens,'$.outputTokens')),0),
            COALESCE(SUM(json_extract(tokens,'$.reasoningOutputTokens')),0),
            COALESCE(SUM(json_extract(tokens,'$.totalTokens')),0),
            COALESCE(SUM(partial),0),MIN(first_seen)
            FROM usage_turns WHERE (?1 IS NULL OR agent=?1)",
            [agent],
            |r| {
                Ok(json!({
                    "turns":r.get::<_,i64>(0)?,"inputTokens":r.get::<_,i64>(1)?,
                    "cachedInputTokens":r.get::<_,i64>(2)?,"outputTokens":r.get::<_,i64>(3)?,
                    "reasoningOutputTokens":r.get::<_,i64>(4)?,"totalTokens":r.get::<_,i64>(5)?,
                    "partialTurns":r.get::<_,i64>(6)?,"firstSeen":r.get::<_,Option<i64>>(7)?
                }))
            },
        )?;
        Ok(
            json!({"data":rows.into_iter().take(50).collect::<Vec<_>>(),"nextCursor":next,"totals":totals}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(turn: &str, n: i64) -> Value {
        let t = json!({"inputTokens":n*10,"cachedInputTokens":n*4,"outputTokens":n*2,
            "reasoningOutputTokens":n,"totalTokens":n*12,"cacheWriteInputTokens":0});
        json!({"threadId":"thread","turnId":turn,"tokenUsage":{"total":t,"last":{
            "inputTokens":10,"cachedInputTokens":4,"outputTokens":2,
            "reasoningOutputTokens":1,"totalTokens":12}}})
    }
    #[test]
    fn daily_buckets_split_a_long_turn_without_rebilling_duplicates() {
        let ledger = Ledger::open(None).unwrap();
        ledger.configure("thread", true, None).unwrap();
        ledger
            .record_at(&snapshot("one", 1), Some("agent"), 86400)
            .unwrap();
        ledger
            .record_at(&snapshot("one", 3), Some("agent"), 172800)
            .unwrap();
        ledger
            .record_at(&snapshot("one", 3), Some("agent"), 172800)
            .unwrap();
        let page = ledger.daily(&json!({"agentId":"agent"})).unwrap();
        assert_eq!(page["data"].as_array().unwrap().len(), 2);
        assert_eq!(page["data"][0]["day"], "1970-01-03");
        assert_eq!(page["data"][0]["inputTokens"], 20);
        assert_eq!(page["data"][0]["outputTokens"], 4);
        assert_eq!(page["data"][1]["day"], "1970-01-02");
        assert_eq!(page["data"][1]["inputTokens"], 10);
        assert!(ledger.daily(&json!({})).is_err());
        assert_eq!(
            ledger.daily(&json!({"agentId":"other"})).unwrap()["data"],
            json!([])
        );
    }

    #[test]
    fn cumulative_updates_are_durable_idempotent_and_attributed_per_turn() {
        let dir = crate::test_support::tempdir();
        let path = dir.path().join("usage.sqlite3");
        let ledger = Ledger::open(Some(&path)).unwrap();
        ledger
            .configure("thread", true, Some("configured-model"))
            .unwrap();
        ledger.record(&snapshot("one", 1), Some("agent")).unwrap();
        ledger.record(&snapshot("one", 3), Some("agent")).unwrap();
        ledger.record(&snapshot("one", 3), Some("agent")).unwrap();
        ledger.record(&snapshot("one", 2), Some("agent")).unwrap();
        ledger
            .completed(&json!({"threadId":"thread","turn":{"id":"one","status":"interrupted"}}))
            .unwrap();
        drop(ledger);
        let ledger = Ledger::open(Some(&path)).unwrap();
        ledger.record(&snapshot("one", 3), Some("agent")).unwrap();
        ledger.record(&snapshot("two", 5), Some("agent")).unwrap();
        let page = ledger.page(&json!({"agentId":"agent"})).unwrap();
        assert_eq!(page["totals"]["totalTokens"], 60);
        assert_eq!(page["totals"]["inputTokens"], 50);
        assert_eq!(page["totals"]["partialTurns"], 0);
        assert_eq!(page["data"][0]["tokens"]["totalTokens"], 24);
        assert_eq!(page["data"][1]["tokens"]["totalTokens"], 36);
        assert_eq!(page["data"][1]["status"], "interrupted");
        assert_eq!(page["data"][1]["configuredModel"], "configured-model");
        assert_eq!(
            ledger.page(&json!({"agentId":"someone-else"})).unwrap()["totals"]["turns"],
            0
        );
    }
    #[test]
    fn existing_sessions_do_not_import_historical_usage_and_bad_events_are_atomic() {
        let ledger = Ledger::open(None).unwrap();
        ledger.record(&snapshot("one", 1000), None).unwrap();
        assert_eq!(
            ledger.page(&json!({})).unwrap()["totals"]["totalTokens"],
            12
        );
        ledger.record(&snapshot("one", 1001), None).unwrap();
        let page = ledger.page(&json!({})).unwrap();
        assert_eq!(page["totals"]["totalTokens"], 24);
        assert_eq!(page["totals"]["partialTurns"], 1);
        let mut bad = snapshot("one", 1002);
        bad["tokenUsage"]["total"]["inputTokens"] = json!(-1);
        assert!(ledger.record(&bad, None).is_err());
        bad["tokenUsage"]["total"]["inputTokens"] = Value::Null;
        assert!(ledger.record(&bad, None).is_err());
        assert_eq!(ledger.page(&json!({})).unwrap(), page);
    }
    #[test]
    fn pagination_and_totals_are_independent_and_survive_archived_sessions() {
        let ledger = Ledger::open(None).unwrap();
        ledger.configure("thread", true, None).unwrap();
        for n in 1..=55 {
            ledger
                .record(&snapshot(&format!("turn-{n}"), n), Some("agent"))
                .unwrap();
        }
        let first = ledger.page(&json!({})).unwrap();
        let second = ledger.page(&json!({"before":first["nextCursor"]})).unwrap();
        assert_eq!(first["data"].as_array().unwrap().len(), 50);
        assert_eq!(second["data"].as_array().unwrap().len(), 5);
        assert_eq!(first["totals"], second["totals"]);
        assert_eq!(first["totals"]["totalTokens"], 660);
        assert_eq!(second["data"][4]["turnId"], "turn-1");
    }
}
