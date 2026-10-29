use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "updater",
    version,
    about = "Install tested GitHub releases of the Agent Manager server"
)]
struct Args {
    #[arg(long)]
    config: PathBuf,
    /// Report the latest release without changing the running server.
    #[arg(long)]
    check: bool,
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    agentmanager::updater::run(&args.config, args.check).await
}
