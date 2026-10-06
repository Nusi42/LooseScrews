//! The Architect (M4 package C5; `docs/milestone-4-plan.md` §3.5): runner
//! and architect bots.
//!
//! - **Runners** fly a navigation route to the exit at the team 1 end,
//!   boosting where the route is wide. A stroke in its telegraph phase that
//!   changes rock (Enclosure+, Density+, Instability+) or pulls (Gravity
//!   Chaos+) is a danger zone they leave or route around. When the way does
//!   not get shorter for a few seconds (rubble, grown rock), they blast the
//!   rock toward the exit with crater ordnance and the laser. Runners never
//!   fight each other (they cannot hurt each other); event props are fair
//!   game.
//! - **The architect** has no ship. Every few seconds, budget permitting,
//!   it places a stroke in front of the runner nearest the exit, between it
//!   and the exit: mostly rock (Enclosure+, Density+), sometimes a well, a
//!   cave-in, an ink blackout, a random event (Absurdity+, when rich) or
//!   once a round the Bored level. The server validates every stroke; a
//!   refused one only costs the bot its cooldown.

use ls_game::ls_net::NetModeState;
use ls_game::ls_sim::mode::objective_kind;
use ls_game::ls_sim::modes::the_architect::{exit_state, stroke_phase, StrokeKind};
use ls_game::ls_sim::ruleset::TheArchitectRules;

use super::*;

/// Seconds without getting closer to the exit before a runner digs.
const RUNNER_STUCK_SECS: u64 = 5;

/// A stroke in the level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeSense {
    pub pos: Vec3,
    pub radius: f32,
    pub kind: Option<StrokeKind>,
    /// `stroke_phase`.
    pub phase: u8,
    pub phase_end: u64,
}

/// The Architect's objectives.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArchitectSense {
    /// The exit gate (centre, radius).
    pub exit: Option<(Vec3, f32)>,
    /// The round is over (results between rounds).
    pub round_over: bool,
    /// This player is the round's architect.
    pub me_architect: bool,
    /// Stroke budget (the architect's meter).
    pub budget: u32,
    pub strokes: Vec<StrokeSense>,
}

impl ArchitectSense {
    pub fn from_mode(m: &NetModeState, me: u8) -> ArchitectSense {
        let mut s = ArchitectSense::default();
        if let Some(own) = m.player(me) {
            s.me_architect = own.flags & ls_game::ls_net::mode_player_flags::ARCHITECT != 0;
            s.budget = u32::from(own.meter);
        }
        for o in &m.objectives {
            match o.kind {
                objective_kind::EXIT => {
                    s.exit = Some((o.pos, o.radius.max(2.0)));
                    s.round_over = o.state & exit_state::ROUND_OVER != 0;
                }
                objective_kind::STROKE => s.strokes.push(StrokeSense {
                    pos: o.pos,
                    radius: o.radius,
                    kind: StrokeKind::from_index(o.state & 0x0f),
                    phase: o.state >> 4,
                    phase_end: u64::from(o.timer_end),
                }),
                _ => {}
            }
        }
        s
    }
}

/// The architect bot's state.
#[derive(Debug, Clone, Default)]
pub(super) struct ArchitectBrain {
    pub rules: Option<TheArchitectRules>,
    pub next_stroke: u64,
    pub verticality_used: bool,
    /// A stroke to send (kind, centre, radius).
    pub outbox: Option<(u8, Vec3, f32)>,
    /// Runner: closest distance to the exit so far and when.
    pub best_dist: f32,
    pub best_tick: u64,
}

impl Brain {
    /// Sets The Architect's rules (stroke costs, radii, cooldown).
    pub fn set_architect_rules(&mut self, rules: TheArchitectRules) {
        self.architect.rules = Some(rules);
    }

    /// The stroke the architect bot wants to place (kind index, centre,
    /// radius), taken once.
    pub fn take_architect_action(&mut self) -> Option<(u8, Vec3, f32)> {
        self.architect.outbox.take()
    }

