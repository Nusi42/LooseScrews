use super::*;
use crate::mechanics::CUE_HIT;
use ls_game::server::{Room, RoomConfig};
use ls_math::terrain::TerrainEdit;
use ls_net::{Reject, RejectReason, PROTOCOL_VERSION};
use ls_sim::ReplicatedKind;
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
fn surface_states_reach_the_mesh_flags_and_expire() {
    let mut g = small_practice(0);
    let cam = g.local_pose().unwrap().pos;
    while g.mesh_next(cam).is_some() {}
    let hit = interior_surface(g.terrain(), cam);
    let home = g.terrain().with_store(|s| s.chunk_at(hit)).unwrap();
    // A charged patch lasting 30 ticks from now.
    let until = g.terrain().display_tick() as u32 + 30;
    g.terrain_mut().apply(TerrainEdit::Surface {
        center: hit,
        radius: 3.0,
        states: ls_math::terrain::SurfaceStates::CHARGED.bits(),
        until,
    });
    let mut flags = Vec::new();
    while let Some(m) = g.mesh_next(cam) {
        if m.coord == home {
            flags = surface_flags(&m.mesh);
            assert_eq!(flags.len(), m.mesh.vertex_count());
        }
    }
    assert!(
        flags.iter().any(|f| f & SURFACE_CHARGED != 0),
        "charged vertices after the edit"
    );
    // Once the state expired the frame loop remeshes the chunk without it.
    for _ in 0..40 {
        g.frame(DT, Controls::default());
    }
    let mut after = None;
    while let Some(m) = g.mesh_next(cam) {
        if m.coord == home {
            after = Some(surface_flags(&m.mesh));
        }
    }
    let after = after.expect("the expired chunk is remeshed");
    assert!(after.iter().all(|f| f & SURFACE_CHARGED == 0));
}

#[test]
fn practice_shell_puts_the_squeeze_wall_on_the_terrain() {
    // The visual gallery's Squeeze scenes (M3 C1): the shell is solid outside the sphere.
    let mut g = small_practice(0);
    let center = g.terrain().bounds().center();
    let shell = ls_sim::ShellSphere {
        center,
        radius: 12.0,
    };
    assert!(g.practice_shell(Some(shell)));
    assert_eq!(g.terrain().shell(), Some(shell));
    let outside = center + Vec3::new(30.0, 0.0, 0.0);
    assert!(
        ls_math::terrain::TerrainQuery::sdf(g.terrain(), outside) < 0.0,
        "rock beyond the shell"
    );
    assert!(g.practice_shell(None));
    assert_eq!(g.terrain().shell(), None);
}

#[test]
fn practice_rejects_bad_share_codes() {
    assert!(GameCore::practice("LS00-nope", 1).is_err());
    let typo = "LS0C-0000-0000-000A-K4M7-8001-8F1Y";
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
    let hello = Message::Hello(ls_game::client_hello("tester", None, None))
        .encode()
        .unwrap();
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
        epoch: 0,
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
    let hello = Message::Hello(ls_game::client_hello("lagged", None, None))
        .encode()
        .unwrap();
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
    let hello = Message::Hello(ls_game::client_hello("gunner", None, None))
        .encode()
        .unwrap();
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
    let you = l.core.scoreboard()[0].clone();
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
        epoch: 0,
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

// ----- M2: entities, visuals, names, weapon HUD and selection ------------------

fn select(key: u8, wheel: i8, x: i8) -> Controls {
    Controls {
        weapon_key: key,
        cycle_primary: wheel,
        cycle_secondary: x,
        ..Default::default()
    }
}

#[test]
fn practice_entities_carry_visuals_and_debris_radius() {
    let mut g = small_practice(0);
    let pos = g.local_pose().unwrap().pos;
    if let Mode::Practice(p) = &mut g.mode {
        p.world.spawn_entity(ls_sim::EntitySpawn::debris(
            pos + Vec3::new(4.0, 0.0, 0.0),
            Vec3::ZERO,
            1.2,
            6.0,
        ));
        p.world
            .spawn_entity(ls_sim::EntitySpawn::dust(pos, 5.0, 600));
    }
    g.frame(DT, Controls::default());
    let ents = g.entities();
    let debris = ents
        .iter()
        .find(|e| e.kind == ReplicatedKind::Debris as u8)
        .expect("debris replicated");
    assert!((debris.radius - 1.2).abs() < 1e-6);
    assert_eq!(debris.state[0], 6, "whole scrap units");
    assert_eq!(debris.visual, ls_sim::BuiltinVisual::Debris.id().0);
    let dust = ents
        .iter()
        .find(|e| e.kind == ReplicatedKind::DustCloud as u8)
        .expect("dust replicated");
    assert_eq!(dust.radius, 5.0);
    // Every visual id resolves in the match table; wire ids are unique.
    for e in &ents {
        assert!(
            g.visuals().get(ls_sim::VisualId(e.visual)).is_some(),
            "{e:?}"
        );
    }
    let mut ids: Vec<u16> = ents.iter().map(|e| e.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), ents.len());
}

