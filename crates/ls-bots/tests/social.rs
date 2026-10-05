//! Bots in the lobby, the server list and duels (M3 package C2), against a
//! real `ls-server` (and a real `ls-directory`) on loopback ports:
//!
//! - bots press Ready in the warm-up, so the countdown starts long before
//!   the warm-up's time limit;
//! - `--directory` bots find the server in the directory and join it;
//! - `--accept-duels` bots accept a challenge, play the duel in the
//!   challenger's private room and come back to the public room after the
//!   duel's match.

use std::net::SocketAddr;
use std::thread;
use std::time::{Duration, Instant};

use ls_bots::{run_bot_blocking, BotConfig, BotStats, StopSignal};
use ls_directory::{Config as DirConfig, DirectoryServer};
use ls_game::ls_sim::{ModeKind, RuleSet};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::RoomConfig;
use ls_game::MatchConfig;
use ls_server::{LobbyTiming, Server, ServerConfig, ServerHandle};
use serde_json::{json, Value};

fn config(lobby: LobbyTiming) -> ServerConfig {
    let mut cfg = ServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        MatchConfig::paint_the_void(ShareCode::new(0xc2c2, Sliders::new(40, 30))),
    );
    cfg.room = RoomConfig {
        pregenerate: false,
        ..RoomConfig::default()
    };
    cfg.lobby = lobby;
    cfg.private_rooms = 2;
    cfg
}

/// Starts `n` bots (names `prefix1..`) with `setup` applied to each config.
fn start_bots(
    n: u32,
    prefix: &str,
    server: SocketAddr,
    stop: &StopSignal,
    setup: impl Fn(&mut BotConfig),
) -> Vec<thread::JoinHandle<BotStats>> {
    (0..n)
        .map(|i| {
            let mut cfg = BotConfig::new(
                format!("{prefix}{}", i + 1),
                format!("ws://{server}/ws"),
                0xc2 + u64::from(i),
            );
            cfg.reconnect_delay = Duration::from_millis(300);
            setup(&mut cfg);
            let stop = stop.clone();
            thread::Builder::new()
                .stack_size(8 << 20)
                .spawn(move || run_bot_blocking(cfg, stop))
                .unwrap()
        })
        .collect()
}

fn finish(stop: &StopSignal, bots: Vec<thread::JoinHandle<BotStats>>) -> Vec<BotStats> {
    stop.stop();
    let stats: Vec<BotStats> = bots.into_iter().map(|h| h.join().unwrap()).collect();
    for s in &stats {
        eprintln!(
            "{}: joins {}, ready {}, duels {}, matches over {}, errors {:?}",
            s.name, s.joins, s.ready_presses, s.duels, s.matches_over, s.errors
        );
        assert_eq!(s.protocol_errors, 0, "{}: {:?}", s.name, s.errors);
        assert_eq!(s.desyncs, 0, "{}: {:?}", s.name, s.errors);
    }
    stats
}

async fn wait_for(what: &str, secs: u64, mut f: impl FnMut() -> bool) -> Duration {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(secs),
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    start.elapsed()
}

fn public(server: &ServerHandle) -> ls_server::RoomStatus {
    server
        .rooms()
        .into_iter()
        .map(|(s, _)| s)
        .find(|s| s.label == "public")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg_attr(
    debug_assertions,
    ignore = "level generation is slow in debug builds; run with --release"
)]
async fn bots_press_ready_so_the_countdown_starts_early() {
    // A 10-minute warm-up: only Ready (everyone) starts the countdown.
    let lobby = LobbyTiming {
        warmup: Duration::from_secs(600),
        countdown: Duration::from_secs(1),
        results: Duration::from_secs(1),
    };
    let server = Server::start(config(lobby)).await.unwrap();
    let stop = StopSignal::new(None);
    let bots = start_bots(2, "ready", server.local_addr(), &stop, |_| {});
    wait_for("two players", 30, || public(&server).players == 2).await;
    assert_eq!(public(&server).phase, "warmup");
    let took = wait_for("the match after Ready", 60, || {
        public(&server).phase == "match"
    })
    .await;
    eprintln!(
        "match started {:.1} s after both joined",
        took.as_secs_f32()
    );
    // Give the new `Welcome` time to reach the bots.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let stats = finish(&stop, bots);
    for s in &stats {
        assert!(s.ready_presses >= 1, "{} pressed Ready", s.name);
        assert_eq!(s.disconnects, 0, "{}", s.name);
        assert!(
            s.match_restarts >= 1,
            "{}: warm-up -> match in place",
            s.name
        );
    }
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg_attr(
    debug_assertions,
    ignore = "level generation is slow in debug builds; run with --release"
)]
async fn bots_find_the_server_in_the_directory() {
    let dir = DirectoryServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        DirConfig {
            heartbeat: Duration::from_secs(1),
            ..DirConfig::default()
        },
    )
    .await
    .unwrap();
    let url = format!("http://{}", dir.local_addr().unwrap());
    let (_dir_stop, rx) = tokio::sync::watch::channel(false);
    dir.spawn(rx);
    let mut cfg = config(LobbyTiming::off(Duration::from_secs(1)));
    cfg.listing.directory = Some(url.clone());
    cfg.listing.server_name = Some("Listed".into());
    let server = Server::start(cfg).await.unwrap();
    let stop = StopSignal::new(None);
    // `--server` points nowhere: only the directory knows the server.
    let bots = start_bots(2, "listed", "127.0.0.1:9".parse().unwrap(), &stop, |c| {
        c.directory = Some(url.clone());
    });
    wait_for("both bots on the listed server", 60, || {
        server.total_players() == 2
    })
    .await;
    let stats = finish(&stop, bots);
    for s in &stats {
        assert!(s.joins >= 1, "{} joined", s.name);
    }
    server.shutdown().await;
}

