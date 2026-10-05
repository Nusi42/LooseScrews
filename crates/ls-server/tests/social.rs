//! Server list, invites, parties and duels (M3 package B7) against real
//! servers and a real `ls-directory` on ephemeral loopback ports:
//!
//! - two servers registered with a local directory are listed (players,
//!   mode, world, roster);
//! - an invite link's party code puts two clients in the same room and
//!   team, also across a match transition and for a leader tagged while
//!   playing;
//! - a duel challenge (local, and relayed through the directory) creates a
//!   private two-player room with the challenger's seed.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use ls_directory::{Config as DirConfig, DirectoryClient, DirectoryServer};
use ls_game::ls_net::{check_welcome, Message, Welcome, PROTOCOL_VERSION};
use ls_game::ls_sim::{MatchPhase, RuleSet};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::RoomConfig;
use ls_game::MatchConfig;
use ls_server::{LobbyTiming, Server, ServerConfig, ServerHandle};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn code() -> ShareCode {
    ShareCode::new(0xb7, Sliders::new(40, 30))
}

/// A Paint the Void server (two teams) without pregeneration or lobby.
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

/// One HTTP/1.1 request on a fresh connection; returns status and JSON.
async fn http(addr: SocketAddr, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    tokio::time::timeout(Duration::from_secs(30), s.read_to_end(&mut raw))
        .await
        .expect("reply in time")
        .unwrap();
    let text = String::from_utf8(raw).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    assert!(
        head.contains("Access-Control-Allow-Origin: *"),
        "CORS on every API reply: {head}"
    );
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    let value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(body).unwrap_or_else(|e| panic!("{e}: {body}"))
    };
    (status, value)
}

async fn post(addr: SocketAddr, path: &str, body: Value) -> Value {
    let (status, v) = http(addr, "POST", path, Some(body)).await;
    assert_eq!(status, 200, "{path}: {v}");
    v
}

/// Connects with a room and party code; returns the socket and `Welcome`.
async fn connect(
    addr: SocketAddr,
    name: &str,
    room: Option<&str>,
    party: Option<&str>,
) -> Result<(Ws, Welcome), String> {
    let (mut ws, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
    let hello = ls_game::client_hello(name, None, None)
        .with_room(room.map(String::from))
        .with_party(party.map(String::from));
    ws.send(WsMessage::Binary(
        Message::Hello(hello).encode().unwrap().into(),
    ))
    .await
    .unwrap();
    let w = next_welcome(&mut ws, name).await?;
    Ok((ws, w))
}

/// Reads until the next `Welcome` (pinging so the connection stays up);
/// `Err` on a `Reject` or `Disconnect`.
async fn next_welcome(ws: &mut Ws, name: &str) -> Result<Welcome, String> {
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
                Ok(m @ Message::Welcome(_)) => return Ok(check_welcome(m).unwrap()),
                Ok(Message::Disconnect { reason }) => {
                    return Err(format!("disconnected: {reason}"))
                }
                Ok(Message::Reject(r)) => return Err(r.message),
                _ => {}
            },
            Ok(Some(Ok(_))) => {}
            other => return Err(format!("{name}: connection ended: {other:?}")),
        }
    }
}

