//! The world side of The Architect (M4; see `crate::modes::the_architect`):
//! validating and accepting strokes ([`World::architect_action`]) and, in
//! stage 7 of every tick, their telegraphs and effects and the change to
//! the next round (respawns).

use ls_math::terrain::TerrainEdit;
use ls_math::{Pcg32, Vec3};

use super::{SimConfig, World};
use crate::behavior::{Command, Cue, CueKind, DamageSource};
use crate::entity::EntitySpawn;
use crate::events::{EventKind, EventRef, EventTable};
use crate::gravity::{DynamicWell, GravitySettings};
use crate::instability::{escapes, plan_cave_in, InstabilitySettings, RubbleSphere};
use crate::mode::{player_flags, ModeKind};
use crate::modes::the_architect::{
    begin_round, plan_reshape, rules, RoundPhase, Stroke, StrokeKind, StrokeReject,
    STROKE_CAVE_IN_RNG_KEY, STROKE_EDIT_TICKS, STROKE_EVENT_RNG_KEY, STROKE_SHIP_CLEARANCE,
    WELL_KEY_BASE,
};
use crate::ship::PlayerId;
use crate::terrain::SimTerrain;
use crate::Event;

/// A private RNG for `key` at tick `now` (like `Ctx::rng`).
fn rng_at(seed: u64, now: u64, key: u64) -> Pcg32 {
    let s = ls_math::rng::mix64(seed ^ ls_math::rng::mix64(key ^ now.rotate_left(32)));
    Pcg32::new(s, crate::behavior::RNG_STREAM_MODULES)
}

