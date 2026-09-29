use super::*;
use ls_game::server::{Room, RoomConfig};
use ls_math::terrain::TerrainEdit;
use ls_net::{Hello, Reject, RejectReason, PROTOCOL_VERSION};
use ls_worldgen::Sliders;

const DT: f64 = 1.0 / 60.0;

fn small_practice(dummies: u8) -> GameCore {
    let gen = Generator::with_size(7, Sliders::new(40, 30), [2, 2, 2]);
    GameCore::practice_with_generator(gen, dummies, false).expect("practice world")
}

fn forward() -> Controls {
    Controls {
        forward: 1.0,
        ..Default::default()
    }
}

/// A rock surface point seen from `from` that is not part of the sealed outer
/// walls (edits there do not change the level).
fn interior_surface(t: &GameTerrain, from: Vec3) -> Vec3 {
    let centre = t.find_open_point(t.bounds().center(), 3.0, 20);
    let mut dirs = Vec::new();
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if (x, y, z) != (0, 0, 0) {
                    dirs.push(Vec3::new(x as f32, y as f32, z as f32));
                }
            }
        }
    }
    [Some(from), centre]
        .into_iter()
        .flatten()
        .flat_map(|o| dirs.iter().filter_map(move |d| t.raycast(o, *d, 300.0)))
        .map(|h| h.point)
        .find(|p| t.with_store(|s| s.generator().room_open(*p)) > 2.0)
        .expect("interior rock surface visible from the spawn")
}

#[test]
fn practice_flies_the_local_ship_and_shows_dummies() {
    let mut g = small_practice(2);
    assert!(!g.is_online());
    let start = g.local_pose().expect("spawned");
    assert!(start.alive);
    let others = g.remote_ships();
    assert_eq!(others.len(), 2);
    assert!(others.iter().all(|s| s.team == 1 && s.alive));
    assert_eq!(g.team(), 0);

    for _ in 0..60 {
        assert!(g.frame(DT, forward()).is_empty(), "practice sends nothing");
    }
    let end = g.local_pose().unwrap();
    assert!(
        end.pos.distance(start.pos) > 3.0,
        "{:?} -> {:?}",
        start.pos,
        end.pos
    );
    assert!(end.vel.length() > 3.0);

    let hud = g.hud();
    assert_eq!(hud.hull, hud.hull_max);
    assert!(!hud.weapon.is_empty());
    assert!(
        hud.ammo.ends_with("shots") || hud.ammo == "∞",
        "{}",
        hud.ammo
    );
    assert!(hud.secondary.contains('×'), "{}", hud.secondary);
    assert!(hud.time_left_secs > 290.0 && hud.time_left_secs < 300.0);
    assert!(hud.status.is_empty());
}

#[test]
fn practice_render_pose_interpolates_between_ticks() {
    let mut g = small_practice(0);
    for _ in 0..30 {
        g.frame(DT, forward());
    }
    // Half a tick: the pose lies between the previous and current tick.
    g.frame(DT * 0.5, forward());
    let Mode::Practice(p) = &g.mode else {
        unreachable!()
    };
    let cur = p.world.ship(p.me).unwrap().pos;
    let prev = p.prev.iter().find(|s| s.player == p.me).unwrap().pos;
    let pose = g.local_pose().unwrap().pos;
    let (a, b) = (prev.distance(pose), cur.distance(pose));
    assert!(a > 0.0 && b > 0.0, "{prev:?} {pose:?} {cur:?}");
    assert!((a + b - prev.distance(cur)).abs() < 1e-3);
}

#[test]
fn practice_weapons_fire_projectiles() {
    let mut g = small_practice(0);
    let paint = g.weapons().find("paint_thrower").unwrap();
    let fire = Controls {
        buttons: ls_sim::Buttons::FIRE_PRIMARY.0,
        weapon_select: paint.0 + 1,
        ..Default::default()
    };
    for _ in 0..10 {
        g.frame(DT, fire);
    }
    let shots = g.projectiles();
    assert!(!shots.is_empty());
    assert!(shots
        .iter()
        .all(|p| p.kind == paint.0 && p.owner == g.player()));
    assert_eq!(g.hud().weapon, "Paint Thrower");
}