#[test]
fn projectiles_carry_their_weapon_visual() {
    let mut g = small_practice(0);
    let fire = Controls {
        buttons: ls_sim::Buttons::FIRE_PRIMARY.0,
        ..Default::default()
    };
    for _ in 0..10 {
        g.frame(DT, fire);
    }
    let shots = g.projectiles();
    assert!(!shots.is_empty());
    for p in shots {
        let v = g.visuals().projectile(WeaponId(p.kind));
        assert_eq!(p.visual, v.0);
    }
}

#[test]
fn practice_players_have_names_and_ship_types() {
    let g = small_practice(2);
    let players = g.players();
    assert_eq!(players.len(), 3);
    let me = players.iter().find(|p| p.id == g.player()).unwrap();
    assert_eq!(me.name, "You");
    assert_eq!(me.ship_type, ShipType::Tinker.index());
    assert!(players
        .iter()
        .filter(|p| p.id != g.player())
        .all(|p| p.name.starts_with("Dummy ")));
    let rows = g.scoreboard();
    assert!(rows.iter().all(|r| !r.name.is_empty()));
    let ships = g.remote_ships();
    assert!(ships
        .iter()
        .all(|s| s.ship_type == ShipType::Tinker.index() && s.radius > 0.0));
}

#[test]
fn practice_number_keys_wheel_and_x_select_carried_weapons() {
    let mut g = small_practice(0);
    if let Mode::Practice(p) = &mut g.mode {
        let me = p.me;
        for w in ["rivet_gun", "plasma_repeater", "seeker"] {
            p.world.give_weapon(me, w).unwrap();
        }
    }
    let view = g.loadout();
    let names: Vec<&str> = view.primaries.iter().map(|e| e.name.as_str()).collect();
    let first = g.weapons().get(view.primaries[0].weapon).id.clone();
    assert_eq!(first, "scrap_laser", "{names:?}");
    let rank =
        |e: &crate::inventory::WeaponEntry| crate::inventory::design_rank(g.weapons(), e.weapon);
    assert!(view.primaries.windows(2).all(|w| rank(&w[0]) < rank(&w[1])));
    assert_eq!(view.primaries[0].key, 1);
    assert!(view.primaries[0].selected);
    let plasma = g.weapons().find("plasma_repeater").unwrap();
    let key_of_plasma = view
        .primaries
        .iter()
        .find(|e| e.weapon == plasma)
        .unwrap()
        .key;

    // A number key selects the n-th carried primary.
    g.frame(DT, select(key_of_plasma, 0, 0));
    let sel = |g: &GameCore| {
        let v = g.loadout();
        (
            v.primaries.iter().find(|e| e.selected).map(|e| e.weapon),
            v.secondaries.iter().find(|e| e.selected).map(|e| e.weapon),
        )
    };
    assert_eq!(sel(&g).0, Some(plasma));
    assert_eq!(g.hud().weapon, "Plasma Repeater");

    // The wheel steps through the primaries (wrapping).
    let n = g.loadout().primaries.len();
    for _ in 0..n {
        g.frame(DT, select(0, 1, 0));
    }
    assert_eq!(sel(&g).0, Some(plasma), "a full turn of the wheel");
    g.frame(DT, select(0, -1, 0));
    assert_ne!(sel(&g).0, Some(plasma));

    // X cycles secondaries.
    let before = sel(&g).1;
    g.frame(DT, select(0, 0, 1));
    let after = sel(&g).1;
    assert_ne!(before, after);
    let name = g.weapons().get(after.unwrap()).name.clone();
    assert!(g.hud().secondary.starts_with(&name));

    // A key pressed on a frame that runs no tick is applied on the next one.
    g.frame(0.0, select(1, 0, 0));
    g.frame(DT, Controls::default());
    assert_eq!(sel(&g).0, Some(g.loadout().primaries[0].weapon));
}