    /// The architect (no ship): places strokes. True when this bot is the
    /// round's architect.
    pub(super) fn plan_architect(&mut self, p: &Perception, counters: &mut BrainCounters) -> bool {
        let Some(a) = p.mode.architect.as_ref() else {
            return false;
        };
        if !a.me_architect {
            return false;
        }
        counters.moods.architect_ticks += 1;
        let Some(rules) = self.architect.rules else {
            return true;
        };
        let Some((exit, _)) = a.exit else {
            return true;
        };
        if a.round_over || !p.mode.running || p.tick < self.architect.next_stroke {
            return true;
        }
        let in_flight = a
            .strokes
            .iter()
            .filter(|s| s.phase == stroke_phase::TELEGRAPH || s.phase == stroke_phase::ACTIVE)
            .count();
        if in_flight >= rules.max_strokes as usize {
            return true;
        }
        // The runner nearest the exit.
        let Some(runner) = p.contacts.iter().min_by(|x, y| {
            x.pos
                .distance_squared(exit)
                .total_cmp(&y.pos.distance_squared(exit))
        }) else {
            return true;
        };
        let cost = |k: StrokeKind| k.cost(&rules);
        let mut options: Vec<(StrokeKind, f32)> = vec![
            (StrokeKind::EnclosurePlus, 3.0),
            (StrokeKind::DensityPlus, 2.0),
            (StrokeKind::GravityChaosPlus, 2.0),
            (StrokeKind::InstabilityPlus, 1.5),
            (StrokeKind::Palette, 0.7),
            (StrokeKind::EnclosureMinus, 0.3),
        ];
        if a.budget >= cost(StrokeKind::AbsurdityPlus) + 40 {
            options.push((StrokeKind::AbsurdityPlus, 1.5));
        }
        if !self.architect.verticality_used && a.budget >= cost(StrokeKind::Verticality) + 60 {
            options.push((StrokeKind::Verticality, 0.5));
        }
        options.retain(|(k, _)| cost(*k) <= a.budget);
        let total: f32 = options.iter().map(|(_, w)| w).sum();
        if total <= 0.0 {
            // Saving up.
            self.architect.next_stroke = p.tick + 30;
            return true;
        }
        let mut roll = self.rng.next_f32() * total;
        let mut kind = options[0].0;
        for (k, w) in &options {
            if roll < *w {
                kind = *k;
                break;
            }
            roll -= w;
        }
        let toward = (exit - runner.pos).normalize_or_zero();
        let radius = rules.min_radius + self.rng.next_f32() * (rules.max_radius - rules.min_radius);
        // Where the runner will be when the telegraph ends, then a bit
        // further toward the exit for rock (it should fly into it).
        let telegraph = rules.telegraph_secs + 0.5;
        let ahead = runner.pos + runner.vel * telegraph;
        let lead = match kind {
            StrokeKind::GravityChaosPlus | StrokeKind::Palette | StrokeKind::InstabilityPlus => 0.0,
            _ => 0.5 * radius,
        };
        let mut centre = ahead + toward * lead;
        // Rock strokes stay clear of the exit.
        let keep = radius + rules.keep_clear + 2.0;
        if centre.distance(exit) < keep {
            centre = exit + (centre - exit).normalize_or_zero() * keep;
        }
        if kind == StrokeKind::Verticality {
            self.architect.verticality_used = true;
        }
        self.architect.outbox = Some((kind.index(), centre, radius));
        counters.moods.architect_strokes += 1;
        let cooldown = (rules.stroke_cooldown_secs * TICKS_PER_SEC) as u64;
        self.architect.next_stroke = p.tick + cooldown + 20 + u64::from(self.rng.below(60));
        true
    }

