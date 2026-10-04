//! `ls-directory`: the optional server list service for Loose Screws.
//!
//! ```text
//! ls-directory --bind 0.0.0.0:7780
//! ls-server --directory http://<directory host>:7780 --server-name "My server" --region eu --public-address game.example.com:7777
//! ```
//!
//! See `docs/architecture/directory.md` for the API.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use ls_directory::{Config, DirectoryServer, DEFAULT_BIND};
use tracing_subscriber::EnvFilter;

/// Loose Screws server directory: game servers register with heartbeats,
/// browsers read the server list (`GET /servers`) and relay invites.
#[derive(Debug, Parser)]
#[command(name = "ls-directory", version)]
struct Args {
    /// Listen address (TCP, plain HTTP; put a TLS proxy in front for https pages).
    #[arg(long, default_value = DEFAULT_BIND)]
    bind: SocketAddr,
    /// Seconds after the last heartbeat until a server disappears.
    #[arg(long, default_value_t = 30)]
    ttl: u64,
    /// Heartbeat interval suggested to servers, seconds.
    #[arg(long, default_value_t = 10)]
    heartbeat: u64,
    /// Most listed servers.
    #[arg(long, default_value_t = 1000)]
    max_servers: usize,
    /// Seconds a relayed invite stays available.
    #[arg(long, default_value_t = 30)]
    invite_ttl: u64,
    /// Most pending invites in total.
    #[arg(long, default_value_t = 10_000)]
    max_invites: usize,
    /// Sustained requests per second per client IP.
    #[arg(long, default_value_t = 10)]
    rate: u32,
    /// Burst of requests per client IP.
    #[arg(long, default_value_t = 40)]
    burst: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let a = Args::parse();
    anyhow::ensure!(
        a.ttl > a.heartbeat && a.heartbeat > 0,
        "--ttl must exceed --heartbeat (> 0)"
    );
    let cfg = Config {
        ttl: Duration::from_secs(a.ttl),
        heartbeat: Duration::from_secs(a.heartbeat),
        max_servers: a.max_servers,
        invite_ttl: Duration::from_secs(a.invite_ttl.max(1)),
        max_invites: a.max_invites,
        rate_per_sec: a.rate.max(1),
        rate_burst: a.burst.max(1),
        ..Config::default()
    };
    let server = DirectoryServer::bind(a.bind, cfg)
        .await
        .with_context(|| format!("binding {}", a.bind))?;
    let (tx, rx) = tokio::sync::watch::channel(false);
    let task = server.spawn(rx);
    tokio::signal::ctrl_c()
        .await
        .context("waiting for Ctrl+C")?;
    let _ = tx.send(true);
    let _ = task.await;
    Ok(())
}
