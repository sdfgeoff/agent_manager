use agentmanager::{
    security,
    server::{App, router},
    store::Store,
};
use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Parser)]
#[command(name = "server", version, about = "Self-hosted Codex session server")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    public_url: String,
    #[arg(long, default_value = ".data/server")]
    data_dir: PathBuf,
    /// Enable signed GitHub release notifications using this secret file.
    #[arg(long, env = "AGENTMANAGER_DEPLOYMENT_SECRET_FILE")]
    deployment_secret_file: Option<PathBuf>,
    #[arg(long, default_value = "sdfgeoff/agent_manager")]
    deployment_repository: String,
    #[command(subcommand)]
    command: Option<Action>,
}
#[derive(Subcommand)]
enum Action {
    /// Create an account or reset its password. Existing logins for that user are invalidated.
    SetPassword {
        #[arg(long, default_value = "admin")]
        username: String,
        /// Read one password line from stdin, for unattended setup.
        #[arg(long)]
        stdin: bool,
    },
    /// Print a single-use, ten-minute worker registration key.
    RegistrationKey,
    /// List all login usernames.
    Users,
    /// Remove an account and invalidate its browser sessions.
    RemoveUser { username: String },
    /// Create or print the private secret to use in GitHub's webhook settings.
    WebhookSecret,
}
#[tokio::main]
async fn main() -> Result<()> {
    agentmanager::tracing_init();
    let args = Args::parse();
    let store = Store::open(&args.data_dir)?;
    match args.command {
        Some(Action::SetPassword { stdin, username }) => {
            let username = security::username(&username)?;
            let password = if stdin {
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                line.trim_end_matches(['\r', '\n']).to_owned()
            } else {
                let password =
                    rpassword::prompt_password("New password (at least 12 characters): ")?;
                if password != rpassword::prompt_password("Repeat password: ")? {
                    bail!("Passwords do not match");
                }
                password
            };
            store.set_user_password(&username, &security::hash_password(&password)?)?;
            println!(
                "Password saved for {username}. That account's existing browser logins are invalidated; no restart is needed."
            );
            return Ok(());
        }
        Some(Action::RegistrationKey) => {
            println!("{}", store.enrollment()?);
            return Ok(());
        }
        Some(Action::Users) => {
            for user in store.users()? {
                println!("{}", user.username);
            }
            return Ok(());
        }
        Some(Action::RemoveUser { username }) => {
            store.remove_user(&username)?;
            println!("Account removed.");
            return Ok(());
        }
        Some(Action::WebhookSecret) => {
            let path = args.data_dir.join("github-webhook.secret");
            if !path.exists() {
                security::private_write(&path, security::token().as_bytes())?;
            }
            println!("{}", std::fs::read_to_string(path)?.trim());
            return Ok(());
        }
        None => {}
    }
    if !args.listen.ip().is_loopback() {
        bail!("Listen on loopback and put Caddy in front for HTTPS; see deploy/Caddyfile");
    }
    let deployment = if let Some(path) = args.deployment_secret_file {
        let secret = std::fs::read_to_string(path)?;
        let (deployment, receiver) = agentmanager::webhook::Deployment::new(
            args.deployment_repository,
            secret.trim().as_bytes().to_vec(),
        )?;
        tokio::spawn(agentmanager::webhook::run(receiver));
        Some(deployment)
    } else {
        None
    };
    let app = App::with_deployment(store, &args.public_url, deployment)?;
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(listen=%args.listen,url=%args.public_url,"server started");
    axum::serve(listener, router(app.clone()))
        .with_graceful_shutdown(async move {
            agentmanager::shutdown_signal().await;
            app.shutdown();
        })
        .await?;
    Ok(())
}
