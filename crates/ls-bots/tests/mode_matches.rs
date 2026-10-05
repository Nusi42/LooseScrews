//! Every mode's default rule set played to a result by 8 bots (M3 package
//! C2): an authoritative `ls_game` `Room` and 8 [`BotCore`]s exchange
//! encoded packets with a fixed latency (2-5 ticks), stepped tick by tick
//! as in `room_play.rs`. Each test plays one full match with the built-in
//! rule set unchanged (its own timer, kill target, lives, deliveries) and
//! checks: a result, 0 desyncs, 0 protocol errors, every bot saw the end,
//! plus evidence that the bots play the mode's objective:
//!
//! - Team Brawl: a winner or a draw with kills on the board.
//! - The Squeeze: last ship flying; ships spend little time against the
//!   walls; eliminated bots spectate.
//! - Heist: the core is dug out and delivered.
//! - Hot Reactor: the reactor changes hands (passes besides detonations);
//!   eliminated bots spectate; the last player with lives wins.
//! - Last Color Standing: decided by kills before the stalemate drain.
//! - Paint the Void: both teams paint; the timer decides.
//!
//! `LS_BOTS_NO_OBJECTIVES=1` plays the same matches with the objective
//! layer switched off (deathmatch bots) and only prints the measurements,
//! for comparison.
//!
//! Needs whole-level generation, so it runs only in release:
//! `cargo test -p ls-bots --release --test mode_matches -- --nocapture`.

use std::collections::{BTreeMap, VecDeque};

use ls_bots::{BotCore, BotStats, BrainConfig, Flow};
use ls_game::ls_net::Message;
use ls_game::ls_sim::mode::{objective_kind, player_flags};
use ls_game::ls_sim::{MatchPhase, ModeKind, PlayerId, RuleSet};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::MatchConfig;

const BOTS: usize = 8;

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

/// Whether the objective layer is on (see the module docs).
fn objectives() -> bool {
    std::env::var("LS_BOTS_NO_OBJECTIVES").map_or(true, |v| v.is_empty() || v == "0")
}

/// One finished match.
struct Played {
    room: Room,
    /// Tick the mode stopped running.
    ended_at: u64,
    /// Per bot: its wire id and statistics.
    bots: Vec<(u8, BotStats)>,
    probe: Probe,
}

/// Per-tick measurements of the authoritative world.
#[derive(Default, Debug)]
struct Probe {
    /// Living ship ticks while walls exist (The Squeeze), and those of them
    /// touching or inside a wall.
    alive_ticks: u64,
    wall_ticks: u64,
    /// Living ship ticks while a next zone is announced (The Squeeze; not
    /// the final close to radius 0), and those of them outside it (where the
    /// closing wall will pass).
    announced_ticks: u64,
    exposed_ticks: u64,
    /// Objective holder changes (Heist core, Hot Reactor), by objective kind.
    holder_changes: BTreeMap<u8, u32>,
    last_holder: BTreeMap<u8, Option<PlayerId>>,
    /// Ship deaths per mode stage (The Squeeze: phase).
    deaths_by_stage: BTreeMap<u8, u32>,
    alive: Vec<PlayerId>,
}

impl Probe {
    fn observe(&mut self, room: &Room) {
        let world = room.world();
        let alive: Vec<PlayerId> = world
            .ships()
            .filter(|s| s.alive)
            .map(|s| s.player)
            .collect();
        let died = self.alive.iter().filter(|p| !alive.contains(p)).count() as u32;
        if died > 0 {
            *self.deaths_by_stage.entry(world.mode().stage).or_default() += died;
        }
        self.alive = alive;
        if !world.mode().is_running() {
            return;
        }
        if let Some(shell) = world.terrain_shell() {
            for s in world.ships().filter(|s| s.alive) {
                self.alive_ticks += 1;
                if s.pos.distance(shell.center) + 1.0 >= shell.radius {
                    self.wall_ticks += 1;
                }
            }
        }
        let next = world
            .mode()
            .objectives
            .iter()
            .find(|o| o.kind == objective_kind::NEXT_ZONE && o.radius >= 1.0);
        if let Some(z) = next {
            for s in world.ships().filter(|s| s.alive) {
                self.announced_ticks += 1;
                if s.pos.distance(z.pos) > z.radius {
                    self.exposed_ticks += 1;
                }
            }
        }
        for o in &world.mode().objectives {
            let last = self.last_holder.entry(o.kind).or_insert(None);
            if o.holder.is_some() && o.holder != *last {
                *self.holder_changes.entry(o.kind).or_default() += 1;
            }
            *last = o.holder;
        }
    }
}

