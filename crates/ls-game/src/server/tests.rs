use super::*;
use ls_math::Vec3;
use ls_worldgen::{ShareCode, Sliders};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use crate::convert::input_to_net;

/// Tests stay below tick 30 (the first paint coverage query), so the level
/// never has to be generated completely (slow in debug builds).
fn cfg() -> RoomConfig {
    RoomConfig {
        pregenerate: false,
        ..RoomConfig::default()
    }
}

fn room() -> Room {
    let code = ShareCode::new(77, Sliders::new(40, 30));
    Room::new(MatchConfig::paint_the_void(code), cfg()).unwrap()
}

fn input_msg(ticks: &[u64], thrust: Vec3, ack: Option<u32>) -> Message {
    Message::Input(InputMsg {
        ack_snapshot: ack,
        cmds: ticks
            .iter()
            .map(|&t| input_to_net(&InputCmd::idle(t).with_thrust(thrust)))
            .collect(),
    })
}

#[test]
fn join_leave_and_team_balance() {
    let mut r = room();
    let a = r.join(&Hello::new("a", Some(1))).unwrap();
    let b = r.join(&Hello::new("b", Some(1))).unwrap();
    let c = r.join(&Hello::new("c", None)).unwrap();
    let teams: Vec<u8> = r.clients().iter().map(|c| c.team).collect();
    // a gets its wish; b's wish would unbalance the teams; c goes to the
    // smaller team (ties go to team 0).
    assert_eq!(teams, vec![1, 0, 0]);
    assert_eq!((a, b, c), (0, 1, 2));
    // Every join queued a reliable Welcome with the right id.
    let out = r.take_outgoing();
    assert_eq!(out.len(), 3);
    for (i, o) in out.iter().enumerate() {
        assert_eq!(o.channel, Channel::Reliable);
        let w = ls_net::check_welcome(Message::decode(&o.bytes).unwrap()).unwrap();
        assert_eq!(w.player_id, i as u8);
        assert_eq!(w.quant, *r.conv().quantizer().config());
        assert_eq!(
            ShareCode::decode(&w.share_code).unwrap(),
            r.match_config().code
        );
    }
    assert!(r.leave(b));
    assert!(!r.leave(b));
    assert_eq!(r.world().ships().count(), 2);
    // The freed id is reused.
    assert_eq!(r.join(&Hello::new("d", None)).unwrap(), 1);
}

#[test]
fn full_room_rejects() {
    let code = ShareCode::new(77, Sliders::new(40, 30));
    let cfg = RoomConfig {
        max_players: 1,
        ..cfg()
    };
    let mut r = Room::new(MatchConfig::paint_the_void(code), cfg).unwrap();
    r.join(&Hello::new("a", None)).unwrap();
    let e = r.join(&Hello::new("b", None)).unwrap_err();
    assert_eq!(e.reason, RejectReason::ServerFull);
}

#[test]
fn late_duplicate_and_missing_inputs() {
    let mut r = room();
    let id = r.join(&Hello::new("a", None)).unwrap();
    let player = r.player_of(id).unwrap();
    let start = r.world().ship(player).unwrap().pos;
    let fwd = Vec3::new(0.0, 0.0, -1.0);
    // Inputs for ticks 1..=3 (with redundancy), then nothing: the room keeps
    // repeating the last input.
    r.handle_message(id, input_msg(&[1], fwd, None)).unwrap();
    r.handle_message(id, input_msg(&[3, 2, 1], fwd, None))
        .unwrap();
    assert_eq!(r.stats().duplicate_inputs, 1);
    for _ in 0..20 {
        r.step();
    }
    assert_eq!(r.stats().missing_inputs, 17);
    let moved = r.world().ship(player).unwrap().pos.distance(start);
    assert!(
        moved > 1.0,
        "ship kept thrusting with the repeated input: {moved}"
    );
    // A late input is counted and ignored.
    r.handle_message(id, input_msg(&[5], Vec3::ZERO, None))
        .unwrap();
    assert_eq!(r.stats().late_inputs, 1);
    // Far-future inputs are dropped.
    r.handle_message(id, input_msg(&[10_000], Vec3::ZERO, None))
        .unwrap();
    assert_eq!(r.stats().dropped_inputs, 1);
}

