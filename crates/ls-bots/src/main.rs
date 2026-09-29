//! `ls-bots` binary: spawns headless bot clients (skeleton).

use clap::Parser;

/// Command line arguments.
#[derive(Debug, Parser)]
#[command(name = "ls-bots", version, about = "Loose Screws headless bot clients")]
struct Args {
    /// WebSocket URL of the server.
    #[arg(long, default_value = ls_bots::DEFAULT_SERVER)]
    server: String,
    /// Number of bots to spawn.
    #[arg(long, default_value_t = 7)]
    count: u32,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    tracing::info!(server = %args.server, count = args.count, "ls-bots skeleton; not implemented yet");
    Ok(())
}
