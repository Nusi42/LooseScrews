//! Hot Reactor (M3 B4) end to end in a headless `Room`: 8 in-process bots
//! play a match on a generated level through the room's input path until
//! the mode ends with a result.
//!
//! The full match on the default rule set (`data/rulesets/hot_reactor.ron`)
//! runs in release builds only (`cargo test -p ls-game --release --test
//! hot_reactor_match`); a short-fuse, one-life variant runs everywhere.

#![cfg(all(feature = "server", not(target_arch = "wasm32")))]

use ls_game::convert::input_to_net;
use ls_game::ls_net::{InputMsg, Message};
use ls_game::ls_sim::mode::{player_flags, ModeKind};
use ls_game::ls_sim::modes::hot_reactor;
use ls_game::ls_sim::{Buttons, InputCmd, PlayerId, RuleSet, World};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::MatchConfig;

const BOTS: usize = 8;

/// A simple in-process bot: the holder chases the nearest ship to ram the
/// reactor onto it; everyone else hunts the nearest ship that is not the
/// holder (hitting the holder would take the reactor) and fires when it
/// roughly faces it, fleeing the holder when it comes close.
fn bot_input(w: &World, me: PlayerId, tick: u64, holder: Option<PlayerId>) -> InputCmd {
    let mut cmd = InputCmd::idle(tick);
    let Some(ship) = w.ship(me).filter(|s| s.alive) else {
        return cmd;
    };
    let nearest = |filter: &dyn Fn(PlayerId) -> bool| {
        w.ships()
            .filter(|s| s.alive && s.player != me && filter(s.player))
            .min_by(|a, b| {
                a.pos
                    .distance_squared(ship.pos)
                    .total_cmp(&b.pos.distance_squared(ship.pos))
            })
            .map(|s| s.pos)
    };
    let holding = holder == Some(me);
    let target = if holding {
        nearest(&|_| true)
    } else {
        nearest(&|p| Some(p) != holder)
    };
    let Some(target) = target else {
        return cmd;
    };
    let local = ship.orientation.conjugate().rotate(target - ship.pos);
    let pitch = ls_game::ls_math::det::atan2(local.y, -local.z).clamp(-0.1, 0.1);
    let yaw = ls_game::ls_math::det::atan2(-local.x, -local.z).clamp(-0.1, 0.1);
    let mut thrust = local.normalize_or_zero();
    // Others flee a holder within 12 u.
    if let Some(h) = holder.filter(|h| *h != me).and_then(|h| w.ship(h)) {
        let away = ship.pos - h.pos;
        if away.length() < 12.0 {
            thrust = ship
                .orientation
                .conjugate()
                .rotate(away)
                .normalize_or_zero();
        }
    }
    let facing = local.z < 0.0 && local.x.abs() < -local.z * 0.3 && local.y.abs() < -local.z * 0.3;
    let mut buttons = Buttons::HORIZON_ASSIST;
    buttons.set(Buttons::FIRE_PRIMARY, !holding && facing);
    buttons.set(
        Buttons::FIRE_SECONDARY,
        !holding && facing && tick % 90 == 0,
    );
    buttons.set(Buttons::BOOST, holding);
    cmd.thrust = thrust;
    cmd.look_pitch = pitch;
    cmd.look_yaw = yaw;
    cmd.buttons = buttons;
    cmd
}

struct Outcome {
    ticks: u64,
    detonations: u32,
    passes: u32,
    winner: Option<u8>,
    eliminated: usize,
}

/// Runs a match of `rules` with 8 bots until the room has results.
fn play(rules: RuleSet, seed: u64, max_ticks: u64) -> Outcome {
    let code = ShareCode::new(seed, Sliders::new(40, 30));
    let mut cfg = MatchConfig::paint_the_void(code);
    cfg.rules = rules;
    let mut room = Room::new(
        cfg,
        RoomConfig {
            pregenerate: false,
            ..RoomConfig::default()
        },
    )
    .unwrap();
    let clients: Vec<u8> = (0..BOTS)
        .map(|i| {
            room.join(&ls_game::client_hello(format!("bot{i}"), None, None))
                .unwrap()
        })
        .collect();
    let epoch = room.epoch();
    let mut passes = 0;
    let mut last_holder = None;
    for _ in 0..max_ticks {
        let tick = room.tick() + 1;
        let holder = hot_reactor::holder(room.world().mode());
        for &c in &clients {
            let player = room.player_of(c).unwrap();
            let cmd = bot_input(room.world(), player, tick, holder);
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
        room.step();
        room.take_outgoing();
        let now = hot_reactor::holder(room.world().mode());
        if last_holder.is_some() && now.is_some() && now != last_holder {
            passes += 1;
        }
        last_holder = now;
        if room.results().is_some() {
            let mode = room.world().mode();
            return Outcome {
                ticks: room.tick(),
                detonations: hot_reactor::detonations(mode),
                passes,
                winner: room.world().results().winner,
                eliminated: mode
                    .players
                    .values()
                    .filter(|p| p.flags & player_flags::ELIMINATED != 0)
                    .count(),
            };
        }
    }
    panic!("no result after {max_ticks} ticks");
}

fn check(o: &Outcome, lives: u32) {
    eprintln!(
        "hot reactor match: {} ticks, {} detonations, {} passes, {} eliminated, winner {:?}",
        o.ticks, o.detonations, o.passes, o.eliminated, o.winner
    );
    // The last player with lives won before the timer.
    assert_eq!(o.eliminated, BOTS - 1);
    assert!(o.winner.is_some());
    assert!(o.detonations >= (BOTS as u32 - 1) * lives);
    assert!(o.passes > 0, "the reactor changed hands");
}

#[test]
fn short_fuse_match_with_eight_bots_reaches_a_result() {
    let mut rules = RuleSet::default_for(ModeKind::HotReactor);
    rules.mode.set_field("lives", 1.0);
    rules.mode.set_field("fuse_secs", 4.0);
    rules.mode.set_field("next_reactor_secs", 1.0);
    let o = play(rules, 0x40_7e_ac, 60 * 120);
    check(&o, 1);
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: cargo test -p ls-game --release --test hot_reactor_match"
)]
fn default_rule_set_match_with_eight_bots_reaches_a_result() {
    let rules = RuleSet::default_for(ModeKind::HotReactor);
    let max = (rules.mode.match_secs() as u64 + 1) * 60;
    let o = play(rules, 0x40_7e_ad, max);
    check(&o, 3);
}