#[test]
fn practice_gadget_key_toggles_the_gadget_in_the_hud() {
    let mut g = small_practice(0);
    let shield = g.gadgets().find("shield").unwrap();
    if let Mode::Practice(p) = &mut g.mode {
        let me = p.me;
        p.world.ship_mut(me).unwrap().gadget = Some(shield);
    }
    assert_eq!(g.loadout().gadget, "Shield");
    assert!(!g.loadout().gadget_active);
    let press = Controls {
        buttons: ls_sim::Buttons::GADGET.0,
        ..Default::default()
    };
    g.frame(DT, press);
    g.frame(DT, Controls::default());
    assert!(g.loadout().gadget_active, "G toggles the Shield on");
}

#[test]
fn online_names_entities_and_the_server_inventory() {
    let mut l = connect();
    for _ in 0..4 {
        l.tick(Controls::default());
    }
    // Names come from the snapshot player list.
    let players = l.core.players();
    let me = players.iter().find(|p| p.id == l.core.player()).unwrap();
    assert_eq!(me.name, "tester");
    assert_eq!(l.core.scoreboard()[0].name, "tester");

    // Server-side debris shows up as an entity.
    let pid = l.room.player_of(l.id).unwrap();
    let pos = l.room.world().ship(pid).unwrap().pos;
    l.room.world_mut().spawn_entity(ls_sim::EntitySpawn::debris(
        pos + Vec3::new(3.0, 0.0, 0.0),
        Vec3::ZERO,
        0.8,
        4.0,
    ));
    // Entities are drawn at the render tick, like remote ships (a few
    // ticks behind the newest snapshot).
    for _ in 0..16 {
        l.tick(Controls::default());
    }
    let debris = l
        .core
        .entities()
        .into_iter()
        .find(|e| e.kind == ReplicatedKind::Debris as u8)
        .expect("debris in the snapshot");
    assert_eq!(debris.state[0], 4);
    assert!(debris.pos.distance(pos + Vec3::new(3.0, 0.0, 0.0)) < 0.5);
    assert!(l
        .core
        .visuals()
        .get(ls_sim::VisualId(debris.visual))
        .is_some());

    // Move the ship onto a weapon pickup on the server: the server reports
    // the new inventory.
    let found = l
        .room
        .world()
        .pickups()
        .iter()
        .find_map(|sp| match sp.content {
            Some((_, ls_sim::PickupKind::Weapon { weapon, .. })) => Some((sp.pos, weapon)),
            _ => None,
        });
    let (spot, weapon) = found.expect("a weapon pickup on the level");
    l.room.world_mut().ship_mut(pid).unwrap().pos = spot;
    for _ in 0..4 {
        l.tick(Controls::default());
    }
    let server_has = l.room.world().ship(pid).unwrap().weapon(weapon).is_some();
    assert!(server_has, "the server gave the weapon");
    let view = l.core.loadout();
    assert!(
        view.primaries
            .iter()
            .chain(&view.secondaries)
            .any(|e| e.weapon == weapon),
        "client inventory has {}",
        l.core.weapons().get(weapon).id
    );
}

fn press(buttons: ls_sim::Buttons) -> Controls {
    Controls {
        buttons: buttons.0,
        ..Default::default()
    }
}

