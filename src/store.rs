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
    pub version: Option<String>,
}

#[derive(Serialize)]
pub struct UserRecord {
    pub username: String,
    pub created_at: i64,
}
pub struct UserCredential {
    pub username: String,
    pub password_hash: String,
    pub version: String,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        security::private_dir(dir)?;
        let mut db = Connection::open(dir.join("agentmanager.sqlite3"))?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS enrollment (hash TEXT PRIMARY KEY, expires INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS workers (id TEXT PRIMARY KEY, name TEXT NOT NULL, credential_hash TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL, version TEXT);
            CREATE TABLE IF NOT EXISTS users (username TEXT PRIMARY KEY, password_hash TEXT NOT NULL, version TEXT NOT NULL, created_at INTEGER NOT NULL);")?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let has_version: bool = tx
            .prepare("PRAGMA table_info(workers)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()?
            .iter()
            .any(|column| column == "version");
        if !has_version {
            tx.execute("ALTER TABLE workers ADD COLUMN version TEXT", [])?;
        }
        tx.execute("INSERT INTO users (username,password_hash,version,created_at) SELECT 'admin',value,?1,?2 FROM settings WHERE key='password' ON CONFLICT(username) DO NOTHING",params![security::token(),now()])?;
        tx.execute("DELETE FROM settings WHERE key='password'", [])?;
        tx.commit()?;
        Ok(Self(Mutex::new(db)))
    }
    pub fn set_password(&self, hash: &str) -> Result<()> {
        self.set_user_password("admin", hash)
    }
    pub fn set_user_password(&self, username: &str, hash: &str) -> Result<()> {
        let username = security::username(username)?;
        self.0.lock().unwrap().execute("INSERT INTO users VALUES (?1,?2,?3,?4) ON CONFLICT(username) DO UPDATE SET password_hash=excluded.password_hash,version=excluded.version",params![username,hash,security::token(),now()])?;
        Ok(())
    }
    pub fn user(&self, username: &str) -> Result<Option<UserCredential>> {
        let username = security::username(username)?;
        Ok(self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT username,password_hash,version FROM users WHERE username=?1",
                [username],
                |r| {
                    Ok(UserCredential {
                        username: r.get(0)?,
                        password_hash: r.get(1)?,
                        version: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }
    pub fn user_version(&self, username: &str) -> Result<Option<String>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT version FROM users WHERE username=?1",
                [username],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn users(&self) -> Result<Vec<UserRecord>> {
        let db = self.0.lock().unwrap();
        let mut stmt = db.prepare("SELECT username,created_at FROM users ORDER BY username")?;
        Ok(stmt
            .query_map([], |r| {
                Ok(UserRecord {
                    username: r.get(0)?,
                    created_at: r.get(1)?,
                })
            })?
            .collect::<Result<_, _>>()?)
    }
    pub fn remove_user(&self, username: &str) -> Result<()> {
        let username = security::username(username)?;
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
        if count <= 1 {
            bail!("Cannot remove the last account");
        }
        if tx.execute("DELETE FROM users WHERE username=?1", [username])? != 1 {
            bail!("User does not exist");
        }
        tx.commit()?;
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
            "INSERT INTO workers (id,name,credential_hash,created_at) VALUES (?1,?2,?3,?4)",
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
            db.prepare("SELECT id,name,created_at,version FROM workers ORDER BY created_at,name")?;
        Ok(stmt
            .query_map([], |r| {
                Ok(WorkerRecord {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                    version: r.get(3)?,
                })
            })?
            .collect::<Result<_, _>>()?)
    }
    pub fn set_worker_version(&self, id: &str, version: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE workers SET version=?1 WHERE id=?2",
            params![version, id],
        )?;
        Ok(())
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
    fn worker_version_migration_preserves_existing_credentials() {
        let dir = crate::test_support::tempdir();
        let credential = security::token();
        let db = Connection::open(dir.path().join("agentmanager.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE workers (id TEXT PRIMARY KEY,name TEXT NOT NULL,credential_hash TEXT NOT NULL UNIQUE,created_at INTEGER NOT NULL);").unwrap();
        db.execute(
            "INSERT INTO workers VALUES ('legacy','Original worker',?1,1)",
            [security::digest(&credential)],
        )
        .unwrap();
        drop(db);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(
            store.worker_for_token(&credential).unwrap().as_deref(),
            Some("legacy")
        );
        assert!(store.workers().unwrap()[0].version.is_none());
        store.set_worker_version("legacy", "0.2.2").unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(
            store.workers().unwrap()[0].version.as_deref(),
            Some("0.2.2")
        );
        let key = store.enrollment().unwrap();
        assert!(store.enroll(&key, "Another worker").is_ok());
    }
    #[test]
    fn legacy_password_migrates_once_without_losing_workers() {
        let dir = crate::test_support::tempdir();
        let store = Store::open(dir.path()).unwrap();
        let hash = security::hash_password("the original password").unwrap();
        store
            .0
            .lock()
            .unwrap()
            .execute("INSERT INTO settings VALUES ('password',?1)", [&hash])
            .unwrap();
        let key = store.enrollment().unwrap();
        let (worker, credential) = store.enroll(&key, "desktop").unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.user("admin").unwrap().unwrap().password_hash, hash);
        assert_eq!(store.worker_for_token(&credential).unwrap(), Some(worker));
        let left: i64 = store
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key='password'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
        store.set_user_password("admin", "changed hash").unwrap();
        drop(store);
        assert_eq!(
            Store::open(dir.path())
                .unwrap()
                .user("admin")
                .unwrap()
                .unwrap()
                .password_hash,
            "changed hash"
        );
    }
    #[test]
    fn usernames_are_normalized_and_last_account_is_protected() {
        let dir = crate::test_support::tempdir();
        let store = Store::open(dir.path()).unwrap();
        store.set_user_password(" Alice ", "alice hash").unwrap();
        assert_eq!(store.user("ALICE").unwrap().unwrap().username, "alice");
        let old = store.user("alice").unwrap().unwrap().version;
        store.set_user_password("alice", "new hash").unwrap();
        assert_ne!(old, store.user("alice").unwrap().unwrap().version);
        assert!(store.remove_user("alice").is_err());
        assert!(store.set_user_password("bad name", "hash").is_err());
        store.set_user_password("bob", "bob hash").unwrap();
        store.remove_user("ALICE").unwrap();
        assert!(store.user("alice").unwrap().is_none());
    }
    #[test]
    fn registration_is_single_use_and_revocable() {
        let dir = crate::test_support::tempdir();
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
        let dir = crate::test_support::tempdir();
        let db = Store::open(dir.path()).unwrap();
        let key = db.enrollment().unwrap();
        db.0.lock()
            .unwrap()
            .execute("UPDATE enrollment SET expires=0", [])
            .unwrap();
        assert!(db.enroll(&key, "laptop").is_err());
    }
}