/// Plays `rules` on `code` with [`BOTS`] bots until the mode has a result
/// (plus 3 s), at most `limit_secs`.
fn play(rules: RuleSet, code: ShareCode, limit_secs: u64) -> Played {
    let mc = MatchConfig {
        rules,
        ..MatchConfig::paint_the_void(code)
    };
    let kind = mc.rules.mode.kind();
    let mut room = Room::new(mc, RoomConfig::default()).unwrap();
    assert!(RoomConfig::default().max_players as usize >= BOTS);
    let brain = BrainConfig {
        objectives: objectives(),
        ..BrainConfig::default()
    };
    let mut clients: Vec<Client> = Vec::new();
    let mut probe = Probe::default();
    let mut ended_at = None;
    let mut now = 0u64;
    while ended_at.is_none_or(|e| now < e + 180) {
        assert!(
            now < limit_secs * 60,
            "{kind:?}: no result after {now} ticks"
        );
        // Staggered joins: one bot every 10 ticks, latencies 2-5 ticks.
        if now.is_multiple_of(10) && clients.len() < BOTS {
            let i = clients.len();
            let delay = 2 + (i as u64 % 4);
            let mut c = Client {
                id: None,
                core: None,
                stats: BotStats::new(format!("bot{}", i + 1)),
                up: Link::new(delay),
                down: Link::new(delay),
            };
            let hello = Message::Hello(ls_bots::bot_hello(c.stats.name.clone(), 700 + i as u64));
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
        probe.observe(&room);
        if ended_at.is_none() && !room.world().mode().is_running() {
            ended_at = Some(now);
        }
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
                        let core = BotCore::from_welcome_with(&w, 700 + i as u64, brain).unwrap();
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
        now += 1;
    }
    let bots = clients
        .iter_mut()
        .map(|c| {
            c.core.as_ref().expect("joined").finish(&mut c.stats);
            (c.id.expect("joined"), std::mem::take(&mut c.stats))
        })
        .collect();
    Played {
        room,
        ended_at: ended_at.unwrap(),
        bots,
        probe,
    }
}

/// The common checks and a summary line; returns the winner side.
fn check(name: &str, played: &Played) -> Option<u8> {
    let world = played.room.world();
    let MatchPhase::Ended { winner } = world.mode().phase else {
        panic!("{name}: no result");
    };
    let results = world.results();
    let kills: u32 = results.rows.iter().map(|r| r.kills).sum();
    let sum = |f: fn(&BotStats) -> u64| played.bots.iter().map(|(_, s)| f(s)).sum::<u64>();
    eprintln!(
        "{name}: ended at {:.1} s, winner {winner:?}, team scores {:?}, {kills} kills; \
         objective ticks {}, spectate ticks {}, dig shots {}, combat ticks {}, hunt ticks {}; {:?}",
        played.ended_at as f32 / 60.0,
        results.team_scores,
        sum(|s| s.behavior.objective_ticks),
        sum(|s| s.behavior.spectate_ticks),
        sum(|s| s.behavior.dig_shots),
        sum(|s| s.behavior.combat_ticks),
        sum(|s| s.behavior.hunt_ticks),
        played.probe,
    );
    for r in &results.rows {
        eprintln!(
            "  {:?} side {:>2}: {:>2} kills {:>2} deaths, score {}",
            r.player, r.side, r.kills, r.deaths, r.score
        );
    }
    assert_eq!(results.rows.len(), BOTS, "{name}");
    assert!(played.room.results().is_some(), "{name}: results sent");
    for (_, s) in &played.bots {
        assert_eq!(s.joins, 1, "{name} {}", s.name);
        assert_eq!(s.protocol_errors, 0, "{name} {}: {:?}", s.name, s.errors);
        assert_eq!(s.desyncs, 0, "{name} {}: {:?}", s.name, s.errors);
        assert_eq!(s.matches_over, 1, "{name} {}: saw the match end", s.name);
    }
    winner
}

/// Eliminated players are out of play, and their bots spectated.
fn check_spectators(name: &str, played: &Played) {
    let world = played.room.world();
    let mut eliminated = 0;
    for (id, p) in &world.mode().players {
        if p.flags & player_flags::ELIMINATED == 0 {
            continue;
        }
        eliminated += 1;
        assert!(
            !world.ship(*id).unwrap().alive,
            "{name}: {id:?} out of play"
        );
        let (_, bot) = played
            .bots
            .iter()
            .find(|(w, _)| Some(*w) == played.room.wire_id(*id))
            .expect("a bot per player");
        assert!(
            bot.behavior.spectate_ticks > 0 || !objectives(),
            "{name}: eliminated {} spectated",
            bot.name
        );
    }
    assert!(eliminated >= BOTS - 1, "{name}: {eliminated} eliminated");
}

fn builtin(kind: ModeKind) -> RuleSet {
    let rules = RuleSet::default_for(kind);
    assert_eq!(rules.mode.kind(), kind);
    // As a host would pass it: round-tripped through its code.
    RuleSet::from_code(&rules.to_code().unwrap()).unwrap()
}

fn world(seed: u64) -> ShareCode {
    ShareCode::new(seed, Sliders::new(40, 30))
}

#[test]
#[cfg_attr(debug_assertions, ignore = "full level generation; run with --release")]
fn team_brawl_full_match() {
    let rules = builtin(ModeKind::TeamBrawl);
    let played = play(rules, world(0x00c2_0001), 11 * 60);
    check("team_brawl", &played);
    let results = played.room.world().results();
    let kills: u32 = results.rows.iter().map(|r| r.kills).sum();
    assert!(kills >= 10, "the bots fought: {kills} kills");
}

#[test]
#[cfg_attr(debug_assertions, ignore = "full level generation; run with --release")]
fn the_squeeze_full_match() {
    let rules = builtin(ModeKind::TheSqueeze);
    let played = play(rules, world(0x00c2_0002), 16 * 60);
    let winner = check("the_squeeze", &played);
    let world = played.room.world();
    let alive = world.ships().filter(|s| s.alive).count();
    assert!(alive <= 1, "last ship flying: {alive} alive");
    assert_eq!(winner.is_some(), alive == 1);
    check_spectators("the_squeeze", &played);
    let p = &played.probe;
    let share = p.wall_ticks as f64 / p.alive_ticks.max(1) as f64;
    eprintln!(
        "the_squeeze: {:.1} % of living ship ticks at a wall",
        share * 100.0
    );
    if objectives() {
        let objective: u64 = played
            .bots
            .iter()
            .map(|(_, s)| s.behavior.objective_ticks)
            .sum();
        assert!(objective > 0, "bots went back to the safe zone");
        assert!(share < 0.05, "bots keep away from the walls: {share:.3}");
    }
}

/// The Squeeze without weapons: nobody can shoot, so only the walls decide.
/// Bots that keep to the safe zone outlive the phases; the last ones fall to
/// the final close.
#[test]
#[cfg_attr(debug_assertions, ignore = "full level generation; run with --release")]
fn the_squeeze_unarmed_bots_keep_to_the_safe_zone() {
    let mut rules = builtin(ModeKind::TheSqueeze);
    rules.weapons.clear();
    rules.gadgets.clear();
    rules.pickups.clear();
    rules.upgrade_pickups.clear();
    rules.gadget_pickups.clear();
    rules.starting_loadout.primaries.clear();
    rules.starting_loadout.secondaries.clear();
    rules.starting_loadout.gadget = None;
    let phases = rules
        .mode
        .fields()
        .iter()
        .find(|f| f.name == "phases")
        .unwrap()
        .value as u8;
    let played = play(rules, world(0x00c2_0012), 16 * 60);
    let winner = check("the_squeeze_unarmed", &played);
    check_spectators("the_squeeze_unarmed", &played);
    let p = &played.probe;
    let share = p.wall_ticks as f64 / p.alive_ticks.max(1) as f64;
    let exposed = p.exposed_ticks as f64 / p.announced_ticks.max(1) as f64;
    let early: u32 = p
        .deaths_by_stage
        .iter()
        .filter(|(stage, _)| **stage <= phases)
        .map(|(_, n)| n)
        .sum();
    eprintln!(
        "the_squeeze_unarmed: winner {winner:?}, {:.2} % of living ship ticks at a wall, \
         {:.1} % outside the announced next zone, deaths by stage {:?} ({early} before the final close)",
        share * 100.0,
        exposed * 100.0,
        p.deaths_by_stage
    );
    if objectives() {
        assert!(
            exposed < 0.1,
            "bots move into the next zone once announced: {exposed:.3}"
        );
        assert!(share < 0.02, "bots keep away from the walls: {share:.3}");
        assert!(
            early <= 2,
            "at most 2 of 8 bots caught by the phase walls: {early}"
        );
    }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "full level generation; run with --release")]
