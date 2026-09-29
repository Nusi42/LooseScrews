//! `ls-bots`: spawns headless bot clients against a Loose Screws server.
//!
//! ```text
//! ls-bots --server ws://127.0.0.1:7777/ws --count 7 [--duration 600]
//! ```
//!
//! Prints a per-bot summary at exit. Exit code 0: all good; 1: a bot saw a
//! protocol error or desync; 2: a bot never managed to join; 3: a bot lost
//! its connection (server crash, kick, timeout; not with
//! `--allow-disconnects`). A server that restarts the match after it ended
//! (`Disconnect` with `ls_game::MATCH_RESTART_REASON`) is not a failure: the
//! bots rejoin.

use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use ls_bots::{
    exit_code, run_bot_blocking, summary, BotConfig, BotStats, DisconnectPolicy, StopSignal,
};

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
    /// Stop after this many seconds (default: run until Ctrl+C).
    #[arg(long)]
    duration: Option<f64>,
    /// Delay between two bots joining, in milliseconds.
    #[arg(long, default_value_t = 750)]
    stagger_ms: u64,
    /// Base seed for the bots' behavior.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Bot names are this prefix plus a number.
    #[arg(long, default_value = "bot")]
    name_prefix: String,
    /// Do not reconnect after a lost connection.
    #[arg(long)]
    no_reconnect: bool,
    /// Lost connections do not fail the run (exit code 3); for load tests
    /// that restart the server on purpose.
    #[arg(long)]
    allow_disconnects: bool,
    /// Log debug output.
    #[arg(long)]
    verbose: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_max_level(if args.verbose {
            tracing::Level::DEBUG
        } else {
            tracing::Level::INFO
        })
        .init();

    let start = Instant::now();
    let deadline = args
        .duration
        .filter(|d| d.is_finite() && *d > 0.0)
        .map(|d| start + Duration::from_secs_f64(d));
    let stop = StopSignal::new(deadline);
    tracing::info!(
        server = %args.server,
        count = args.count,
        duration = ?args.duration,
        "starting bots"
    );

    let mut handles = Vec::new();
    for i in 0..args.count {
        let name = format!("{}{}", args.name_prefix, i + 1);
        let mut cfg = BotConfig::new(
            name.clone(),
            args.server.clone(),
            args.seed ^ (u64::from(i) + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15),
        );
        cfg.start_delay = Duration::from_millis(args.stagger_ms * u64::from(i));
        cfg.reconnect = !args.no_reconnect;
        let stop = stop.clone();
        let spawned = thread::Builder::new()
            .name(name.clone())
            .stack_size(8 << 20)
            .spawn(move || run_bot_blocking(cfg, stop));
        match spawned {
            Ok(h) => handles.push((name, Some(h))),
            Err(e) => {
                tracing::error!("cannot spawn {name}: {e}");
                handles.push((name, None));
            }
        }
    }

    wait_for_end(&stop, &handles);
    stop.stop();

    let stats: Vec<BotStats> = handles
        .into_iter()
        .map(|(name, h)| match h.map(|h| h.join()) {
            Some(Ok(s)) => s,
            Some(Err(_)) | None => {
                let mut s = BotStats::new(name);
                s.protocol_errors += 1;
                s.note_error("bot thread panicked or could not start");
                s
            }
        })
        .collect();
    let policy = if args.allow_disconnects {
        DisconnectPolicy::Allow
    } else {
        DisconnectPolicy::Fail
    };
    println!("{}", summary(&stats, start.elapsed().as_secs_f64(), policy));
    ExitCode::from(exit_code(&stats, policy) as u8)
}

type Handles = [(String, Option<thread::JoinHandle<BotStats>>)];

/// Blocks until Ctrl+C, the deadline, or every bot thread has finished.
fn wait_for_end(stop: &StopSignal, handles: &Handles) {
    let all_done = || {
        handles
            .iter()
            .all(|(_, h)| h.as_ref().is_none_or(|h| h.is_finished()))
    };
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("no runtime for Ctrl+C handling: {e}");
            while !stop.should_stop() && !all_done() {
                thread::sleep(Duration::from_millis(200));
            }
            return;
        }
    };
    rt.block_on(async {
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        loop {
            tokio::select! {
                _ = &mut ctrl_c => {
                    tracing::info!("interrupted, stopping bots");
                    break;
                }
                _ = tokio::time::sleep(Duration::from_millis(200)) => {
                    if stop.should_stop() || all_done() {
                        break;
                    }
                }
            }
        }
    });
}
