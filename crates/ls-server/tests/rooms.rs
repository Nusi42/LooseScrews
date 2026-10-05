//! Rotation and rooms against a real server on an ephemeral port: matches in
//! a row on one WebSocket connection (a new `Welcome` instead of a
//! disconnect), joins during the next-match build, private rooms next to the
//! public room, and `--ruleset`-style configs without a playlist.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use ls_game::ls_net::{check_welcome, Message, Welcome};
use ls_game::ls_sim::{MatchPhase, RuleSet};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::RoomConfig;
use ls_game::MatchConfig;
use ls_server::link::PeerLink;
use ls_server::{
    LobbyTiming, Playlist, PlaylistEntry, PrivateRoomSpec, Server, ServerConfig, ServerHandle,
    ServerRules, WorldChoice,
};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn code() -> ShareCode {
    ShareCode::new(0x5eed, Sliders::new(40, 30))
}

/// A server without pregeneration (fast builds in debug), no lobby and
/// 0.2 s of results.
fn config() -> ServerConfig {
    let mut cfg = ServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        MatchConfig::paint_the_void(code()),
    );
    cfg.room = RoomConfig {
        pregenerate: false,
        ..RoomConfig::default()
    };
    cfg.lobby = LobbyTiming::off(Duration::from_millis(200));
    cfg.private_rooms = 2;
    cfg
}

fn entry(name: &str) -> PlaylistEntry {
    let mut rules = RuleSet::paint_the_void();
    rules.name = name.into();
    PlaylistEntry {
        source: name.into(),
        rules: ServerRules {
            base: rules,
            bans: Vec::new(),
            balance_list: None,
        },
        world: WorldChoice::Fixed(code()),
        min_players: 1,
        max_players: 32,
    }
}

async fn connect(addr: SocketAddr, name: &str) -> (Ws, Welcome, Duration) {
    connect_to(addr, name, None).await
}

