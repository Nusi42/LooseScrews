//! The directory over real HTTP on `127.0.0.1:0`: register, heartbeat,
//! expiry, list filters, CORS, invite relay, rate limiting and the
//! heartbeat task `ls-server` uses.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use ls_directory::api::{
    data_hash_hex, Heartbeat, InviteRequest, RosterEntry, ServerInfo, Unregister,
};
use ls_directory::client::ClientError;
use ls_directory::{new_secret, spawn_heartbeats, Config, DirectoryClient, DirectoryServer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;

const HASH: u64 = 0x0123_4567_89ab_cdef;

fn info(address: &str) -> ServerInfo {
    ServerInfo {
        name: format!("Server {address}"),
        address: address.into(),
        cert_hash: None,
        region: "local".into(),
        mode: "paint_the_void".into(),
        map_code: "LS2-TEST".into(),
        rules_code: None,
        players: 2,
        max_players: 16,
        protocol: 5,
        data_hash: data_hash_hex(HASH),
        roster: vec![RosterEntry {
            name: "Ace".into(),
            id: "player-0001".into(),
        }],
    }
}

/// Generous rate limits: every test request comes from 127.0.0.1.
fn test_config() -> Config {
    Config {
        rate_per_sec: 10_000,
        rate_burst: 10_000,
        ..Config::default()
    }
}

struct Running {
    addr: SocketAddr,
    client: DirectoryClient,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Running {
    async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

async fn start(cfg: Config) -> Running {
    let server = DirectoryServer::bind("127.0.0.1:0".parse().unwrap(), cfg)
        .await
        .unwrap();
    let addr = server.local_addr().unwrap();
    let (stop, rx) = watch::channel(false);
    let task = server.spawn(rx);
    let client = DirectoryClient::new(&format!("http://{addr}")).unwrap();
    Running {
        addr,
        client,
        stop,
        task,
    }
}

/// Sends a raw request and returns the full reply text.
async fn raw(addr: SocketAddr, request: &str) -> String {
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    String::from_utf8(out).unwrap()
}

fn hb(secret: &str, server: ServerInfo) -> Heartbeat {
    Heartbeat {
        secret: secret.into(),
        server,
    }
}

#[tokio::test]
async fn register_heartbeat_list_and_unregister() {
    let d = start(test_config()).await;
    let s1 = new_secret();
    let s2 = new_secret();
    let r1 = d
        .client
        .heartbeat(&hb(&s1, info("a.test:7777")))
        .await
        .unwrap();
    assert_eq!((r1.ttl_secs, r1.heartbeat_secs), (30, 10));
    let r2 = d
        .client
        .heartbeat(&hb(&s2, info("b.test:7777")))
        .await
        .unwrap();
    assert_ne!(r1.id, r2.id);

    // A heartbeat updates the entry and keeps its id.
    let mut busy = info("a.test:7777");
    busy.players = 9;
    busy.mode = "team_brawl".into();
    assert_eq!(d.client.heartbeat(&hb(&s1, busy)).await.unwrap().id, r1.id);

    let list = d.client.list(None, None).await.unwrap();
    assert_eq!(list.servers.len(), 2);
    let a = &list.servers[0];
    assert_eq!(a.id, r1.id);
    assert_eq!(
        (a.server.players, a.server.mode.as_str()),
        (9, "team_brawl")
    );
    assert_eq!(a.server.roster[0].id, "player-0001");
    assert!(a.age_ms < 5_000);

    // Someone else cannot take over or remove an entry.
    let err = d
        .client
        .heartbeat(&hb(&new_secret(), info("a.test:7777")))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::Status { status: 403, .. }),
        "{err}"
    );
    let steal = Unregister {
        address: "a.test:7777".into(),
        secret: s2.clone(),
    };
    assert!(matches!(
        d.client.unregister(&steal).await,
        Err(ClientError::Status { status: 403, .. })
    ));

    d.client
        .unregister(&Unregister {
            address: "a.test:7777".into(),
            secret: s1,
        })
        .await
        .unwrap();
    let list = d.client.list(None, None).await.unwrap();
    assert_eq!(list.servers.len(), 1);
    assert_eq!(list.servers[0].server.address, "b.test:7777");

    // Invalid bodies are 400 with a JSON error.
    let mut bad = info("c.test:7777");
    bad.data_hash = "nope".into();
    match d.client.heartbeat(&hb(&new_secret(), bad)).await {
        Err(ClientError::Status { status, message }) => {
            assert_eq!(status, 400);
            assert!(message.contains("data_hash"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    d.stop().await;
}

#[tokio::test]
async fn entries_expire_after_the_ttl() {
    let d = start(Config {
        ttl: Duration::from_millis(400),
        ..test_config()
    })
    .await;
    let secret = new_secret();
    d.client
        .heartbeat(&hb(&secret, info("a.test:1")))
        .await
        .unwrap();
    d.client
        .heartbeat(&hb(&secret, info("b.test:1")))
        .await
        .unwrap();
    // Keep `a` alive with heartbeats while `b` goes quiet.
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(900) {
        d.client
            .heartbeat(&hb(&secret, info("a.test:1")))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let list = d.client.list(None, None).await.unwrap();
    let addrs: Vec<_> = list
        .servers
        .iter()
        .map(|e| e.server.address.as_str())
        .collect();
    assert_eq!(addrs, ["a.test:1"]);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(d.client.list(None, None).await.unwrap().servers.is_empty());
    d.stop().await;
}

#[tokio::test]
async fn list_filters_mismatched_version_and_data_hash() {
    let d = start(test_config()).await;
    let secret = new_secret();
    d.client
        .heartbeat(&hb(&secret, info("match.test:1")))
        .await
        .unwrap();
    let mut old_protocol = info("old.test:1");
    old_protocol.protocol = 4;
    d.client
        .heartbeat(&hb(&secret, old_protocol))
        .await
        .unwrap();
    let mut other_data = info("data.test:1");
    other_data.data_hash = data_hash_hex(HASH ^ 1);
    d.client.heartbeat(&hb(&secret, other_data)).await.unwrap();

    let all = d.client.list(None, None).await.unwrap();
    assert_eq!(all.servers.len(), 3);
    let mine = d
        .client
        .list(Some(5), Some(&data_hash_hex(HASH)))
        .await
        .unwrap();
    let addrs: Vec<_> = mine
        .servers
        .iter()
        .map(|e| e.server.address.as_str())
        .collect();
    assert_eq!(addrs, ["match.test:1"]);
    d.stop().await;
}

#[tokio::test]
async fn cors_headers_for_the_browser() {
    let d = start(test_config()).await;
    let get = raw(
        d.addr,
        "GET /servers?protocol=5 HTTP/1.1\r\nHost: x\r\nOrigin: http://localhost:41873\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(get.starts_with("HTTP/1.1 200 OK"), "{get}");
    assert!(get.contains("Access-Control-Allow-Origin: *"), "{get}");
    assert!(get.contains("Content-Type: application/json"), "{get}");
    assert!(get.ends_with(r#"{"servers":[]}"#), "{get}");
    let pre = raw(
        d.addr,
        "OPTIONS /invites HTTP/1.1\r\nHost: x\r\nOrigin: http://localhost:41873\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Headers: content-type\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(pre.starts_with("HTTP/1.1 204"), "{pre}");
    assert!(pre.contains("Access-Control-Allow-Methods: GET, POST, DELETE, OPTIONS"));
    assert!(pre.contains("Access-Control-Allow-Headers: Content-Type"));
    let missing = raw(d.addr, "GET /nope HTTP/1.1\r\nConnection: close\r\n\r\n").await;
    assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");
    assert!(missing.contains("Access-Control-Allow-Origin: *"));
    // Keep-alive: two requests on one connection.
    let two = raw(
        d.addr,
        "GET /servers HTTP/1.1\r\nHost: x\r\n\r\nGET /servers HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(two.matches("HTTP/1.1 200 OK").count(), 2, "{two}");
    d.stop().await;
}

#[tokio::test]
async fn invites_are_relayed_by_player_id() {
    let d = start(test_config()).await;
    let inv = InviteRequest {
        to: "player-0002".into(),
        kind: "duel".into(),
        from_name: "Ace".into(),
        from_id: "player-0001".into(),
        address: "a.test:7777".into(),
        room: Some("R-7KQ2MX".into()),
        world_code: Some("LS2-TEST".into()),
        rules_code: None,
    };
    let sent = d.client.invite(&inv).await.unwrap();
    assert_eq!(sent.expires_in_secs, 30);
    let got = d.client.invites("player-0002").await.unwrap();
    assert_eq!(got.invites.len(), 1);
    assert_eq!(got.invites[0].id, sent.id);
    assert_eq!(got.invites[0].invite, inv);
    assert!(got.invites[0].expires_in_ms > 25_000);
    assert!(d
        .client
        .invites("player-0003")
        .await
        .unwrap()
        .invites
        .is_empty());
    // The answer travels the same way.
    let answer = InviteRequest {
        to: "player-0001".into(),
        kind: "decline".into(),
        from_name: "Bee".into(),
        from_id: "player-0002".into(),
        room: None,
        world_code: None,
        ..inv
    };
    d.client.invite(&answer).await.unwrap();
    let back = d.client.invites("player-0001").await.unwrap();
    assert_eq!(back.invites[0].invite.kind, "decline");
    d.stop().await;
}

#[tokio::test]
async fn rate_limited_clients_get_429() {
    let d = start(Config {
        rate_per_sec: 1,
        rate_burst: 3,
        ..Config::default()
    })
    .await;
    for _ in 0..3 {
        d.client.list(None, None).await.unwrap();
    }
    let err = d.client.list(None, None).await.unwrap_err();
    assert!(
        matches!(err, ClientError::Status { status: 429, .. }),
        "{err}"
    );
    d.stop().await;
}

#[tokio::test]
async fn heartbeat_task_registers_refreshes_and_leaves() {
    let d = start(Config {
        heartbeat: Duration::from_secs(1),
        ..test_config()
    })
    .await;
    let (stop, rx) = watch::channel(false);
    let (players_tx, players_rx) = watch::channel(1u16);
    let task = spawn_heartbeats(
        d.client.clone(),
        new_secret(),
        move || ServerInfo {
            players: *players_rx.borrow(),
            ..info("game.test:7777")
        },
        rx,
    );
    let wait_for = |want: Option<u16>| {
        let client = d.client.clone();
        async move {
            let start = Instant::now();
            loop {
                let l = client.list(None, None).await.unwrap();
                let got = l.servers.first().map(|e| e.server.players);
                if got == want {
                    return;
                }
                assert!(
                    start.elapsed() < Duration::from_secs(5),
                    "{got:?} != {want:?}"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };
    wait_for(Some(1)).await;
    players_tx.send(4).unwrap();
    wait_for(Some(4)).await;
    stop.send(true).unwrap();
    task.await.unwrap();
    wait_for(None).await;
    d.stop().await;
}

#[tokio::test]
async fn heartbeat_task_survives_a_missing_directory() {
    // Nothing listens on this port (bound then dropped): the task logs and
    // retries, and still stops promptly on shutdown.
    let addr = {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap()
    };
    let client = DirectoryClient::new(&format!("http://{addr}")).unwrap();
    let (stop, rx) = watch::channel(false);
    let task = spawn_heartbeats(client, new_secret(), || info("game.test:1"), rx);
    tokio::time::sleep(Duration::from_millis(200)).await;
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("stops on shutdown")
        .unwrap();
}
