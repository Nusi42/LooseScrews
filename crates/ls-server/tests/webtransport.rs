//! End-to-end test of the WebTransport endpoint: a real server on ephemeral
//! ports, `wtransport` clients that trust the self-signed certificate by the
//! hash from `GET /cert-hash` (like browsers with `serverCertificateHashes`),
//! driven by `ls_game::client::ClientSession` at 60 Hz in real time.
//!
//! Checks `/cert-hash`, that an unknown certificate is refused, the handshake
//! on the framed stream (Reject for a bad version, complete Welcome), inputs
//! as datagrams, snapshots as datagrams, the stream fallback for unreliable
//! packets over the datagram limit, certificate rotation and graceful
//! shutdown.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use ls_game::client::{ClientSession, Incoming};
use ls_game::ls_math::Vec3;
use ls_game::ls_net::PROTOCOL_VERSION;
use ls_game::ls_net::{check_welcome, Channel, Hello, Message, RejectReason, Welcome};
use ls_game::ls_sim::{InputCmd, ShipTuning, WeaponTable};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::MatchConfig;
use ls_server::wt::{append_frame, FrameDecoder, WebTransportEndpoint};
use ls_server::{Server, ServerConfig, ServerHandle};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Connection, Endpoint, SendStream};

const TICK: Duration = Duration::from_nanos(1_000_000_000 / 60);

/// Minimal HTTP/1.1 GET; returns status, head and body.
async fn http_get(addr: SocketAddr, path: &str) -> (u16, String, String) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut buf))
        .await
        .unwrap()
        .unwrap();
    let text = String::from_utf8(buf).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    (
        head[9..12].parse().unwrap(),
        head.to_string(),
        body.to_string(),
    )
}

/// Value of `"key":` in a flat JSON object (strings unquoted).
fn json_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let at = json.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = json[at..].trim_start();
    if let Some(s) = rest.strip_prefix('"') {
        return s.split('"').next();
    }
    rest.split([',', '}']).next().map(str::trim)
}

fn parse_hex32(hex: &str) -> [u8; 32] {
    assert_eq!(hex.len(), 64, "{hex}");
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

/// Fetches `/cert-hash`: (hash, port).
async fn cert_hash(http: SocketAddr) -> ([u8; 32], u16) {
    let (status, head, body) = http_get(http, "/cert-hash").await;
    assert_eq!(status, 200, "{head}");
    assert!(head.contains("application/json"), "{head}");
    assert!(head.contains("Access-Control-Allow-Origin: *"), "{head}");
    assert_eq!(json_field(&body, "algorithm"), Some("sha-256"), "{body}");
    assert_eq!(json_field(&body, "path"), Some("/wt"), "{body}");
    let expires: i64 = json_field(&body, "expires").unwrap().parse().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert!(expires > now && expires <= now + 14 * 24 * 3600, "{body}");
    let hash = parse_hex32(json_field(&body, "hash").unwrap());
    let port = json_field(&body, "port").unwrap().parse().unwrap();
    (hash, port)
}

async fn open_session(port: u16, hash: [u8; 32]) -> Result<Connection, String> {
    let cfg = ClientConfig::builder()
        .with_bind_default()
        .with_server_certificate_hashes([Sha256Digest::new(hash)])
        .build();
    let endpoint = Endpoint::client(cfg).map_err(|e| e.to_string())?;
    tokio::time::timeout(
        Duration::from_secs(10),
        endpoint.connect(format!("https://127.0.0.1:{port}/wt")),
    )
    .await
    .map_err(|_| "connect timed out".to_string())?
    .map_err(|e| e.to_string())
}

/// A WebTransport client: a stream for reliable packets, datagrams for
/// unreliable ones; a reader task merges both into `rx`.
struct WtClient {
    conn: Connection,
    send: SendStream,
    rx: mpsc::UnboundedReceiver<(Channel, Vec<u8>)>,
}

impl WtClient {
    async fn open(port: u16, hash: [u8; 32], hello: Hello) -> WtClient {
        let conn = open_session(port, hash).await.expect("session opens");
        let (mut send, mut recv) = conn.open_bi().await.unwrap().await.unwrap();
        let mut buf = Vec::new();
        append_frame(&mut buf, &Message::Hello(hello).encode().unwrap());
        send.write_all(&buf).await.unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let mut d = FrameDecoder::new(64 << 20);
            let mut chunk = vec![0u8; 65536];
            while let Ok(Some(n)) = recv.read(&mut chunk).await {
                d.extend(&chunk[..n]);
                while let Some(f) = d.next_frame().unwrap() {
                    let _ = tx.send((Channel::Reliable, f));
                }
            }
        });
        let c2 = conn.clone();
        tokio::spawn(async move {
            while let Ok(d) = c2.receive_datagram().await {
                let _ = tx2.send((Channel::Unreliable, d.payload().to_vec()));
            }
        });
        WtClient { conn, send, rx }
    }

    /// The first reliable packet (datagrams before it are dropped, as the
    /// real clients do).
    async fn first_reliable(&mut self) -> Vec<u8> {
        loop {
            match tokio::time::timeout(Duration::from_secs(120), self.rx.recv())
                .await
                .expect("answer in time")
            {
                Some((Channel::Reliable, b)) => return b,
                Some(_) => continue,
                None => panic!("session closed before the first packet"),
            }
        }
    }

    async fn send_msg(&mut self, m: &Message) {
        let bytes = m.encode().unwrap();
        let max = self.conn.max_datagram_size().unwrap_or(0);
        if m.channel() == Channel::Unreliable && bytes.len() <= max {
            self.conn.send_datagram(&bytes).unwrap();
        } else {
            let mut buf = Vec::new();
            append_frame(&mut buf, &bytes);
            self.send.write_all(&buf).await.unwrap();
        }
    }
}

