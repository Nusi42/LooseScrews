//! Event replay hashes (M4 wave B): one scripted 8-ship replay per built
//! comedy event, forced by the schedule, for the native-vs-WASM determinism
//! cross-check (`docs/milestone-4-plan.md` §4: "one 20 s replay per
//! event").
//!
//! Each replay plays the [`crate::golden`] scripted inputs (pseudo-random
//! buttons and thrust, aiming at the nearest enemy) in the golden arena
//! with the Sandbox rule set (every weapon as a pickup, so pickups, ordnance
//! and debris are about) and Absurdity 0, forces the event at
//! [`EVENT_REPLAY_AT`] and runs until 20 s of its active phase have passed
//! (or it ended, plus 2 s). Gravity coffee break gets the `world/gravity`
//! replay's three wells (it needs wells); every other event runs without.
//!
//! The hashes live in `crates/ls-sim/tests/fixtures/mood_replays.txt`
//! (lines `event/<id>`), checked by the unit test below on native and
//! wasm32 for every event whose module is built (`implemented()`).
//! Re-bless after a deliberate change with
//! `LS_BLESS_MOOD_REPLAYS=1 cargo test -p ls-sim --lib events::replays`
//! (rewrites only the lines of the events it covers; packages with their
//! own scenario replay, see [`has_own_replay`], bless theirs).

use ls_math::{Pcg32, Vec3};

use crate::events::EventKind;
use crate::golden::{arena, inputs, setup};
use crate::ruleset::BUILTIN_RULESETS;
use crate::{DirectorEvent, Event, PlayerId, RuleSet, ShipType, World, WorldConfig};

/// Tick the event is forced at (the announce phase starts).
pub const EVENT_REPLAY_AT: u64 = 60;
/// Seconds of the active phase a replay covers (at most).
pub const EVENT_REPLAY_ACTIVE_SECS: u64 = 20;
/// Ships per replay.
pub const EVENT_REPLAY_SHIPS: usize = 8;
/// Seed of every replay.
pub const EVENT_REPLAY_SEED: u64 = 2026;

/// Outcome of one [`event_replay`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventReplay {
    pub kind: EventKind,
    /// Final state hash folded with the state hash of every second.
    pub hash: u64,
    pub ticks: u64,
    /// The event's active phase started and it ended (or ran to the end of
    /// the replay).
    pub started: bool,
    pub ended: bool,
    /// Events seen (kills, damage, cues, ...).
    pub events: usize,
}

impl EventReplay {
    /// The fixture line: `event/<id> <hash>`.
    pub fn line(&self) -> String {
        format!("event/{} {:#018x}", self.kind.id(), self.hash)
    }
}

fn sandbox() -> RuleSet {
    let (f, text) = BUILTIN_RULESETS
        .iter()
        .find(|(f, _)| f.ends_with("sandbox_all_weapons.ron"))
        .expect("built-in rule set");
    let mut r = RuleSet::from_ron(f, text).expect("valid");
    r.mutators.clear();
    r
}

