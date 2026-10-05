//! The lobby state machine of a room (plan §3.1):
//!
//! ```text
//! Warmup -> Countdown -> Match -> Results -> Warmup -> ...
//! ```
//!
//! - **Warmup** (the plan's "Lobby"): players fly in the next match's level
//!   with weapons off (a warm-up room, [`warmup_rules`]) while the match room
//!   is built on a worker thread. The countdown starts once the player count
//!   fits the playlist entry and either everyone is ready or
//!   [`LobbyTiming::warmup`] has passed since players arrived. If the
//!   warm-up time passes with a player count the entry does not fit, the
//!   room picks another entry ([`Action::Repick`]).
//! - **Countdown** ([`LobbyTiming::countdown`]): back to Warmup if the
//!   player count stops fitting; at zero the match room is swapped in (as
//!   soon as it is built) and every player gets its `Welcome`.
//! - **Match**: until the mode ends.
//! - **Results** (`--intermission`): the next entry's room is built on a
//!   worker thread meanwhile and swapped in at the end (as soon as it is
//!   built).
//!
//! With [`LobbyTiming::off`] (warm-up and countdown 0) there is no warm-up
//! room: Results swaps the next match room in directly (`Results -> Match`),
//! and a match starts when its first player joins.
//!
//! [`Lobby`] is pure (time in sim-thread ticks); the sim thread
//! ([`crate::sim`]) feeds it and carries out its [`Action`]s.

use std::time::Duration;

use ls_math::TICK_HZ;
use ls_sim::{ModeRules, PaintTheVoidRules, RuleSet};

/// Default longest warm-up before the countdown starts.
pub const DEFAULT_WARMUP: Duration = Duration::from_secs(30);
/// Default countdown before a match.
pub const DEFAULT_COUNTDOWN: Duration = Duration::from_secs(10);
/// Match length of a warm-up room (the warm-up ends long before).
const WARMUP_MATCH_SECS: f32 = 3600.0;

/// Lobby timing of a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LobbyTiming {
    /// Longest warm-up with enough players before the countdown starts
    /// (shorter when everyone is ready).
    pub warmup: Duration,
    pub countdown: Duration,
    /// How long the results of a finished match stay on screen.
    pub results: Duration,
}

impl Default for LobbyTiming {
    /// 30 s warm-up, 10 s countdown, 10 s results.
    fn default() -> Self {
        LobbyTiming {
            warmup: DEFAULT_WARMUP,
            countdown: DEFAULT_COUNTDOWN,
            results: crate::sim::DEFAULT_INTERMISSION,
        }
    }
}

impl LobbyTiming {
    /// No warm-up and no countdown (matches start when the first player
    /// joins); `results` as given.
    pub fn off(results: Duration) -> Self {
        LobbyTiming {
            warmup: Duration::ZERO,
            countdown: Duration::ZERO,
            results,
        }
    }

    /// True if rooms have no warm-up phase.
    pub fn is_off(&self) -> bool {
        self.warmup.is_zero() && self.countdown.is_zero()
    }
}

/// Seconds to sim-thread ticks.
pub fn ticks(d: Duration) -> u64 {
    (d.as_secs_f64() * f64::from(TICK_HZ)).round() as u64
}