#[test]
fn chunks_stream_and_edits_remesh_dirty_chunks_first() {
    let mut g = small_practice(0);
    let cam = g.local_pose().unwrap().pos;
    let mut meshed = Vec::new();
    while let Some(m) = g.mesh_next(cam) {
        let v = m.mesh.vertex_count();
        assert_eq!(m.mesh.normals.len(), v);
        assert_eq!(m.mesh.teams.len(), v);
        assert!(m.mesh.indices.iter().all(|&i| (i as usize) < v));
        assert_eq!(m.aabb, m.coord.aabb());
        meshed.push(m.coord);
    }
    assert_eq!(meshed.len(), 8, "2x2x2 level fully in range");
    assert_eq!(g.pending_chunks(cam), 0);

    // A crater at a surface point dirties some chunks; they come back first.
    let hit = interior_surface(g.terrain(), cam);
    let home = g.terrain().with_store(|s| s.chunk_at(hit)).unwrap();
    let before = g.terrain_mut().mesh_chunk(home);
    g.terrain_mut().apply(TerrainEdit::SubtractSphere {
        center: hit,
        radius: 3.0,
    });
    let pending = g.pending_chunks(cam);
    assert!((1..=8).contains(&pending), "{pending}");
    let mut remeshed = Vec::new();
    while let Some(m) = g.mesh_next(cam) {
        remeshed.push((m.coord, m.mesh));
    }
    assert_eq!(remeshed.len(), pending);
    let (_, after) = remeshed
        .iter()
        .find(|(c, _)| *c == home)
        .expect("hit chunk remeshed");
    assert_ne!(after.hash(), before.hash(), "the crater changed the mesh");
}

#[test]
fn practice_rejects_bad_share_codes() {
    assert!(GameCore::practice("LS00-nope", 1).is_err());
    let typo = "LS04-0000-0000-000A-K4M7-8001-8F1P";
    assert!(GameCore::practice(typo, 1)
        .unwrap_err()
        .contains("checksum"));
}

// ----- online ---------------------------------------------------------------

struct Link {
    room: Room,
    id: u8,
    core: GameCore,
}

fn connect() -> Link {
    let code = ShareCode::new(4242, Sliders::new(40, 30));
    let cfg = RoomConfig {
        pregenerate: false,
        ..RoomConfig::default()
    };
    let mut room = Room::new(MatchConfig::paint_the_void(code), cfg).unwrap();
    let hello = Message::Hello(Hello::new("tester", None)).encode().unwrap();
    let id = room.join_packet(&hello).expect("join");
    let out = room.take_outgoing();
    assert_eq!(out.len(), 1);
    let core = GameCore::online(&out[0].bytes).expect("welcome accepted");
    Link { room, id, core }
}

impl Link {
    /// One server tick with one 1/60 s client frame, zero latency.
    fn tick(&mut self, controls: Controls) {
        for bytes in self.core.frame(DT, controls) {
            self.room
                .handle_packet(self.id, &bytes)
                .expect("server accepts client packets");
        }
        self.room.step();
        for o in self.room.take_outgoing() {
            assert_eq!(o.client, self.id);
            let ended = self
                .core
                .handle_packet(&o.bytes)
                .expect("client accepts server packets");
            assert_eq!(ended, None);
        }
    }
}

#[test]
fn online_session_predicts_and_reports_hud_from_snapshots() {
    let mut l = connect();
    assert!(l.core.is_online());
    assert_eq!(l.core.player(), l.id);
    assert_eq!(
        l.core.share_code(),
        l.room.terrain().share_code().encode().unwrap()
    );
    assert_eq!(l.core.hud().status, "Waiting for the server");
    assert!(l.core.local_pose().is_none());

    // Stay below tick 30: the first paint score update generates the whole
    // level, which is slow in debug builds.
    for _ in 0..28 {
        l.tick(forward());
    }
    let pose = l.core.local_pose().expect("prediction anchored");
    let server = l
        .room
        .world()
        .ship(l.room.player_of(l.id).unwrap())
        .unwrap();
    // The client runs a few ticks ahead of the server; positions are close.
    assert!(
        pose.pos.distance(server.pos) < 2.0,
        "{:?} vs {:?}",
        pose.pos,
        server.pos
    );
    let hud = l.core.hud();
    assert_eq!(hud.hull, hud.hull_max);
    assert!(hud.alive);
    assert_eq!(hud.weapon, "Scrap Laser", "server loadout primary");
    assert!(
        hud.secondary.starts_with("Bolt Rocket ×3"),
        "{}",
        hud.secondary
    );
    assert!(
        hud.time_left_secs > 295.0 && hud.time_left_secs <= 300.0,
        "{}",
        hud.time_left_secs
    );
    assert!(l.core.remote_ships().is_empty(), "alone in the room");

    // Terrain streams from the session's own copy of the level.
    let m = l.core.mesh_next(pose.pos).expect("a chunk near the ship");
    assert!(ls_math::terrain::TerrainQuery::bounds(l.core.terrain()).contains(m.aabb.center()));
}

