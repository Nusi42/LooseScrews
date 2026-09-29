//! End-to-end test: a real server on an ephemeral port, WebSocket clients
//! driven by `ls_game::client::ClientSession` at 60 Hz in real time.
//!
//! Checks the HTTP side (static files, path traversal, `/cert-hash`), the
//! handshake (version mismatch is rejected, `Welcome` is complete), input
//! exchange, 30 Hz snapshots with deltas, terrain edits replicated to every
//! client (live and via a late joiner's edit log), disconnects and graceful
//! shutdown.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{FutureExt, SinkExt, StreamExt};
use ls_game::client::{ClientSession, Incoming};
use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_math::Vec3;
use ls_game::ls_net::{check_welcome, Hello, Message, RejectReason, Welcome, PROTOCOL_VERSION};
use ls_game::ls_sim::{Buttons, InputCmd, ShipTuning, WeaponTable};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::MatchConfig;
use ls_server::{Server, ServerConfig, ServerHandle};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Script = Box<dyn FnMut(u64, &ClientSession) -> InputCmd + Send>;

const TICK: Duration = Duration::from_nanos(1_000_000_000 / 60);

struct TestClient {
    name: String,
    ws: Ws,
    welcome: Welcome,
    session: ClientSession,
    /// Frames run so far (the client's local 60 Hz clock).
    frames: u64,
    disconnected: Option<String>,
    closed: bool,
}

