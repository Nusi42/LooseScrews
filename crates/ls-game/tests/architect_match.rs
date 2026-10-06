//! The Architect (M4 B4) end to end in a headless `Room` on a generated
//! level: runner bots navigate a lattice distance field to the exit, an
//! architect bot places a stroke every few seconds through the wire
//! (`ArchitectAction`, cycling through all eight kinds just ahead of the
//! leading runner), a runner forges a stroke every round, and the match is
//! played through all its rounds to a result.
//!
//! Checks: the match ends with min(players, 3) rounds and every player once
//! the architect, strokes are accepted and forged ones refused and counted,
//! no stroke has more than 32 edits, no runner ever sits in rock, and it
//! prints the cost of accepting a stroke (reshape planning) and of
//! remeshing the chunks stroke edits touch.
//!
//! Release only (whole-level generation and 4 minutes of play):
//! `cargo test -p ls-game --release --test architect_match -- --nocapture`.

#![cfg(all(feature = "server", not(target_arch = "wasm32")))]

use std::collections::BTreeSet;
use std::time::Instant;

use ls_game::convert::input_to_net;
use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_math::{Aabb, Vec3};
use ls_game::ls_net::{ArchitectActionMsg, InputMsg, Message};
use ls_game::ls_sim::mode::player_flags;
use ls_game::ls_sim::modes::the_architect::{StrokeKind, EXIT_ID, MAX_STROKE_EDITS};
use ls_game::ls_sim::{Buttons, Event, InputCmd, ModeKind, PlayerId, RuleSet, World};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::{GameTerrain, MatchConfig};

const PLAYERS: usize = 4;
/// Lattice spacing and clearance of the runners' navigation (u).
const STEP: f32 = 3.0;
const CLEAR: f32 = 1.6;

/// A lattice over the level: open cells (`sdf >= CLEAR`) and the
/// breadth-first distance (in cells, 26-connected) to the exit.
struct Nav {
    min: Vec3,
    n: [usize; 3],
    open: Vec<bool>,
    dist: Vec<u32>,
}

impl Nav {
    fn new(t: &GameTerrain) -> Nav {
        let b = t.bounds();
        let size = b.max - b.min;
        let n = [
            (size.x / STEP) as usize,
            (size.y / STEP) as usize,
            (size.z / STEP) as usize,
        ];
        let mut nav = Nav {
            min: b.min,
            n,
            open: vec![false; n[0] * n[1] * n[2]],
            dist: Vec::new(),
        };
        nav.refresh(t, b);
        nav
    }

    fn centre(&self, c: [usize; 3]) -> Vec3 {
        self.min
            + Vec3::new(
                (c[0] as f32 + 0.5) * STEP,
                (c[1] as f32 + 0.5) * STEP,
                (c[2] as f32 + 0.5) * STEP,
            )
    }

    fn index(&self, c: [usize; 3]) -> usize {
        (c[2] * self.n[1] + c[1]) * self.n[0] + c[0]
    }

    fn cell(&self, p: Vec3) -> Option<[usize; 3]> {
        let q = (p - self.min) / STEP;
        let ok = |v: f32, n: usize| v >= 0.0 && (v as usize) < n;
        (ok(q.x, self.n[0]) && ok(q.y, self.n[1]) && ok(q.z, self.n[2])).then_some([
            q.x as usize,
            q.y as usize,
            q.z as usize,
        ])
    }

    /// Re-samples the cells in `region` (terrain edits).
    fn refresh(&mut self, t: &GameTerrain, region: Aabb) {
        let lo = self.cell(region.min.max(self.min)).unwrap_or([0, 0, 0]);
        let hi = self
            .cell(region.max.min(self.min + Vec3::splat(1e6)))
            .unwrap_or([self.n[0] - 1, self.n[1] - 1, self.n[2] - 1]);
        for z in lo[2]..=hi[2].min(self.n[2] - 1) {
            for y in lo[1]..=hi[1].min(self.n[1] - 1) {
                for x in lo[0]..=hi[0].min(self.n[0] - 1) {
                    let c = [x, y, z];
                    let i = self.index(c);
                    self.open[i] = t.sdf(self.centre(c)) >= CLEAR;
                }
            }
        }
    }