impl World {
    /// An Architect stroke request (`ArchitectAction`, protocol 6) from
    /// `player`, checked against the round, role, kind, radius, range,
    /// cooldown, strokes in flight and budget (see
    /// `crate::modes::the_architect`). An accepted stroke is paid, planned
    /// (rock strokes sample `terrain` now) and telegraphed from the next
    /// tick on; returns its id. Call it between ticks, in a fixed order
    /// (the server: in arrival order, before the tick's step).
    pub fn architect_action<T: SimTerrain + ?Sized>(
        &mut self,
        terrain: &T,
        player: PlayerId,
        stroke: u8,
        center: Vec3,
        radius: f32,
    ) -> Result<u8, StrokeReject> {
        let cfg = std::sync::Arc::clone(&self.config);
        let r = rules(&cfg.rules.mode);
        let now = self.tick;
        if self.mode.kind != ModeKind::TheArchitect || !self.mode.is_running() {
            return Err(StrokeReject::NotArchitectMode);
        }
        let Some(a) = self.mode.architect.as_deref() else {
            return Err(StrokeReject::NotArchitectMode);
        };
        if a.architect != Some(player) {
            return Err(StrokeReject::NotArchitect);
        }
        if a.phase != RoundPhase::Playing {
            return Err(StrokeReject::RoundOver);
        }
        let kind = StrokeKind::from_index(stroke).ok_or(StrokeReject::UnknownStroke)?;
        if !(radius.is_finite() && r.min_radius <= radius && radius <= r.max_radius) {
            return Err(StrokeReject::Radius);
        }
        // Range: inside the level; rock strokes clear of the exit and the
        // runners' spawn points.
        let b = terrain.bounds();
        let inside = center.is_finite()
            && (b.min.x..=b.max.x).contains(&center.x)
            && (b.min.y..=b.max.y).contains(&center.y)
            && (b.min.z..=b.max.z).contains(&center.z);
        if !inside {
            return Err(StrokeReject::OutOfRange);
        }
        if kind.changes_rock() {
            let clear = r.keep_clear + radius;
            let exit = self
                .mode
                .objective(crate::modes::the_architect::EXIT_ID)
                .map(|o| (o.pos, o.radius));
            let near_exit = exit.is_some_and(|(p, er)| p.distance(center) < clear + er);
            let near_spawn = cfg
                .setup
                .spawns
                .iter()
                .any(|s| s.pos.distance(center) < clear);
            if near_exit || near_spawn {
                return Err(StrokeReject::OutOfRange);
            }
        }
        if now < a.next_stroke_tick {
            return Err(StrokeReject::Cooldown);
        }
        if a.strokes.len() >= r.max_strokes as usize {
            return Err(StrokeReject::TooManyStrokes);
        }
        let cost = kind.cost(&r) * crate::modes::the_architect::BUDGET_UNIT;
        if a.budget < cost {
            return Err(StrokeReject::Budget);
        }
        if kind == StrokeKind::Verticality && a.verticality_used {
            return Err(StrokeReject::OncePerRound);
        }
        let telegraph = ls_math::Tick::ticks_for_secs(r.telegraph_secs).max(1);
        let apply_tick = now + telegraph;
        let id = a.next_stroke_id;
        // Plan (and refuse a stroke that would do nothing).
        let mut edits = Vec::new();
        let mut event = None;
        let end_tick = match kind {
            StrokeKind::EnclosurePlus | StrokeKind::EnclosureMinus | StrokeKind::DensityPlus => {
                let (slider, sign) = kind.reshape().expect("reshape stroke");
                let delta = (r.reshape_delta.min(100) as i8) * sign;
                edits = plan_reshape(terrain, slider, delta, center, radius);
                if edits.is_empty() {
                    return Err(StrokeReject::NothingToChange);
                }
                apply_tick + STROKE_EDIT_TICKS
            }
            StrokeKind::InstabilityPlus => {
                let mut rng = rng_at(cfg.seed, now, STROKE_CAVE_IN_RNG_KEY);
                let s = InstabilitySettings::builtin();
                if plan_cave_in(s, terrain, self.up, center, &mut rng, &[]).is_none() {
                    return Err(StrokeReject::NothingToChange);
                }
                apply_tick + STROKE_EDIT_TICKS
            }
            StrokeKind::GravityChaosPlus => apply_tick + ls_math::Tick::ticks_for_secs(r.well_secs),
            StrokeKind::Palette => apply_tick + ls_math::Tick::ticks_for_secs(r.palette_secs),
            StrokeKind::AbsurdityPlus | StrokeKind::Verticality => {
                if cfg.rules.absurdity == 0 {
                    return Err(StrokeReject::EventsOff);
                }
                if self.director.current.is_some() || !self.director.schedule.is_empty() {
                    return Err(StrokeReject::EventBusy);
                }
                let kind = if kind == StrokeKind::Verticality {
                    let k = EventKind::BoredLevel;
                    (cfg.banned_events & (1 << k.index()) == 0).then_some(k)
                } else {
                    self.stroke_event(&cfg, terrain, now)
                };
                event = Some(kind.ok_or(StrokeReject::NothingToChange)?);
                apply_tick + ls_math::TICK_HZ as u64
            }
        };
        // Accepted.
        if let Some(e) = event {
            self.director.schedule(apply_tick, e);
        }
        if kind == StrokeKind::GravityChaosPlus {
            let settings = GravitySettings::builtin();
            self.add_well(DynamicWell {
                key: WELL_KEY_BASE | id as u32,
                pos: center,
                peak_accel: settings.max_peak_accel,
                // On the wire's 0.25 u grid (`ls_net::well_state`), so
                // predicting clients pull exactly like the server.
                range: ls_math::det::round((1.5 * radius).clamp(20.0, 40.0) * 4.0) / 4.0,
                start_tick: apply_tick,
                end_tick,
                owner: Some(player),
            });
        }
        let cooldown = ls_math::Tick::ticks_for_secs(r.stroke_cooldown_secs);
        let a = self.mode.architect.as_deref_mut().expect("checked above");
        a.budget -= cost;
        a.next_stroke_tick = now + cooldown;
        a.next_stroke_id = id.wrapping_add(1);
        if kind == StrokeKind::Verticality {
            a.verticality_used = true;
        }
        a.strokes.push(Stroke {
            id,
            kind,
            center,
            radius,
            architect: player,
            apply_tick,
            end_tick,
            announced: false,
            planned: false,
            active: false,
            edits,
        });
        let budget = a.budget_points().min(255) as u8;
        if let Some(p) = self.mode.player_mut(player) {
            p.meter = budget;
        }
        Ok(id)
    }