#[test]
fn online_edits_from_snapshots_dirty_loaded_chunks() {
    let mut l = connect();
    for _ in 0..6 {
        l.tick(Controls::default());
    }
    let cam = l.core.local_pose().expect("anchored").pos;
    let hit = interior_surface(l.core.terrain(), cam);
    let home = l.core.terrain().with_store(|s| s.chunk_at(hit)).unwrap();
    // Load chunks (nearest first) until the one containing the hit is shown.
    let mut guard = 0;
    while !l.core.streamer.is_loaded(home) {
        l.core.mesh_next(cam).expect("chunk in range");
        guard += 1;
        assert!(guard < 300);
    }
    let before = l.core.edit_count();
    // A crater the server logged, delivered in a (full) snapshot newer than
    // anything the client has seen.
    let Mode::Online(o) = &l.core.mode else {
        unreachable!()
    };
    let newest = o.session.latest_snapshot().unwrap().tick;
    let edit = TerrainEdit::SubtractSphere {
        center: hit,
        radius: 4.0,
    };
    let packet = Message::Snapshot(Box::new(ls_net::SnapshotMsg {
        baseline: None,
        snapshot: ls_net::PackedSnapshot {
            tick: newest + 2,
            ack_input: newest + 2,
            edit_base: before,
            edits: vec![edit.into()],
            scores: vec![0, 0, 1],
            match_ticks_left: 100,
            ..Default::default()
        },
    }))
    .encode()
    .unwrap();
    assert_eq!(l.core.handle_packet(&packet).unwrap(), None);
    assert_eq!(l.core.edit_count(), before + 1);
    assert_eq!(l.core.terrain().edit_log(), &[edit]);
    let dirty = l.core.streamer.dirty_count();
    assert!(dirty >= 1, "the crater dirtied a shown chunk");
    let mut remeshed = Vec::new();
    for _ in 0..dirty {
        remeshed.push(l.core.mesh_next(cam).unwrap().coord);
    }
    assert!(
        remeshed.contains(&home),
        "dirty chunks come first: {remeshed:?}"
    );
    assert_eq!(l.core.streamer.dirty_count(), 0);
}

#[test]
fn online_rejects_are_reported() {
    let reject = Message::Reject(Reject {
        reason: RejectReason::ServerFull,
        server_version: PROTOCOL_VERSION,
        message: "room is full".into(),
    })
    .encode()
    .unwrap();
    let err = GameCore::online(&reject).expect_err("reject is an error");
    assert!(err.contains("room is full"), "{err}");
    assert!(GameCore::online(&[1, 2, 3]).is_err());
}

#[test]
fn ammo_and_feed_text() {
    let w = WeaponTable::builtin();
    let laser = w.get(w.find("scrap_laser").unwrap());
    let rocket = w.get(w.find("bolt_rocket").unwrap());
    assert!(ammo_text(laser, 100.0, 0.0, None).ends_with(" shots"));
    assert_eq!(ammo_text(rocket, 0.0, 0.0, Some(2)), "2");
    assert_eq!(ammo_text(rocket, 0.0, 0.0, None), "?");
    assert_eq!(
        kill_line(Some("You".into()), "Player 2".into()),
        "You destroyed Player 2"
    );
    assert_eq!(kill_line(None, "You".into()), "You crashed");
    assert_eq!(
        match_over_text(Some(1), 1),
        "Match over: Blue wins (your team)"
    );
    assert_eq!(match_over_text(None, 0), "Match over: draw");
}

/// Server -> client and client -> server queues with latency and jitter
/// (order preserved, like a WebSocket).
struct Lane {
    queue: std::collections::VecDeque<(u64, Vec<u8>)>,
    last_due: u64,
}

