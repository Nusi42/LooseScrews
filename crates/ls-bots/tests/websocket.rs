//! The WebSocket runner against real sockets: a minimal in-process server
//! (tokio-tungstenite + an `ls_game` `Room` stepped at 60 Hz on its own
//! thread, one binary frame per message), plus servers that misbehave.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use ls_bots::{
    exit_code, run_bot_blocking, summary, BotConfig, BotStats, DisconnectPolicy, StopSignal,
};
use ls_game::ls_net::Message;
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::MatchConfig;
use tokio::net::TcpListener;
use tokio::sync::mpsc as tmpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;

enum ToRoom {
    Join(u64, Vec<u8>, tmpsc::UnboundedSender<Vec<u8>>),
    Packet(u64, Vec<u8>),
    Leave(u64),
}

/// Starts a tokio runtime on a thread, binds 127.0.0.1:0 and runs `serve`
/// for each accepted WebSocket. Returns the address.
fn spawn_ws_server<F, Fut>(serve: F) -> SocketAddr
where
    F: Fn(u64, tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> Fut
        + Send
        + Sync
        + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            let serve = Arc::new(serve);
            let mut next_conn = 0;
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let conn = next_conn;
                next_conn += 1;
                let serve = serve.clone();
                tokio::spawn(async move {
                    if let Ok(ws) = tokio_tungstenite::accept_async(stream).await {
                        serve(conn, ws).await;
                    }
                });
            }
        });
    });
    rx.recv().unwrap()
}

