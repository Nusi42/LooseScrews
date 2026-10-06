//! Spectacle event replay hashes (M4 B3): one scripted 8-ship replay per
//! spectacle event ([`SPECTACLE_EVENTS`]: Cat paw, Tired announcer, Duck
//! parade, Hat rain, Disco inferno), with the event forced 1 s in and
//! played until it ended (at most 40 s), for the native-vs-WASM
//! determinism cross-check. The other built events have their replays in
//! [`crate::events::replays`] (which also lists every event line,
//! `event_replay_hashes`) and [`crate::events::visitor_replays`].
//!
//! Every replay uses the [`crate::golden`] arena and scripted inputs
//! (pseudo-random buttons and thrust, aiming at the nearest enemy), Team
//! Brawl with 6 Bolt Rockets each, Absurdity 0 (only the forced event
//! runs) and one network route across the arena for Duck parade
//! ([`EVENT_REPLAY_ROUTE`]).
//!
//! The hashes live in `crates/ls-sim/tests/fixtures/mood_replays.txt`
//! (lines `event/<id>`) and are checked by the unit test below on native
//! and wasm32. Re-bless after a deliberate change with
//! `LS_BLESS_MOOD_REPLAYS=1 cargo test -p ls-sim --lib event_replays`
//! (`LS_BLESS_EVENT_REPLAYS=1` works too; rewrites only these events'
//! lines).

use ls_math::terrain::TerrainEdit;
use ls_math::{Pcg32, Vec3};

use crate::entity::EntityKind;
use crate::events::EventKind;
use crate::golden::{arena, inputs, setup};
use crate::ruleset::BUILTIN_RULESETS;
use crate::{CueKind, Event, PlayerId, RuleSet, ShipType, World, WorldConfig};

/// Ships per replay.
pub const EVENT_REPLAY_SHIPS: usize = 8;
/// Seed of every replay.
pub const EVENT_REPLAY_SEED: u64 = 2026;
/// Tick the event is forced at (its announcement).
pub const EVENT_REPLAY_START: u64 = 60;
/// Longest replay (40 s).
pub const EVENT_REPLAY_MAX_TICKS: u64 = 40 * 60;
/// The arena's network route (Duck parade): 80 u across the arena.
pub const EVENT_REPLAY_ROUTE: [Vec3; 4] = [
    Vec3::new(-28.0, 2.0, -12.0),
    Vec3::new(0.0, 2.0, -12.0),
    Vec3::new(0.0, 2.0, 12.0),
    Vec3::new(28.0, 2.0, 12.0),
];

/// Outcome of one [`event_replay`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventReplay {
    pub kind: EventKind,
    /// Final state hash folded with the state hash of every second.
    pub hash: u64,
    pub ticks: u64,
    pub events: usize,
    pub kills: usize,
    /// Ticks the event was announced, active or ending.
    pub event_ticks: u64,
    /// Most event props (ducks, hats, paw, ball) at once.
    pub props: usize,
    /// Terrain edits (Cat paw's grooves; craters of the rockets too).
    pub edits: usize,
    /// Kill-cam cues (Tired announcer).
    pub kill_cams: usize,
    /// Projectiles absorbed by an event prop (ducks).
    pub absorbed: usize,
    /// Most hats one ship wore.
    pub hats: u8,
    /// Awards at the end (Hat Baron).
    pub awards: usize,
}

/// Fixture name of an event's replay.
pub fn event_replay_name(kind: EventKind) -> String {
    format!("event/{}", kind.id())
}

/// The spectacle events (B3), in [`EventKind`] order.
pub const SPECTACLE_EVENTS: [EventKind; 5] = [
    EventKind::CatPaw,
    EventKind::TiredAnnouncer,
    EventKind::DuckParade,
    EventKind::HatRain,
    EventKind::DiscoInferno,
];

/// The events with a replay here: the built spectacle events, in
/// [`EventKind`] order.
pub fn replayed_events() -> Vec<EventKind> {
    EventKind::ALL
        .into_iter()
        .filter(|k| k.module().implemented() && SPECTACLE_EVENTS.contains(k))
        .collect()
}

fn rules() -> RuleSet {
    let (f, text) = BUILTIN_RULESETS
        .iter()
        .find(|(f, _)| f.ends_with("team_brawl.ron"))
        .expect("built-in rule set");
    let mut r = RuleSet::from_ron(f, text).expect("valid");
    r.mutators.clear();
    r.starting_loadout.secondaries = vec![("bolt_rocket".to_string(), 6)];
    r
}

