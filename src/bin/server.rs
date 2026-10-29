use agentmanager::{
    security,
    server::{App, router},
    store::Store,
};
use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Parser)]
#[command(about = "Self-hosted Codex session server")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    public_url: String,
    #[arg(long, default_value = ".data/server")]
    data_dir: PathBuf,
    #[command(subcommand)]
    command: Option<Action>,
}
#[derive(Subcommand)]
enum Action {
    /// Set or replace the login password. Restart the server after changing it.
    SetPassword {
        /// Read one password line from stdin, for unattended setup.
        #[arg(long)]
        stdin: bool,
    },
    /// Print a single-use, ten-minute worker registration key.
    RegistrationKey,
}
#[tokio::main]
async fn main() -> Result<()> {
    agentmanager::tracing_init();
    let args = Args::parse();
    let store = Store::open(&args.data_dir)?;
    match args.command {
        Some(Action::SetPassword { stdin }) => {
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
            store.set_password(&security::hash_password(&password)?)?;
            println!(
                "Password saved. Restart a running server to apply it and invalidate browser sessions."
            );
            return Ok(());
        }
        Some(Action::RegistrationKey) => {
            println!("{}", store.enrollment()?);
            return Ok(());
        }
        None => {}
    }
    if !args.listen.ip().is_loopback() {
        bail!("Listen on loopback and put Caddy in front for HTTPS; see deploy/Caddyfile");
    }
    let app = App::new(store, &args.public_url)?;
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
