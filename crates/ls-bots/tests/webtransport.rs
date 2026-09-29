//! Bots over WebTransport against the real `ls-server`: the certificate hash
//! comes from `/cert-hash`, inputs travel as datagrams, snapshots arrive as
//! datagrams.

use std::thread;
use std::time::{Duration, Instant};

use ls_bots::{
    exit_code, run_bot_blocking, summary, BotConfig, BotStats, DisconnectPolicy, StopSignal,
    TransportKind,
};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::MatchConfig;
use ls_server::{Server, ServerConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn run_bots(url: &str, count: usize, secs: f64) -> Vec<BotStats> {
    let stop = StopSignal::new(Some(Instant::now() + Duration::from_secs_f64(secs)));
    let handles: Vec<_> = (0..count)
        .map(|i| {
            let mut cfg = BotConfig::new(format!("bot{}", i + 1), url, 70 + i as u64);
            cfg.transport = TransportKind::WebTransport;
            cfg.start_delay = Duration::from_millis(300 * i as u64);
            cfg.connect_timeout = Duration::from_secs(5);
            cfg.reconnect = false;
            let stop = stop.clone();
            thread::spawn(move || run_bot_blocking(cfg, stop))
        })
        .collect();
    handles.into_iter().map(|h| h.join().unwrap()).collect()
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "full level generation is slow in debug builds; run with --release"
)]
fn bots_play_over_webtransport() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let code = ShareCode::new(0x0077_0c4e, Sliders::new(40, 30));
    let mut cfg = ServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        MatchConfig::paint_the_void(code),
    );
    cfg.webtransport = Some("127.0.0.1:0".parse().unwrap());
    let server = rt.block_on(Server::start(cfg)).unwrap();
    let url = format!("ws://{}/ws", server.local_addr());

    let secs = 10.0;
    let stats = run_bots(&url, 3, secs);
    let text = summary(&stats, secs, DisconnectPolicy::Fail);
    eprintln!("{text}");
    let server_stats = server.stats();
    rt.block_on(server.shutdown());
    assert_eq!(exit_code(&stats, DisconnectPolicy::Fail), 0, "{text}");
    for s in &stats {
        assert_eq!(s.joins, 1, "{}: {:?}", s.name, s.errors);
        assert!(
            s.inputs_sent > 60 * 5,
            "{}: {} inputs",
            s.name,
            s.inputs_sent
        );
        assert!(
            s.snapshots > 30 * 5,
            "{}: {} snapshots",
            s.name,
            s.snapshots
        );
        assert!(s.pongs > 5, "{}: {} pongs", s.name, s.pongs);
        assert!(
            s.max_reconcile_error < 1.0,
            "{}: {}",
            s.name,
            s.max_reconcile_error
        );
    }
    assert_eq!(server_stats.kicked, 0, "{server_stats:?}");
    assert_eq!(server_stats.connections, 3, "{server_stats:?}");
}

/// A server without WebTransport (404 at `/cert-hash`): the bot never joins
/// and says why.
#[test]
fn server_without_webtransport_counts_as_never_joined() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf).await;
                let _ = s
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
            });
        }
    });
    let stats = run_bots(&format!("ws://{addr}/ws"), 1, 1.5);
    assert_eq!(exit_code(&stats, DisconnectPolicy::Fail), 2);
    assert_eq!(stats[0].joins, 0);
    assert!(
        stats[0]
            .errors
            .iter()
            .any(|e| e.contains("no WebTransport")),
        "{:?}",
        stats[0].errors
    );
}
