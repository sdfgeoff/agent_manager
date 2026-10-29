use crate::{now, security};
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::{path::Path, sync::Mutex};

pub struct Store(Mutex<Connection>);

#[derive(Serialize)]
pub struct WorkerRecord {
    pub id: String,
    pub name: String,
    pub created_at: i64,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        security::private_dir(dir)?;
        let db = Connection::open(dir.join("agentmanager.sqlite3"))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS enrollment (hash TEXT PRIMARY KEY, expires INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS workers (id TEXT PRIMARY KEY, name TEXT NOT NULL, credential_hash TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL);")?;
        Ok(Self(Mutex::new(db)))
    }
    pub fn password(&self) -> Result<Option<String>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .query_row("SELECT value FROM settings WHERE key='password'", [], |r| {
                r.get(0)
            })
            .optional()?)
    }
    pub fn set_password(&self, hash: &str) -> Result<()> {
        self.0.lock().unwrap().execute("INSERT INTO settings VALUES ('password',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [hash])?;
        Ok(())
    }
    pub fn enrollment(&self) -> Result<String> {
        let key = security::token();
        let db = self.0.lock().unwrap();
        db.execute("DELETE FROM enrollment WHERE expires < ?1", [now()])?;
        db.execute(
            "INSERT INTO enrollment VALUES (?1,?2)",
            params![security::digest(&key), now() + 600],
        )?;
        Ok(key)
    }
    pub fn enroll(&self, key: &str, name: &str) -> Result<(String, String)> {
        if name.trim().is_empty() || name.len() > 100 || key.len() != 64 {
            bail!("Invalid registration");
        }
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        if tx.execute(
            "DELETE FROM enrollment WHERE hash=?1 AND expires>=?2",
            params![security::digest(key), now()],
        )? != 1
        {
            bail!("Invalid or expired registration key");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let credential = security::token();
        tx.execute(
            "INSERT INTO workers VALUES (?1,?2,?3,?4)",
            params![id, name.trim(), security::digest(&credential), now()],
        )?;
        tx.commit()?;
        Ok((id, credential))
    }
    pub fn worker_for_token(&self, token: &str) -> Result<Option<String>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT id FROM workers WHERE credential_hash=?1",
                [security::digest(token)],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn workers(&self) -> Result<Vec<WorkerRecord>> {
        let db = self.0.lock().unwrap();
        let mut stmt =
            db.prepare("SELECT id,name,created_at FROM workers ORDER BY created_at,name")?;
        Ok(stmt
            .query_map([], |r| {
                Ok(WorkerRecord {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })?
            .collect::<Result<_, _>>()?)
    }
    pub fn revoke(&self, id: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .execute("DELETE FROM workers WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn worker_exists(&self, id: &str) -> Result<bool> {
        Ok(self.0.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM workers WHERE id=?1)",
            [id],
            |r| r.get(0),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_is_single_use_and_revocable() {
        let dir = tempfile::tempdir().unwrap();
        let db = Store::open(dir.path()).unwrap();
        let key = db.enrollment().unwrap();
        let (id, credential) = db.enroll(&key, "laptop").unwrap();
        assert!(db.enroll(&key, "other").is_err());
        assert_eq!(db.worker_for_token(&credential).unwrap(), Some(id.clone()));
        assert_eq!(db.worker_for_token(&key).unwrap(), None);
        db.revoke(&id).unwrap();
        assert_eq!(db.worker_for_token(&credential).unwrap(), None);
    }
    #[test]
    fn expired_registration_cannot_be_used() {
        let dir = tempfile::tempdir().unwrap();
        let db = Store::open(dir.path()).unwrap();
        let key = db.enrollment().unwrap();
        db.0.lock()
            .unwrap()
            .execute("UPDATE enrollment SET expires=0", [])
            .unwrap();
        assert!(db.enroll(&key, "laptop").is_err());
    }
}