fn heist_full_match() {
    let rules = builtin(ModeKind::Heist);
    assert_eq!(rules.mode.teams(), 2);
    let played = play(rules, world(0x00c2_0003), 13 * 60);
    check("heist", &played);
    let results = played.room.world().results();
    let deliveries: i64 = results.team_scores.iter().sum();
    let pickups = played
        .probe
        .holder_changes
        .get(&objective_kind::CORE)
        .copied()
        .unwrap_or(0);
    eprintln!("heist: {deliveries} deliveries, core taken {pickups} times");
    if objectives() {
        let digs: u64 = played.bots.iter().map(|(_, s)| s.behavior.dig_shots).sum();
        assert!(digs > 0, "the bots dug at the core");
        assert!(pickups > 0, "the core was taken");
        assert!(deliveries > 0, "the core was delivered");
    }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "full level generation; run with --release")]
fn hot_reactor_full_match() {
    let rules = builtin(ModeKind::HotReactor);
    let played = play(rules, world(0x00c2_0004), 16 * 60);
    let winner = check("hot_reactor", &played);
    check_spectators("hot_reactor", &played);
    let world = played.room.world();
    let with_lives = world
        .mode()
        .players
        .values()
        .filter(|p| p.flags & player_flags::ELIMINATED == 0)
        .count();
    assert!(with_lives <= 1, "{with_lives} players with lives left");
    assert!(winner.is_some() || with_lives == 0);
    let changes = played
        .probe
        .holder_changes
        .get(&objective_kind::REACTOR)
        .copied()
        .unwrap_or(0);
    eprintln!("hot_reactor: the reactor changed hands {changes} times");
    if objectives() {
        // 3 lives x 7 eliminated players = at least 21 detonations, each a
        // new reactor; passes come on top.
        assert!(changes > 21, "the reactor was passed on: {changes}");
    }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "full level generation; run with --release")]