impl TestClient {
    async fn connect(addr: SocketAddr, name: &str) -> TestClient {
        let (mut ws, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
        let hello = Message::Hello(Hello::new(name, None)).encode().unwrap();
        ws.send(WsMessage::Binary(hello.into())).await.unwrap();
        let welcome = loop {
            match tokio::time::timeout(Duration::from_secs(120), ws.next())
                .await
                .expect("welcome in time")
            {
                Some(Ok(WsMessage::Binary(b))) => {
                    break check_welcome(Message::decode(&b).unwrap()).unwrap();
                }
                Some(Ok(_)) => continue,
                other => panic!("{name}: no welcome: {other:?}"),
            }
        };
        let session = tokio::task::block_in_place(|| {
            ClientSession::from_welcome(&welcome, WeaponTable::builtin(), ShipTuning::tinker())
        })
        .unwrap();
        TestClient {
            name: name.to_string(),
            ws,
            welcome,
            session,
            frames: 0,
            disconnected: None,
            closed: false,
        }
    }

    /// Reads every frame that is already waiting.
    fn drain(&mut self) {
        while !self.closed {
            match self.ws.next().now_or_never() {
                None => break,
                Some(Some(Ok(WsMessage::Binary(b)))) => {
                    let r = tokio::task::block_in_place(|| self.session.handle_packet(&b));
                    match r {
                        Ok(Incoming::Disconnected(reason)) => self.disconnected = Some(reason),
                        Ok(_) => {}
                        Err(e) => panic!("{}: bad packet: {e}", self.name),
                    }
                }
                Some(Some(Ok(WsMessage::Close(_)))) | Some(None) | Some(Some(Err(_))) => {
                    self.closed = true;
                }
                Some(Some(Ok(_))) => {}
            }
        }
    }

    /// Runs the client in real time at 60 Hz until `done` or `timeout`.
    /// Returns whether `done` became true.
    async fn drive(
        &mut self,
        script: &mut Script,
        timeout: Duration,
        mut done: impl FnMut(&TestClient) -> bool,
    ) -> bool {
        let start = Instant::now();
        let mut ran = 0u64;
        loop {
            self.drain();
            if done(self) {
                return true;
            }
            if self.closed || start.elapsed() > timeout {
                return false;
            }
            let due_total = (start.elapsed().as_nanos() / TICK.as_nanos()) as u64;
            let due = due_total.saturating_sub(ran).min(30) as u32;
            if due > 0 {
                ran = due_total;
                self.frames += due as u64;
                let controls = script(self.frames, &self.session);
                let msgs = self.session.frame(due, controls);
                for m in msgs {
                    let bytes = m.encode().unwrap();
                    if self.ws.feed(WsMessage::Binary(bytes.into())).await.is_err() {
                        self.closed = true;
                    }
                }
                if self.ws.flush().await.is_err() {
                    self.closed = true;
                }
            }
            tokio::time::sleep(Duration::from_millis(4)).await;
        }
    }
}

/// Runs two clients concurrently (each on its own task) for one phase,
/// until `done` holds for both (each keeps playing until the other is done
/// too, so neither starves the server of input) or `timeout`.
async fn drive_pair(
    a: TestClient,
    mut sa: Script,
    b: TestClient,
    mut sb: Script,
    timeout: Duration,
    done: impl Fn(&TestClient) -> bool + Send + Clone + 'static,
) -> (TestClient, Script, bool, TestClient, Script, bool) {
    let flags = Arc::new([AtomicBool::new(false), AtomicBool::new(false)]);
    let until = |me: usize| {
        let flags = flags.clone();
        let done = done.clone();
        move |c: &TestClient| {
            if done(c) {
                flags[me].store(true, Ordering::SeqCst);
            }
            flags.iter().all(|f| f.load(Ordering::SeqCst))
        }
    };
    let (ua, ub) = (until(0), until(1));
    let ta = tokio::spawn(async move {
        let mut a = a;
        let ok = a.drive(&mut sa, timeout, ua).await;
        (a, sa, ok)
    });
    let tb = tokio::spawn(async move {
        let mut b = b;
        let ok = b.drive(&mut sb, timeout, ub).await;
        (b, sb, ok)
    });
    let (a, sa, oka) = ta.await.unwrap();
    let (b, sb, okb) = tb.await.unwrap();
    (a, sa, oka, b, sb, okb)
}

/// Aims down at the terrain, approaches it and fires rockets (craters) and
/// the paint thrower (paint) while `fire` is on.
fn gunner(paint_thrower: u8) -> Script {
    Box::new(move |_, s| {
        let mut cmd = InputCmd::idle(0);
        let Some(ship) = s.local_ship().filter(|_| s.is_synced()) else {
            return cmd;
        };
        let fwd = ship.orientation.forward();
        cmd.weapon_select = paint_thrower + 1;
        if fwd.y > -0.9 {
            cmd.look_pitch = -0.04;
        } else {
            let dist = s
                .terrain()
                .raycast(ship.pos, fwd, 100.0)
                .map_or(100.0, |h| h.distance);
            if dist > 12.0 {
                cmd.thrust = Vec3::new(0.0, 0.0, -1.0);
            } else {
                cmd.buttons = Buttons::FIRE_PRIMARY | Buttons::FIRE_SECONDARY;
            }
        }
        cmd
    })
}

/// Flies a looping pattern without firing.
fn flyer() -> Script {
    Box::new(|frame, s| {
        let mut cmd = InputCmd::idle(0);
        if !s.is_synced() {
            return cmd;
        }
        let phase = (frame / 40) % 4;
        cmd.thrust = match phase {
            0 => Vec3::new(0.0, 0.3, -1.0),
            1 => Vec3::new(0.6, 0.0, 0.0),
            2 => Vec3::new(0.0, -0.3, 1.0),
            _ => Vec3::new(-0.6, 0.0, 0.0),
        };
        cmd.look_yaw = 0.004;
        cmd
    })
}

fn idle() -> Script {
    Box::new(|_, _| InputCmd::idle(0))
}

/// Minimal HTTP/1.1 GET; returns status, headers and body.
async fn http_get(addr: SocketAddr, path: &str) -> (u16, String, Vec<u8>) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut buf))
        .await
        .unwrap()
        .unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(buf[..split].to_vec()).unwrap();
    let status = head[9..12].parse().unwrap();
    (status, head, buf[split + 4..].to_vec())
}

