//! `ls-server` binary: the authoritative Loose Screws game server.
//!
//! ```text
//! ls-server --bind 0.0.0.0:7777 --code <share code> \
//!     --ruleset data/rulesets/paint_the_void.ron --max-players 8 --web-root web/dist
//! ls-server --ruleset LRAV-741Q-J900-01HQ-M --ban tectonic_charge --ban railspike \
//!     --balance-list balance.ron
//! ls-server --playlist playlist.ron --warmup 20 --countdown 5 --private-rooms 2
//! ```
//!
//! Rules: `--ruleset` takes a rule set file or an `LR...` rule set code (from
//! the browser rule set editor); `--ban <weapon_id>` (repeatable) and
//! `--balance-list <file>` (a `ls_sim::ruleset::BalanceList`: weapons and
//! gadgets banned or retuned server-wide) apply on top. The balance list is
//! re-read before every match. The effective rules are logged with their
//! code and sent to every player in the `Welcome`.
//!
//! Rotation: `--playlist <file>` (see `ls_server::playlist`); without it,
//! `--ruleset` plays that rule set on every match (a one-entry playlist) and
//! neither flag plays the default rotation (the built-in mode rule sets).
//! Each match runs through the lobby: warm-up (weapons off, up to
//! `--warmup` seconds or until everyone is ready), `--countdown`, the match,
//! then its result for `--intermission` seconds while the next match is
//! built; players stay connected and get the next match's `Welcome`.
//! `--warmup 0 --countdown 0` turns the lobby off (matches start at once).
//! `--private-rooms` caps the private rooms (duels, parties) next to the
//! public room.
//!
//! Server list: `--directory http://host:port` registers the server with an
//! `ls-directory` service (heartbeat every 10 s, removed on shutdown) as
//! `--server-name` in `--region`, reachable at `--public-address`
//! (`host:port`; default: the bound address). Without `--directory` the
//! server is reachable by link or address only. Invites, parties and duels
//! use the JSON API at `/api` (see `ls_server::api`).
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
use ls_server::{match_config_with, LobbyTiming, Playlist, Server, ServerConfig, ServerRules};
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

    /// Share code of the world (a random world if omitted). With the
    /// default rotation, every match plays on this world.
    #[arg(long, conflicts_with = "playlist")]
    code: Option<String>,

    /// Rule set file, LR rule set code or built-in rule set id, played on
    /// every match (the default rotation if omitted).
    #[arg(long, value_name = "FILE|CODE|ID", conflicts_with = "playlist")]
    ruleset: Option<String>,

    /// Playlist file: the match rotation (rule set, world, player range per
    /// entry).
    #[arg(long, value_name = "FILE")]
    playlist: Option<PathBuf>,

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

    /// Seconds a finished match's result stays on screen before the next
    /// match's warm-up (players stay connected).
    #[arg(long, default_value_t = 10.0, value_parser = parse_interval)]
    intermission: f64,

    /// Longest warm-up in seconds (weapons off) before the countdown starts,
    /// once enough players are there; shorter when everyone is ready. 0
    /// together with --countdown 0 turns the lobby off.
    #[arg(long, default_value_t = 30.0, value_parser = parse_lobby_secs)]
    warmup: f64,

    /// Countdown in seconds before a match starts.
    #[arg(long, default_value_t = 10.0, value_parser = parse_lobby_secs)]
    countdown: f64,

    /// Most private rooms (duels, parties, custom matches) at once, each on
    /// its own thread; capped at one per CPU core beyond the public room's.
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(0..=64))]
    private_rooms: u8,

    /// Directory service to register with for the server list
    /// (`http://host:port[/path]`; none by default).
    #[arg(long, value_name = "URL")]
    directory: Option<String>,

    /// Server name in the server list.
    #[arg(long, value_name = "NAME", default_value = ls_server::directory::DEFAULT_SERVER_NAME)]
    server_name: String,

    /// Region tag in the server list (e.g. eu-west).
    #[arg(long, value_name = "TAG", default_value = "")]
    region: String,

    /// Public host:port players reach this server at (server list, invite
    /// links) [default: the --bind address; localhost for an unspecified
    /// one].
    #[arg(long, value_name = "HOST:PORT")]
    public_address: Option<String>,

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

fn parse_lobby_secs(s: &str) -> Result<f64, String> {
    match s.parse::<f64>() {
        Ok(v) if v.is_finite() && (0.0..=3600.0).contains(&v) => Ok(v),
        _ => Err("expected seconds between 0 and 3600".into()),
    }
}