fn last_color_standing_full_match() {
    let rules = builtin(ModeKind::LastColorStanding);
    let stalemate = rules
        .mode
        .fields()
        .iter()
        .find(|f| f.name == "stalemate_secs")
        .unwrap()
        .value;
    let played = play(rules, world(0x00c2_0005), 16 * 60);
    let winner = check("last_color_standing", &played);
    check_spectators("last_color_standing", &played);
    let world = played.room.world();
    let with_colour: Vec<_> = world
        .mode()
        .players
        .iter()
        .filter(|(_, p)| p.meter > 0)
        .map(|(id, _)| world.ship(*id).unwrap().team)
        .collect();
    assert!(with_colour.len() <= 1, "{with_colour:?}");
    assert_eq!(winner, with_colour.first().copied());
    eprintln!(
        "last_color_standing: ended at {:.0} s, stalemate drain from {stalemate} s",
        played.ended_at as f32 / 60.0
    );
    if objectives() {
        assert!(
            (played.ended_at as f32) < stalemate * 60.0,
            "decided by kills before the stalemate drain"
        );
    }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "full level generation; run with --release")]
fn paint_the_void_full_match() {
    let rules = builtin(ModeKind::PaintTheVoid);
    let played = play(rules, world(0x00c2_0006), 6 * 60);
    check("paint_the_void", &played);
    let results = played.room.world().results();
    assert!(
        results.team_scores.iter().all(|s| *s > 0),
        "both teams painted: {:?}",
        results.team_scores
    );
}
