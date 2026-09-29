//! `ls-server` binary: the authoritative Loose Screws game server.
//!
//! ```text
//! ls-server --bind 0.0.0.0:7777 --code <share code> \
//!     --ruleset data/rulesets/paint_the_void.ron --max-players 8 --web-root web/dist
//! ls-server --ruleset LR6V-69CM-G006-58R --ban tectonic_charge --ban railspike \
//!     --balance-list balance.ron
//! ```
//!
//! Rules: `--ruleset` takes a rule set file or an `LR...` rule set code (from
//! the browser rule set editor); `--ban <weapon_id>` (repeatable) and
//! `--balance-list <file>` (a `ls_sim::ruleset::BalanceList`: weapons and
//! gadgets banned or retuned server-wide) apply on top. The balance list is
//! re-read before every match. The effective rules are logged with their
//! code and sent to every player in the `Welcome`.
//!
//! After a match ends, players see the result for `--intermission` seconds,
//! then get `Disconnect` with `ls_game::MATCH_RESTART_REASON` and rejoin; the
//! first join builds the next match on the same level.
//!
//! WebTransport listens on UDP `--wt-bind` (default: the `--bind` address,
//! so the same port number); the hash of its self-signed certificate is
//! served at `GET /cert-hash`. `--no-webtransport` turns it off.
//!
//! Log filtering follows `RUST_LOG` (default `info`).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use ls_server::{match_config_with, Server, ServerConfig, ServerRules};
use tracing_subscriber::EnvFilter;

/// Command line arguments.
#[derive(Debug, Parser)]
#[command(
    name = "ls-server",
    version,
    about = "Loose Screws authoritative game server"
)]
struct Args {
    /// Address to bind (HTTP, and the WebSocket at /ws).
    #[arg(long, default_value = ls_server::DEFAULT_BIND)]
    bind: SocketAddr,

    /// UDP address of the WebTransport endpoint [default: the --bind address].
    #[arg(long, conflicts_with = "no_webtransport")]
    wt_bind: Option<SocketAddr>,

    /// Do not start the WebTransport endpoint (clients use WebSocket).
    #[arg(long)]
    no_webtransport: bool,

    /// Share code of the world (a random world if omitted).
    #[arg(long)]
    code: Option<String>,

    /// Rule set file or LR rule set code (the built-in Paint the Void rules
    /// if omitted).
    #[arg(long, value_name = "FILE|CODE")]
    ruleset: Option<String>,

    /// Bans a weapon on top of the rule set (repeatable).
    #[arg(long = "ban", value_name = "WEAPON_ID")]
    bans: Vec<String>,

    /// Balance list file (weapons banned or tuned server-wide, applied on
    /// top of the rule set; re-read before every match).
    #[arg(long, value_name = "FILE")]
    balance_list: Option<PathBuf>,

    /// Maximum number of players.
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u8).range(1..=ls_net::MAX_SHIPS as i64))]
    max_players: u8,

    /// Directory with the built web client, served over HTTP for local play
    /// [default: web/dist if it exists].
    #[arg(long)]
    web_root: Option<PathBuf>,

    /// Seconds between two tick statistics log lines.
    #[arg(long, default_value_t = 10.0, value_parser = parse_interval)]
    stats_interval: f64,

    /// Seconds a finished match stays on screen before players are sent
    /// off to rejoin the next match.
    #[arg(long, default_value_t = 10.0, value_parser = parse_interval)]
    intermission: f64,

    /// Shut down gracefully once every player has left (after at least one
    /// joined). For scripted runs such as tools/e2e.*.
    #[arg(long)]
    exit_when_empty: bool,
}