    /// The Absurdity+ stroke's event: a weighted pick among the enabled,
    /// not banned events the mutators allow and whose needs are met now
    /// (the mode's veto of director rolls does not apply).
    fn stroke_event<T: SimTerrain + ?Sized>(
        &self,
        cfg: &SimConfig,
        terrain: &T,
        now: u64,
    ) -> Option<EventKind> {
        let leader = cfg.mode_module.leader(&self.mode_ref(cfg));
        let mood = self.director.mood;
        let (cands, _) = self.collect_hook(cfg, terrain, |ctx| {
            let mut out: Vec<(EventKind, f32)> = Vec::new();
            for (kind, d) in EventTable::builtin().iter() {
                if !d.enabled || d.weight <= 0.0 || cfg.banned_events & (1 << kind.index()) != 0 {
                    continue;
                }
                if !cfg.mutator_modules.iter().all(|m| m.allows_event(kind)) {
                    continue;
                }
                let r = EventRef {
                    view: &ctx.view,
                    mode: &self.mode,
                    setup: &cfg.setup,
                    def: d,
                    mood,
                    leader,
                };
                if kind.module().eligible(&r) {
                    out.push((kind, d.weight));
                }
            }
            out
        });
        let weights: Vec<f32> = cands.iter().map(|(_, w)| *w).collect();
        let mut rng = rng_at(cfg.seed, now, STROKE_EVENT_RNG_KEY);
        rng.weighted_index(&weights).map(|i| cands[i].0)
    }