#[test]
fn practice_cues_gadget_state_and_charge_meter() {
    let mut g = small_practice(0);
    let me = g.player();
    // The default loadout carries the Shield: G switches it on, a Gadget cue
    // goes out and the gadget view reports it.
    assert_eq!(g.gadget().key, "shield");
    assert!(!g.gadget().active && g.gadget().toggle);
    g.frame(DT, press(ls_sim::Buttons::GADGET));
    g.frame(DT, Controls::default());
    let cues = g.take_cues();
    let on = cues
        .iter()
        .find(|c| c.kind() == ls_sim::CueKind::Gadget)
        .expect("gadget cue");
    assert_eq!(on.source, me);
    assert!(g.gadget().active);
    assert!(g.take_cues().is_empty(), "taken once");

    // Fusion Charger: holding the trigger fills the charge meter; releasing
    // fires and empties it.
    let fusion = g.weapons().find("fusion_charger").unwrap();
    if let Mode::Practice(p) = &mut g.mode {
        let id = p.me;
        p.world.give_weapon(id, "fusion_charger").unwrap();
    }
    let hold = Controls {
        buttons: ls_sim::Buttons::FIRE_PRIMARY.0,
        weapon_select: fusion.0 + 1,
        ..Default::default()
    };
    for _ in 0..61 {
        g.frame(DT, hold);
    }
    let c = g.charge().expect("charging");
    assert_eq!(c.weapon, fusion.0);
    assert_eq!(c.name, "Fusion Charger");
    assert!((c.fraction - 0.5).abs() < 0.03, "{}", c.fraction);
    assert!(!c.overcharge);
    g.frame(DT, Controls::default());
    assert!(g.charge().is_none());
    assert!(g.projectiles().iter().any(|p| p.kind == fusion.0));
}

#[test]
fn practice_ship_choice_and_paint_colour() {
    let gen = Generator::with_size(7, Sliders::new(40, 30), [2, 2, 2]);
    let mut g = GameCore::practice_with_generator_as(gen, 0, false, ShipType::Hauler).unwrap();
    assert_eq!(g.ship_type(), ShipType::Hauler);
    let choices = g.ship_choices();
    assert_eq!(choices.len(), 4);
    assert!(
        choices.iter().all(|c| c.allowed),
        "Paint the Void allows every type"
    );
    assert!(choices[ShipType::Hauler.index() as usize].current);
    assert!(g.next_ship().is_none());

    assert_eq!(
        g.select_ship(ShipType::Skitter),
        Ok(None),
        "practice sends nothing"
    );
    assert_eq!(g.next_ship(), Some(ShipType::Skitter));
    assert!(g.ship_choices()[0].next);
    if let Mode::Practice(p) = &g.mode {
        let ship = p.world.ship(p.me).unwrap();
        assert_eq!(ship.next_ship_type, Some(ShipType::Skitter));
    }

    assert_eq!(g.paint_color(), 0, "no Paint Thrower selected");
    let pt = g.weapons().find("paint_thrower").unwrap();
    // The Paint the Void loadout carries it.
    let select = Controls {
        weapon_select: pt.0 + 1,
        ..Default::default()
    };
    g.frame(DT, select);
    g.frame(DT, Controls::default());
    assert_eq!(g.paint_color(), DEFAULT_PAINT_COLOR);
    g.take_cues();

    // V goes to the simulation as `Buttons::WEAPON_MODE` (a press on a frame
    // that runs no tick is kept for the next tick): the ship's Paint Thrower
    // state turns blue and a PaintColor cue goes out.
    g.frame(0.0, press(ls_sim::Buttons::WEAPON_MODE));
    g.frame(DT, Controls::default());
    assert_eq!(g.paint_color(), ls_math::terrain::PAINT_BLUE);
    let cue = g
        .take_cues()
        .into_iter()
        .find(|c| c.kind() == ls_sim::CueKind::PaintColor)
        .expect("paint colour cue");
    assert_eq!(cue.value, ls_math::terrain::PAINT_BLUE as u16);

    // The next blob carries blue (its replicated custom bits).
    for _ in 0..4 {
        g.frame(DT, press(ls_sim::Buttons::FIRE_PRIMARY));
    }
    if let Mode::Practice(p) = &g.mode {
        let blob = p
            .world
            .projectiles()
            .iter()
            .find(|x| x.weapon == pt)
            .expect("paint blob");
        assert_eq!(
            ls_sim::weapons::specials::paint_thrower::projectile_color(blob),
            ls_math::terrain::PAINT_BLUE
        );
    }
    // Held V does not cycle again; a second press goes to yellow.
    g.frame(DT, press(ls_sim::Buttons::WEAPON_MODE));
    g.frame(DT, press(ls_sim::Buttons::WEAPON_MODE));
    assert_eq!(g.paint_color(), ls_math::terrain::PAINT_YELLOW);
    g.frame(DT, Controls::default());
    g.frame(DT, press(ls_sim::Buttons::WEAPON_MODE));
    assert_eq!(g.paint_color(), ls_math::terrain::PAINT_RED);
}