/// Plays the replay of `kind` (see the module docs).
pub fn event_replay(kind: EventKind) -> EventReplay {
    let mut s = setup();
    if kind == EventKind::GravityCoffeeBreak {
        s.gravity_chaos = 100;
        s.wells = [
            (Vec3::new(16.0, 6.0, 10.0), 2.0),
            (Vec3::new(-16.0, -6.0, -10.0), 2.0),
            (Vec3::new(0.0, 10.0, -20.0), 1.5),
        ]
        .into_iter()
        .map(|(pos, core_radius)| ls_math::level::WellSite {
            pos,
            core_radius,
            hollow_radius: 8.0,
            strength: 1.0,
            range: 40.0,
            kind: ls_math::level::WellKind::Core,
        })
        .collect();
    }
    let mut cfg = WorldConfig::paint_the_void(EVENT_REPLAY_SEED, s);
    cfg.rules = sandbox();
    let mut w = World::new(cfg).expect("replay world");
    let mut ids: Vec<PlayerId> = Vec::new();
    for ty in ShipType::ALL.iter().cycle().take(4 * ShipType::ALL.len()) {
        if ids.len() == EVENT_REPLAY_SHIPS {
            break;
        }
        if let Ok(id) = w.add_player(None, *ty) {
            ids.push(id);
        }
    }
    w.schedule_event(EVENT_REPLAY_AT, kind);
    let mut t = arena();
    let mut rng = Pcg32::new(EVENT_REPLAY_SEED, 23);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let fold = |h: u64, v: u64| (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
    let mut r = EventReplay {
        kind,
        hash: 0,
        ticks: 0,
        started: false,
        ended: false,
        events: 0,
    };
    let mut stop: Option<u64> = None;
    for tick in 1..=EVENT_REPLAY_AT + 180 + 60 * (EVENT_REPLAY_ACTIVE_SECS + 10) {
        let input = inputs(&mut rng, &w, &ids, tick);
        for e in w.step(&mut t, &input) {
            r.events += 1;
            match e {
                Event::Director(DirectorEvent::Started { kind: k }) if k == kind => {
                    r.started = true;
                    stop = Some(w.tick() + 60 * EVENT_REPLAY_ACTIVE_SECS);
                }
                Event::Director(DirectorEvent::Ended { kind: k }) if k == kind => {
                    r.ended = true;
                    stop = stop.map(|s| s.min(w.tick() + 120));
                }
                _ => {}
            }
        }
        r.ticks = tick;
        if tick % 60 == 0 {
            hash = fold(hash, w.state_hash());
        }
        if stop.is_some_and(|s| tick >= s) || !w.mode().is_running() {
            break;
        }
    }
    r.hash = fold(hash, w.state_hash());
    r
}

/// Events whose package recorded its own scenario replay (B2's visitors in
/// [`crate::events::visitor_replays`], B3's spectacles in
/// [`crate::event_replays`]); this generic replay skips them.
pub fn has_own_replay(kind: EventKind) -> bool {
    crate::events::visitor_replays::VISITOR_EVENTS.contains(&kind)
        || crate::event_replays::SPECTACLE_EVENTS.contains(&kind)
}

/// The events with a built module and no replay of their own, in
/// [`EventKind`] order.
pub fn replayed_events() -> Vec<EventKind> {
    EventKind::ALL
        .into_iter()
        .filter(|k| k.module().implemented() && !has_own_replay(*k))
        .collect()
}

/// Fixture lines of every built event's replay, this generic one and the
/// packages' own (`tools/determinism-check.mjs` compares them with the WASM
/// build).
pub fn event_replay_hashes() -> Vec<String> {
    let mut lines: Vec<String> = replayed_events()
        .into_iter()
        .map(|k| event_replay(k).line())
        .collect();
    lines.extend(crate::events::visitor_replays::visitor_replay_hashes());
    lines.extend(crate::event_replays::spectacle_replay_hashes());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const FIXTURE: &str = include_str!("../../tests/fixtures/mood_replays.txt");

    /// An `event/` line of an event this generic replay covers.
    fn ours(line: &str) -> bool {
        replayed_events()
            .iter()
            .any(|k| line.starts_with(&format!("event/{} ", k.id())))
    }

    fn fixture_event_lines() -> Vec<String> {
        FIXTURE
            .lines()
            .map(str::trim)
            .filter(|l| ours(l))
            .map(str::to_string)
            .collect()
    }

    /// Every built event's replay matches the fixture (native and wasm32)
    /// and really runs the event. `LS_BLESS_MOOD_REPLAYS=1` rewrites the
    /// `event/` lines of the fixture (native).
    #[test]
    fn event_replays_match_the_fixture() {
        let replays: Vec<EventReplay> = replayed_events().into_iter().map(event_replay).collect();
        for r in &replays {
            println!("{r:?}");
            assert!(r.started, "{:?} never started", r.kind);
            assert!(r.events > 100, "{:?}", r.kind);
        }
        let got: Vec<String> = replays.iter().map(EventReplay::line).collect();
        #[cfg(not(target_arch = "wasm32"))]
        if std::env::var_os("LS_BLESS_MOOD_REPLAYS").is_some() {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/mood_replays.txt"
            );
            let current = std::fs::read_to_string(path).expect("read the fixture");
            let mut text: String = current
                .lines()
                .filter(|l| !ours(l.trim()))
                .map(|l| format!("{l}\n"))
                .collect();
            for l in &got {
                text.push_str(l);
                text.push('\n');
            }
            std::fs::write(path, text).expect("write the fixture");
            return;
        }
        assert_eq!(
            got,
            fixture_event_lines(),
            "event replay hashes changed: re-bless with LS_BLESS_MOOD_REPLAYS=1 if deliberate"
        );
    }

    /// The same replay twice gives the same hash; the event changes it.
    #[test]
    fn event_replays_are_deterministic_and_the_event_acts() {
        let a = event_replay(EventKind::LevelHiccups);
        assert_eq!(a, event_replay(EventKind::LevelHiccups));
        assert_ne!(a.hash, event_replay(EventKind::BigHeadDay).hash);
    }
}
