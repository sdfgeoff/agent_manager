use crate::security;
use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncWriteExt;

const ASSET: &str = "server-x86_64-unknown-linux-musl";
#[derive(Deserialize)]
pub struct Config {
    repository: String,
    binary: PathBuf,
    data_dir: PathBuf,
    service: String,
    health_url: String,
    state_dir: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Installed {
    tag: String,
    sha256: String,
}

pub(crate) fn version(tag: &str) -> Result<[u32; 3]> {
    let text = tag
        .strip_prefix('v')
        .context("Release tag must start with v")?;
    let parts = text
        .split('.')
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    parts
        .try_into()
        .map_err(|_| anyhow::anyhow!("Expected a stable release tag such as v0.2.0"))
}
fn checksum(manifest: &str, name: &str) -> Result<String> {
    let mut matches = manifest.lines().filter_map(|line| {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() == 2 && parts[1].trim_start_matches('*') == name {
            Some(parts[0])
        } else {
            None
        }
    });
    let hash = matches.next().context("Release checksum is missing")?;
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) || matches.next().is_some()
    {
        bail!("Invalid or ambiguous release checksum");
    }
    Ok(hash.to_ascii_lowercase())
}
fn replace(source: &Path, target: &Path, mode: u32) -> Result<()> {
    let stage = target.with_extension(format!("update-{}", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&stage)?;
        std::io::copy(&mut File::open(source)?, &mut output)?;
        output.sync_all()?;
        fs::rename(&stage, target)?;
        File::open(target.parent().context("Missing parent directory")?)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(stage);
    }
    result
}
async fn bytes(client: &Client, url: &str, maximum: usize) -> Result<Vec<u8>> {
    let mut response = client.get(url).send().await?.error_for_status()?;
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if data.len().saturating_add(chunk.len()) > maximum {
            bail!("Release response exceeds the size limit");
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}
async fn download(client: &Client, url: &str, path: &Path, expected: &str) -> Result<()> {
    let mut response = client.get(url).send().await?.error_for_status()?;
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o700)
        .open(path)?;
    let mut output = tokio::fs::File::from_std(file);
    let mut hash = Sha256::new();
    let mut size = 0usize;
    while let Some(chunk) = response.chunk().await? {
        size = size.saturating_add(chunk.len());
        if size > 128 * 1024 * 1024 {
            bail!("Release binary exceeds 128 MiB");
        }
        hash.update(&chunk);
        output.write_all(&chunk).await?;
    }
    output.sync_all().await?;
    if hex::encode(hash.finalize()) != expected {
        bail!("Release checksum verification failed; server was not changed");
    }
    Ok(())
}