    /// Stage 7 for The Architect: the next round when the results are
    /// over, then every stroke's telegraph and effect.
    pub(super) fn step_architect<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        now: u64,
        events: &mut Vec<Event>,
    ) {
        let next_round = self.mode.architect.as_deref().is_some_and(|a| {
            a.phase == RoundPhase::Over && a.round + 1 < a.rounds && now >= self.mode.stage_end_tick
        });
        if next_round {
            self.next_round(cfg, now, events);
        }
        let Some(mut a) = self.mode.architect.take() else {
            return;
        };
        let mut strokes = std::mem::take(&mut a.strokes);
        self.mode.architect = Some(a);
        let settings = InstabilitySettings::builtin();
        for s in &mut strokes {
            if !s.announced {
                s.announced = true;
                let value = (s.kind.index() as u16) << 8 | (s.radius.min(255.0) as u16);
                events.push(Event::Cue(Cue {
                    kind: CueKind::StrokeTelegraph,
                    source: Some(s.architect),
                    target: None,
                    pos: s.center,
                    value,
                }));
                if s.kind == StrokeKind::InstabilityPlus {
                    let warn = s.apply_tick.saturating_sub(now).max(1);
                    events.push(Event::Cue(Cue {
                        kind: CueKind::CaveInWarning,
                        source: None,
                        target: None,
                        pos: s.center,
                        value: warn.min(u16::MAX as u64) as u16,
                    }));
                }
            }
            if now < s.apply_tick {
                continue;
            }
            s.active = true;
            if s.kind == StrokeKind::InstabilityPlus && !s.planned {
                s.planned = true;
                self.plan_stroke_cave_in(cfg, terrain, now, s, settings, events);
            }
            let batch = s.batch(now).to_vec();
            if !batch.is_empty() {
                self.apply_stroke_batch(cfg, terrain, &batch, settings, events);
            }
        }
        strokes.retain(|s| now < s.end_tick);
        if let Some(a) = self.mode.architect.as_deref_mut() {
            // Strokes accepted meanwhile (none: actions come between ticks)
            // stay after the older ones.
            let newer = std::mem::take(&mut a.strokes);
            a.strokes = strokes;
            a.strokes.extend(newer);
        }
    }

    /// The next round: roles, lives and budget (see
    /// [`begin_round`]), the old round's wells gone, every runner respawned
    /// at the start and the new architect's ship out of play.
    fn next_round(&mut self, cfg: &SimConfig, now: u64, events: &mut Vec<Event>) {
        let r = rules(&cfg.rules.mode);
        if let Some(a) = self.mode.architect.as_deref_mut() {
            a.round += 1;
            for s in &a.strokes {
                if s.kind == StrokeKind::GravityChaosPlus {
                    self.wells
                        .dynamics
                        .retain(|w| w.key != WELL_KEY_BASE | s.id as u32);
                }
            }
        }
        begin_round(&mut self.mode, &r, now);
        let ids: Vec<PlayerId> = self.mode.players.keys().copied().collect();
        for id in ids {
            let flags = self.mode.player(id).map_or(0, |p| p.flags);
            if flags & player_flags::ARCHITECT != 0 {
                self.retire(id);
            } else if flags & player_flags::SPECTATOR == 0 {
                if let Some(pos) = self.respawn(id) {
                    events.push(Event::Respawned { player: id, pos });
                }
            }
        }
    }

    /// Instability+ at its telegraph's end: a cave-in planned now around the
    /// living ships (rubble that would overlap or seal one in is left out);
    /// its hollow and rubble become the stroke's edits, the rubble is weak
    /// rock, debris falls and ships next to the rubble take damage.
    fn plan_stroke_cave_in<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        now: u64,
        s: &mut Stroke,
        settings: &InstabilitySettings,
        events: &mut Vec<Event>,
    ) {
        let ships: Vec<(Vec3, f32)> = self
            .ships
            .values()
            .filter(|s| s.alive)
            .map(|s| (s.pos, cfg.tuning(s.ship_type).radius))
            .collect();
        let mut rng = rng_at(cfg.seed, now, STROKE_CAVE_IN_RNG_KEY ^ s.id as u64);
        let Some(plan) = plan_cave_in(settings, &*terrain, self.up, s.center, &mut rng, &ships)
        else {
            return;
        };
        if let Some(h) = plan.hollow {
            s.edits.push(TerrainEdit::SubtractSphere {
                center: h.center,
                radius: h.radius,
            });
        }
        for rb in &plan.rubble {
            s.edits.push(TerrainEdit::AddSphere {
                center: rb.center,
                radius: rb.radius,
            });
        }
        s.edits
            .truncate(crate::modes::the_architect::MAX_STROKE_EDITS);
        self.instability.add_rubble(settings, &plan.rubble);
        let mut cmds = Vec::new();
        for &(p, v, rad, scrap) in &plan.debris {
            cmds.push(Command::SpawnEntity(Box::new(EntitySpawn::debris(
                p, v, rad, scrap,
            ))));
        }
        for sh in self.ships.values().filter(|s| s.alive) {
            let near = plan.rubble.iter().find(|r| {
                sh.pos.distance(r.center) - r.radius - cfg.tuning(sh.ship_type).radius
                    < settings.damage_radius
            });
            if let Some(r) = near {
                let push = (sh.pos - r.center).normalize_or_zero() * 4.0;
                cmds.push(Command::Damage {
                    target: sh.player,
                    amount: settings.damage,
                    knockback: push,
                    source: DamageSource::ENVIRONMENT,
                    direct: false,
                });
            }
        }
        self.apply_commands(cfg, terrain, cmds, events);
    }

    /// Applies one tick's share of a stroke's edits: an `AddSphere` that
    /// would overlap a living ship is left out, and so is, for a ship near
    /// the batch that could reach open space `escape_reach` away before,
    /// the sphere nearest to it until it still can.
    fn apply_stroke_batch<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        batch: &[TerrainEdit],
        settings: &InstabilitySettings,
        events: &mut Vec<Event>,
    ) {
        let ships: Vec<(Vec3, f32)> = self
            .ships
            .values()
            .filter(|s| s.alive)
            .map(|s| (s.pos, cfg.tuning(s.ship_type).radius))
            .collect();
        let mut adds: Vec<RubbleSphere> = Vec::new();
        let mut cmds = Vec::new();
        for e in batch {
            match *e {
                TerrainEdit::AddSphere { center, radius } => {
                    let clear = ships.iter().all(|&(p, sr)| {
                        let min = radius + sr + STROKE_SHIP_CLEARANCE;
                        p.distance_squared(center) >= min * min
                    });
                    if clear {
                        adds.push(RubbleSphere { center, radius });
                    }
                }
                other => cmds.push(Command::Edit(other)),
            }
        }
        let reach = settings.escape_reach;
        for &(p, sr) in &ships {
            let near = adds.iter().any(|s| p.distance(s.center) - s.radius < reach);
            if !near || escapes(&*terrain, &adds, p, sr, reach) {
                continue;
            }
            if !escapes(&*terrain, &[], p, sr, reach) {
                continue;
            }
            while !adds.is_empty() && !escapes(&*terrain, &adds, p, sr, reach) {
                let (i, _) = adds
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (i, s.center.distance_squared(p)))
                    .fold(
                        (0, f32::INFINITY),
                        |best, x| if x.1 < best.1 { x } else { best },
                    );
                adds.remove(i);
            }
        }
        cmds.extend(adds.iter().map(|s| {
            Command::Edit(TerrainEdit::AddSphere {
                center: s.center,
                radius: s.radius,
            })
        }));
        // Stroke edits add no strain (Instability+ is the cave-in stroke).
        self.no_strain = true;
        self.apply_commands(cfg, terrain, cmds, events);
        self.no_strain = false;
    }
}
