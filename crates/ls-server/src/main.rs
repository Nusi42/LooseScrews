//! `ls-server` binary: the authoritative Loose Screws game server (skeleton).

use clap::Parser;

/// Command line arguments.
#[derive(Debug, Parser)]
#[command(
    name = "ls-server",
    version,
    about = "Loose Screws authoritative game server"
)]
struct Args {
    /// Address to bind the game server to.
    #[arg(long, default_value = ls_server::DEFAULT_BIND)]
    bind: String,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    tracing::info!(bind = %args.bind, tick_hz = ls_math::TICK_HZ, "ls-server skeleton; not implemented yet");
    Ok(())
}