#[test]
fn online_ship_selection_reaches_the_room_and_cues_arrive() {
    let mut l = connect();
    for _ in 0..8 {
        l.tick(Controls::default());
    }
    // SelectShip: the packet the host sends reliably; the room takes it.
    let packet = l
        .core
        .select_ship(ShipType::Tug)
        .unwrap()
        .expect("online returns the SelectShip packet");
    l.room
        .handle_packet(l.id, &packet)
        .expect("room accepts it");
    let pid = l.room.player_of(l.id).unwrap();
    assert_eq!(
        l.room.world().ship(pid).unwrap().next_ship_type,
        Some(ShipType::Tug)
    );
    assert_eq!(l.core.next_ship(), Some(ShipType::Tug));
    let choices = l.core.ship_choices();
    assert!(choices[ShipType::Tug.index() as usize].next);
    assert!(choices[ShipType::Tinker.index() as usize].current);
    assert!(!choices[1].primaries.is_empty(), "loadout preview");

    // G switches the loadout Shield on: the cue reaches the client, the
    // gadget view shows it on.
    let mut cue = None;
    l.tick(press(ls_sim::Buttons::GADGET));
    for _ in 0..30 {
        l.tick(Controls::default());
        if let Some(c) = l
            .core
            .take_cues()
            .into_iter()
            .find(|c| c.kind() == ls_sim::CueKind::Gadget)
        {
            cue = Some(c);
            break;
        }
    }
    let cue = cue.expect("gadget cue from the server");
    assert_eq!(cue.source, l.id);
    for _ in 0..4 {
        l.tick(Controls::default());
    }
    let gadget = l.core.gadget();
    assert_eq!(gadget.key, "shield");
    assert!(gadget.active);
}

#[test]
fn online_decoy_cooldown_and_paint_colour_follow_the_server() {
    let mut l = connect();
    for _ in 0..8 {
        l.tick(Controls::default());
    }
    let pid = l.room.player_of(l.id).unwrap();
    l.room.world_mut().give_gadget(pid, "decoy").unwrap();
    for _ in 0..8 {
        l.tick(Controls::default());
    }
    assert_eq!(l.core.gadget().key, "decoy");
    assert!(l.core.gadget().cooldown_left <= 0.0, "ready");

    // An instant gadget: the server's USED cue starts the cooldown estimate.
    l.tick(press(ls_sim::Buttons::GADGET));
    let mut used = false;
    for _ in 0..10 {
        l.tick(Controls::default());
        used |= l.core.take_cues().iter().any(|c| {
            c.kind() == ls_sim::CueKind::Gadget && ls_sim::gadget_cue::starts_cooldown(c.value)
        });
    }
    assert!(used, "decoy use cue");
    let g = l.core.gadget();
    assert!(g.cooldown_left > 0.5, "cooling down: {}", g.cooldown_left);

    // V with the Paint Thrower selected: the server's colour comes back.
    let pt = l.core.weapons().find("paint_thrower").unwrap();
    l.tick(Controls {
        weapon_select: pt.0 + 1,
        ..Default::default()
    });
    for _ in 0..6 {
        l.tick(Controls::default());
    }
    assert_eq!(l.core.paint_color(), ls_math::terrain::PAINT_RED);
    l.tick(press(ls_sim::Buttons::WEAPON_MODE));
    assert_eq!(
        l.core.paint_color(),
        ls_math::terrain::PAINT_BLUE,
        "guessed ahead"
    );
    for _ in 0..6 {
        l.tick(Controls::default());
    }
    assert_eq!(l.core.paint_color(), ls_math::terrain::PAINT_BLUE);
    let ship = l.room.world().ship(pid).unwrap();
    assert_eq!(
        ls_sim::weapons::specials::paint_thrower::selected_color(ship, pt),
        ls_math::terrain::PAINT_BLUE,
        "the server switched too"
    );
}