async fn wait_for<T>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn directory() -> (SocketAddr, tokio::sync::watch::Sender<bool>) {
    let cfg = DirConfig {
        heartbeat: Duration::from_secs(1),
        ..DirConfig::default()
    };
    let server = DirectoryServer::bind("127.0.0.1:0".parse().unwrap(), cfg)
        .await
        .unwrap();
    let addr = server.local_addr().unwrap();
    let (tx, rx) = tokio::sync::watch::channel(false);
    server.spawn(rx);
    (addr, tx)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_server_list_shows_two_local_servers() {
    let (dir_addr, _dir) = directory().await;
    let url = format!("http://{dir_addr}");
    let mut servers: Vec<ServerHandle> = Vec::new();
    for (name, region) in [("Alpha", "eu"), ("Beta", "us")] {
        let mut cfg = config();
        cfg.listing.directory = Some(url.clone());
        cfg.listing.server_name = Some(name.into());
        cfg.listing.region = region.into();
        servers.push(Server::start(cfg).await.unwrap());
    }
    // One player on Alpha, present in its roster too.
    let alpha = servers[0].local_addr();
    let (_ws, _) = connect(alpha, "pilot", None, None).await.unwrap();
    post(
        alpha,
        "/api/presence",
        json!({"id": "player-0001", "token": "token-0123456789abcdef", "name": "Pilot"}),
    )
    .await;

    // What the browser's server list reads: GET /servers, filtered to its
    // protocol and data hash.
    let client = DirectoryClient::new(&url).unwrap();
    let hash = ls_directory::api::data_hash_hex(MatchConfig::paint_the_void(code()).data_hash());
    let list = wait_for("both servers with Alpha's player", async || {
        let l = client
            .list(Some(PROTOCOL_VERSION), Some(&hash))
            .await
            .ok()?;
        let alpha = l.servers.iter().find(|e| e.server.name == "Alpha")?;
        (l.servers.len() == 2 && alpha.server.players == 1 && !alpha.server.roster.is_empty())
            .then_some(l)
    })
    .await;
    let by_name = |n: &str| {
        list.servers
            .iter()
            .find(|e| e.server.name == n)
            .unwrap()
            .server
            .clone()
    };
    let (a, b) = (by_name("Alpha"), by_name("Beta"));
    assert_eq!(a.address, alpha.to_string());
    assert_eq!(b.address, servers[1].local_addr().to_string());
    assert_eq!((a.region.as_str(), b.region.as_str()), ("eu", "us"));
    assert_eq!(a.mode, "paint_the_void");
    assert_eq!(a.map_code, code().encode().unwrap());
    assert_eq!((a.players, b.players), (1, 0));
    assert_eq!(a.max_players, 8);
    assert_eq!(a.roster[0].name, "Pilot");
    assert_eq!(a.roster[0].id, "player-0001");
    // Another build's clients do not see them.
    let other = client
        .list(Some(PROTOCOL_VERSION), Some("0000000000000000"))
        .await
        .unwrap();
    assert!(other.servers.is_empty());

    // The same over the server's own API.
    let (status, info) = http(alpha, "GET", "/api/info", None).await;
    assert_eq!(status, 200);
    assert_eq!(info["name"], "Alpha");
    assert_eq!(info["directory"], url.as_str());
    assert_eq!(info["rooms"][0]["label"], "public");
    assert_eq!(info["players"], 1);

    // Shutdown removes the entries.
    for s in servers {
        s.shutdown().await;
    }
    let left = client.list(None, None).await.unwrap();
    assert!(left.servers.is_empty(), "{left:?}");
}

/// The team of the player with wire id `w.player_id`, from the room.
async fn team_of(server: &ServerHandle, w: &Welcome) -> u8 {
    let id = w.player_id;
    server
        .with_room(move |room| {
            room.clients()
                .into_iter()
                .find(|c| c.id == id)
                .map(|c| c.team)
                .unwrap()
        })
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_invite_with_a_party_code_puts_friends_in_one_room_and_team() {
    let server = Server::start(config()).await.unwrap();
    let addr = server.local_addr();
    let (_x, wx) = connect(addr, "stranger", None, None).await.unwrap();

    // The leader creates a party while playing in the public room: its
    // connection is tagged (player id), the invite link carries the code.
    let (mut lead, wl) = connect(addr, "leader", None, None).await.unwrap();
    let party = post(addr, "/api/party", json!({"player": wl.player_id})).await;
    let code = party["party"].as_str().unwrap().to_string();
    assert!(code.starts_with("P-"), "{party}");
    assert_eq!(party["room"], Value::Null);
    assert_eq!(party["address"], addr.to_string());
    assert_ne!(team_of(&server, &wx).await, team_of(&server, &wl).await);

    // The friend opens the link: the Hello carries the party code (any
    // spelling) and lands on the leader's team.
    let lenient = code[2..].to_lowercase();
    let (mut friend, wf) = connect(addr, "friend", None, Some(&lenient)).await.unwrap();
    assert_eq!(wf.room_code, wl.room_code, "same room");
    assert_eq!(team_of(&server, &wf).await, team_of(&server, &wl).await);
    assert_eq!(wf.team, wl.team);

    // Across a match transition the party stays together (1 vs 2 here).
    server
        .with_room(|room| room.world_mut().mode_mut().phase = MatchPhase::Ended { winner: None })
        .await
        .unwrap();
    let l2 = next_welcome(&mut lead, "leader").await.unwrap();
    let f2 = next_welcome(&mut friend, "friend").await.unwrap();
    assert_ne!(l2.epoch, wl.epoch);
    assert_eq!(l2.team, f2.team, "the party plays on one team");

    // A party for a custom match: its own private room with the given
    // world and rules; members join it with the party code.
    let world = ShareCode::new(0xc0de, Sliders::new(30, 40))
        .encode()
        .unwrap();
    let rules = RuleSet::sandbox_all_weapons().to_code().unwrap();
    let custom = post(
        addr,
        "/api/party",
        json!({"world_code": world, "rules_code": rules}),
    )
    .await;
    let room = custom["room"].as_str().unwrap().to_string();
    let pcode = custom["party"].as_str().unwrap();
    let (_a, wa) = connect(addr, "a", None, Some(pcode)).await.unwrap();
    let (_b, wb) = connect(addr, "b", Some("R-000000"), Some(pcode))
        .await
        .unwrap();
    assert_eq!(wa.room_code, room);
    assert_eq!(wb.room_code, room, "the party code wins over a room code");
    assert_eq!(wa.share_code, world);
    assert_eq!(wa.ruleset_name, RuleSet::sandbox_all_weapons().name);
    let statuses = server.rooms();
    let private = statuses.iter().find(|(s, _)| s.label == room).unwrap();
    assert_eq!(private.0.players, 2);
    assert!(server.social().parties().len(Instant::now()) >= 2);

    // Unknown party codes are refused.
    let e = connect(addr, "z", None, Some("P-ZZZZZZ"))
        .await
        .unwrap_err();
    assert!(e.contains("unknown party"), "{e}");
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_duel_challenge_opens_a_private_two_player_room_with_the_challengers_seed() {
    let server = Server::start(config()).await.unwrap();
    let addr = server.local_addr();
    let (ta, tb) = ("token-aaaaaaaaaaaaaaaa", "token-bbbbbbbbbbbbbbbb");
    let presence = |id: &'static str, token: &'static str, name: &'static str| {
        post(
            addr,
            "/api/presence",
            json!({"id": id, "token": token, "name": name}),
        )
    };
    presence("player-ace1", ta, "Ace").await;
    presence("player-bea1", tb, "Bea").await;
    let (status, roster) = http(addr, "GET", "/api/players", None).await;
    assert_eq!(status, 200);
    assert_eq!(roster["players"].as_array().unwrap().len(), 2);

    // Ace challenges Bea on Ace's seed.
    let seed = ShareCode::new(0xd0e1_5eed, Sliders::new(30, 50));
    let world = seed.encode().unwrap();
    let c = post(
        addr,
        "/api/duel",
        json!({"id": "player-ace1", "token": ta, "name": "Ace", "to": "player-bea1", "world_code": world}),
    )
    .await;
    assert_eq!(c["relayed"], false);
    // Bea's next poll shows the challenge (the toast).
    let inbox = presence("player-bea1", tb, "Bea").await;
    let note = &inbox["invites"][0];
    assert_eq!(note["kind"], "duel");
    assert_eq!(note["from_id"], "player-ace1");
    assert_eq!(note["from_name"], "Ace");
    assert_eq!(note["world_code"], world.as_str());
    assert!(note["expires_in_ms"].as_u64().unwrap() > 20_000);

    // Bea accepts: a private room opens; Ace's next poll has its code.
    let answer = post(
        addr,
        "/api/duel/answer",
        json!({"from": "player-ace1", "id": "player-bea1", "accept": true}),
    )
    .await;
    let room = answer["room"].as_str().unwrap().to_string();
    assert!(room.starts_with("R-"));
    let inbox = presence("player-ace1", ta, "Ace").await;
    let note = &inbox["invites"][0];
    assert_eq!(note["kind"], "accepted");
    assert_eq!(note["room"], room.as_str());
    assert_eq!(note["from_name"], "Bea");
    // A second answer finds no open challenge.
    let (status, _) = http(
        addr,
        "POST",
        "/api/duel/answer",
        Some(json!({"from": "player-ace1", "id": "player-bea1", "accept": true})),
    )
    .await;
    assert_eq!(status, 404);

    // Both join the room: two players on the challenger's world; a third
    // is turned away.
    let (_a, wa) = connect(addr, "Ace", Some(&room), None).await.unwrap();
    let (_b, wb) = connect(addr, "Bea", Some(&room), None).await.unwrap();
    assert_eq!(wa.share_code, world);
    assert_eq!(wb.share_code, world);
    assert_eq!(wa.room_code, room);
    let rules =
        RuleSet::from_ron(&wa.ruleset_name, std::str::from_utf8(&wa.ruleset).unwrap()).unwrap();
    assert_eq!(rules.mode.teams(), 0, "the duel rules: free-for-all");
    let statuses = server.rooms();
    let duel = statuses.iter().find(|(s, _)| s.label == room).unwrap();
    assert_eq!((duel.0.players, duel.0.max_players), (2, 2));
    assert_eq!(duel.0.share_code, world);
    let e = connect(addr, "Cid", Some(&room), None).await.unwrap_err();
    assert!(e.contains("full"), "{e}");

    // A declined challenge reaches the challenger as such.
    post(
        addr,
        "/api/duel",
        json!({"id": "player-bea1", "token": tb, "name": "Bea", "to": "player-ace1"}),
    )
    .await;
    let declined = post(
        addr,
        "/api/duel/answer",
        json!({"from": "player-bea1", "id": "player-ace1", "accept": false}),
    )
    .await;
    assert_eq!(declined["room"], Value::Null);
    let inbox = presence("player-bea1", tb, "Bea").await;
    assert!(inbox["invites"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["kind"] == "declined"));
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_challenge_to_another_server_goes_through_the_directory() {
    let (dir_addr, _dir) = directory().await;
    let url = format!("http://{dir_addr}");
    let mut cfg = config();
    cfg.listing.directory = Some(url.clone());
    let server = Server::start(cfg).await.unwrap();
    let addr = server.local_addr();
    let token = "token-aaaaaaaaaaaaaaaa";
    post(
        addr,
        "/api/presence",
        json!({"id": "player-ace1", "token": token, "name": "Ace"}),
    )
    .await;
    let rules = RuleSet::sandbox_all_weapons().to_code().unwrap();
    let c = post(
        addr,
        "/api/duel",
        json!({"id": "player-ace1", "token": token, "name": "Ace", "to": "far-away-99",
               "to_name": "Far", "rules_code": rules}),
    )
    .await;
    assert_eq!(c["relayed"], true);
    // The other player's browser polls the directory by its public id.
    let client = DirectoryClient::new(&url).unwrap();
    let invites = client.invites("far-away-99").await.unwrap();
    let inv = &invites.invites[0].invite;
    assert_eq!(inv.kind, "duel");
    assert_eq!(inv.from_id, "player-ace1");
    assert_eq!(inv.address, addr.to_string(), "the challenger's server");
    assert_eq!(inv.rules_code.as_deref(), Some(rules.as_str()));
    // ...and answers on the challenger's server.
    let answer = post(
        addr,
        "/api/duel/answer",
        json!({"from": "player-ace1", "id": "far-away-99", "accept": true}),
    )
    .await;
    let room = answer["room"].as_str().unwrap();
    let (_ws, w) = connect(addr, "Far", Some(room), None).await.unwrap();
    assert_eq!(w.ruleset_name, RuleSet::sandbox_all_weapons().name);

    // Without a directory, an absent player cannot be challenged.
    let lone = Server::start(config()).await.unwrap();
    let a = lone.local_addr();
    post(
        a,
        "/api/presence",
        json!({"id": "player-ace1", "token": token, "name": "Ace"}),
    )
    .await;
    let (status, e) = http(
        a,
        "POST",
        "/api/duel",
        Some(json!({"id": "player-ace1", "token": token, "name": "Ace", "to": "far-away-99"})),
    )
    .await;
    assert_eq!(status, 404, "{e}");
    lone.shutdown().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_api_refuses_bad_requests() {
    let server = Server::start(config()).await.unwrap();
    let addr = server.local_addr();
    let (s, _) = http(addr, "OPTIONS", "/api/duel", None).await;
    assert_eq!(s, 204, "CORS preflight");
    let (s, v) = http(addr, "GET", "/api/nope", None).await;
    assert_eq!(s, 404, "{v}");
    let (s, _) = http(addr, "GET", "/api/duel", None).await;
    assert_eq!(s, 405);
    let mut raw = TcpStream::connect(addr).await.unwrap();
    raw.write_all(b"POST /api/presence HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\nConnection: close\r\n\r\n{x}")
        .await
        .unwrap();
    let mut out = String::new();
    raw.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 400"), "{out}");
    let token = "token-aaaaaaaaaaaaaaaa";
    post(
        addr,
        "/api/presence",
        json!({"id": "player-ace1", "token": token, "name": "Ace"}),
    )
    .await;
    let (s, _) = http(
        addr,
        "POST",
        "/api/presence",
        Some(json!({"id": "player-ace1", "token": "token-zzzzzzzzzzzzzzzz", "name": "Eve"})),
    )
    .await;
    assert_eq!(s, 403, "an id belongs to its first token");
    let (s, _) = http(
        addr,
        "POST",
        "/api/duel",
        Some(json!({"id": "player-ace1", "token": token, "name": "Ace", "to": "player-bea1", "world_code": "nonsense"})),
    )
    .await;
    assert_eq!(s, 400, "bad world code");
    let (s, _) = http(
        addr,
        "POST",
        "/api/party",
        Some(json!({"room": "R-ZZZZZZ"})),
    )
    .await;
    assert_eq!(s, 404);
    let (s, v) = http(addr, "POST", "/api/rooms", Some(json!({}))).await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = http(addr, "POST", "/api/rooms", Some(json!({}))).await;
    assert_eq!(s, 200, "{v}");
    let (s, _) = http(addr, "POST", "/api/rooms", Some(json!({}))).await;
    assert_eq!(s, 503, "private rooms capped at 2");
    server.shutdown().await;
}