/// The rules of a warm-up room for `rules`: same ships and teams, every
/// weapon and gadget banned (empty loadout, no pickups, no mutators: a
/// mutator such as Instagib would hand its weapon back) and a long match
/// clock, so players can fly the level but not fight.
///
/// The warm-up never ends early, whatever the next match's mode: instead
/// of that mode (whose kill target, lives, last-side-standing check or
/// objectives could end the room or eliminate players), the warm-up plays
/// [`warmup_mode`], whose only end condition is its clock (which the sim
/// thread keeps pushing back while the warm-up lasts, see
/// [`extend_warmup_clock`]). The lobby announces the real next mode in
/// `LobbyState`.
pub fn warmup_rules(rules: &RuleSet) -> RuleSet {
    let mut r = rules.clone();
    r.name = format!("Warm-up: {}", rules.name);
    r.weapons.clear();
    r.gadgets.clear();
    r.pickups.clear();
    r.upgrade_pickups.clear();
    r.gadget_pickups.clear();
    r.scrap_pickup = None;
    r.starting_loadout.primaries.clear();
    r.starting_loadout.secondaries.clear();
    r.starting_loadout.gadget = None;
    // Mutators would rewrite the arsenal back in (Instagib: the Railspike).
    r.mutators.clear();
    // A timer-only mode whatever the next mode (no reactor, no closing
    // walls, no kill target or lives can end a warm-up).
    r.mode = warmup_mode(&rules.mode);
    r
}

/// The mode of a warm-up room before a match of `mode`: Paint the Void (no
/// lives, no score target, kills do not score, and with every weapon banned
/// nobody paints), which ends only by its clock ([`WARMUP_MATCH_SECS`]).
/// It keeps the match's team count (2 to 4); a free-for-all match warms up
/// on two sides (Paint the Void has no free-for-all; without weapons the
/// sides only colour the ships).
pub fn warmup_mode(mode: &ModeRules) -> ModeRules {
    let teams = mode.teams();
    ModeRules::PaintTheVoid(PaintTheVoidRules {
        teams: if (2..=4).contains(&teams) { teams } else { 2 },
        match_secs: WARMUP_MATCH_SECS,
        ..PaintTheVoidRules::default()
    })
}

/// Clock a warm-up room keeps ahead of it ([`extend_warmup_clock`]).
const WARMUP_CLOCK_MARGIN_TICKS: u64 = 600 * TICK_HZ as u64;

/// Keeps a warm-up room's match clock from running out however long the
/// warm-up lasts (a lone player waiting for a mode's minimum): sets the
/// end tick [`WARMUP_MATCH_SECS`] ahead whenever less than ten minutes are
/// left. Returns true if it moved the clock.
pub fn extend_warmup_clock(world: &mut ls_sim::World) -> bool {
    let now = world.tick();
    let mode = world.mode_mut();
    if mode.is_running() && mode.end_tick.saturating_sub(now) < WARMUP_CLOCK_MARGIN_TICKS {
        mode.end_tick = now + (f64::from(WARMUP_MATCH_SECS) * f64::from(TICK_HZ)) as u64;
        true
    } else {
        false
    }
}

/// Lobby phase. Times are sim-thread ticks ([`Lobby::update`]'s `now`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Warm-up; `since` is when players were first present (`None` while
    /// the room is empty).
    Warmup {
        since: Option<u64>,
    },
    Countdown {
        ends: u64,
    },
    Match,
    Results {
        ends: u64,
    },
}

impl Phase {
    /// The phase on the wire (`LobbyState`).
    pub fn to_net(&self) -> ls_net::LobbyPhase {
        match self {
            Phase::Warmup { .. } => ls_net::LobbyPhase::Lobby,
            Phase::Countdown { .. } => ls_net::LobbyPhase::Countdown,
            Phase::Match => ls_net::LobbyPhase::Match,
            Phase::Results { .. } => ls_net::LobbyPhase::Results,
        }
    }

    /// Short name for logs and status.
    pub fn name(&self) -> &'static str {
        match self {
            Phase::Warmup { .. } => "warmup",
            Phase::Countdown { .. } => "countdown",
            Phase::Match => "match",
            Phase::Results { .. } => "results",
        }
    }
}

/// What the room looks like this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LobbyInput {
    /// Sim-thread tick.
    pub now: u64,
    pub players: usize,
    /// Every connected player has sent `Ready` (false without players).
    pub all_ready: bool,
    /// The player count fits the current playlist entry.
    pub fits: bool,
    /// The current room's mode has ended (Match only).
    pub match_over: bool,
    /// The room to swap in next has been built.
    pub next_ready: bool,
}

