//! A bot follows the next match on its open connection: a second `Welcome`
//! (another level, the next match epoch) rebuilds the bot in place, packets
//! of the old match still in flight are dropped, and it keeps playing the
//! new room without errors.
//!
//! Builds two levels, which is slow in debug builds, so it runs only in
//! release: `cargo test -p ls-bots --release`.

use std::time::Duration;

use ls_bots::{BotCore, BotStats, Flow};
use ls_game::ls_net::{check_welcome, Message};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::MatchConfig;

const SEED: u64 = 42;

fn room(code: ShareCode, epoch: u8) -> Room {
    let cfg = RoomConfig {
        epoch,
        ..RoomConfig::default()
    };
    Room::new(MatchConfig::paint_the_void(code), cfg).unwrap()
}

/// Steps `room` and the bot for `ticks` ticks (zero latency).
fn play(room: &mut Room, id: u8, core: &mut BotCore, stats: &mut BotStats, ticks: u32) {
    for _ in 0..ticks {
        for m in core.tick(stats) {
            room.handle_packet(id, &m.encode().unwrap()).unwrap();
        }
        room.step();
        for o in room.take_outgoing() {
            assert_eq!(o.client, id);
            match core.handle_packet(&o.bytes, stats) {
                Ok(Flow::Continue) => {}
                other => panic!("{other:?} {:?}", stats.errors),
            }
        }
    }
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "level generation is slow in debug builds; run with --release"
)]
fn second_welcome_moves_the_bot_to_the_next_match_in_place() {
    let hello = Message::Hello(ls_bots::bot_hello("bot1", SEED))
        .encode()
        .unwrap();
    let mut stats = BotStats::new("bot1");

    let code_a = ShareCode::new(0x00a1_0001, Sliders::new(40, 30));
    let mut a = room(code_a, 0);
    let id_a = a.join_packet(&hello).unwrap();
    let welcome = a.take_outgoing().remove(0);
    let w = check_welcome(Message::decode(&welcome.bytes).unwrap()).unwrap();
    let mut core = BotCore::from_welcome(&w, SEED).unwrap();
    assert!(core.wait_for_nav(Duration::from_secs(120)));
    play(&mut a, id_a, &mut core, &mut stats, 120);
    assert!(core.session().is_synced());

    // The next match: another level and epoch, same connection.
    let code_b = ShareCode::new(0x00b2_0002, Sliders::new(50, 40));
    let mut b = room(code_b, 1);
    let id_b = b.join_packet(&hello).unwrap();
    // A snapshot of the old match is still in flight behind the Welcome.
    a.step();
    a.step();
    let stale: Vec<Vec<u8>> = a.take_outgoing().into_iter().map(|o| o.bytes).collect();
    for o in b.take_outgoing() {
        assert_eq!(
            core.handle_packet(&o.bytes, &mut stats).unwrap(),
            Flow::Continue
        );
    }
    assert_eq!(stats.match_restarts, 1);
    assert_eq!(core.session().epoch(), 1);
    assert_eq!(core.session().share_code(), code_b.encode().unwrap());
    for bytes in &stale {
        assert_eq!(
            core.handle_packet(bytes, &mut stats).unwrap(),
            Flow::Continue
        );
    }
    assert!(core.session().stats().stale_epoch_snapshots >= 1);
    assert!(core.wait_for_nav(Duration::from_secs(120)));
    play(&mut b, id_b, &mut core, &mut stats, 240);
    assert!(core.session().is_synced());
    assert_eq!(b.stats().stale_epoch_inputs, 0);
    assert_eq!(stats.protocol_errors, 0, "{:?}", stats.errors);
    assert_eq!(stats.desyncs, 0, "{:?}", stats.errors);
}