impl Lane {
    fn new() -> Self {
        Lane {
            queue: Default::default(),
            last_due: 0,
        }
    }
    fn push(&mut self, due: u64, bytes: Vec<u8>) {
        self.last_due = self.last_due.max(due);
        self.queue.push_back((self.last_due, bytes));
    }
    fn pop(&mut self, now: u64) -> Option<Vec<u8>> {
        if self.queue.front().is_some_and(|(d, _)| *d <= now) {
            self.queue.pop_front().map(|(_, b)| b)
        } else {
            None
        }
    }
}

/// Online client over a lagged, jittery link with uneven frame times and a
/// delay spike that makes the clock catch up (more ticks than due) and later
/// skip (fewer): prediction stays converged, the HUD reports sync, and every
/// bit of mouse look is applied exactly once.
#[test]
fn online_client_under_latency_applies_look_exactly_once() {
    let code = ShareCode::new(4242, Sliders::new(40, 30));
    // Runs past tick 30, so the room generates the whole level (slow in
    // debug builds, like the ls-game netplay tests).
    let mut room = Room::new(MatchConfig::paint_the_void(code), RoomConfig::default()).unwrap();
    let hello = Message::Hello(Hello::new("lagged", None)).encode().unwrap();
    let id = room.join_packet(&hello).expect("join");
    let mut down = Lane::new();
    let mut up = Lane::new();
    for o in room.take_outgoing() {
        down.push(5, o.bytes);
    }
    let mut core: Option<GameCore> = None;
    let mut rng = ls_math::rng::Pcg32::new(7, 1);
    let mut look_in = 0.0f64;
    let mut cmds: std::collections::BTreeMap<u32, f32> = Default::default();
    let mut synced_at = None;
    let mut before_spike = ls_game::client::PredictionStats::default();
    let mut caught_up = false;
    let total = 1400u64;
    for now in 1..=total {
        // Client -> server.
        while let Some(b) = up.pop(now) {
            room.handle_packet(id, &b)
                .expect("server accepts client packets");
        }
        room.step();
        let spike = (300..420).contains(&now);
        for o in room.take_outgoing() {
            let jitter = u64::from(rng.next_u32() % 3);
            down.push(now + 5 + jitter, o.bytes);
        }
        // Server -> client.
        while let Some(b) = down.pop(now) {
            match &mut core {
                None => core = Some(GameCore::online(&b).expect("welcome")),
                Some(c) => assert_eq!(c.handle_packet(&b).expect("valid packet"), None),
            }
        }
        let Some(c) = &mut core else { continue };
        // Uneven frames: a stall every 50 frames, made up by the next one.
        let dt = match now % 50 {
            0 => 0.0,
            1 => 2.0 * DT,
            _ => DT,
        };
        let Mode::Online(o) = &c.mode else {
            unreachable!()
        };
        let synced = o.session.is_synced();
        if synced && synced_at.is_none() {
            synced_at = Some(now);
        }
        // Mouse look once synced, stopping well before the end so the
        // accumulator drains.
        let yaw_right = if synced && now < total - 60 {
            0.003
        } else {
            0.0
        };
        let before_ticks = o.session.next_input_tick();
        let packets = c.frame(
            dt,
            Controls {
                yaw_right,
                ..Default::default()
            },
        );
        let Mode::Online(o) = &c.mode else {
            unreachable!()
        };
        let ran = o.session.next_input_tick() - before_ticks;
        if synced && ran > u64::from((dt / DT).round() as u32) + 1 {
            caught_up = true;
        }
        if yaw_right != 0.0 {
            look_in += f64::from(-yaw_right);
        }
        for p in packets {
            if let Message::Input(m) = Message::decode(&p).unwrap() {
                for cmd in &m.cmds {
                    let cmd = ls_game::convert::net_to_input(cmd);
                    cmds.insert(cmd.tick as u32, cmd.look_yaw);
                }
            }
            let delay = if spike { 14 } else { 4 } + u64::from(rng.next_u32() % 3);
            up.push(now + delay, p);
        }
        if now == 290 {
            before_spike = o.session.prediction_stats();
        }
    }
    let c = core.expect("joined");
    let synced_at = synced_at.expect("synchronized");
    assert!(synced_at < 60, "synced at {synced_at}");
    assert!(c.hud().status.is_empty(), "{}", c.hud().status);
    assert!(caught_up, "the delay spike made the clock catch up");
    let Mode::Online(o) = &c.mode else {
        unreachable!()
    };
    // Every look delta went out exactly once (up to the wire rounding the
    // accumulator carries).
    let look_out: f64 = cmds.values().map(|&y| f64::from(y)).sum();
    assert!(
        (look_out - look_in).abs() < 4.0 * f64::from(crate::controls::LOOK_STEP),
        "sent {look_out} rad of yaw for {look_in} rad of mouse input"
    );
    // Prediction converged before the spike and again after it.
    assert!(
        before_spike.corrected + before_spike.snapped <= 3,
        "{before_spike:?}"
    );
    let p = o.session.prediction_stats();
    let tick = o.session.latest_snapshot().unwrap().tick as u64;
    let predicted = o.session.predictor().state_at(tick).unwrap();
    let server = room.world().ship(room.player_of(id).unwrap()).unwrap();
    assert!(predicted.pos.is_finite() && server.pos.is_finite());
    assert!(p.accepted > 300, "{p:?}");

    // The server ends the session: the reason comes back.
    let mut c = c;
    let bye = Message::Disconnect {
        reason: "match over".into(),
    }
    .encode()
    .unwrap();
    assert_eq!(
        c.handle_packet(&bye).unwrap().as_deref(),
        Some("match over")
    );
}