/// What the sim thread has to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    /// Warmup -> Countdown.
    CountdownStarted,
    /// Countdown -> Warmup (player count stopped fitting).
    CountdownCancelled,
    /// Warm-up over with a player count the entry does not fit: pick another
    /// entry and build its warm-up room (swapped in when built, then
    /// [`Lobby::warmup_swapped`]).
    Repick,
    /// Countdown over and the match room is built: swap it in (Match).
    StartMatch,
    /// The match ended (Results): pick the next entry and build its room.
    MatchOver,
    /// Results over and the next room is built: swap it in (Warmup, or Match
    /// without a lobby).
    NextRoom,
}

/// The state machine; see the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lobby {
    timing: LobbyTiming,
    phase: Phase,
}

impl Lobby {
    /// Starts in Warmup, or in Match without a lobby.
    pub fn new(timing: LobbyTiming) -> Self {
        let phase = if timing.is_off() {
            Phase::Match
        } else {
            Phase::Warmup { since: None }
        };
        Lobby { timing, phase }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn timing(&self) -> &LobbyTiming {
        &self.timing
    }

    /// True while the room is a warm-up room.
    pub fn in_warmup(&self) -> bool {
        matches!(self.phase, Phase::Warmup { .. } | Phase::Countdown { .. })
    }

    /// Ticks until the current phase ends (`None`: open-ended).
    pub fn remaining(&self, now: u64) -> Option<u64> {
        match self.phase {
            Phase::Warmup { since: Some(s) } => {
                Some((s + ticks(self.timing.warmup)).saturating_sub(now))
            }
            Phase::Countdown { ends } | Phase::Results { ends } => Some(ends.saturating_sub(now)),
            _ => None,
        }
    }

    /// Advances the state machine by one tick.
    pub fn update(&mut self, i: LobbyInput) -> Action {
        let lobby = !self.timing.is_off();
        match self.phase {
            Phase::Warmup { since } => {
                if i.players == 0 {
                    self.phase = Phase::Warmup { since: None };
                    return Action::None;
                }
                let since = since.unwrap_or(i.now);
                self.phase = Phase::Warmup { since: Some(since) };
                let waited = i.now.saturating_sub(since) >= ticks(self.timing.warmup);
                if i.fits && (i.all_ready || waited) {
                    self.phase = Phase::Countdown {
                        ends: i.now + ticks(self.timing.countdown),
                    };
                    Action::CountdownStarted
                } else if !i.fits && waited {
                    // Waits for the new warm-up room (warmup_swapped).
                    self.phase = Phase::Warmup { since: Some(i.now) };
                    Action::Repick
                } else {
                    Action::None
                }
            }
            Phase::Countdown { ends } => {
                if i.players == 0 || !i.fits {
                    self.phase = Phase::Warmup {
                        since: (i.players > 0).then_some(i.now),
                    };
                    Action::CountdownCancelled
                } else if i.now >= ends && i.next_ready {
                    self.phase = Phase::Match;
                    Action::StartMatch
                } else {
                    Action::None
                }
            }
            Phase::Match => {
                if i.match_over {
                    self.phase = Phase::Results {
                        ends: i.now + ticks(self.timing.results),
                    };
                    Action::MatchOver
                } else {
                    Action::None
                }
            }
            Phase::Results { ends } => {
                if i.now >= ends && i.next_ready {
                    self.phase = if lobby {
                        Phase::Warmup {
                            since: (i.players > 0).then_some(i.now),
                        }
                    } else {
                        Phase::Match
                    };
                    Action::NextRoom
                } else {
                    Action::None
                }
            }
        }
    }

    /// Starts the countdown now (no playlist entry fits better than the
    /// current one after [`Action::Repick`]).
    pub fn start_countdown(&mut self, now: u64) {
        self.phase = Phase::Countdown {
            ends: now + ticks(self.timing.countdown),
        };
    }

    /// A new warm-up room (after [`Action::Repick`]) was swapped in: the
    /// warm-up starts over.
    pub fn warmup_swapped(&mut self, now: u64, players: usize) {
        self.phase = Phase::Warmup {
            since: (players > 0).then_some(now),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_game::server::{Room, RoomConfig};
    use ls_game::MatchConfig;
    use ls_worldgen::{ShareCode, Sliders};

    fn timing() -> LobbyTiming {
        LobbyTiming {
            warmup: Duration::from_secs(30),
            countdown: Duration::from_secs(10),
            results: Duration::from_secs(10),
        }
    }

    fn input(now: u64, players: usize) -> LobbyInput {
        LobbyInput {
            now,
            players,
            fits: players > 0,
            ..LobbyInput::default()
        }
    }

    #[test]
    fn full_cycle_with_warmup_timeout() {
        let mut l = Lobby::new(timing());
        assert_eq!(l.phase(), Phase::Warmup { since: None });
        // Empty: nothing happens, however long.
        assert_eq!(l.update(input(10_000, 0)), Action::None);
        // Players arrive at 20_000; 30 s later the countdown starts.
        assert_eq!(l.update(input(20_000, 2)), Action::None);
        assert_eq!(l.remaining(20_000), Some(1800));
        assert_eq!(l.update(input(21_799, 2)), Action::None);
        assert_eq!(l.update(input(21_800, 2)), Action::CountdownStarted);
        assert_eq!(l.phase(), Phase::Countdown { ends: 22_400 });
        // At zero, but the match room is not built yet: wait.
        assert_eq!(l.update(input(22_400, 2)), Action::None);
        let built = LobbyInput {
            next_ready: true,
            ..input(22_410, 2)
        };
        assert_eq!(l.update(built), Action::StartMatch);
        assert_eq!(l.phase(), Phase::Match);
        assert_eq!(l.update(input(30_000, 2)), Action::None);
        let over = LobbyInput {
            match_over: true,
            ..input(30_001, 2)
        };
        assert_eq!(l.update(over), Action::MatchOver);
        assert_eq!(l.phase(), Phase::Results { ends: 30_601 });
        let built = LobbyInput {
            next_ready: true,
            ..input(30_600, 2)
        };
        assert_eq!(l.update(built), Action::None, "results still showing");
        let built = LobbyInput {
            now: 30_601,
            ..built
        };
        assert_eq!(l.update(built), Action::NextRoom);
        assert_eq!(
            l.phase(),
            Phase::Warmup {
                since: Some(30_601)
            }
        );
    }

    #[test]
    fn everyone_ready_skips_the_rest_of_the_warmup() {
        let mut l = Lobby::new(timing());
        l.update(input(0, 3));
        let ready = LobbyInput {
            all_ready: true,
            ..input(60, 3)
        };
        assert_eq!(l.update(ready), Action::CountdownStarted);
    }

    #[test]
    fn countdown_cancels_when_players_leave_or_stop_fitting() {
        let mut l = Lobby::new(timing());
        l.update(input(0, 4));
        assert_eq!(l.update(input(1800, 4)), Action::CountdownStarted);
        let unfit = LobbyInput {
            fits: false,
            ..input(1900, 3)
        };
        assert_eq!(l.update(unfit), Action::CountdownCancelled);
        assert_eq!(l.phase(), Phase::Warmup { since: Some(1900) });
        // Still not fitting after the warm-up: pick another entry.
        let unfit = LobbyInput {
            fits: false,
            ..input(3700, 3)
        };
        assert_eq!(l.update(unfit), Action::Repick);
        l.warmup_swapped(3800, 3);
        assert_eq!(l.phase(), Phase::Warmup { since: Some(3800) });
        assert_eq!(l.update(input(3800 + 1800, 3)), Action::CountdownStarted);
        assert_eq!(l.update(input(5700, 0)), Action::CountdownCancelled);
        assert_eq!(l.phase(), Phase::Warmup { since: None });
    }

    #[test]
    fn without_a_lobby_results_lead_straight_to_the_next_match() {
        let mut l = Lobby::new(LobbyTiming::off(Duration::from_secs(1)));
        assert_eq!(l.phase(), Phase::Match);
        assert!(!l.in_warmup());
        let over = LobbyInput {
            match_over: true,
            ..input(100, 1)
        };
        assert_eq!(l.update(over), Action::MatchOver);
        let next = LobbyInput {
            next_ready: true,
            ..input(160, 1)
        };
        assert_eq!(l.update(next), Action::NextRoom);
        assert_eq!(l.phase(), Phase::Match);
    }

    #[test]
    fn instagib_warmup_has_no_railspike() {
        let mut rules = RuleSet::paint_the_void();
        rules.mutators.push(ls_sim::ruleset::Mutator::Instagib);
        let warm = warmup_rules(&rules).effective();
        assert!(warm.mutators.is_empty());
        assert!(warm.availability("railspike").is_banned());
        assert!(warm.starting_loadout.primaries.is_empty());
    }

    #[test]
    fn hot_reactor_warmup_has_no_reactor() {
        // The warm-up plays Paint the Void (B7's `warmup_mode`): no reactor
        // can cost lives or end it.
        let hot = RuleSet::default_for(ls_sim::ModeKind::HotReactor);
        let w = warmup_rules(&hot);
        assert_eq!(w.mode.kind(), ls_sim::ModeKind::PaintTheVoid);
        w.resolve(&ls_sim::WeaponTable::builtin()).unwrap();
    }

    #[test]
    fn warmup_rooms_have_no_weapons_and_a_long_clock() {
        let rules = warmup_rules(&RuleSet::paint_the_void());
        let mc = MatchConfig {
            rules,
            ..MatchConfig::paint_the_void(ShareCode::new(5, Sliders::new(40, 30)))
        };
        let rules = mc.rules.clone();
        rules.resolve_with(&mc.weapons, &mc.gadgets).unwrap();
        assert!(rules.name.starts_with("Warm-up: "));
        assert!(mc
            .weapons
            .iter()
            .all(|(_, w)| rules.availability(&w.id).is_banned()));
        assert_eq!(rules.mode.match_secs(), WARMUP_MATCH_SECS);
        let squeeze = warmup_rules(&RuleSet::default_for(ls_sim::ModeKind::TheSqueeze));
        assert!(squeeze.resolve(&mc.weapons).is_ok());
        // No walls close in a warm-up: it plays Paint the Void.
        assert_eq!(squeeze.mode.kind(), ls_sim::ModeKind::PaintTheVoid);
        // A room builds, a player joins, holds fire for a while: nothing breaks.
        let cfg = RoomConfig {
            pregenerate: false,
            ..RoomConfig::default()
        };
        let mut room = Room::new(mc, cfg).unwrap();
        let id = room.join(&ls_game::client_hello("w", None, None)).unwrap();
        let player = room.player_of(id).unwrap();
        for t in 1..20u64 {
            let mut cmd = ls_sim::InputCmd::idle(t);
            cmd.buttons = ls_sim::Buttons(
                ls_sim::Buttons::FIRE_PRIMARY.0 | ls_sim::Buttons::FIRE_SECONDARY.0,
            );
            let msg = ls_net::Message::Input(ls_net::InputMsg {
                epoch: 0,
                ack_snapshot: None,
                cmds: vec![ls_game::convert::input_to_net(&cmd)],
            });
            room.handle_message(id, msg).unwrap();
            room.step();
        }
        assert!(room.world().ship(player).is_some());
        assert!(room.world().mode().is_running());
        assert_eq!(room.world().projectiles().len(), 0, "nothing to fire");
    }

    /// Every mode's rules (team and free-for-all variants, with Instagib),
    /// as the playlist would play them.
    fn every_mode_rules() -> Vec<RuleSet> {
        let mut out = Vec::new();
        for kind in ls_sim::ModeKind::ALL {
            let base = RuleSet::default_for(kind);
            let mut ffa = base.clone();
            ffa.mode.set_field("teams", 0.0);
            let mut instagib = base.clone();
            instagib.mutators = vec![ls_sim::Mutator::Instagib];
            out.extend([base, ffa, instagib]);
        }
        out
    }

    #[test]
    fn warmup_rules_drop_every_early_end_condition() {
        for rules in every_mode_rules() {
            let w = warmup_rules(&rules);
            let what = format!("{} ({:?})", rules.name, rules.mode.kind());
            // A timer-only mode: no lives, no kill target, no objectives.
            assert_eq!(w.mode.kind(), ls_sim::ModeKind::PaintTheVoid, "{what}");
            assert_eq!(w.mode.lives(), 0, "{what}");
            assert_eq!(w.mode.match_secs(), WARMUP_MATCH_SECS, "{what}");
            let teams = rules.mode.teams();
            let want = if (2..=4).contains(&teams) { teams } else { 2 };
            assert_eq!(w.mode.teams(), want, "{what}");
            assert!(w.mutators.is_empty(), "{what}");
            assert_eq!(w.allowed_ship_types, rules.allowed_ship_types, "{what}");
            let mc = MatchConfig::paint_the_void(ShareCode::new(5, Sliders::new(40, 30)));
            w.resolve_with(&mc.weapons, &mc.gadgets)
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            assert!(
                mc.weapons
                    .iter()
                    .all(|(_, d)| w.availability(&d.id).is_banned()),
                "{what}"
            );
        }
    }

    /// A warm-up room does not end or eliminate anyone, whatever happens to
    /// scores and lives, and its clock keeps being pushed back.
    #[test]
    fn warmup_rooms_never_end_early() {
        let rules = RuleSet::default_for(ls_sim::ModeKind::TheSqueeze);
        let mc = MatchConfig {
            rules: warmup_rules(&rules),
            ..MatchConfig::paint_the_void(ShareCode::new(6, Sliders::new(40, 30)))
        };
        let cfg = RoomConfig {
            pregenerate: false,
            ..RoomConfig::default()
        };
        let mut room = Room::new(mc, cfg).unwrap();
        let ids: Vec<u8> = (0..3)
            .map(|i| {
                room.join(&ls_game::client_hello(format!("p{i}"), None, None))
                    .unwrap()
            })
            .collect();
        for _ in 0..5 {
            room.step();
        }
        // What would end a kill-target or last-side-standing match: huge
        // scores, one life left, all but one side gone from the score.
        {
            let mode = room.world_mut().mode_mut();
            mode.score_target = 0;
            mode.team_scores = [1000, 0, 0, 0];
            for p in mode.players.values_mut() {
                p.score = 1000;
                p.lives = 1;
            }
        }
        for _ in 0..120 {
            room.step();
        }
        assert!(room.world().mode().is_running());
        assert!(room.results().is_none(), "no MatchResults in a warm-up");
        for id in &ids {
            let player = room.player_of(*id).unwrap();
            assert!(room.world().ship(player).is_some());
            assert!(!room.world().mode().players[&player].is_out());
        }
        // The clock: pushed back when it gets close, left alone otherwise.
        assert!(!extend_warmup_clock(room.world_mut()));
        let now = room.world().tick();
        room.world_mut().mode_mut().end_tick = now + 2;
        assert!(extend_warmup_clock(room.world_mut()));
        for _ in 0..10 {
            room.step();
        }
        assert!(room.world().mode().is_running());
        assert!(room.world().mode().end_tick > now + 3000 * 60);
    }
}