async fn wait_for(
    server: &ServerHandle,
    what: &str,
    f: impl Fn(&ls_server::StatsSnapshot) -> bool,
) {
    let start = Instant::now();
    while !f(&server.stats()) {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}: {:?}",
            server.stats()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn web_root() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ls-server-test-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(
        dir.join("index.html"),
        "<!doctype html><title>LS test</title>",
    )
    .unwrap();
    std::fs::write(dir.join("assets").join("app.js"), "console.log(1);").unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn websocket_clients_exchange_inputs_snapshots_and_terrain_edits() {
    // Server logs (tick stats, joins) show with `--nocapture`; RUST_LOG filters.
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
    let root = web_root();
    let code = ShareCode::new(0x5eed_1234, Sliders::new(40, 30));
    let mut cfg = ServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        MatchConfig::paint_the_void(code),
    );
    cfg.web_root = Some(root.clone());
    cfg.stats_interval = Duration::from_secs(2);
    let server = Server::start(cfg).await.expect("server starts");
    let addr = server.local_addr();
    assert_eq!(server.share_code(), code.encode().unwrap());

    // --- HTTP on the same port ------------------------------------------------
    let (status, head, body) = http_get(addr, "/").await;
    assert_eq!(status, 200, "{head}");
    assert!(head.contains("text/html"), "{head}");
    assert!(String::from_utf8_lossy(&body).contains("LS test"));
    let (status, head, body) = http_get(addr, "/assets/app.js").await;
    assert_eq!(status, 200);
    assert!(head.contains("text/javascript"), "{head}");
    assert_eq!(body, b"console.log(1);");
    assert_eq!(http_get(addr, "/%2e%2e/secret.txt").await.0, 403);
    assert_eq!(http_get(addr, "/missing.js").await.0, 404);
    assert_eq!(http_get(addr, "/cert-hash").await.0, 404);

    // --- A client with the wrong protocol version is rejected ------------------
    {
        let (mut ws, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
        let hello = Hello {
            protocol_version: PROTOCOL_VERSION + 1,
            ..Hello::new("future", None)
        };
        let bytes = Message::Hello(hello).encode().unwrap();
        ws.send(WsMessage::Binary(bytes.into())).await.unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .unwrap();
        match reply {
            Some(Ok(WsMessage::Binary(b))) => match Message::decode(&b).unwrap() {
                Message::Reject(r) => assert_eq!(r.reason, RejectReason::VersionMismatch),
                other => panic!("expected Reject, got {other:?}"),
            },
            other => panic!("expected Reject, got {other:?}"),
        }
        assert_eq!(server.stats().rejected, 1);
    }

    // --- Two players join -------------------------------------------------------
    let a = TestClient::connect(addr, "alpha").await;
    let b = TestClient::connect(addr, "bravo").await;
    assert_ne!(a.welcome.player_id, b.welcome.player_id);
    assert_eq!(a.welcome.share_code, server.share_code());
    assert_eq!(a.welcome.ruleset_name, server.ruleset_name());
    assert!(!a.welcome.ruleset.is_empty());
    assert_ne!(a.welcome.team, b.welcome.team, "teams are balanced");
    wait_for(&server, "2 players", |s| s.players == 2).await;

    // Phase 0: both idle until snapshots flow; note where bravo starts.
    let (a, _, ok_a, b, _, ok_b) = drive_pair(a, idle(), b, idle(), Duration::from_secs(60), |c| {
        c.session.stats().snapshots > 10
    })
    .await;
    assert!(ok_a && ok_b, "no snapshots: {:?}", server.stats());
    let bravo_id = b.welcome.player_id;
    let bravo_seen_by_alpha = |a: &TestClient| {
        a.session
            .latest_snapshot()
            .unwrap()
            .ships
            .iter()
            .find(|s| s.id == bravo_id)
            .expect("alpha sees bravo")
            .pos
    };
    let bravo_start = bravo_seen_by_alpha(&a);

    // Phase 1: alpha shoots the terrain, bravo flies around, until alpha's
    // craters and paint have come back in snapshots.
    let paint = WeaponTable::builtin().find("paint_thrower").unwrap().0;
    let (a, _, ok_a, b, _, ok_b) = drive_pair(
        a,
        gunner(paint),
        b,
        flyer(),
        Duration::from_secs(180),
        |c| {
            let s = c.session.stats();
            s.snapshots > 90 && (c.name != "alpha" || c.session.edit_count() >= 4)
        },
    )
    .await;
    assert!(
        ok_a && ok_b,
        "phase 1 timed out: alpha {:?} edits {}, bravo {:?}, server {:?}",
        a.session.stats(),
        a.session.edit_count(),
        b.session.stats(),
        server.stats()
    );
    // Bravo's flying is visible to alpha.
    let moved = bravo_seen_by_alpha(&a).distance(bravo_start);
    assert!(
        moved > 2.0,
        "bravo moved only {moved} u in alpha's snapshots"
    );

    // Phase 2: stop firing and let projectiles in flight land.
    let (a, _, _, b, _, _) =
        drive_pair(a, idle(), b, idle(), Duration::from_secs(4), |_| false).await;
    let (target, hash, log) = server
        .with_room(|r| {
            let t = r.terrain();
            (t.edit_count(), t.state_hash(), t.edit_log().to_vec())
        })
        .await
        .unwrap();
    assert!(
        log.iter().any(|e| e.changes_geometry()),
        "no crater: {log:?}"
    );
    assert!(
        log.iter().any(|e| !e.changes_geometry()),
        "no paint: {log:?}"
    );

    // Phase 3: every edit reaches both clients through snapshots.
    let (a, _, ok_a, b, _, ok_b) =
        drive_pair(a, idle(), b, idle(), Duration::from_secs(20), move |c| {
            c.session.edit_count() >= target
        })
        .await;
    assert!(ok_a && ok_b, "edits did not replicate");
    for c in [&a, &b] {
        assert_eq!(c.session.edit_count(), target, "{} edits", c.name);
        assert_eq!(c.session.terrain().state_hash(), hash, "{} terrain", c.name);
        let st = c.session.stats();
        assert!(st.snapshots > 90, "{}: {st:?}", c.name);
        assert!(st.delta_snapshots > st.full_snapshots, "{}: {st:?}", c.name);
        assert!(c.session.is_synced(), "{}", c.name);
        let snap = c.session.latest_snapshot().unwrap();
        assert_eq!(snap.ships.len(), 2, "{}: both ships replicated", c.name);
        // The server has been applying this client's real inputs.
        assert!(
            snap.ack_input as u64 + 30 >= snap.tick as u64
                && snap.ack_input > c.welcome.server_tick + 60,
            "{}: ack_input {} tick {}",
            c.name,
            snap.ack_input,
            snap.tick
        );
        assert_eq!(snap.scores.len(), 3);
    }

    // --- A late joiner gets the whole edit log in its Welcome ------------------
    let c = TestClient::connect(addr, "charlie").await;
    assert_eq!(c.welcome.edit_log.len() as u32, target);
    assert_eq!(c.session.terrain().state_hash(), hash);
    wait_for(&server, "3 players", |s| s.players == 3).await;

    let st = server.stats();
    assert!(st.snapshots_sent > 100, "{st:?}");
    assert!(st.tick > 200, "{st:?}");
    assert!(st.connections >= 3, "{st:?}");
    assert_eq!(st.edit_count as u32, target);

    // --- Disconnects ------------------------------------------------------------
    // Charlie says goodbye with a protocol Disconnect, bravo just closes.
    let mut c = c;
    let bye = Message::Disconnect {
        reason: "bye".into(),
    }
    .encode()
    .unwrap();
    c.ws.send(WsMessage::Binary(bye.into())).await.unwrap();
    wait_for(&server, "charlie leaving", |s| s.players == 2).await;
    let mut b = b;
    b.ws.close(None).await.unwrap();
    drop(b);
    wait_for(&server, "bravo leaving", |s| s.players == 1).await;

    // --- Graceful shutdown: alpha is told and the socket closes -----------------
    let mut a = a;
    server.shutdown().await;
    let start = Instant::now();
    while !a.closed && start.elapsed() < Duration::from_secs(10) {
        a.drain();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(a.closed, "socket closed after shutdown");
    assert_eq!(a.disconnected.as_deref(), Some("server shutting down"));
    let _ = std::fs::remove_dir_all(root);
}
