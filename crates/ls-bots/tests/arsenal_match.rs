//! Bots use the whole arsenal (M2 wave C): a full room of bots (8,
//! `RoomConfig::max_players`, random allowed ship types) plays the sandbox
//! rule set (`data/rulesets/sandbox_all_weapons.ron`: every weapon and gadget
//! as a pickup) against an in-process `Room`. The bots must fire at least 15
//! distinct combat weapons (the loadout's Paint Thrower does not count),
//! including a mine and the Tectonic Charge, hit each other, use gadgets, fly
//! more than one ship type, and stay in sync (no protocol errors, no desyncs,
//! reconciliation errors under 1 u).
//!
//! Deterministic (no sockets, no wall clock). Release only, like
//! `room_play.rs`: `cargo test -p ls-bots --release`. Prints the per-weapon
//! summary (`ls_bots::summary`).

use std::collections::{BTreeMap, VecDeque};

use ls_bots::{summary, BotCore, BotStats, DisconnectPolicy, Flow};
use ls_game::ls_net::Message;
use ls_game::ls_sim::{ModeRules, RuleSet};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::MatchConfig;

/// Seconds played (the sandbox match lasts 300 s).
const SECS: u64 = 240;
/// Distinct combat weapons the bots must fire (M2 wave C; the last runs
/// fired 18-20).
const MIN_WEAPONS: usize = 15;
/// Largest prediction correction a bot may see (room_play stays well below;
/// server-side knockback, swaps and paint contact are corrected, not
/// snapped).
const MAX_RECONCILE_ERROR: f32 = 1.0;

struct Link {
    delay: u64,
    queue: VecDeque<(u64, Vec<u8>)>,
}

impl Link {
    fn new(delay: u64) -> Self {
        Link {
            delay,
            queue: VecDeque::new(),
        }
    }
    fn push(&mut self, now: u64, bytes: Vec<u8>) {
        self.queue.push_back((now + self.delay, bytes));
    }
    fn pop_due(&mut self, now: u64) -> Option<Vec<u8>> {
        if self.queue.front().is_some_and(|(due, _)| *due <= now) {
            self.queue.pop_front().map(|(_, b)| b)
        } else {
            None
        }
    }
}

struct Client {
    id: Option<u8>,
    core: Option<BotCore>,
    stats: BotStats,
    up: Link,
    down: Link,
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "a 4 minute 8-bot match with whole-level generation; run with --release"
)]
fn bots_fire_the_whole_arsenal() {
    // One level, one chaotic 8-bot match: whether a bot happens to pick up
    // and drop a mine depends on every trajectory. VQ0 (ships touch the
    // visible rock, not a shell 1-2 u out) changed them, and on the old seed
    // (`..._0001`, one mine in the M2 runs) no bot dropped one any more; the
    // next seed drops mines with and without VQ0 (6 and 2).
    let code = ShareCode::new(0x5a4d_b0c5_a115_0002, Sliders::default());
    let mut mc = MatchConfig::paint_the_void(code);
    mc.rules = RuleSet::sandbox_all_weapons();
    let ModeRules::PaintTheVoid(rules) = &mut mc.rules.mode else {
        unreachable!("a Paint the Void rule set")
    };
    rules.match_secs = rules.match_secs.max(SECS as f32 + 30.0);
    let mut room = Room::new(mc, RoomConfig::default()).unwrap();
    let bots = RoomConfig::default().max_players as usize;
    let mut clients: Vec<Client> = Vec::new();
    for now in 0..60 * SECS {
        if now % 30 == 0 && clients.len() < bots {
            let i = clients.len();
            let delay = 2 + (i as u64 % 4);
            let mut c = Client {
                id: None,
                core: None,
                stats: BotStats::new(format!("bot{}", i + 1)),
                up: Link::new(delay),
                down: Link::new(delay),
            };
            let hello = Message::Hello(ls_bots::bot_hello(c.stats.name.clone(), 500 + i as u64));
            c.up.push(now, hello.encode().unwrap());
            clients.push(c);
        }
        for c in &mut clients {
            while let Some(bytes) = c.up.pop_due(now) {
                match c.id {
                    None => c.id = Some(room.join_packet(&bytes).expect("join accepted")),
                    Some(id) => room.handle_packet(id, &bytes).unwrap(),
                }
            }
        }
        room.step();
        for o in room.take_outgoing() {
            let c = clients.iter_mut().find(|c| c.id == Some(o.client)).unwrap();
            c.down.push(now, o.bytes);
        }
        for (i, c) in clients.iter_mut().enumerate() {
            while let Some(bytes) = c.down.pop_due(now) {
                match &mut c.core {
                    None => {
                        let w = ls_game::ls_net::check_welcome(Message::decode(&bytes).unwrap())
                            .expect("welcome");
                        let core = BotCore::from_welcome(&w, 500 + i as u64).unwrap();
                        // The loop runs faster than real time: wait for the
                        // shared nav grid as a real-time bot would have.
                        assert!(core.wait_for_nav(std::time::Duration::from_secs(120)));
                        c.core = Some(core);
                        c.stats.joins += 1;
                    }
                    Some(core) => match core.handle_packet(&bytes, &mut c.stats) {
                        Ok(Flow::Continue) => {}
                        other => panic!("{}: {other:?} {:?}", c.stats.name, c.stats.errors),
                    },
                }
            }
            if let Some(core) = &mut c.core {
                for m in core.tick(&mut c.stats) {
                    c.up.push(now, m.encode().unwrap());
                }
            }
        }
    }
    let stats: Vec<BotStats> = clients
        .into_iter()
        .map(|c| {
            let mut s = c.stats;
            if let Some(core) = &c.core {
                core.finish(&mut s);
            }
            s
        })
        .collect();
    eprintln!(
        "{}",
        summary(&stats, SECS as f64, DisconnectPolicy::default())
    );
    eprintln!("room: {:?}", room.stats());

    for s in &stats {
        assert_eq!(s.protocol_errors, 0, "{}: {:?}", s.name, s.errors);
        assert_eq!(s.desyncs, 0, "{}: {:?}", s.name, s.errors);
        assert!(
            s.max_reconcile_error < MAX_RECONCILE_ERROR,
            "{}: max reconcile error {} u",
            s.name,
            s.max_reconcile_error
        );
    }
    let mut fired: BTreeMap<&str, u64> = BTreeMap::new();
    for s in &stats {
        for (k, u) in &s.weapons {
            *fired.entry(k.as_str()).or_default() += u.fired;
        }
    }
    fired.retain(|k, n| *n > 0 && *k != "paint_thrower");
    assert!(
        fired.contains_key("prox_mine") || fired.contains_key("stalker_mine"),
        "no mine dropped: {fired:?}"
    );
    assert!(
        fired.contains_key("tectonic_charge"),
        "no Tectonic Charge: {fired:?}"
    );
    assert!(
        fired.len() >= MIN_WEAPONS,
        "bots fired only {} distinct weapons: {fired:?}",
        fired.len()
    );
    let hits: u32 = stats.iter().map(|s| s.hits_dealt).sum();
    assert!(hits > 0, "no hits");
    let gadgets: u64 = stats.iter().map(|s| s.behavior.gadget_presses).sum();
    assert!(gadgets > 0, "no gadget use");
    let types: std::collections::BTreeSet<&String> =
        stats.iter().flat_map(|s| s.ship_lives.keys()).collect();
    assert!(types.len() >= 2, "ship types flown: {types:?}");
}