/// A joined player.
struct Player {
    name: &'static str,
    client: WtClient,
    welcome: Welcome,
    session: ClientSession,
    frames: u64,
    /// Snapshots received per channel (datagram, stream).
    snaps_dgram: u64,
    snaps_stream: u64,
    disconnected: Option<String>,
    closed: bool,
}

impl Player {
    async fn join(http: SocketAddr, name: &'static str) -> Player {
        let (hash, port) = cert_hash(http).await;
        let mut client = WtClient::open(port, hash, ls_game::client_hello(name, None, None)).await;
        let first = client.first_reliable().await;
        let welcome = check_welcome(Message::decode(&first).unwrap()).unwrap();
        let session = tokio::task::block_in_place(|| {
            ClientSession::from_welcome(&welcome, WeaponTable::builtin(), ShipTuning::tinker())
        })
        .unwrap();
        Player {
            name,
            client,
            welcome,
            session,
            frames: 0,
            snaps_dgram: 0,
            snaps_stream: 0,
            disconnected: None,
            closed: false,
        }
    }

    fn drain(&mut self) {
        loop {
            match self.client.rx.try_recv() {
                Ok((channel, b)) => {
                    let r = tokio::task::block_in_place(|| self.session.handle_packet(&b));
                    match r {
                        Ok(Incoming::Snapshot { .. }) => match channel {
                            Channel::Unreliable => self.snaps_dgram += 1,
                            Channel::Reliable => self.snaps_stream += 1,
                        },
                        Ok(Incoming::Disconnected(reason)) => self.disconnected = Some(reason),
                        Ok(_) => {}
                        Err(e) => panic!("{}: bad packet: {e}", self.name),
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.closed = true;
                    break;
                }
            }
        }
    }

    async fn step(&mut self, due: u32, cmd: InputCmd) {
        self.frames += due as u64;
        for m in self.session.frame(due, cmd) {
            self.client.send_msg(&m).await;
        }
    }
}

/// Runs players in real time at 60 Hz until `done` holds for all or `timeout`.
async fn drive(
    players: &mut [&mut Player],
    timeout: Duration,
    script: impl Fn(u64, &ClientSession) -> InputCmd,
    done: impl Fn(&Player) -> bool,
) -> bool {
    let start = Instant::now();
    let mut ran = 0u64;
    loop {
        for p in players.iter_mut() {
            p.drain();
        }
        if players.iter().all(|p| done(p)) {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        let due_total = (start.elapsed().as_nanos() / TICK.as_nanos()) as u64;
        let due = due_total.saturating_sub(ran).min(30) as u32;
        if due > 0 {
            ran = due_total;
            for p in players.iter_mut() {
                let cmd = script(p.frames, &p.session);
                p.step(due, cmd).await;
            }
        }
        tokio::time::sleep(Duration::from_millis(4)).await;
    }
}

fn flyer(frame: u64, s: &ClientSession) -> InputCmd {
    let mut cmd = InputCmd::idle(0);
    if !s.is_synced() {
        return cmd;
    }
    cmd.thrust = match (frame / 40) % 4 {
        0 => Vec3::new(0.0, 0.3, -1.0),
        1 => Vec3::new(0.6, 0.0, 0.0),
        2 => Vec3::new(0.0, -0.3, 1.0),
        _ => Vec3::new(-0.6, 0.0, 0.0),
    };
    cmd.look_yaw = 0.004;
    cmd
}

fn init_logs() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
}

fn config() -> ServerConfig {
    let code = ShareCode::new(0x5eed_4321, Sliders::new(40, 30));
    let mut cfg = ServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        MatchConfig::paint_the_void(code),
    );
    cfg.webtransport = Some("127.0.0.1:0".parse().unwrap());
    cfg.stats_interval = Duration::from_secs(2);
    cfg
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webtransport_clients_exchange_inputs_and_snapshots() {
    init_logs();
    let server = Server::start(config()).await.expect("server starts");
    let http = server.local_addr();
    let wt_addr = server.webtransport_addr().expect("webtransport enabled");

    // --- /cert-hash -----------------------------------------------------------
    let (hash, port) = cert_hash(http).await;
    assert_eq!(port, wt_addr.port());

    // --- An unknown certificate is refused -----------------------------------
    let mut wrong = hash;
    wrong[0] ^= 0xff;
    assert!(
        open_session(port, wrong).await.is_err(),
        "wrong hash accepted"
    );

    // --- A bad protocol version gets a Reject on the stream ------------------
    {
        let hello = Hello {
            protocol_version: PROTOCOL_VERSION + 1,
            ..Hello::new("future", None)
        };
        let mut c = WtClient::open(port, hash, hello).await;
        match Message::decode(&c.first_reliable().await).unwrap() {
            Message::Reject(r) => assert_eq!(r.reason, RejectReason::VersionMismatch),
            other => panic!("expected Reject, got {other:?}"),
        }
        assert_eq!(server.stats().rejected, 1);
    }

    // --- Two players join and play ---------------------------------------------
    let mut a = Player::join(http, "alpha").await;
    let mut b = Player::join(http, "bravo").await;
    assert_ne!(a.welcome.player_id, b.welcome.player_id);
    assert_eq!(a.welcome.share_code, server.share_code());
    wait_for(&server, "2 players", |s| s.players == 2).await;
    let ok = drive(
        &mut [&mut a, &mut b],
        Duration::from_secs(120),
        flyer,
        |p| p.snaps_dgram > 150,
    )
    .await;
    assert!(
        ok,
        "snapshots: alpha {}+{}, bravo {}+{}, server {:?}",
        a.snaps_dgram,
        a.snaps_stream,
        b.snaps_dgram,
        b.snaps_stream,
        server.stats()
    );
    for p in [&a, &b] {
        assert!(p.session.is_synced(), "{}", p.name);
        let snap = p.session.latest_snapshot().unwrap();
        assert_eq!(snap.ships.len(), 2, "{}: both ships replicated", p.name);
        // The server applies this client's inputs, which travel as datagrams.
        assert!(
            snap.ack_input as u64 + 30 >= snap.tick as u64
                && snap.ack_input > p.welcome.server_tick + 60,
            "{}: ack_input {} tick {}",
            p.name,
            snap.ack_input,
            snap.tick
        );
        let st = p.session.stats();
        assert!(st.delta_snapshots > st.full_snapshots, "{}: {st:?}", p.name);
    }
    let st = server.stats();
    assert!(st.snapshots_sent > 300, "{st:?}");
    assert_eq!(st.kicked, 0, "{st:?}");

    // --- Bravo leaves with a Disconnect on the stream ------------------------------
    b.client
        .send_msg(&Message::Disconnect {
            reason: "bye".into(),
        })
        .await;
    wait_for(&server, "bravo leaving", |s| s.players == 1).await;

    // --- Graceful shutdown: alpha is told, then the session closes -------------
    server.shutdown().await;
    let start = Instant::now();
    while !a.closed && start.elapsed() < Duration::from_secs(10) {
        a.drain();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(a.disconnected.as_deref(), Some("server shutting down"));
    assert!(a.closed, "session closed after shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unreliable_packets_over_the_datagram_limit_use_the_stream() {
    init_logs();
    let mut cfg = config();
    // Smaller than any snapshot: every snapshot must take the stream.
    cfg.conn.link.max_unreliable_size = 16;
    let server = Server::start(cfg).await.expect("server starts");
    let mut a = Player::join(server.local_addr(), "alpha").await;
    let ok = drive(&mut [&mut a], Duration::from_secs(60), flyer, |p| {
        p.snaps_stream > 60
    })
    .await;
    assert!(
        ok,
        "stream snapshots {} {:?}",
        a.snaps_stream,
        server.stats()
    );
    assert_eq!(a.snaps_dgram, 0);
    assert!(a.session.is_synced());
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn certificate_rotation_publishes_the_new_hash() {
    init_logs();
    let mut cfg = config();
    cfg.webtransport = None;
    let mut server = Server::start(cfg).await.expect("server starts");
    let http = server.local_addr();
    assert_eq!(http_get(http, "/cert-hash").await.0, 404);
    let wt = WebTransportEndpoint::bind("127.0.0.1:0".parse().unwrap())
        .unwrap()
        .with_rotation(Duration::from_millis(1500));
    server.spawn_endpoint(Box::new(wt));
    let (first, port) = cert_hash(http).await;
    let start = Instant::now();
    let second = loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (h, p) = cert_hash(http).await;
        assert_eq!(p, port);
        if h != first {
            break h;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "no rotation");
    };
    // New sessions use the current certificate (retry once in case it
    // rotated again meanwhile).
    let mut current = second;
    let mut ok = false;
    for _ in 0..2 {
        if open_session(port, current).await.is_ok() {
            ok = true;
            break;
        }
        current = cert_hash(http).await.0;
    }
    assert!(ok, "no session with the rotated certificate");
    server.shutdown().await;
}