    /// Breadth-first distances from the open cells near `goal`.
    fn flood(&mut self, goal: Vec3) {
        let total = self.open.len();
        self.dist = vec![u32::MAX; total];
        let mut queue = std::collections::VecDeque::new();
        if let Some(g) = self.cell(goal) {
            for dz in -1i32..=1 {
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        if let Some(c) = self.offset(g, dx, dy, dz) {
                            let i = self.index(c);
                            if self.open[i] {
                                self.dist[i] = 0;
                                queue.push_back(c);
                            }
                        }
                    }
                }
            }
        }
        while let Some(c) = queue.pop_front() {
            let d = self.dist[self.index(c)];
            for dz in -1i32..=1 {
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let Some(nb) = self.offset(c, dx, dy, dz) else {
                            continue;
                        };
                        let i = self.index(nb);
                        if self.open[i] && self.dist[i] == u32::MAX {
                            self.dist[i] = d + 1;
                            queue.push_back(nb);
                        }
                    }
                }
            }
        }
    }

    fn offset(&self, c: [usize; 3], dx: i32, dy: i32, dz: i32) -> Option<[usize; 3]> {
        let f = |v: usize, d: i32, n: usize| {
            let r = v as i32 + d;
            (r >= 0 && (r as usize) < n).then_some(r as usize)
        };
        Some([
            f(c[0], dx, self.n[0])?,
            f(c[1], dy, self.n[1])?,
            f(c[2], dz, self.n[2])?,
        ])
    }

    /// The next waypoint from `p` downhill (two cells ahead).
    fn next(&self, p: Vec3) -> Option<Vec3> {
        let mut c = self.cell(p)?;
        // Off the lattice's open cells: the nearest reachable neighbour.
        for _ in 0..2 {
            let mut best: Option<([usize; 3], u32)> = None;
            for dz in -1i32..=1 {
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        if let Some(nb) = self.offset(c, dx, dy, dz) {
                            let d = self.dist[self.index(nb)];
                            if d != u32::MAX && best.is_none_or(|(_, b)| d < b) {
                                best = Some((nb, d));
                            }
                        }
                    }
                }
            }
            c = best?.0;
        }
        Some(self.centre(c))
    }
}

fn steer(w: &World, me: PlayerId, target: Vec3, tick: u64) -> InputCmd {
    let mut cmd = InputCmd::idle(tick);
    let Some(ship) = w.ship(me).filter(|s| s.alive) else {
        return cmd;
    };
    let local = ship.orientation.conjugate().rotate(target - ship.pos);
    cmd.look_pitch = ls_game::ls_math::det::atan2(local.y, -local.z).clamp(-0.1, 0.1);
    cmd.look_yaw = ls_game::ls_math::det::atan2(-local.x, -local.z).clamp(-0.1, 0.1);
    // Slow down near the waypoint, thrust along the way otherwise.
    let want = local.normalize_or_zero() * 14.0;
    let vel = ship.orientation.conjugate().rotate(ship.vel);
    cmd.thrust = ((want - vel) * 0.25).clamp(Vec3::splat(-1.0), Vec3::splat(1.0));
    let mut b = Buttons::HORIZON_ASSIST;
    b.set(Buttons::FIRE_PRIMARY, tick % 7 == 0);
    cmd.buttons = b;
    cmd
}

struct Played {
    ended: bool,
    rounds: usize,
    architects: Vec<Option<PlayerId>>,
    finished: usize,
    accepted: u64,
    rejected: u64,
    kinds: BTreeSet<u8>,
    stroke_step_ms: Vec<f64>,
    plain_step_ms: Vec<f64>,
    remesh_ms: Vec<f64>,
    max_chunks_per_tick: usize,
    max_edits: usize,
    in_rock: usize,
}

