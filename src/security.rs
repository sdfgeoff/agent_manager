use anyhow::{Context, Result, bail};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};
use url::Url;

pub fn token() -> String {
    let mut bytes = [0; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

pub fn username(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    {
        bail!("Username must contain 1–64 letters, numbers, dots, underscores, or hyphens");
    }
    Ok(value.to_ascii_lowercase())
}

pub fn hash_password(password: &str) -> Result<String> {
    if password.len() < 12 || password.len() > 1024 {
        bail!("Password must contain 12–1024 bytes");
    }
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!(e.to_string()))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    password.len() <= 1024
        && PasswordHash::new(hash).is_ok_and(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
}

pub fn private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    // Atomic replacement; never follow a symlink at the destination.
    let temp = path.with_extension(format!("{}.tmp", token()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

pub fn origin(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("Expected an absolute server URL")?;
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        bail!("URL must be an origin, such as https://agents.example.com (no path or credentials)");
    }
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        bail!("HTTPS is required except for loopback development URLs");
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_connections_require_tls() {
        assert!(origin("http://203.0.113.1").is_err());
        assert!(origin("https://agents.example.com").is_ok());
        assert!(origin("http://127.0.0.1:8080").is_ok());
        assert!(origin("https://user:secret@example.com").is_err());
        assert!(origin("https://example.com/path").is_err());
    }
    #[test]
    fn password_round_trip() {
        let hash = hash_password("a long test password").unwrap();
        assert!(verify_password("a long test password", &hash));
        assert!(!verify_password("wrong password", &hash));
        assert!(hash_password("short").is_err());
    }
    #[test]
    fn credentials_are_private_and_atomic() {
        let dir = crate::test_support::tempdir();
        let path = dir.path().join("credential");
        private_write(&path, b"first").unwrap();
        private_write(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