/// A client that never saw the Paint Thrower's colour cues (it joined or
/// rejoined after the colour was picked, or came through a match
/// transition) shows the server's colour: it travels in the inventory's
/// weapon state byte (protocol 5).
#[test]
fn paint_colour_comes_from_the_inventory_after_a_rejoin() {
    let mut l = connect();
    for _ in 0..4 {
        l.tick(Controls::default());
    }
    let pt = l.core.weapons().find("paint_thrower").unwrap();
    // A second client joins a ship whose Paint Thrower is already yellow
    // (picked before this session existed: no cue will ever tell it).
    let hello = Message::Hello(ls_game::client_hello("rejoiner", None, None))
        .encode()
        .unwrap();
    let id2 = l.room.join_packet(&hello).unwrap();
    let out = l.room.take_outgoing();
    let welcome = out.iter().find(|o| o.client == id2).unwrap();
    let mut core2 = GameCore::online(&welcome.bytes).unwrap();
    let p2 = l.room.player_of(id2).unwrap();
    {
        let ship = l.room.world_mut().ship_mut(p2).unwrap();
        ship.primary = Some(pt);
        ship.weapon_state
            .insert(pt, ls_math::terrain::PAINT_YELLOW as u32);
    }
    assert_eq!(core2.paint_color(), 0, "no inventory yet");
    for _ in 0..8 {
        for bytes in core2.frame(DT, Controls::default()) {
            l.room.handle_packet(id2, &bytes).unwrap();
        }
        l.room.step();
        for o in l.room.take_outgoing() {
            if o.client == id2 {
                core2.handle_packet(&o.bytes).unwrap();
            }
        }
    }
    assert!(core2
        .take_cues()
        .iter()
        .all(|c| c.kind() != ls_sim::CueKind::PaintColor));
    assert_eq!(core2.paint_color(), ls_math::terrain::PAINT_YELLOW);
    let hud = core2.loadout();
    let entry = hud.primaries.iter().find(|e| e.weapon == pt).unwrap();
    assert_eq!(entry.state, ls_math::terrain::PAINT_YELLOW);
}