fn play(round_secs: f32, seed: u64) -> Played {
    let code = ShareCode::new(seed, Sliders::new(40, 30));
    let mut cfg = MatchConfig::paint_the_void(code);
    let mut rules = RuleSet::default_for(ModeKind::TheArchitect);
    rules.mode.set_field("round_secs", round_secs);
    cfg.rules = rules;
    let mut room = Room::new(
        cfg,
        RoomConfig {
            pregenerate: true,
            ..RoomConfig::default()
        },
    )
    .unwrap();
    let clients: Vec<u8> = (0..PLAYERS)
        .map(|i| {
            room.join(&ls_game::client_hello(format!("bot{i}"), None, None))
                .unwrap()
        })
        .collect();
    let epoch = room.epoch();
    // A client-side copy of the level for meshing (the renderer's work).
    let mut mesh_terrain = GameTerrain::new(room.terrain().generator());
    mesh_terrain.generate_all();
    for c in mesh_terrain.chunk_coords() {
        let _ = mesh_terrain.mesh_chunk(c);
    }
    mesh_terrain.take_dirty_chunks();
    let mut nav = Nav::new(room.terrain());
    room.step();
    let exit = room.world().mode().objective(EXIT_ID).unwrap().pos;
    nav.flood(exit);
    let mut out = Played {
        ended: false,
        rounds: 0,
        architects: Vec::new(),
        finished: 0,
        accepted: 0,
        rejected: 0,
        kinds: BTreeSet::new(),
        stroke_step_ms: Vec::new(),
        plain_step_ms: Vec::new(),
        remesh_ms: Vec::new(),
        max_chunks_per_tick: 0,
        max_edits: 0,
        in_rock: 0,
    };
    let mut k = 0usize;
    let mut forged_round = u8::MAX;
    let mut dirty_nav: Option<Aabb> = None;
    for _ in 0..(round_secs as u64 + 20) * 60 * 3 {
        let tick = room.tick() + 1;
        let w = room.world();
        let state = w.mode().architect.as_deref().cloned();
        let Some(state) = state else { break };
        // The architect: a stroke every 3 s ahead of the leading runner.
        if let Some(arch) = state.architect.filter(|_| tick % 180 == 0) {
            let lead = state
                .runners
                .iter()
                .filter_map(|r| w.ship(*r).filter(|s| s.alive))
                .min_by(|a, b| {
                    a.pos
                        .distance_squared(exit)
                        .total_cmp(&b.pos.distance_squared(exit))
                })
                .map(|s| s.pos);
            if let Some(lead) = lead {
                let kind = StrokeKind::ALL[k % StrokeKind::ALL.len()];
                k += 1;
                let ahead = lead + (exit - lead).normalize_or_zero() * 30.0;
                let c = clients
                    .iter()
                    .copied()
                    .find(|c| room.player_of(*c) == Some(arch))
                    .unwrap();
                let msg = Message::ArchitectAction(ArchitectActionMsg {
                    stroke: kind.index(),
                    center: ahead,
                    radius: 16.0,
                });
                room.handle_packet(c, &msg.encode().unwrap()).unwrap();
                out.kinds.insert(kind.index());
            }
        }
        // A runner forges a stroke once a round.
        if forged_round != state.round {
            if let Some(r) = state.runners.iter().next() {
                let c = clients
                    .iter()
                    .copied()
                    .find(|c| room.player_of(*c) == Some(*r))
                    .unwrap();
                room.handle_message(
                    c,
                    Message::ArchitectAction(ArchitectActionMsg {
                        stroke: 0,
                        center: exit,
                        radius: 12.0,
                    }),
                )
                .unwrap();
                forged_round = state.round;
            }
        }
        // Runners.
        let cmds: Vec<(u8, InputCmd)> = clients
            .iter()
            .map(|&c| {
                let w = room.world();
                let p = room.player_of(c).unwrap();
                let cmd = match w.ship(p).filter(|s| s.alive) {
                    Some(s) => {
                        let target = if s.pos.distance(exit) < 8.0 {
                            exit
                        } else {
                            nav.next(s.pos).unwrap_or(exit)
                        };
                        steer(w, p, target, tick)
                    }
                    None => InputCmd::idle(tick),
                };
                (c, cmd)
            })
            .collect();
        for (c, cmd) in cmds {
            room.handle_message(
                c,
                Message::Input(InputMsg {
                    epoch,
                    ack_snapshot: None,
                    cmds: vec![input_to_net(&cmd)],
                }),
            )
            .unwrap();
        }
        let accepted_before = room.stats().architect_strokes;
        let t0 = Instant::now();
        let report = room.step();
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        if room.stats().architect_strokes > accepted_before {
            out.stroke_step_ms.push(ms);
        } else {
            out.plain_step_ms.push(ms);
        }
        room.take_outgoing();
        // Edits: the client's remesh cost, the runners' lattice.
        for e in &report.events {
            if let Event::TerrainEdit(edit) = e {
                mesh_terrain.apply(*edit);
                let b = edit.bounds();
                dirty_nav = Some(match dirty_nav {
                    Some(d) => Aabb::new(d.min.min(b.min), d.max.max(b.max)),
                    None => b,
                });
            }
        }
        let dirty = mesh_terrain.take_dirty_chunks();
        out.max_chunks_per_tick = out.max_chunks_per_tick.max(dirty.len());
        for c in dirty {
            let t0 = Instant::now();
            let _ = mesh_terrain.mesh_chunk(c);
            out.remesh_ms.push(t0.elapsed().as_secs_f64() * 1e3);
        }
        if let Some(d) = dirty_nav.filter(|_| tick % 60 == 0) {
            nav.refresh(
                room.terrain(),
                Aabb::new(d.min - Vec3::splat(3.0), d.max + Vec3::splat(3.0)),
            );
            nav.flood(exit);
            dirty_nav = None;
        }
        let w = room.world();
        if let Some(a) = w.mode().architect.as_deref() {
            for s in &a.strokes {
                out.max_edits = out.max_edits.max(s.edits.len());
            }
            out.finished = out
                .finished
                .max(a.history.iter().map(|r| r.finished as usize).sum());
            for r in &a.runners {
                if let Some(s) = w.ship(*r).filter(|s| s.alive) {
                    if room.terrain().sdf(s.pos) < -0.25 {
                        out.in_rock += 1;
                    }
                }
            }
        }
        if room.results().is_some() {
            break;
        }
    }
    let w = room.world();
    let a = w.mode().architect.as_deref().unwrap();
    out.ended = !w.mode().is_running() && room.results().is_some();
    out.rounds = a.history.len();
    out.architects = a.history.iter().map(|r| r.architect).collect();
    out.finished = a.history.iter().map(|r| r.finished as usize).sum();
    out.accepted = room.stats().architect_strokes;
    out.rejected = room.stats().rejected_architect_actions;
    let _ = player_flags::ARCHITECT;
    out
}