trait Control {
    async fn service(&self, action: &str) -> Result<()>;
    async fn healthy(&self, version: &str) -> Result<()>;
}
struct Systemd<'a> {
    config: &'a Config,
    client: Client,
}
impl Control for Systemd<'_> {
    async fn service(&self, action: &str) -> Result<()> {
        let output = tokio::time::timeout(
            Duration::from_secs(60),
            tokio::process::Command::new("systemctl")
                .args(["--user", action, &self.config.service])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .context("Service command timed out")??;
        if !output.status.success() {
            bail!(
                "systemctl {action} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }
    async fn healthy(&self, version: &str) -> Result<()> {
        for _ in 0..40 {
            if let Ok(response) = self.client.get(&self.config.health_url).send().await
                && response.status().is_success()
                && let Ok(body) = response.json::<Value>().await
                && body["version"].as_str() == Some(version)
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        bail!("Server did not become healthy at version {version}")
    }
}

async fn deploy(
    config: &Config,
    candidate: &Path,
    tag: &str,
    control: &impl Control,
) -> Result<PathBuf> {
    let backup = config
        .state_dir
        .join("backups")
        .join(format!("{}-{}", tag, uuid::Uuid::new_v4()));
    security::private_dir(&backup)?;
    let database = config.data_dir.join("agentmanager.sqlite3");
    if !database.is_file() || !config.binary.is_file() {
        bail!("Initialize and run the server before enabling updates");
    }
    control.service("stop").await?;
    let preparation = (|| -> Result<()> {
        fs::copy(&config.binary, backup.join("server"))?;
        let conn = rusqlite::Connection::open(&database)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute(
            "VACUUM main INTO ?1",
            [backup
                .join("agentmanager.sqlite3")
                .to_str()
                .context("Invalid backup path")?],
        )?;
        replace(candidate, &config.binary, 0o700)?;
        Ok(())
    })();
    if let Err(error) = preparation {
        // A replacement can have succeeded even if its directory fsync failed.
        if backup.join("server").exists() {
            replace(&backup.join("server"), &config.binary, 0o700)?;
        }
        control.service("start").await?;
        return Err(error.context("Update preparation failed; previous server restarted"));
    }
    let activation = async {
        control.service("start").await?;
        control.healthy(tag.trim_start_matches('v')).await
    }
    .await;
    if let Err(error) = activation {
        control
            .service("stop")
            .await
            .context("Cannot stop failed update for rollback")?;
        replace(&backup.join("server"), &config.binary, 0o700)?;
        // The service is stopped, so no process may retain the new database's WAL.
        for suffix in ["-wal", "-shm"] {
            let path = PathBuf::from(format!("{}{suffix}", database.display()));
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        replace(&backup.join("agentmanager.sqlite3"), &database, 0o600)?;
        control
            .service("start")
            .await
            .context("Rollback restored files but could not restart the previous server")?;
        return Err(error.context("Update failed; previous binary and database restored"));
    }
    Ok(backup)
}

pub async fn run(path: &Path, check: bool) -> Result<()> {
    let config: Config = serde_json::from_slice(&fs::read(path)?)?;
    if config.repository.split('/').count() != 2
        || !config
            .repository
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/_.-".contains(&c))
    {
        bail!("Invalid GitHub repository");
    }
    if !config.binary.is_absolute()
        || !config.data_dir.is_absolute()
        || !config.state_dir.is_absolute()
    {
        bail!("Deployment paths must be absolute");
    }
    if !config.service.ends_with(".service")
        || !config
            .service
            .starts_with(|c: char| c.is_ascii_alphanumeric())
        || !config
            .service
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.@-".contains(&c))
    {
        bail!("Invalid service name");
    }
    let health = url::Url::parse(&config.health_url)?;
    if health.scheme() != "http"
        || !matches!(health.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
    {
        bail!("Health URL must use local HTTP");
    }
    if std::env::consts::ARCH != "x86_64" {
        bail!("Automatic releases currently support x86_64 Linux");
    }
    security::private_dir(&config.state_dir)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(config.state_dir.join("update.lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            println!("Another updater is running");
            return Ok(());
        }
        Err(error) => return Err(anyhow::anyhow!(error)),
    }
    let client = Client::builder()
        .user_agent(concat!("agentmanager-updater/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .build()?;
    let response = client
        .get(format!(
            "https://api.github.com/repos/{}/releases/latest",
            config.repository
        ))
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        println!("No published releases yet");
        return Ok(());
    }
    let release: Value = response.error_for_status()?.json().await?;
    if release["draft"] == true || release["prerelease"] == true {
        bail!("Refusing an unpublished or prerelease build");
    }
    let tag = release["tag_name"]
        .as_str()
        .context("Release is missing a tag")?;
    let latest = version(tag)?;
    let installed_path = config.state_dir.join("installed.json");
    if installed_path.exists() {
        let installed: Installed = serde_json::from_slice(&fs::read(&installed_path)?)?;
        if version(&installed.tag)? >= latest {
            println!("Already running {}", installed.tag);
            return Ok(());
        }
    }
    println!("Published release available: {tag}");
    if check {
        return Ok(());
    }
    let base = format!(
        "https://github.com/{}/releases/download/{tag}",
        config.repository
    );
    let manifest =
        String::from_utf8(bytes(&client, &format!("{base}/SHA256SUMS"), 2 * 1024 * 1024).await?)?;
    let expected = checksum(&manifest, ASSET)?;
    let stage = config
        .state_dir
        .join(format!("staging-{}", uuid::Uuid::new_v4()));
    security::private_dir(&stage)?;
    let _cleanup = Staging(stage.clone());
    let candidate = stage.join("server");
    download(&client, &format!("{base}/{ASSET}"), &candidate, &expected).await?;
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new(&candidate)
            .arg("--version")
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    if !output.status.success()
        || String::from_utf8_lossy(&output.stdout).trim()
            != format!("server {}", tag.trim_start_matches('v'))
    {
        bail!("Release binary version does not match its tag");
    }
    let control = Systemd {
        config: &config,
        client: Client::builder()
            .timeout(Duration::from_secs(2))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
    };
    let backup = deploy(&config, &candidate, tag, &control).await?;
    security::private_write(
        &installed_path,
        &serde_json::to_vec(&Installed {
            tag: tag.into(),
            sha256: expected,
        })?,
    )?;
    println!(
        "Installed {tag}; previous binary and database backed up in {}",
        backup.display()
    );
    Ok(())
}
struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_versions_and_checksums_are_strict() {
        assert!(version("v0.2.0").unwrap() > version("v0.1.9").unwrap());
        assert!(version("v0.2.0-rc1").is_err());
        assert!(version("v1.2.3/evil").is_err());
        let line = format!("{}  {ASSET}", "a".repeat(64));
        assert_eq!(checksum(&line, ASSET).unwrap(), "a".repeat(64));
        assert!(checksum(&format!("{line}\n{line}"), ASSET).is_err());
        assert!(checksum("garbage", ASSET).is_err());
    }
    struct FakeControl {
        fail_health: bool,
        actions: std::sync::Mutex<Vec<String>>,
        db: PathBuf,
    }
    impl Control for FakeControl {
        async fn service(&self, action: &str) -> Result<()> {
            self.actions.lock().unwrap().push(action.into());
            Ok(())
        }
        async fn healthy(&self, _version: &str) -> Result<()> {
            rusqlite::Connection::open(&self.db)?
                .execute("UPDATE marker SET value='migrated'", [])?;
            if self.fail_health {
                bail!("Simulated startup failure");
            }
            Ok(())
        }
    }
    #[tokio::test]
    async fn failed_start_restores_binary_and_database_before_restarting() {
        let dir = crate::test_support::tempdir();
        let binary = dir.path().join("server");
        let candidate = dir.path().join("candidate");
        fs::write(&binary, b"old binary").unwrap();
        fs::write(&candidate, b"new binary").unwrap();
        let db = dir.path().join("agentmanager.sqlite3");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE marker(value TEXT); INSERT INTO marker VALUES ('original');",
        )
        .unwrap();
        drop(conn);
        let config = Config {
            repository: "sdfgeoff/agent_manager".into(),
            binary: binary.clone(),
            data_dir: dir.path().into(),
            service: "test.service".into(),
            health_url: "http://127.0.0.1/healthz".into(),
            state_dir: dir.path().join("state"),
        };
        let control = FakeControl {
            fail_health: true,
            actions: Default::default(),
            db: db.clone(),
        };
        assert!(
            deploy(&config, &candidate, "v0.2.0", &control)
                .await
                .is_err()
        );
        assert_eq!(fs::read(binary).unwrap(), b"old binary");
        assert_eq!(
            rusqlite::Connection::open(db)
                .unwrap()
                .query_row("SELECT value FROM marker", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "original"
        );
        assert_eq!(
            *control.actions.lock().unwrap(),
            ["stop", "start", "stop", "start"]
        );
    }
    #[tokio::test]
    async fn successful_update_keeps_a_consistent_database_backup() {
        let dir = crate::test_support::tempdir();
        let binary = dir.path().join("server");
        let candidate = dir.path().join("candidate");
        fs::write(&binary, b"old binary").unwrap();
        fs::write(&candidate, b"new binary").unwrap();
        let db = dir.path().join("agentmanager.sqlite3");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE marker(value TEXT); INSERT INTO marker VALUES ('original');",
        )
        .unwrap();
        drop(conn);
        let config = Config {
            repository: "sdfgeoff/agent_manager".into(),
            binary: binary.clone(),
            data_dir: dir.path().into(),
            service: "test.service".into(),
            health_url: "http://127.0.0.1/healthz".into(),
            state_dir: dir.path().join("state"),
        };
        let control = FakeControl {
            fail_health: false,
            actions: Default::default(),
            db: db.clone(),
        };
        let backup = deploy(&config, &candidate, "v0.2.0", &control)
            .await
            .unwrap();
        assert_eq!(fs::read(binary).unwrap(), b"new binary");
        assert_eq!(fs::read(backup.join("server")).unwrap(), b"old binary");
        assert_eq!(
            rusqlite::Connection::open(backup.join("agentmanager.sqlite3"))
                .unwrap()
                .query_row("SELECT value FROM marker", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "original"
        );
        assert_eq!(
            rusqlite::Connection::open(db)
                .unwrap()
                .query_row("SELECT value FROM marker", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "migrated"
        );
    }
}