/// Connects with a room code in the `Hello` (`None`: the public room).
async fn connect_to(addr: SocketAddr, name: &str, room: Option<&str>) -> (Ws, Welcome, Duration) {
    let (mut ws, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
    let hello = ls_game::client_hello(name, None, None).with_room(room.map(String::from));
    let hello = Message::Hello(hello).encode().unwrap();
    let t0 = Instant::now();
    ws.send(WsMessage::Binary(hello.into())).await.unwrap();
    let welcome = next_welcome(&mut ws, name).await;
    (ws, welcome, t0.elapsed())
}

/// Reads until the next `Welcome`, answering nothing (pings keep the
/// connection alive; snapshots only flow after inputs, which these clients
/// never send). Fails on `Disconnect` or a closed socket.
async fn next_welcome(ws: &mut Ws, name: &str) -> Welcome {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut last_ping = Instant::now();
    loop {
        assert!(Instant::now() < deadline, "{name}: no Welcome in time");
        if last_ping.elapsed() > Duration::from_millis(500) {
            last_ping = Instant::now();
            let ping = Message::Ping {
                nonce: 1,
                client_time_ms: 0,
            };
            ws.send(WsMessage::Binary(ping.encode().unwrap().into()))
                .await
                .unwrap();
        }
        match tokio::time::timeout(Duration::from_millis(100), ws.next()).await {
            Err(_) => continue,
            Ok(Some(Ok(WsMessage::Binary(b)))) => match Message::decode(&b) {
                Ok(m @ Message::Welcome(_)) => return check_welcome(m).unwrap(),
                Ok(Message::Disconnect { reason }) => panic!("{name}: disconnected: {reason}"),
                Ok(Message::Reject(r)) => panic!("{name}: rejected: {r:?}"),
                _ => {}
            },
            Ok(Some(Ok(_))) => {}
            other => panic!("{name}: connection ended: {other:?}"),
        }
    }
}

fn rules_name(w: &Welcome) -> String {
    RuleSet::from_ron(&w.ruleset_name, std::str::from_utf8(&w.ruleset).unwrap())
        .unwrap()
        .name
}

async fn end_match(server: &ServerHandle) {
    server
        .with_room(|room| room.world_mut().mode_mut().phase = MatchPhase::Ended { winner: None })
        .await
        .unwrap();
}

/// Extra time of every next-match build in the rotation test.
const BUILD_DELAY: Duration = Duration::from_secs(2);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn matches_rotate_on_one_connection_and_joins_never_wait_for_a_build() {
    let mut cfg = config();
    cfg.playlist = Some(Playlist::new(vec![entry("First"), entry("Second")]).unwrap());
    // Every next-match build takes at least 2 s.
    cfg.build_delay = BUILD_DELAY;
    let server = Server::start(cfg).await.unwrap();
    let addr = server.local_addr();

    let (mut a, w1, _) = connect(addr, "a").await;
    assert_eq!(rules_name(&w1), "First");

    // Match 1 ends; the next one is built while the results show.
    end_match(&server).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    // A join during the build is welcomed at once, by the finished match's
    // room: its epoch proves the sim thread served the join before the 2 s
    // build had finished and been swapped in (a join that waited for the
    // build would get the next room's Welcome), whatever the machine's
    // load. The wall clock only backs that up with a bound far below the
    // build time; the 200 ms target holds on a quiet machine (printed) and
    // in the e2e/soak runs, not under a parallel test load.
    let (mut b, wb, took) = connect(addr, "b").await;
    eprintln!("join during the next-match build: Welcome after {took:?} (target < 200 ms)");
    assert_eq!(
        wb.epoch, w1.epoch,
        "served by the running room, not the next one"
    );
    assert_eq!(rules_name(&wb), "First");
    assert!(took < BUILD_DELAY / 2, "Welcome took {took:?}");

    // Both get the second match's Welcome on the same connection.
    let a2 = next_welcome(&mut a, "a").await;
    let b2 = next_welcome(&mut b, "b").await;
    assert_eq!(rules_name(&a2), "Second");
    assert_eq!(rules_name(&b2), "Second");
    assert_ne!(a2.epoch, w1.epoch, "the next room has its own epoch");
    assert_eq!(b2.epoch, a2.epoch);
    assert_ne!(a2.player_id, b2.player_id);

    // And a third match, still on the same connections.
    end_match(&server).await;
    let a3 = next_welcome(&mut a, "a").await;
    let _ = next_welcome(&mut b, "b").await;
    assert_eq!(rules_name(&a3), "First");

    let st = server.stats();
    assert_eq!(st.matches, 3);
    assert_eq!(st.connections, 2, "no reconnects");
    assert_eq!(st.kicked, 0);
    assert_eq!(server.total_players(), 2);
    server.shutdown().await;
}

/// Receives frames from a hub-opened link until a `Welcome`.
async fn link_welcome(peer: &PeerLink) -> Welcome {
    loop {
        let f = tokio::time::timeout(Duration::from_secs(60), peer.next_outgoing())
            .await
            .expect("Welcome in time")
            .expect("link open");
        if let Ok(m @ Message::Welcome(_)) = Message::decode(&f.data) {
            return check_welcome(m).unwrap();
        }
    }
}

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn private_rooms_run_next_to_the_public_room_and_close_when_empty() {
    let server = Server::start(config()).await.unwrap();
    let addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let hub = server.hub().clone();

    let (_public, _, _) = connect(server.local_addr(), "pub").await;

    let mut duel = RuleSet::paint_the_void();
    duel.name = "Duel".into();
    let rules = ServerRules {
        base: duel,
        bans: Vec::new(),
        balance_list: None,
    };
    let world = ShareCode::new(0xd0e1, Sliders::new(30, 50));
    let code = server
        .create_private_room(PrivateRoomSpec::custom(rules, Some(world)))
        .await
        .unwrap();
    assert!(code.as_str().starts_with("R-"), "{code}");

    let hello = |n: &str| ls_game::client_hello(n, None, None);
    // Over a real WebSocket, with the code in the Hello.
    let (_ws, w0, _) = connect_to(server.local_addr(), "w", Some(code.as_str())).await;
    assert_eq!(rules_name(&w0), "Duel");
    let p1 = hub
        .open_in(Some(code.as_str()), "test", addr, hello("x"), 1200)
        .unwrap();
    // Codes are lenient (lower case, no prefix).
    let lenient = code.as_str()[2..].to_ascii_lowercase();
    let p2 = hub
        .open_in(Some(&lenient), "test", addr, hello("y"), 1200)
        .unwrap();
    let w1 = link_welcome(&p1).await;
    let w2 = link_welcome(&p2).await;
    assert_eq!(rules_name(&w1), "Duel");
    assert_eq!(w1.share_code, world.encode().unwrap());
    assert_ne!(w1.player_id, w2.player_id);

    let rooms = server.rooms();
    assert_eq!(rooms.len(), 2, "{rooms:?}");
    assert_eq!(rooms[0].0.label, "public");
    assert_eq!(rooms[0].0.players, 1);
    assert_eq!(rooms[1].0.label, code.as_str());
    assert_eq!(rooms[1].0.players, 3);
    assert_eq!(rooms[1].0.ruleset, "Duel");
    assert_eq!(server.total_players(), 4);
    // An unknown code in a Hello is rejected over the wire too.
    let (mut ws, _) = connect_async(format!("ws://{}/ws", server.local_addr()))
        .await
        .unwrap();
    let unknown = hello("u").with_room(Some("R-ZZZZZZ".into()));
    ws.send(WsMessage::Binary(
        Message::Hello(unknown).encode().unwrap().into(),
    ))
    .await
    .unwrap();
    match tokio::time::timeout(Duration::from_secs(10), ws.next()).await {
        Ok(Some(Ok(WsMessage::Binary(b)))) => match Message::decode(&b).unwrap() {
            Message::Reject(r) => assert!(r.message.contains("unknown room"), "{r:?}"),
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    }

    // Unknown codes are rejected.
    assert!(hub
        .open_in(Some("R-ZZZZZZ"), "test", addr, hello("z"), 1200)
        .is_err());
    // The cap (2 here) holds.
    let spec = || PrivateRoomSpec::custom(ServerRules::default(), Some(world));
    let second = server.create_private_room(spec()).await.unwrap();
    let e = server.create_private_room(spec()).await.unwrap_err();
    assert!(e.to_string().contains("in use"), "{e}");

    // Everyone leaves: the room closes and its code is gone.
    drop(_ws);
    for p in [&p1, &p2] {
        p.push_incoming(
            Message::Disconnect {
                reason: "bye".into(),
            }
            .encode()
            .unwrap(),
        )
        .await
        .unwrap();
    }
    wait_until("the duel room to close", || {
        !server.rooms().iter().any(|(s, _)| s.label == code.as_str())
    })
    .await;
    assert!(hub
        .open_in(Some(code.as_str()), "test", addr, hello("x"), 1200)
        .is_err());
    // The public room is unaffected; a slot is free again.
    assert_eq!(server.rooms()[0].0.players, 1);
    let third = server.create_private_room(spec()).await.unwrap();
    assert_ne!(third, second);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ruleset_without_a_playlist_plays_every_match() {
    // What `ls-server --ruleset <code>` builds: rules, no playlist.
    let mut cfg = config();
    let code = RuleSet::sandbox_all_weapons().to_code().unwrap();
    cfg.rules = Some(ServerRules::new(Some(&code), Vec::new(), None).unwrap());
    let server = Server::start(cfg).await.unwrap();
    let (mut a, w, _) = connect(server.local_addr(), "a").await;
    let name = rules_name(&w);
    assert_eq!(name, RuleSet::sandbox_all_weapons().name);
    end_match(&server).await;
    let w2 = next_welcome(&mut a, "a").await;
    assert_eq!(rules_name(&w2), name);
    assert_eq!(w2.share_code, w.share_code, "same world");
    server.shutdown().await;
}