/// A new `Welcome` on the open connection rebuilds the online core for
/// the next match: the old level's chunks are unloaded, the new mode state
/// arrives with the snapshots, the results of the old match are cleared.
#[test]
fn online_match_transition_rebuilds_the_core_in_place() {
    let mut l = connect();
    for _ in 0..6 {
        l.tick(Controls::default());
    }
    let cam = l.core.local_pose().unwrap().pos;
    let mut loaded = Vec::new();
    for _ in 0..5 {
        loaded.push(l.core.mesh_next(cam).expect("a chunk").coord);
    }
    let old_code = l.core.share_code();
    assert_eq!(
        l.core.mode_state().kind,
        ls_sim::ModeKind::PaintTheVoid.index()
    );
    // The match ends: results arrive.
    l.room.world_mut().mode_mut().phase = ls_sim::MatchPhase::Ended { winner: None };
    l.tick(Controls::default());
    l.tick(Controls::default());
    assert!(l.core.results().is_some());
    assert!(!l.core.mode_state().is_running());

    // The next match on the same connection.
    let code = ShareCode::new(777, Sliders::new(20, 60));
    let cfg = RoomConfig {
        pregenerate: false,
        epoch: 1,
        ..RoomConfig::default()
    };
    let mut room = Room::new(MatchConfig::paint_the_void(code), cfg).unwrap();
    room.set_room_code("R-2");
    let id = room
        .join(&ls_game::client_hello("tester", None, None))
        .unwrap();
    for o in room.take_outgoing() {
        assert_eq!(l.core.handle_packet(&o.bytes).unwrap(), None);
    }
    assert!(l.core.take_match_transition());
    assert!(!l.core.take_match_transition(), "reported once");
    assert_eq!(l.core.transitions(), 1);
    assert_ne!(l.core.share_code(), old_code);
    assert_eq!(l.core.room_code(), "R-2");
    assert!(l.core.results().is_none());
    let mut unloads = l.core.take_unloads(cam);
    unloads.sort();
    loaded.sort();
    assert_eq!(unloads, loaded, "the old level's chunks go away");
    l.room = room;
    l.id = id;
    for _ in 0..30 {
        l.tick(Controls::default());
    }
    let m = l.core.mode_state();
    assert!(m.is_running());
    assert!(m.player(l.core.player()).is_some());
    assert!(l.core.debug_stats().synced);
    assert!(l.core.local_pose().is_some());
    // Lobby requests go out as packets the room queues for its host.
    let ready = l.core.ready_packet(true).unwrap();
    l.room.handle_packet(l.id, &ready).unwrap();
    let team = l.core.pick_team_packet(None).unwrap();
    l.room.handle_packet(l.id, &team).unwrap();
    assert_eq!(l.room.take_lobby_requests().len(), 2);
}

#[test]
fn practice_hits_on_ships_become_hit_cues() {
    let mut g = small_practice(1);
    let me = g.player();
    // Park the dummy right in front of the own ship and fire the laser at it.
    if let Mode::Practice(p) = &mut g.mode {
        let own = p.world.ship(p.me).unwrap();
        let ahead = own.pos + own.orientation * Vec3::new(0.0, 0.0, -4.0);
        let dummy = p.world.ships().find(|s| s.player != p.me).unwrap().player;
        let d = p.world.ship_mut(dummy).unwrap();
        d.pos = ahead;
        d.vel = Vec3::ZERO;
    }
    g.take_cues();
    let fire = Controls {
        buttons: ls_sim::Buttons::FIRE_PRIMARY.0,
        ..Default::default()
    };
    let mut hit = None;
    for _ in 0..90 {
        g.frame(DT, fire);
        if let Some(c) = g.take_cues().into_iter().find(|c| c.kind == CUE_HIT) {
            hit = Some(c);
            break;
        }
    }
    let hit = hit.expect("a hit cue");
    assert_eq!(hit.source, me);
    assert_ne!(hit.target, me);
    assert!(hit.value > 0, "damage");
    assert!(hit.pos.length() > 0.0, "impact point");
}

#[test]
fn online_hits_become_hit_cues() {
    let mut l = connect();
    for _ in 0..4 {
        l.tick(Controls::default());
    }
    l.core.take_cues();
    let me = l.core.player();
    let hits = Message::Events(vec![NetEvent::Hit {
        tick: 9,
        attacker: 5,
        victim: me,
        weapon: 0,
        damage: 12,
        pos: Vec3::new(1.0, 2.0, 3.0),
    }])
    .encode()
    .unwrap();
    assert_eq!(l.core.handle_packet(&hits).unwrap(), None);
    let cues = l.core.take_cues();
    let hit = cues.iter().find(|c| c.kind == CUE_HIT).expect("hit cue");
    assert_eq!(
        (hit.source, hit.target, hit.value, hit.tick),
        (5, me, 12, 9)
    );
    assert_eq!(hit.pos, Vec3::new(1.0, 2.0, 3.0));
    assert!(l.core.take_feed().is_empty(), "hits stay out of the feed");
}