fn stats(v: &mut [f64]) -> (f64, f64, f64) {
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let q = |f: f64| v[((v.len() - 1) as f64 * f) as usize];
    (q(0.5), q(0.99), v[v.len() - 1])
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "whole-level generation and 4 minutes of play; run with --release"
)]
fn architect_match_with_bots_reaches_a_result() {
    let mut p = play(75.0, 0x00b4_0001);
    let (s50, s99, smax) = stats(&mut p.stroke_step_ms);
    let (t50, t99, tmax) = stats(&mut p.plain_step_ms);
    let (m50, m99, mmax) = stats(&mut p.remesh_ms);
    println!(
        "rounds {} architects {:?} finished {} strokes accepted {} refused {} kinds {:?} max edits {}",
        p.rounds, p.architects, p.finished, p.accepted, p.rejected, p.kinds, p.max_edits
    );
    println!("room step with a stroke accepted: p50 {s50:.2} ms, p99 {s99:.2} ms, max {smax:.2} ms ({} steps)", p.stroke_step_ms.len());
    println!("room step otherwise: p50 {t50:.2} ms, p99 {t99:.2} ms, max {tmax:.2} ms");
    println!("client remesh per chunk an edit touched: p50 {m50:.2} ms, p99 {m99:.2} ms, max {mmax:.2} ms ({} chunks, at most {} in one tick)", p.remesh_ms.len(), p.max_chunks_per_tick);
    assert!(p.ended, "the match reached a result");
    assert_eq!(p.rounds, 3);
    let distinct: BTreeSet<_> = p.architects.iter().collect();
    assert_eq!(distinct.len(), 3, "the role rotates: {:?}", p.architects);
    assert!(p.accepted >= 6, "strokes accepted: {}", p.accepted);
    assert!(p.rejected >= 3, "forged strokes refused: {}", p.rejected);
    assert!(p.max_edits <= MAX_STROKE_EDITS);
    assert_eq!(p.in_rock, 0, "a runner sat in rock");
    assert!(p.finished >= 1, "nobody reached the exit");
}