/// The server configuration the flags describe (up to the network setup).
fn server_config(args: &Args) -> anyhow::Result<ServerConfig> {
    let rules = ServerRules::new(
        args.ruleset.as_deref(),
        args.bans.clone(),
        args.balance_list.clone(),
    )
    .context("invalid match configuration")?;
    let match_cfg =
        match_config_with(args.code.as_deref(), &rules).context("invalid match configuration")?;
    let playlist = match (&args.playlist, &args.ruleset) {
        (Some(path), _) => Some(
            Playlist::load(path, &args.bans, args.balance_list.as_deref())
                .context("invalid playlist")?,
        ),
        // --ruleset: a one-entry playlist (ServerConfig::effective_playlist).
        (None, Some(_)) => None,
        (None, None) => {
            let world = args.code.is_some().then_some(match_cfg.code);
            Some(
                Playlist::default_rotation(world, &args.bans, args.balance_list.as_deref())
                    .context("invalid default rotation")?,
            )
        }
    };
    let mut cfg = ServerConfig::new(args.bind, match_cfg);
    cfg.rules = Some(rules);
    cfg.playlist = playlist;
    cfg.webtransport = (!args.no_webtransport).then(|| args.wt_bind.unwrap_or(args.bind));
    cfg.room.max_players = args.max_players;
    cfg.stats_interval = Duration::from_secs_f64(args.stats_interval);
    cfg.lobby = LobbyTiming {
        warmup: Duration::from_secs_f64(args.warmup),
        countdown: Duration::from_secs_f64(args.countdown),
        results: Duration::from_secs_f64(args.intermission),
    };
    cfg.private_rooms = usize::from(args.private_rooms);
    if let Some(url) = &args.directory {
        ls_directory::DirectoryClient::new(url).context("invalid --directory")?;
    }
    if let Some(a) = &args.public_address {
        ls_directory::api::validate_address(a)
            .map_err(|e| anyhow::anyhow!("invalid --public-address {a:?}: {e}"))?;
    }
    cfg.listing = ls_server::Listing {
        directory: args.directory.clone(),
        server_name: Some(args.server_name.clone()),
        region: args.region.clone(),
        public_address: args.public_address.clone(),
    };
    Ok(cfg)
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();

    let mut cfg = server_config(&args)?;
    cfg.web_root = args.web_root.clone().or_else(|| {
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

/// Resolves once at least one player has joined and every room has been
/// empty for a moment since (short gaps, e.g. a reconnect, do not count).
async fn wait_until_empty(server: &ls_server::ServerHandle) {
    const GRACE: Duration = Duration::from_secs(2);
    let mut empty_since: Option<std::time::Instant> = None;
    let mut had_players = false;
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let players = server.total_players();
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

    #[test]
    fn rotation_and_lobby_flags_build_the_config() {
        // Neither --ruleset nor --playlist: the default rotation.
        let cfg = server_config(&parse(&[]).unwrap()).unwrap();
        let p = cfg.playlist.as_ref().expect("default rotation");
        assert!(p.entries().iter().any(|e| e.source == "paint_the_void"));
        assert_eq!(cfg.lobby, LobbyTiming::default());
        assert_eq!(cfg.private_rooms, 4);
        // --code: every entry on that world.
        let code = ls_worldgen::ShareCode::new(9, ls_worldgen::Sliders::default())
            .encode()
            .unwrap();
        let cfg = server_config(&parse(&["--code", &code]).unwrap()).unwrap();
        assert!(cfg.playlist.unwrap().entries().iter().all(|e| matches!(
            e.world,
            ls_server::WorldChoice::Fixed(c) if c.encode().unwrap() == code
        )));
        // --ruleset alone: a one-entry playlist of that rule set.
        let a = parse(&[
            "--ruleset",
            "sandbox_all_weapons",
            "--warmup",
            "0",
            "--countdown",
            "0",
            "--intermission",
            "3",
            "--private-rooms",
            "0",
        ])
        .unwrap();
        let cfg = server_config(&a).unwrap();
        assert!(cfg.playlist.is_none());
        let p = cfg.effective_playlist();
        assert_eq!(p.len(), 1);
        assert_eq!(
            p.entries()[0].rules.base,
            ls_sim::RuleSet::sandbox_all_weapons()
        );
        assert!(cfg.lobby.is_off());
        assert_eq!(cfg.lobby.results, Duration::from_secs(3));
        assert_eq!(cfg.private_rooms, 0);
        // Server list flags.
        assert_eq!(cfg.listing.directory, None);
        let cfg = server_config(
            &parse(&[
                "--directory",
                "http://127.0.0.1:7780",
                "--server-name",
                "Den",
                "--region",
                "eu",
                "--public-address",
                "play.example.com:7777",
            ])
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            cfg.listing.directory.as_deref(),
            Some("http://127.0.0.1:7780")
        );
        assert_eq!(cfg.listing.server_name.as_deref(), Some("Den"));
        assert_eq!(cfg.listing.region, "eu");
        assert_eq!(
            cfg.listing.public_address.as_deref(),
            Some("play.example.com:7777")
        );
        assert!(server_config(&parse(&["--directory", "https://x"]).unwrap()).is_err());
        assert!(server_config(&parse(&["--public-address", "a b"]).unwrap()).is_err());
        // --playlist conflicts with --ruleset and --code; ranges are checked.
        assert!(parse(&["--playlist", "p.ron", "--ruleset", "x"]).is_err());
        assert!(parse(&["--playlist", "p.ron", "--code", &code]).is_err());
        assert!(parse(&["--private-rooms", "65"]).is_err());
        assert!(parse(&["--warmup", "-1"]).is_err());
        let e =
            server_config(&parse(&["--playlist", "no/such/playlist.ron"]).unwrap()).unwrap_err();
        assert!(format!("{e:#}").contains("no/such/playlist.ron"), "{e:#}");
    }
}