fn parse_interval(s: &str) -> Result<f64, String> {
    match s.parse::<f64>() {
        Ok(v) if v.is_finite() && (0.1..=3600.0).contains(&v) => Ok(v),
        _ => Err("expected seconds between 0.1 and 3600".into()),
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();

    let rules = ServerRules::new(
        args.ruleset.as_deref(),
        args.bans.clone(),
        args.balance_list.clone(),
    )
    .context("invalid match configuration")?;
    let match_cfg =
        match_config_with(args.code.as_deref(), &rules).context("invalid match configuration")?;
    let mut cfg = ServerConfig::new(args.bind, match_cfg);
    cfg.rules = Some(rules);
    cfg.webtransport = (!args.no_webtransport).then(|| args.wt_bind.unwrap_or(args.bind));
    cfg.room.max_players = args.max_players;
    cfg.stats_interval = Duration::from_secs_f64(args.stats_interval);
    cfg.intermission = Duration::from_secs_f64(args.intermission);
    cfg.web_root = args.web_root.or_else(|| {
        let default = PathBuf::from(ls_server::DEFAULT_WEB_ROOT);
        default.is_dir().then_some(default)
    });
    if let Some(root) = &cfg.web_root {
        tracing::info!(web_root = %root.display(), "serving the web client");
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("ls-io")
        .build()?;
    runtime.block_on(async move {
        let server = Server::start(cfg).await.context("server failed to start")?;
        tracing::info!(
            "share code {} - open http://{} to play",
            server.share_code(),
            display_addr(server.local_addr())
        );
        let ctrl_c = tokio::signal::ctrl_c();
        if args.exit_when_empty {
            tokio::select! {
                r = ctrl_c => r.context("cannot listen for Ctrl+C")?,
                _ = wait_until_empty(&server) => {
                    tracing::info!("all players left; exiting (--exit-when-empty)");
                }
            }
        } else {
            ctrl_c.await.context("cannot listen for Ctrl+C")?;
        }
        let s = server.stats();
        server.shutdown().await;
        tracing::info!(
            ticks = s.total_ticks,
            connections = s.connections,
            rejected = s.rejected,
            kicked = s.kicked,
            timeouts = s.timeouts,
            skipped_ticks = s.skipped_ticks,
            snapshots_sent = s.snapshots_sent,
            snapshot_bytes = s.snapshot_bytes,
            edits = s.total_edits,
            matches = s.matches,
            desync_warnings = s.desync_warnings,
            "final stats"
        );
        Ok(())
    })
}

/// Resolves once at least one player has joined and the room has been empty
/// for a moment since (short gaps, e.g. a reconnect, do not count).
async fn wait_until_empty(server: &ls_server::ServerHandle) {
    const GRACE: Duration = Duration::from_secs(2);
    let mut empty_since: Option<std::time::Instant> = None;
    let mut had_players = false;
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let players = server.stats().players;
        had_players |= players > 0;
        if !had_players || players > 0 {
            empty_since = None;
            continue;
        }
        let since = *empty_since.get_or_insert_with(std::time::Instant::now);
        if since.elapsed() >= GRACE {
            return;
        }
    }
}

/// `0.0.0.0` is not browsable; show `localhost` instead.
fn display_addr(addr: SocketAddr) -> String {
    if addr.ip().is_unspecified() {
        format!("localhost:{}", addr.port())
    } else {
        addr.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, clap::Error> {
        Args::try_parse_from(std::iter::once("ls-server").chain(args.iter().copied()))
    }

    #[test]
    fn rule_flags_parse() {
        let a = parse(&[]).unwrap();
        assert_eq!((a.ruleset, a.bans.len(), a.balance_list), (None, 0, None));
        let a = parse(&[
            "--ruleset",
            "LR6V-69CM-G006-58R",
            "--ban",
            "railspike",
            "--ban",
            "tectonic_charge",
            "--balance-list",
            "balance.ron",
        ])
        .unwrap();
        assert_eq!(a.ruleset.as_deref(), Some("LR6V-69CM-G006-58R"));
        assert_eq!(a.bans, ["railspike", "tectonic_charge"]);
        assert_eq!(a.balance_list, Some(PathBuf::from("balance.ron")));
        let a = parse(&["--ruleset", "data/rulesets/paint_the_void.ron"]).unwrap();
        assert_eq!(
            a.ruleset.as_deref(),
            Some("data/rulesets/paint_the_void.ron")
        );
        assert!(parse(&["--ban"]).is_err(), "--ban needs a weapon id");
        assert!(parse(&["--balance-list"]).is_err());
    }

    /// What `main` does with the parsed flags, up to the match config.
    #[test]
    fn parsed_rule_flags_build_the_effective_rules() {
        let code = ls_sim::RuleSet::paint_the_void().to_code().unwrap();
        let a = parse(&["--ruleset", &code, "--ban", "railspike"]).unwrap();
        let rules = ServerRules::new(a.ruleset.as_deref(), a.bans, a.balance_list).unwrap();
        let mc = match_config_with(None, &rules).unwrap();
        assert!(mc.rules.availability("railspike").is_banned());
        assert!(!mc.rules.pickups.contains_key("railspike"));
        let a = parse(&["--ban", "no_such_gun"]).unwrap();
        let e = ServerRules::new(None, a.bans, None)
            .unwrap_err()
            .to_string();
        assert!(e.contains("no_such_gun"), "{e}");
        let a = parse(&["--ruleset", "LR00-0000"]).unwrap();
        assert!(ServerRules::new(a.ruleset.as_deref(), vec![], None).is_err());
    }
}
