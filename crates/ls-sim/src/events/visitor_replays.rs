//! Replay hashes of the visitor events (M4 B2): one 20 s forced replay per
//! event (Tax drone, Pizza delivery, Customer survey, Mimic pickups, Gossip
//! wall) for the native-vs-WASM determinism cross-check.
//!
//! Each replay plays the [`crate::golden`] arena with 8 ships on the
//! scripted inputs (pseudo-random buttons and thrust, aiming at the nearest
//! enemy) under Team Brawl, gives player 0 the lead and player 3 a damaged
//! hull, forces the event at tick 60 and runs 23 s (the 3 s announcement
//! and 20 s after it), so the drones fly, get shot, tax, deliver and the
//! mimics bite.
//!
//! The hashes live in `crates/ls-sim/tests/fixtures/mood_replays.txt`
//! (lines `event/<id>` of these five events) and are checked by the unit
//! test below on native and wasm32. Re-bless after a deliberate change with
//! `LS_BLESS_MOOD_REPLAYS=1 cargo test -p ls-sim --lib visitor_replays`
//! (rewrites only these lines).

use ls_math::Pcg32;

use crate::entity::EntityKind;
use crate::events::EventKind;
use crate::golden::{arena, inputs, setup};
use crate::ruleset::BUILTIN_RULESETS;
use crate::{CueKind, Event, PlayerId, RuleSet, ShipType, World, WorldConfig};

/// Ticks per replay (23 s: the announcement and 20 s).
pub const VISITOR_REPLAY_TICKS: u64 = 23 * 60;
/// Tick the event is forced at.
pub const VISITOR_EVENT_TICK: u64 = 60;
/// Seed of every replay.
pub const VISITOR_REPLAY_SEED: u64 = 2026;
/// The events, in fixture order.
pub const VISITOR_EVENTS: [EventKind; 5] = [
    EventKind::TaxDrone,
    EventKind::GossipWall,
    EventKind::PizzaDelivery,
    EventKind::CustomerSurvey,
    EventKind::MimicPickups,
];

/// Outcome of one [`visitor_replay`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisitorReplay {
    /// Fixture name (`event/<id>`).
    pub name: String,
    /// Final state hash folded with the state hash of every second.
    pub hash: u64,
    pub ticks: u64,
    pub events: usize,
    /// `CueKind::EventOutcome` cues of the event.
    pub outcomes: usize,
    /// Ticks an event prop (drone, mimic, face) existed.
    pub prop_ticks: u64,
}

fn rules() -> RuleSet {
    let (f, text) = BUILTIN_RULESETS
        .iter()
        .find(|(f, _)| f.ends_with("team_brawl.ron"))
        .expect("built-in rule set");
    let mut r = RuleSet::from_ron(f, text).expect("valid");
    r.mutators.clear();
    r
}

/// Plays the replay of `kind` (see the module docs).
pub fn visitor_replay(kind: EventKind) -> VisitorReplay {
    let mut cfg = WorldConfig::paint_the_void(VISITOR_REPLAY_SEED, setup());
    cfg.rules = rules();
    let mut w = World::new(cfg).expect("replay world");
    let mut t = arena();
    let mut ids: Vec<PlayerId> = Vec::new();
    for ty in ShipType::ALL.iter().cycle().take(4 * ShipType::ALL.len()) {
        if ids.len() == 8 {
            break;
        }
        if let Ok(id) = w.add_player(None, *ty) {
            ids.push(id);
        }
    }
    let lead = ids[0];
    let team = w.ship(lead).map_or(0, |s| s.team) as usize;
    w.mode_mut().team_scores[team] = 3;
    if let Some(p) = w.mode_mut().players.get_mut(&lead) {
        p.score = 3;
    }
    if let Some(s) = w.ship_mut(ids[3]) {
        s.hull *= 0.3;
    }
    w.schedule_event(VISITOR_EVENT_TICK, kind);
    let mut rng = Pcg32::new(VISITOR_REPLAY_SEED, 23);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let fold = |h: u64, v: u64| (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
    let mut r = VisitorReplay {
        name: format!("event/{}", kind.id()),
        hash: 0,
        ticks: 0,
        events: 0,
        outcomes: 0,
        prop_ticks: 0,
    };
    for tick in 1..=VISITOR_REPLAY_TICKS {
        let input = inputs(&mut rng, &w, &ids, tick);
        for e in w.step(&mut t, &input) {
            r.events += 1;
            if let Event::Cue(c) = e {
                if c.kind == CueKind::EventOutcome && (c.value >> 8) as u8 == kind.index() {
                    r.outcomes += 1;
                }
            }
        }
        if w.entities().iter().any(|e| {
            matches!(
                e.kind,
                EntityKind::Drone | EntityKind::Mimic | EntityKind::GossipFace
            )
        }) {
            r.prop_ticks += 1;
        }
        r.ticks = tick;
        if tick % 60 == 0 {
            hash = fold(hash, w.state_hash());
        }
        if !w.mode().is_running() {
            break;
        }
    }
    r.hash = fold(hash, w.state_hash());
    r
}

/// Every visitor replay, in [`VISITOR_EVENTS`] order.
pub fn visitor_replays() -> Vec<VisitorReplay> {
    VISITOR_EVENTS.iter().map(|k| visitor_replay(*k)).collect()
}

/// Fixture lines (`<name> <hash as 0x%016x>`) of every visitor replay.
pub fn visitor_replay_hashes() -> Vec<String> {
    visitor_replays()
        .iter()
        .map(|r| format!("{} {:#018x}", r.name, r.hash))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const FIXTURE: &str = include_str!("../../tests/fixtures/mood_replays.txt");

    fn ours(line: &str) -> bool {
        VISITOR_EVENTS
            .iter()
            .any(|k| line.starts_with(&format!("event/{} ", k.id())))
    }

    /// Every visitor replay matches the fixture (native and wasm32) and
    /// exercises its event. `LS_BLESS_MOOD_REPLAYS=1` rewrites these lines
    /// of the fixture (native).
    #[test]
    fn visitor_replays_match_the_fixture() {
        let replays = visitor_replays();
        for r in &replays {
            println!("{r:?}");
            assert!(
                r.prop_ticks > 0 || r.name == "event/customer_survey",
                "{r:?}"
            );
        }
        let by = |k: EventKind| {
            replays
                .iter()
                .find(|r| r.name == format!("event/{}", k.id()))
                .unwrap()
        };
        assert!(by(EventKind::TaxDrone).outcomes >= 1);
        assert!(by(EventKind::CustomerSurvey).outcomes >= 1);
        assert!(by(EventKind::MimicPickups).prop_ticks > 600);
        let got: Vec<String> = replays
            .iter()
            .map(|r| format!("{} {:#018x}", r.name, r.hash))
            .collect();
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
        let want: Vec<String> = FIXTURE
            .lines()
            .map(str::trim)
            .filter(|l| ours(l))
            .map(str::to_string)
            .collect();
        assert_eq!(
            got, want,
            "visitor replay hashes changed: re-bless with LS_BLESS_MOOD_REPLAYS=1 if deliberate"
        );
    }

    #[test]
    fn visitor_replays_are_deterministic() {
        assert_eq!(
            visitor_replay(EventKind::TaxDrone),
            visitor_replay(EventKind::TaxDrone)
        );
    }
}