// ----- shot prediction, overlays --------------------------------------------

/// Over a 4-tick (each way) link the player's shot is drawn on the frame the
/// trigger is pulled, long before any snapshot can carry it; the server's
/// projectiles then take over without a visible jump, nothing is dropped or
/// unexpected, and the ping is measured.
#[test]
fn online_shots_are_predicted_and_confirmed_under_latency() {
    let code = ShareCode::new(4242, Sliders::new(40, 30));
    let cfg = RoomConfig {
        pregenerate: false,
        ..RoomConfig::default()
    };
    let mut room = Room::new(MatchConfig::paint_the_void(code), cfg).unwrap();
    let hello = Message::Hello(Hello::new("gunner", None)).encode().unwrap();
    let id = room.join_packet(&hello).expect("join");
    let mut down = Lane::new();
    let mut up = Lane::new();
    for o in room.take_outgoing() {
        down.push(4, o.bytes);
    }
    let mut core: Option<GameCore> = None;
    let mut fire_from = None;
    let mut first_seen = None;
    let mut max_offset = 0.0f32;
    let laser = WeaponTable::builtin().find("scrap_laser").unwrap();
    for now in 1..=150u64 {
        while let Some(b) = up.pop(now) {
            room.handle_packet(id, &b).expect("valid input");
        }
        room.step();
        for o in room.take_outgoing() {
            down.push(now + 4, o.bytes);
        }
        // The host clock: one frame per tick.
        let clock_ms = now as f64 * 1000.0 / 60.0;
        if let Some(c) = &mut core {
            c.set_clock_ms(clock_ms);
        }
        while let Some(b) = down.pop(now) {
            match &mut core {
                None => {
                    let mut c = GameCore::online(&b).expect("welcome");
                    c.set_clock_ms(clock_ms);
                    core = Some(c);
                }
                Some(c) => assert_eq!(c.handle_packet(&b).expect("valid packet"), None),
            }
        }
        let Some(c) = &mut core else { continue };
        let Mode::Online(o) = &c.mode else {
            unreachable!()
        };
        if o.session.is_synced() && fire_from.is_none() {
            fire_from = Some(now);
        }
        let firing = fire_from.is_some_and(|f| now < f + 60);
        let controls = Controls {
            forward: 0.5,
            yaw_right: 0.002,
            buttons: if firing {
                ls_sim::Buttons::FIRE_PRIMARY.0
            } else {
                0
            },
            weapon_select: laser.0 + 1,
            ..Default::default()
        };
        for p in c.frame(DT, controls) {
            up.push(now + 4, p);
        }
        let me = c.player();
        let own = c.projectiles().iter().filter(|p| p.owner == me).count();
        if firing && own > 0 && first_seen.is_none() {
            first_seen = Some(now);
        }
        let Mode::Online(o) = &c.mode else {
            unreachable!()
        };
        if let Some(snap) = o.session.latest_snapshot() {
            for p in &snap.projectiles {
                max_offset = max_offset.max(o.shots.offset(p.id).length());
            }
        }
    }
    let c = core.expect("joined");
    let fire_from = fire_from.expect("synchronized");
    assert!(fire_from < 40, "synced at {fire_from}");
    assert_eq!(
        first_seen,
        Some(fire_from),
        "the first shot is drawn on the frame it is fired"
    );
    let d = c.debug_stats();
    let s = d.shots;
    assert!(s.predicted >= 4, "{s:?}");
    assert_eq!(s.unpredicted, 0, "{s:?}");
    assert!(s.dropped <= 1, "{s:?}");
    assert!(s.confirmed + s.dropped >= s.predicted - 1, "{s:?}");
    assert!(max_offset < 0.1, "reconciliation offset {max_offset}");
    assert!(s.confirmed >= 3, "the server fired them too: {s:?}");
    let ping = d.ping_ms.expect("pongs arrived");
    // 4 + 4 ticks of link latency (133 ms) on the host clock.
    assert!((125.0..=155.0).contains(&ping), "ping {ping} ms");
    assert!(d.synced);
    // The first snapshot anchors the prediction (counted as a snap).
    assert!(d.corrected + d.snapped <= 1, "{d:?}");
}

