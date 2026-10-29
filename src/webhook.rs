use anyhow::{Result, bail};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::sync::mpsc;

pub struct Deployment {
    pub repository: String,
    secret: Vec<u8>,
    pub notify: mpsc::Sender<()>,
}
impl Deployment {
    pub fn new(repository: String, secret: Vec<u8>) -> Result<(Self, mpsc::Receiver<()>)> {
        if secret.len() < 32 || secret.len() > 1024 {
            bail!("Webhook secret must contain 32–1024 bytes");
        }
        let (notify, receiver) = mpsc::channel(8);
        Ok((
            Self {
                repository,
                secret,
                notify,
            },
            receiver,
        ))
    }
    pub fn verify(&self, signature: &str, body: &[u8]) -> bool {
        let Some(hex) = signature.strip_prefix("sha256=") else {
            return false;
        };
        let Ok(signature) = hex::decode(hex) else {
            return false;
        };
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.secret).expect("HMAC accepts any key length");
        mac.update(body);
        mac.verify_slice(&signature).is_ok()
    }
}
pub async fn run(mut notifications: mpsc::Receiver<()>) {
    while notifications.recv().await.is_some() {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::process::Command::new("systemctl")
                .args([
                    "--user",
                    "start",
                    "--no-block",
                    "agentmanager-update.service",
                ])
                .kill_on_drop(true)
                .output(),
        )
        .await;
        match result {
            Ok(Ok(output)) if output.status.success() => {
                tracing::info!("Release webhook queued the local updater")
            }
            Ok(Ok(output)) => {
                tracing::error!(error=%String::from_utf8_lossy(&output.stderr),"Could not start release updater")
            }
            error => tracing::error!(?error, "Could not start release updater"),
        }
    }
}