/// A tiny game server: sockets forward frames to a room thread that steps
/// an `ls_game` `Room` at 60 Hz. Returns the URL and a stop flag. With
/// `drop_first_after`, the first connection is dropped (no close frame)
/// after the server sent it that many frames.
fn spawn_game_server(drop_first_after: Option<usize>) -> (String, Arc<AtomicBool>) {
    let (to_room, from_sockets) = mpsc::channel::<ToRoom>();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_room = stop.clone();
    thread::spawn(move || {
        let code = ShareCode::new(0x0005_0c4e, Sliders::new(40, 30));
        let mut room = Room::new(MatchConfig::paint_the_void(code), RoomConfig::default()).unwrap();
        let mut conns: BTreeMap<u64, (Option<u8>, tmpsc::UnboundedSender<Vec<u8>>)> =
            BTreeMap::new();
        let period = Duration::from_secs_f64(1.0 / 60.0);
        let start = Instant::now();
        let mut ticks: u32 = 0;
        while !stop_room.load(Ordering::SeqCst) {
            while let Ok(ev) = from_sockets.try_recv() {
                match ev {
                    ToRoom::Join(conn, bytes, tx) => match room.join_packet(&bytes) {
                        Ok(id) => {
                            conns.insert(conn, (Some(id), tx));
                        }
                        Err(r) => {
                            let _ = tx.send(Message::Reject(r).encode().unwrap());
                        }
                    },
                    ToRoom::Packet(conn, bytes) => {
                        if let Some((Some(id), _)) = conns.get(&conn) {
                            // Bad packets are the client's problem; keep serving.
                            let _ = room.handle_packet(*id, &bytes);
                        }
                    }
                    ToRoom::Leave(conn) => {
                        if let Some((Some(id), _)) = conns.remove(&conn) {
                            room.leave(id);
                        }
                    }
                }
            }
            room.step();
            for o in room.take_outgoing() {
                if let Some((_, tx)) = conns.values().find(|(id, _)| *id == Some(o.client)) {
                    let _ = tx.send(o.bytes);
                }
            }
            ticks += 1;
            let next = start + period * ticks;
            if let Some(wait) = next.checked_duration_since(Instant::now()) {
                thread::sleep(wait);
            }
        }
    });
    let addr = spawn_ws_server(move |conn, ws| {
        let to_room = to_room.clone();
        async move {
            let (mut sink, mut stream) = ws.split();
            let (tx, mut rx) = tmpsc::unbounded_channel::<Vec<u8>>();
            let mut joined = false;
            let drop_after = drop_first_after.filter(|_| conn == 0);
            let mut sent = 0;
            loop {
                tokio::select! {
                    out = rx.recv() => match out {
                        Some(bytes) => {
                            if sink.send(WsMessage::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                            sent += 1;
                            if drop_after.is_some_and(|n| sent >= n) {
                                break;
                            }
                        }
                        None => break,
                    },
                    incoming = stream.next() => match incoming {
                        Some(Ok(WsMessage::Binary(b))) => {
                            let ev = if joined {
                                ToRoom::Packet(conn, b.to_vec())
                            } else {
                                joined = true;
                                ToRoom::Join(conn, b.to_vec(), tx.clone())
                            };
                            if to_room.send(ev).is_err() {
                                break;
                            }
                        }
                        Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                        Some(Ok(_)) => {}
                    },
                }
            }
            let _ = to_room.send(ToRoom::Leave(conn));
        }
    });
    (format!("ws://{addr}/ws"), stop)
}

fn run_bots(url: &str, count: usize, secs: f64, reconnect: bool) -> Vec<BotStats> {
    run_bots_with(url, count, secs, |cfg| cfg.reconnect = reconnect)
}

fn run_bots_with(
    url: &str,
    count: usize,
    secs: f64,
    tweak: impl Fn(&mut BotConfig),
) -> Vec<BotStats> {
    let stop = StopSignal::new(Some(Instant::now() + Duration::from_secs_f64(secs)));
    let handles: Vec<_> = (0..count)
        .map(|i| {
            let mut cfg = BotConfig::new(format!("bot{}", i + 1), url, 40 + i as u64);
            cfg.start_delay = Duration::from_millis(300 * i as u64);
            cfg.connect_timeout = Duration::from_secs(5);
            tweak(&mut cfg);
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
fn bots_play_over_websocket() {
    let (url, server_stop) = spawn_game_server(None);
    let secs = 10.0;
    let stats = run_bots(&url, 4, secs, true);
    server_stop.store(true, Ordering::SeqCst);
    let text = summary(&stats, secs, DisconnectPolicy::Fail);
    eprintln!("{text}");
    assert_eq!(exit_code(&stats, DisconnectPolicy::Fail), 0, "{text}");
    for s in &stats {
        assert_eq!(s.joins, 1, "{}: {:?}", s.name, s.errors);
        assert_eq!(s.disconnects, 0, "{}: {:?}", s.name, s.errors);
        // ~9 s of play at 60 Hz (level setup takes a moment after joining).
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
    assert!(text.contains("result: OK"));
}

#[test]
fn unreachable_server_counts_as_never_joined() {
    // Bind and drop a listener to get a port nobody listens on.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let stats = run_bots(&format!("ws://127.0.0.1:{port}/ws"), 1, 1.5, false);
    assert_eq!(stats[0].joins, 0);
    assert!(stats[0].connect_failures >= 1);
    assert!(!stats[0].failed());
    assert_eq!(exit_code(&stats, DisconnectPolicy::Fail), 2);
}

#[test]
fn garbage_from_the_server_is_a_protocol_error() {
    let addr = spawn_ws_server(|_, mut ws| async move {
        // Wait for Hello, answer with bytes that are no protocol message.
        let _ = ws.next().await;
        let _ = ws
            .send(WsMessage::Binary(vec![0xff, 0x00, 0x13].into()))
            .await;
        let _ = ws.next().await;
    });
    let stats = run_bots(&format!("ws://{addr}/ws"), 1, 2.0, false);
    assert_eq!(stats[0].protocol_errors, 1, "{:?}", stats[0]);
    assert_eq!(exit_code(&stats, DisconnectPolicy::Fail), 1);
}

#[test]
fn server_full_is_not_a_protocol_error() {
    let addr = spawn_ws_server(|_, mut ws| async move {
        let _ = ws.next().await;
        let reject = Message::Reject(ls_game::ls_net::Reject {
            reason: ls_game::ls_net::RejectReason::ServerFull,
            server_version: ls_game::ls_net::PROTOCOL_VERSION,
            message: "full".into(),
        });
        let _ = ws
            .send(WsMessage::Binary(reject.encode().unwrap().into()))
            .await;
        let _ = ws.next().await;
    });
    let stats = run_bots(&format!("ws://{addr}/ws"), 1, 1.5, false);
    assert_eq!(stats[0].protocol_errors, 0, "{:?}", stats[0]);
    assert_eq!(stats[0].connect_failures, 1);
    assert_eq!(exit_code(&stats, DisconnectPolicy::Fail), 2);
}

/// A valid `Welcome` for a fresh room (no full level generation).
fn welcome_bytes() -> Vec<u8> {
    let code = ShareCode::new(0x0005_0c4e, Sliders::new(40, 30));
    let cfg = RoomConfig {
        pregenerate: false,
        ..RoomConfig::default()
    };
    let mut room = Room::new(MatchConfig::paint_the_void(code), cfg).unwrap();
    let hello = Message::Hello(ls_game::client_hello("x", None, None))
        .encode()
        .unwrap();
    room.join_packet(&hello).unwrap();
    room.take_outgoing().remove(0).bytes
}

#[test]
fn a_server_that_stops_talking_ends_the_session() {
    let welcome = welcome_bytes();
    let addr = spawn_ws_server(move |_, mut ws| {
        let welcome = welcome.clone();
        async move {
            let _ = ws.next().await;
            let _ = ws.send(WsMessage::Binary(welcome.into())).await;
            // Hang: never read again, never send (a stuck sim thread or a
            // half-open connection). Keep the socket open.
            tokio::time::sleep(Duration::from_secs(120)).await;
            drop(ws);
        }
    });
    let secs = 60.0;
    let started = Instant::now();
    let stats = run_bots_with(&format!("ws://{addr}/ws"), 1, secs, |cfg| {
        cfg.reconnect = false;
        cfg.silence_timeout = Duration::from_secs(2);
        cfg.write_timeout = Duration::from_secs(2);
    });
    let took = started.elapsed();
    let s = &stats[0];
    assert_eq!(s.joins, 1, "{s:?}");
    assert_eq!(s.disconnects, 1, "{s:?}");
    assert!(
        s.errors.iter().any(|e| e.contains("no packets")),
        "{:?}",
        s.errors
    );
    assert!(took < Duration::from_secs(30), "took {took:?}");
    assert_eq!(exit_code(&stats, DisconnectPolicy::Fail), 3);
    assert_eq!(exit_code(&stats, DisconnectPolicy::Allow), 0);
}

#[test]
fn match_restart_rejoins_without_failing() {
    let welcome = welcome_bytes();
    let addr = spawn_ws_server(move |_, mut ws| {
        let welcome = welcome.clone();
        async move {
            let _ = ws.next().await;
            let _ = ws.send(WsMessage::Binary(welcome.into())).await;
            let bye = Message::Disconnect {
                reason: ls_game::MATCH_RESTART_REASON.into(),
            };
            let _ = ws
                .send(WsMessage::Binary(bye.encode().unwrap().into()))
                .await;
            while let Some(Ok(_)) = ws.next().await {}
        }
    });
    let stats = run_bots_with(&format!("ws://{addr}/ws"), 1, 8.0, |cfg| {
        cfg.reconnect = true;
    });
    let s = &stats[0];
    assert!(s.joins >= 2, "{s:?}");
    assert!(s.match_restarts >= 1, "{s:?}");
    assert_eq!(s.disconnects, 0, "{s:?}");
    assert_eq!(exit_code(&stats, DisconnectPolicy::Fail), 0);
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "full level generation is slow in debug builds; run with --release"
)]
fn reconnects_after_a_lost_connection() {
    // ~2 s of snapshots, then the connection drops.
    let (url, server_stop) = spawn_game_server(Some(60));
    let stats = run_bots_with(&url, 1, 8.0, |cfg| {
        cfg.reconnect = true;
        cfg.reconnect_delay = Duration::from_millis(200);
    });
    server_stop.store(true, Ordering::SeqCst);
    let s = &stats[0];
    eprintln!("{s:?}");
    assert_eq!(s.joins, 2, "{:?}", s.errors);
    assert_eq!(s.disconnects, 1, "{:?}", s.errors);
    assert_eq!(s.protocol_errors, 0, "{:?}", s.errors);
    assert_eq!(s.desyncs, 0, "{:?}", s.errors);
    // Counters add up over both sessions.
    assert!(s.snapshots > 60, "{s:?}");
    assert_eq!(s.delta_snapshots + s.full_snapshots, s.snapshots);
    assert_eq!(exit_code(&stats, DisconnectPolicy::Fail), 3);
}