#[test]
fn online_scoreboard_counts_kills_and_match_info_reports_the_end() {
    let mut l = connect();
    for _ in 0..8 {
        l.tick(Controls::default());
    }
    let rows = l.core.scoreboard();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].is_you && rows[0].alive);
    assert_eq!((rows[0].kills, rows[0].deaths), (0, 0));
    assert!(!l.core.match_info().over);

    let me = l.core.player();
    let kills = Message::Events(vec![
        NetEvent::Kill {
            tick: 5,
            killer: me,
            victim: 5,
            weapon: 0,
        },
        NetEvent::Kill {
            tick: 6,
            killer: 5,
            victim: me,
            weapon: 0,
        },
        NetEvent::Kill {
            tick: 7,
            killer: me,
            victim: me,
            weapon: 0,
        },
        NetEvent::Kill {
            tick: 8,
            killer: u8::MAX,
            victim: me,
            weapon: 0,
        },
    ])
    .encode()
    .unwrap();
    assert_eq!(l.core.handle_packet(&kills).unwrap(), None);
    let feed = l.core.take_feed();
    assert_eq!(feed.len(), 4, "{feed:?}");
    let you = l.core.scoreboard()[0];
    assert_eq!(
        (you.kills, you.deaths),
        (1, 3),
        "self-destructs and crashes are deaths, not kills"
    );

    // A snapshot after the timer ran out: the match is over, the leader wins.
    let Mode::Online(o) = &l.core.mode else {
        unreachable!()
    };
    let newest = o.session.latest_snapshot().unwrap().tick;
    let end = Message::Snapshot(Box::new(ls_net::SnapshotMsg {
        baseline: None,
        snapshot: ls_net::PackedSnapshot {
            tick: newest + 2,
            ack_input: newest + 2,
            edit_base: l.core.edit_count(),
            scores: vec![30, 70, 400],
            match_ticks_left: 0,
            ..Default::default()
        },
    }))
    .encode()
    .unwrap();
    assert_eq!(l.core.handle_packet(&end).unwrap(), None);
    let m = l.core.match_info();
    assert!(m.over);
    assert_eq!(m.winner, Some(1));
    assert_eq!((m.team_cells, m.surface_cells), ([30, 70], 400));
    // Time since the end grows with the server time estimate.
    let before = m.secs_since_end;
    for _ in 0..30 {
        l.core.frame(DT, Controls::default());
    }
    let after = l.core.match_info().secs_since_end;
    assert!(after > before + 0.05, "{before} -> {after}");
    // The own row is still listed although the snapshot had no ships.
    assert!(l.core.scoreboard().iter().any(|r| r.is_you));
}

#[test]
fn practice_scoreboard_and_match_info() {
    let mut g = small_practice(2);
    let rows = g.scoreboard();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.iter().filter(|r| r.is_you).count(), 1);
    assert_eq!(rows[0].team, 0, "sorted by team");
    assert!(rows[1..].iter().all(|r| r.team == 1));
    let m = g.match_info();
    assert!(!m.over);
    assert_eq!(m.secs_since_end, 0.0);
    assert_eq!(g.debug_stats().ping_ms, None);
    // End the match: the world stops, the end screen time runs.
    if let Mode::Practice(p) = &mut g.mode {
        p.world.mode_mut().end_tick = p.world.tick() + 1;
    }
    for _ in 0..10 {
        g.frame(DT, Controls::default());
    }
    let m = g.match_info();
    assert!(m.over);
    assert!(m.secs_since_end > 0.1, "{}", m.secs_since_end);
}