/// One JSON request on a fresh connection.
async fn post(addr: SocketAddr, path: &str, body: Value) -> Value {
    ls_bots::social::http_json(&addr.to_string(), "POST", path, Some(&body))
        .await
        .unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg_attr(
    debug_assertions,
    ignore = "level generation is slow in debug builds; run with --release"
)]
async fn bots_accept_duels_and_come_back_afterwards() {
    let server = Server::start(config(LobbyTiming::off(Duration::from_secs(1))))
        .await
        .unwrap();
    let addr = server.local_addr();
    let stop = StopSignal::new(None);
    let bots = start_bots(1, "duelist", addr, &stop, |c| c.accept_duels = true);
    let me = ls_bots::social::Identity::for_bot("duelist1", 0xc2);

    // The bot shows up in the roster (its presence).
    let roster = || async {
        ls_bots::social::http_json(&addr.to_string(), "GET", "/api/players", None)
            .await
            .unwrap()
    };
    let start = Instant::now();
    loop {
        let r = roster().await;
        if r["players"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == me.id.as_str())
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "bot in the roster"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // A challenger asks for a short duel (20 s) on its own seed.
    let token = "token-challenger-0123456789";
    post(
        addr,
        "/api/presence",
        json!({"id": "player-ace1", "token": token, "name": "Ace"}),
    )
    .await;
    let mut rules = RuleSet::default_for(ModeKind::TeamBrawl);
    rules.mode.set_field("teams", 0.0);
    rules.mode.set_field("match_secs", 20.0);
    let world = ShareCode::new(0xd0e1, Sliders::new(40, 30))
        .encode()
        .unwrap();
    post(
        addr,
        "/api/duel",
        json!({"id": "player-ace1", "token": token, "name": "Ace", "to": me.id,
               "world_code": world, "rules_code": rules.to_code().unwrap()}),
    )
    .await;
    // The bot accepts: the challenger's inbox gets the room.
    let start = Instant::now();
    let room = loop {
        let inbox = post(
            addr,
            "/api/presence",
            json!({"id": "player-ace1", "token": token, "name": "Ace"}),
        )
        .await;
        if let Some(n) = inbox["invites"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["kind"] == "accepted")
        {
            break n["room"].as_str().unwrap().to_string();
        }
        assert!(start.elapsed() < Duration::from_secs(30), "duel accepted");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    // The challenger (another bot) joins the duel room too.
    let room_code = room.clone();
    let challenger = start_bots(1, "ace", addr, &stop, move |c| {
        c.room = Some(room_code.clone());
        c.reconnect = false;
    });
    let duel_room = |players: usize| {
        server
            .rooms()
            .into_iter()
            .any(|(s, _)| s.label == room && s.players == players && !s.closed)
    };
    wait_for("both players in the duel room", 60, || duel_room(2)).await;
    assert_eq!(public(&server).players, 0, "the bot left the public room");
    let room_status = server
        .rooms()
        .into_iter()
        .find(|(s, _)| s.label == room)
        .unwrap()
        .0;
    assert_eq!(room_status.share_code, world, "the challenger's seed");
    // After the duel's match the bot is back in the public room.
    wait_for("the bot back in the public room", 90, || {
        public(&server).players == 1
    })
    .await;
    let mut stats = finish(&stop, bots);
    stats.extend(finish(&StopSignal::new(None), challenger));
    let bot = &stats[0];
    assert_eq!(bot.duels, 1);
    assert!(bot.joins >= 3, "home, duel, home: {}", bot.joins);
    assert!(bot.matches_over >= 1, "the duel ended");
    assert_eq!(bot.disconnects, 0);
    server.shutdown().await;
}