#[test]
fn no_missing_inputs_before_the_first_input() {
    let mut r = room();
    let id = r.join(&Hello::new("a", None)).unwrap();
    // The client is still setting up: nothing it predicted can diverge.
    for _ in 0..10 {
        r.step();
    }
    assert_eq!(r.stats().missing_inputs, 0);
    // From its first applied input on, gaps count.
    r.handle_message(id, input_msg(&[11, 12], Vec3::ZERO, None))
        .unwrap();
    for _ in 0..5 {
        r.step();
    }
    assert_eq!(r.stats().missing_inputs, 3);
}

#[test]
fn snapshots_at_30_hz_with_delta_after_ack() {
    let mut r = room();
    let id = r.join(&Hello::new("a", None)).unwrap();
    r.take_outgoing();
    let mut history = SnapshotHistory::default();
    let mut full = 0;
    let mut delta = 0;
    for tick in 1..=20u64 {
        let rep = r.step();
        assert_eq!(rep.snapshot, tick % 2 == 0);
        for o in r.take_outgoing() {
            if let Message::Snapshot(sm) = Message::decode_with(&o.bytes, &history).unwrap() {
                assert_eq!(sm.snapshot.tick as u64, tick);
                if sm.baseline.is_some() {
                    delta += 1;
                } else {
                    full += 1;
                }
                let own = sm.snapshot.ship(id).unwrap();
                assert!(own.status.is_some());
                let ack = sm.snapshot.tick;
                history.push(sm.snapshot);
                r.handle_message(id, input_msg(&[tick + 2], Vec3::ZERO, Some(ack)))
                    .unwrap();
            }
        }
    }
    assert_eq!(full, 1);
    assert_eq!(delta, 9);
}

#[test]
fn history_covers_250_ms() {
    let mut r = room();
    let id = r.join(&Hello::new("a", None)).unwrap();
    let p = r.player_of(id).unwrap();
    for _ in 0..28 {
        r.step();
    }
    let h = r.history();
    assert_eq!(h.newest_tick(), Some(28));
    assert_eq!(
        h.oldest_tick(),
        Some(28 - r.config().lag_comp_max_ticks - 1)
    );
    assert!(r.rewound_position(p, 20).is_some());
    assert!(r.rewound_position(p, 29).is_none());
}

#[test]
fn leave_drops_queued_packets_before_the_id_is_reused() {
    let mut r = room();
    let a = r.join(&Hello::new("a", None)).unwrap();
    r.take_outgoing();
    // A snapshot (and later events) queued for `a`, not yet taken by the host.
    r.step();
    r.step();
    assert!(r.outgoing.iter().any(|o| o.client == a));
    assert!(r.leave(a));
    let b = r.join(&Hello::new("b", None)).unwrap();
    assert_eq!(b, a, "the freed id is reused");
    let out = r.take_outgoing();
    assert_eq!(out.len(), 1, "only the new client's Welcome: {out:?}");
    assert!(matches!(
        Message::decode(&out[0].bytes).unwrap(),
        Message::Welcome(_)
    ));
}

#[test]
fn each_redundant_command_gets_its_own_rewind() {
    let mut r = room();
    let id = r.join(&Hello::new("a", None)).unwrap();
    for _ in 0..10 {
        r.step();
    }
    // Newest command first, as `InputSequencer` sends them; ack = tick 10.
    r.handle_message(id, input_msg(&[14, 13, 12], Vec3::ZERO, Some(10)))
        .unwrap();
    let view = r.view_tick(10);
    let c = &r.clients[&id];
    for t in [12u64, 13, 14] {
        assert_eq!(c.inputs[&t].rewind, (t as i64 - view) as u64, "tick {t}");
    }
    assert_eq!(c.inputs[&14].rewind, c.inputs[&12].rewind + 2);
}

#[test]
fn view_tick_is_half_a_snapshot_interval_past_ack_minus_interp() {
    let r = room();
    let interp = r.config().interp_delay_ticks as i64;
    assert_eq!(r.view_tick(100), 100 - interp + 1);
}

#[test]
fn unencodable_welcome_rejects_without_a_ghost_ship() {
    let code = ShareCode::new(77, Sliders::new(40, 30));
    let mut mc = MatchConfig::paint_the_void(code);
    // A rule set far over the wire limit for the Welcome.
    mc.rules.name = "x".repeat(200_000);
    let mut r = Room::new(mc, cfg()).unwrap();
    let e = r.join(&Hello::new("a", None)).unwrap_err();
    assert_eq!(e.reason, RejectReason::Other);
    assert_eq!(r.player_count(), 0);
    assert_eq!(r.world().ships().count(), 0);
    assert!(r.take_outgoing().is_empty());
    assert_eq!(r.stats().encode_errors, 1);
}