    /// A runner flies to the exit (see the module docs). False when the bot
    /// is no runner of a running round.
    pub(super) fn plan_runner<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
        counters: &mut BrainCounters,
    ) -> bool {
        let Some(a) = p.mode.architect.as_ref() else {
            return false;
        };
        let Some((exit, _)) = a.exit else {
            return false;
        };
        if a.me_architect || a.round_over {
            return false;
        }
        // Telegraphed strokes are danger zones.
        for s in &a.strokes {
            let threat = matches!(
                s.kind,
                Some(
                    StrokeKind::EnclosurePlus
                        | StrokeKind::DensityPlus
                        | StrokeKind::InstabilityPlus
                        | StrokeKind::GravityChaosPlus
                )
            );
            if threat && s.phase == stroke_phase::TELEGRAPH {
                self.set_danger(s.pos, s.radius + 3.0, s.phase_end.max(p.tick + 30));
            }
        }
        if self.plan_flee(p, terrain, plan) {
            return true;
        }
        counters.moods.runner_ticks += 1;
        let d = p.pos.distance(exit);
        if d < self.architect.best_dist - 2.0 || self.architect.best_dist == 0.0 {
            self.architect.best_dist = d;
            self.architect.best_tick = p.tick;
        }
        let cruise = self.tuning.cruise_speed;
        if self
            .plan_travel(p, terrain, exit, cruise, true, plan)
            .is_none()
        {
            plan.desired_vel = arrive(p.pos, exit, cruise, 4.0);
            plan.goal_dist = d;
        }
        // Stuck: blast the rock toward the exit.
        if p.tick > self.architect.best_tick + RUNNER_STUCK_SECS * TICK_HZ as u64 {
            let dir = (exit - p.pos).normalize_or_zero();
            if let Some(hit) = terrain.raycast(p.pos, dir, 25.0) {
                plan.aim = Some(hit.point);
                let err = angle_between(p.orientation.forward(), hit.point - p.pos);
                if err < 0.08 {
                    counters.moods.runner_dig_shots += 1;
                    let rocket = self.owned(p).iter().find_map(|(w, ammo)| {
                        let g = self.arsenal.gun(*w)?;
                        (*ammo > 0 && matches!(g.role, Role::BoltRocket | Role::Quicksilver))
                            .then_some(g)
                    });
                    match rocket {
                        Some(g) if hit.distance > g.splash_radius + 1.5 => {
                            plan.select_secondary = Some(g.id);
                            plan.fire_secondary = p.secondary == Some(g.id);
                        }
                        _ => {
                            if let Some(l) = self.arsenal.laser {
                                plan.select = Some(l.id);
                                plan.fire_primary = p.primary == Some(l.id);
                            }
                        }
                    }
                }
            }
            if p.tick > self.architect.best_tick + 4 * RUNNER_STUCK_SECS * TICK_HZ as u64 {
                // Try a fresh route.
                self.nav.clear_route();
                self.architect.best_dist = d;
                self.architect.best_tick = p.tick;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_game::ls_net::{NetModePlayer, NetObjective};
    use ls_game::ls_sim::ModeKind;

    #[test]
    fn architect_sense_reads_exit_strokes_and_role() {
        let mut m = NetModeState {
            kind: ModeKind::TheArchitect.index(),
            ..NetModeState::default()
        };
        m.players.push(NetModePlayer {
            id: 2,
            flags: ls_game::ls_net::mode_player_flags::ARCHITECT,
            meter: 120,
            ..NetModePlayer::default()
        });
        m.objectives.push(NetObjective {
            id: 0,
            kind: objective_kind::EXIT,
            radius: 6.0,
            pos: Vec3::new(100.0, 0.0, 0.0),
            ..NetObjective::default()
        });
        m.objectives.push(NetObjective {
            id: 1,
            kind: objective_kind::STROKE,
            state: StrokeKind::DensityPlus.index() | (stroke_phase::TELEGRAPH << 4),
            radius: 14.0,
            timer_end: 900,
            ..NetObjective::default()
        });
        let s = ArchitectSense::from_mode(&m, 2);
        assert!(s.me_architect);
        assert_eq!(s.budget, 120);
        assert_eq!(s.exit, Some((Vec3::new(100.0, 0.0, 0.0), 6.0)));
        assert_eq!(s.strokes[0].kind, Some(StrokeKind::DensityPlus));
        assert_eq!(s.strokes[0].phase, stroke_phase::TELEGRAPH);
        assert!(!ArchitectSense::from_mode(&m, 3).me_architect);
    }
}
