use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "worker",
    version,
    about = "Connect this machine to an Agent Manager server"
)]
struct Args {
    #[arg(long)]
    host: Option<String>,
    /// Single-use enrollment key. Prefer AGENTMANAGER_KEY to avoid shell history.
    #[arg(long, env = "AGENTMANAGER_KEY", hide_env_values = true)]
    key: Option<String>,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    credential_file: Option<PathBuf>,
    #[arg(long, default_value = "codex")]
    codex: String,
}
#[tokio::main]
async fn main() -> Result<()> {
    agentmanager::tracing_init();
    let args = Args::parse();
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        });
    let file = args
        .credential_file
        .unwrap_or_else(|| base.join("agentmanager/worker.credential.json"));
    let name = args.name.unwrap_or_else(|| {
        std::fs::read_to_string("/etc/hostname")
            .unwrap_or_else(|_| "worker".into())
            .trim()
            .to_owned()
    });
    let credential =
        agentmanager::worker::credential(args.host.as_deref(), args.key.as_deref(), &name, &file)
            .await?;
    tokio::select! {
        result=agentmanager::worker::run(credential,args.codex,file.with_extension("sessions.json"))=>result,
        _=agentmanager::shutdown_signal()=>Ok(()),
    }
}