/// Plays the replay of `kind` (see the module docs).
pub fn event_replay(kind: EventKind) -> EventReplay {
    let mut s = setup();
    s.routes = vec![EVENT_REPLAY_ROUTE.to_vec()];
    let mut cfg = WorldConfig::paint_the_void(EVENT_REPLAY_SEED, s);
    cfg.rules = rules();
    let mut w = World::new(cfg).expect("replay world");
    let mut t = arena();
    let mut ids: Vec<PlayerId> = Vec::new();
    for ty in ShipType::ALL.iter().cycle().take(4 * ShipType::ALL.len()) {
        if ids.len() == EVENT_REPLAY_SHIPS {
            break;
        }
        let team = Some((ids.len() % 2) as u8);
        if let Ok(id) = w.add_player(team, *ty) {
            ids.push(id);
        }
    }
    w.schedule_event(EVENT_REPLAY_START, kind);
    let mut rng = Pcg32::new(EVENT_REPLAY_SEED, kind.index() as u64 + 1);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let fold = |h: u64, v: u64| (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
    let mut r = EventReplay {
        kind,
        hash: 0,
        ticks: 0,
        events: 0,
        kills: 0,
        event_ticks: 0,
        props: 0,
        edits: 0,
        kill_cams: 0,
        absorbed: 0,
        hats: 0,
        awards: 0,
    };
    let mut ended = None;
    for tick in 1..=EVENT_REPLAY_MAX_TICKS {
        let mut input = inputs(&mut rng, &w, &ids, tick);
        if kind == EventKind::HatRain {
            hunt_hats(&w, &mut input);
        }
        for e in w.step(&mut t, &input) {
            r.events += 1;
            match e {
                Event::Killed { .. } => r.kills += 1,
                Event::TerrainEdit(TerrainEdit::SubtractSphere { .. }) => r.edits += 1,
                Event::Cue(c) if c.kind == CueKind::KillCam => r.kill_cams += 1,
                Event::Cue(c) if c.kind == CueKind::EventHit => r.absorbed += 1,
                Event::Director(crate::DirectorEvent::Ended { .. }) => ended = Some(tick),
                _ => {}
            }
        }
        r.ticks = tick;
        if w.mood_view().event.is_some() {
            r.event_ticks += 1;
        }
        let props = w
            .entities()
            .iter()
            .filter(|e| matches!(e.kind, EntityKind::Prop(_)))
            .count();
        r.props = r.props.max(props);
        r.hats = r.hats.max(w.ships().map(|s| s.hats).max().unwrap_or(0));
        if tick % 60 == 0 {
            hash = fold(hash, w.state_hash());
        }
        if !w.mode().is_running() || ended.is_some_and(|e| tick >= e + 60) {
            break;
        }
    }
    r.awards = w.results().awards.len();
    r.hash = fold(hash, w.state_hash());
    r
}

/// Hat rain replay: odd players fly at the nearest hat (the rest keep the
/// scripted inputs), so hats are worn, revealed and scattered.
fn hunt_hats(w: &World, input: &mut [(PlayerId, crate::InputCmd)]) {
    let hats: Vec<Vec3> = w
        .entities()
        .iter()
        .filter(|e| e.kind == EntityKind::Prop(crate::ReplicatedKind::Hat))
        .map(|e| e.pos)
        .collect();
    for (id, cmd) in input.iter_mut() {
        if id.0 % 2 == 0 {
            continue;
        }
        let Some(me) = w.ship(*id).filter(|s| s.alive) else {
            continue;
        };
        let Some(target) = hats.iter().copied().min_by(|a, b| {
            a.distance_squared(me.pos)
                .total_cmp(&b.distance_squared(me.pos))
        }) else {
            continue;
        };
        let local = me.orientation.conjugate().rotate(target - me.pos);
        cmd.look_pitch = ls_math::det::atan2(local.y, -local.z).clamp(-0.1, 0.1);
        cmd.look_yaw = ls_math::det::atan2(-local.x, -local.z).clamp(-0.1, 0.1);
        cmd.thrust = Vec3::new(0.0, 0.0, -1.0);
    }
}

/// Every event replay, in [`replayed_events`] order.
pub fn event_replays() -> Vec<EventReplay> {
    replayed_events().into_iter().map(event_replay).collect()
}

/// Fixture lines (`event/<id> <hash as 0x%016x>`) of every spectacle
/// replay.
pub fn spectacle_replay_hashes() -> Vec<String> {
    event_replays()
        .iter()
        .map(|r| format!("{} {:#018x}", event_replay_name(r.kind), r.hash))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const FIXTURE: &str = include_str!("../tests/fixtures/mood_replays.txt");

    /// An `event/` line of a spectacle event.
    fn ours(line: &str) -> bool {
        SPECTACLE_EVENTS
            .iter()
            .any(|k| line.starts_with(&format!("{} ", event_replay_name(*k))))
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
    /// and shows its event at work. `LS_BLESS_EVENT_REPLAYS=1` rewrites the
    /// `event/` lines of the fixture (native).
    #[test]
    fn event_replays_match_the_fixture() {
        let replays = event_replays();
        for r in &replays {
            println!("{r:?}");
            assert!(r.event_ticks > 180, "{:?} ran", r.kind);
        }
        let by = |k: EventKind| replays.iter().find(|r| r.kind == k);
        if let Some(r) = by(EventKind::DuckParade) {
            assert_eq!(r.props, 5);
            assert!(r.absorbed >= 1, "ducks absorbed shots");
        }
        if let Some(r) = by(EventKind::HatRain) {
            assert!(r.props >= 20, "hats rained");
            assert!(r.hats >= 1 && r.awards == 1, "a Hat Baron");
        }
        if let Some(r) = by(EventKind::CatPaw) {
            assert_eq!(r.props, 1);
        }
        if let Some(r) = by(EventKind::DiscoInferno) {
            assert_eq!(r.props, 1);
        }
        if let Some(r) = by(EventKind::TiredAnnouncer) {
            assert!(r.kill_cams >= 1 && r.kill_cams <= r.kills);
        }
        let got: Vec<String> = replays
            .iter()
            .map(|r| format!("{} {:#018x}", event_replay_name(r.kind), r.hash))
            .collect();
        #[cfg(not(target_arch = "wasm32"))]
        if std::env::var_os("LS_BLESS_EVENT_REPLAYS").is_some()
            || std::env::var_os("LS_BLESS_MOOD_REPLAYS").is_some()
        {
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

    #[test]
    fn event_replays_are_deterministic() {
        let a = event_replay(EventKind::CatPaw);
        let b = event_replay(EventKind::CatPaw);
        assert_eq!(a, b);
    }
}
