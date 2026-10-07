use super::*;
use ls_math::Vec3;
use ls_worldgen::{ShareCode, Sliders};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use crate::client_hello;
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
        epoch: 0,
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
    let a = r.join(&client_hello("a", Some(1), None)).unwrap();
    let b = r.join(&client_hello("b", Some(1), None)).unwrap();
    let c = r.join(&client_hello("c", None, None)).unwrap();
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
    assert_eq!(r.join(&client_hello("d", None, None)).unwrap(), 1);
}

#[test]
fn full_room_rejects() {
    let code = ShareCode::new(77, Sliders::new(40, 30));
    let cfg = RoomConfig {
        max_players: 1,
        ..cfg()
    };
    let mut r = Room::new(MatchConfig::paint_the_void(code), cfg).unwrap();
    r.join(&client_hello("a", None, None)).unwrap();
    let e = r.join(&client_hello("b", None, None)).unwrap_err();
    assert_eq!(e.reason, RejectReason::ServerFull);
}

#[test]
fn late_duplicate_and_missing_inputs() {
    let mut r = room();
    let id = r.join(&client_hello("a", None, None)).unwrap();
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
    let id = r.join(&client_hello("a", None, None)).unwrap();
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
    let id = r.join(&client_hello("a", None, None)).unwrap();
    r.take_outgoing();
    // The client's first input (it has built its session).
    r.handle_message(id, input_msg(&[2], Vec3::ZERO, None))
        .unwrap();
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

/// A client slow to acknowledge its first (full) snapshot gets no second
/// full one until [`FULL_SNAPSHOT_RETRY_TICKS`] passed: one full snapshot per
/// join or match transition (M3 soak); after the acknowledgement, deltas.
#[test]
fn one_full_snapshot_until_acknowledged_or_retry() {
    let mut r = room();
    let id = r.join(&client_hello("a", None, None)).unwrap();
    r.take_outgoing();
    r.handle_message(id, input_msg(&[2], Vec3::ZERO, None))
        .unwrap();
    let mut history = SnapshotHistory::default();
    let mut kinds = Vec::new();
    let step = |r: &mut Room, history: &mut SnapshotHistory, kinds: &mut Vec<(u64, bool)>| {
        let rep = r.step();
        for o in r.take_outgoing() {
            if let Message::Snapshot(sm) = Message::decode_with(&o.bytes, history).unwrap() {
                kinds.push((rep.tick, sm.baseline.is_none()));
                history.push(sm.snapshot);
            }
        }
    };
    // No acknowledgement (a client busy with its level, a slow link): one
    // full snapshot, then a retry once the retry time passed.
    let retry = u64::from(FULL_SNAPSHOT_RETRY_TICKS);
    for _ in 0..retry + 2 {
        step(&mut r, &mut history, &mut kinds);
    }
    assert_eq!(kinds, vec![(2, true), (2 + retry, true)], "{kinds:?}");
    // Acknowledged: deltas from now on.
    let acked = history.get(2 + retry as u32).map(|s| s.tick).unwrap();
    r.handle_message(id, input_msg(&[retry + 4], Vec3::ZERO, Some(acked)))
        .unwrap();
    kinds.clear();
    for _ in 0..6 {
        step(&mut r, &mut history, &mut kinds);
    }
    assert_eq!(kinds.len(), 3);
    assert!(kinds.iter().all(|&(_, full)| !full), "{kinds:?}");
    assert_eq!(r.stats().full_snapshots, 2);
}

/// A client that is still building its level (no input yet) gets no
/// snapshots: they would queue up unread, all full (nothing is acknowledged
/// yet), and cost one full snapshot, one starvation report and one
/// prediction snap each when the client finally handles them (M1 issue B).
#[test]
fn no_snapshots_until_the_first_input() {
    let mut r = room();
    let id = r.join(&client_hello("a", None, None)).unwrap();
    r.take_outgoing();
    let snapshots = |out: &[Outgoing]| {
        out.iter()
            .filter(|o| {
                matches!(
                    Message::decode_with(&o.bytes, &SnapshotHistory::default()),
                    Ok(Message::Snapshot(_))
                )
            })
            .count()
    };
    for _ in 0..16 {
        r.step();
    }
    assert_eq!(snapshots(&r.take_outgoing()), 0);
    assert_eq!(r.stats().snapshots_sent, 0);
    // The first input (late: the client stamped it for tick 4) starts them.
    r.handle_message(id, input_msg(&[4], Vec3::ZERO, None))
        .unwrap();
    r.step();
    r.step();
    assert_eq!(snapshots(&r.take_outgoing()), 1);
}

#[test]
fn history_covers_250_ms() {
    let mut r = room();
    let id = r.join(&client_hello("a", None, None)).unwrap();
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
    let a = r.join(&client_hello("a", None, None)).unwrap();
    r.take_outgoing();
    r.handle_message(a, input_msg(&[1], Vec3::ZERO, None))
        .unwrap();
    // A snapshot (and later events) queued for `a`, not yet taken by the host.
    r.step();
    r.step();
    assert!(r.outgoing.iter().any(|o| o.client == a));
    assert!(r.leave(a));
    let b = r.join(&client_hello("b", None, None)).unwrap();
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
    let id = r.join(&client_hello("a", None, None)).unwrap();
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
    let e = r.join(&client_hello("a", None, None)).unwrap_err();
    assert_eq!(e.reason, RejectReason::Other);
    assert_eq!(r.player_count(), 0);
    assert_eq!(r.world().ships().count(), 0);
    assert!(r.take_outgoing().is_empty());
    assert_eq!(r.stats().encode_errors, 1);
}

#[test]
fn data_hash_mismatch_is_rejected_with_a_clear_reason() {
    let mut r = room();
    assert_eq!(r.data_hash(), ls_sim::data_hash());
    let bad = Hello::new("old client", None).with_data_hash(ls_sim::data_hash() ^ 1);
    let e = r.join(&bad).unwrap_err();
    assert_eq!(e.reason, RejectReason::DataMismatch);
    assert!(e.message.contains("game data mismatch"), "{}", e.message);
    assert_eq!(r.player_count(), 0);
    // Through the raw packet path too.
    let bytes = Message::Hello(bad).encode().unwrap();
    assert_eq!(
        r.join_packet(&bytes).unwrap_err().reason,
        RejectReason::DataMismatch
    );
    // The Welcome carries the hash and the ship type.
    let id = r
        .join(&client_hello("a", None, Some(ls_sim::ShipType::Hauler)))
        .unwrap();
    let out = r.take_outgoing();
    let w = ls_net::check_welcome(Message::decode(&out[0].bytes).unwrap()).unwrap();
    assert_eq!(w.data_hash, ls_sim::data_hash());
    assert_eq!(w.ship_type, ls_sim::ShipType::Hauler.index());
    let p = r.player_of(id).unwrap();
    assert_eq!(
        r.world().ship(p).unwrap().ship_type,
        ls_sim::ShipType::Hauler
    );
}

#[test]
fn ship_choice_respects_the_rule_set() {
    let code = ShareCode::new(77, Sliders::new(40, 30));
    let mut mc = MatchConfig::paint_the_void(code);
    mc.rules.allowed_ship_types = vec![ls_sim::ShipType::Skitter, ls_sim::ShipType::Tug];
    let mut r = Room::new(mc, cfg()).unwrap();
    // Not allowed: falls back to the first allowed type (no Tinker here).
    let a = r
        .join(&client_hello("a", None, Some(ls_sim::ShipType::Hauler)))
        .unwrap();
    let pa = r.player_of(a).unwrap();
    assert_eq!(
        r.world().ship(pa).unwrap().ship_type,
        ls_sim::ShipType::Skitter
    );
    // SelectShip picks the type for the next life; disallowed ones are ignored.
    r.handle_message(a, Message::SelectShip { ship_type: 2 })
        .unwrap();
    assert_eq!(r.world().ship(pa).unwrap().next_ship_type, None);
    r.handle_message(a, Message::SelectShip { ship_type: 3 })
        .unwrap();
    assert_eq!(
        r.world().ship(pa).unwrap().next_ship_type,
        Some(ls_sim::ShipType::Tug)
    );
    assert_eq!(r.clients()[0].ship_type, ls_sim::ShipType::Skitter);
}

/// Snapshots carry players (names, ship types) and generic entities; a
/// cloaked enemy is left out for the other team but not for teammates.
#[test]
fn snapshots_carry_players_entities_and_hide_cloaked_enemies() {
    let code = ShareCode::new(77, Sliders::new(40, 30));
    let mut mc = MatchConfig::paint_the_void(code);
    mc.rules.starting_loadout.gadget = Some("cloak".into());
    mc.rules
        .gadgets
        .insert("cloak".into(), ls_sim::Availability::LoadoutAndPickup);
    let mut r = Room::new(mc, cfg()).unwrap();
    let a = r.join(&client_hello("alice", Some(0), None)).unwrap();
    let b = r.join(&client_hello("bob", Some(1), None)).unwrap();
    let c = r.join(&client_hello("carol", Some(0), None)).unwrap();
    r.take_outgoing();
    let pa = r.player_of(a).unwrap();
    let pb = r.player_of(b).unwrap();
    let pc = r.player_of(c).unwrap();
    // Put a far from b, next to c.
    r.world_mut().ship_mut(pa).unwrap().pos = Vec3::new(40.0, 60.0, 128.0);
    r.world_mut().ship_mut(pb).unwrap().pos = Vec3::new(200.0, 60.0, 128.0);
    r.world_mut().ship_mut(pc).unwrap().pos = Vec3::new(45.0, 60.0, 128.0);
    r.world_mut().spawn_entity(ls_sim::EntitySpawn::debris(
        Vec3::new(50.0, 60.0, 128.0),
        Vec3::ZERO,
        0.5,
        5.0,
    ));
    // a switches the cloak on (G, press edge).
    let g = |t: u64, on: bool| {
        let mut cmd = InputCmd::idle(t);
        if on {
            cmd = cmd.with_buttons(ls_sim::Buttons::GADGET);
        }
        Message::Input(InputMsg {
            epoch: 0,
            ack_snapshot: None,
            cmds: vec![input_to_net(&cmd)],
        })
    };
    r.handle_message(a, g(1, true)).unwrap();
    for id in [b, c] {
        r.handle_message(id, g(1, false)).unwrap();
    }
    r.step();
    r.step();
    assert!(r.world().ship(pa).unwrap().gadget_active);
    let mut seen: BTreeMap<u8, Snapshot> = BTreeMap::new();
    for o in r.take_outgoing() {
        if let Ok(Message::Snapshot(sm)) = Message::decode(&o.bytes) {
            seen.insert(o.client, r.conv().quantizer().unpack_snapshot(&sm.snapshot));
        }
    }
    let names: Vec<&str> = seen[&b].players.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["alice", "bob", "carol"]);
    assert!(seen[&b]
        .entities
        .iter()
        .any(|e| e.kind == ls_sim::ReplicatedKind::Debris as u8));
    assert!(
        seen[&b].ships.iter().all(|s| s.id != a),
        "cloaked enemy hidden"
    );
    assert!(seen[&c].ships.iter().any(|s| s.id == a), "teammate sees it");
    assert!(
        seen[&a].ships.iter().any(|s| s.id == a),
        "own ship always sent"
    );
    let own = seen[&c].ships.iter().find(|s| s.id == a).unwrap();
    assert!(own.flags & ls_net::ship_flags::GADGET_ACTIVE != 0);
    let cloak = r.world().gadgets().find("cloak").unwrap().0;
    assert_eq!(seen[&c].players[0].gadget, cloak);
}

/// The own inventory is sent reliably when it changes (and only then), and
/// moving debris far from a viewer is left out of its snapshots.
#[test]
fn inventory_messages_and_far_moving_debris() {
    let mut r = room();
    let a = r.join(&client_hello("a", Some(0), None)).unwrap();
    r.take_outgoing();
    let pa = r.player_of(a).unwrap();
    r.world_mut().ship_mut(pa).unwrap().pos = Vec3::new(40.0, 60.0, 128.0);
    r.handle_message(a, input_msg(&[1], Vec3::ZERO, None))
        .unwrap();
    let inventories = |r: &mut Room| -> Vec<ls_net::InventoryMsg> {
        r.take_outgoing()
            .iter()
            .filter_map(|o| match Message::decode(&o.bytes) {
                Ok(Message::Inventory(inv)) => {
                    assert_eq!(o.channel, Channel::Reliable);
                    Some(inv)
                }
                _ => None,
            })
            .collect()
    };
    r.step();
    r.step();
    // The first (full) snapshot, kept as the baseline of later ones.
    let out = r.take_outgoing();
    let mut history = SnapshotHistory::default();
    for o in &out {
        if let Ok(Message::Snapshot(sm)) = Message::decode(&o.bytes) {
            history.push(sm.snapshot);
        }
    }
    let first: Vec<ls_net::InventoryMsg> = out
        .iter()
        .filter_map(|o| match Message::decode(&o.bytes) {
            Ok(Message::Inventory(inv)) => Some(inv),
            _ => None,
        })
        .collect();
    assert_eq!(first.len(), 1, "sent once the client is ready");
    // The client acknowledges the first (full) snapshot, so more follow.
    r.handle_message(a, input_msg(&[3], Vec3::ZERO, Some(2)))
        .unwrap();
    let ship = r.world().ship(pa).unwrap();
    assert_eq!(first[0].weapons.len(), ship.weapons.len());
    assert_eq!(first[0].primary, ship.primary.unwrap().0);
    r.step();
    r.step();
    assert!(inventories(&mut r).is_empty(), "unchanged: not sent again");
    r.world_mut().give_weapon(pa, "bolt_rocket").unwrap();
    let rocket = r.world().weapons().find("bolt_rocket").unwrap();
    // Moving debris near and far from the viewer.
    let near = r.world_mut().spawn_entity(ls_sim::EntitySpawn::debris(
        Vec3::new(50.0, 60.0, 128.0),
        Vec3::new(3.0, 0.0, 0.0),
        0.5,
        5.0,
    ));
    let far = r.world_mut().spawn_entity(ls_sim::EntitySpawn::debris(
        Vec3::new(200.0, 60.0, 128.0),
        Vec3::new(3.0, 0.0, 0.0),
        0.5,
        5.0,
    ));
    r.step();
    let out = r.take_outgoing();
    let snap = out
        .iter()
        .find_map(|o| match Message::decode_with(&o.bytes, &history) {
            Ok(Message::Snapshot(sm)) => Some(r.conv().quantizer().unpack_snapshot(&sm.snapshot)),
            _ => None,
        });
    r.step();
    let out2 = r.take_outgoing();
    let snap = snap.or_else(|| {
        out2.iter()
            .find_map(|o| match Message::decode_with(&o.bytes, &history) {
                Ok(Message::Snapshot(sm)) => {
                    Some(r.conv().quantizer().unpack_snapshot(&sm.snapshot))
                }
                _ => None,
            })
    });
    let inv: Vec<ls_net::InventoryMsg> = out
        .iter()
        .chain(&out2)
        .filter_map(|o| match Message::decode(&o.bytes) {
            Ok(Message::Inventory(inv)) => Some(inv),
            _ => None,
        })
        .collect();
    assert_eq!(inv.len(), 1, "the new weapon is reported");
    assert!(inv[0].weapons.iter().any(|w| w.weapon == rocket.0));
    let snap = snap.expect("a snapshot");
    let ids: Vec<u16> = snap.entities.iter().map(|e| e.id).collect();
    assert!(ids.contains(&((near & 0x7fff) as u16)), "{ids:?}");
    assert!(
        !ids.contains(&((far & 0x7fff) as u16)),
        "far moving debris culled"
    );
    let d = snap
        .entities
        .iter()
        .find(|e| e.id == (near & 0x7fff) as u16)
        .unwrap();
    assert_eq!(d.vel, Vec3::ZERO, "debris is sent without velocity");
}

/// A client session fed hand-made snapshots: the own ship at rest, plus
/// optional extras per snapshot.
fn session_with(
    extra: impl Fn(u32, &mut Snapshot),
) -> (crate::client::ClientSession, Vec<Snapshot>) {
    let mut r = room();
    let id = r.join(&client_hello("a", Some(0), None)).unwrap();
    let welcome = r
        .take_outgoing()
        .iter()
        .find_map(|o| ls_net::check_welcome(Message::decode(&o.bytes).ok()?).ok())
        .unwrap();
    let mut s =
        crate::client::ClientSession::from_welcome_with(&welcome, ls_sim::GameData::builtin())
            .unwrap();
    let pid = r.player_of(id).unwrap();
    let ship = r.world().ship(pid).unwrap().clone();
    let tick0 = welcome.server_tick + 2;
    let mut sent = Vec::new();
    for k in 0..3u32 {
        let tick = tick0 + 2 * k;
        let mut snap = Snapshot {
            tick,
            ack_input: tick + 4,
            ships: vec![r.conv().ship_to_net(id, &ship, true)],
            match_ticks_left: 9000,
            ..Default::default()
        };
        extra(tick, &mut snap);
        let packed = s.conv().quantizer().pack_snapshot(&snap);
        s.handle_message(Message::Snapshot(Box::new(SnapshotMsg {
            epoch: 0,
            baseline: None,
            snapshot: packed,
        })))
        .unwrap();
        sent.push(snap);
    }
    (s, sent)
}

#[test]
fn guided_projectiles_freeze_the_predicted_ship_and_entities_interpolate() {
    let thrust = |s: &mut crate::client::ClientSession| {
        let cmd = InputCmd::idle(0).with_thrust(Vec3::new(0.0, 0.0, -1.0));
        for _ in 0..20 {
            s.tick(cmd);
        }
        s.local_ship().unwrap().vel.length()
    };
    let (mut free, _) = session_with(|_, _| {});
    let me = free.player();
    let (mut guiding, _) = session_with(|_, snap| {
        snap.projectiles.push(ls_net::NetProjectile {
            id: 7,
            kind: 0,
            owner: me,
            pos: Vec3::new(100.0, 60.0, 100.0),
            vel: Vec3::new(0.0, 0.0, -20.0),
            state: crate::client::GUIDED_STATE_BIT,
            chicken: false,
        });
    });
    assert!(free.is_synced() && guiding.is_synced());
    assert!(thrust(&mut free) > 1.0, "thrust moves a free ship");
    assert!(
        thrust(&mut guiding) < 1e-3,
        "a ship guiding a missile gets no thrust (like the server)"
    );

    // An entity moving between snapshots is drawn in between at the render
    // tick, and holds at the newest snapshot beyond it.
    let (s, sent) = session_with(|tick, snap| {
        snap.entities.push(ls_net::NetEntity {
            id: 5,
            kind: ls_sim::ReplicatedKind::Debris as u8,
            visual: 0,
            owner: 255,
            team: 255,
            pos: Vec3::new(100.0 + (tick % 100) as f32, 60.0, 100.0),
            vel: Vec3::ZERO,
            rot: ls_math::Quat::IDENTITY,
            state: [1, 0, 5, 0],
        });
    });
    let (t1, t2) = (sent[1].tick as f64, sent[2].tick as f64);
    let x = |t: f64| s.entities_at(t)[0].pos.x;
    let (x1, x2) = (x(t1), x(t2));
    assert!((x2 - x1 - 2.0).abs() < 0.1, "{x1} {x2}");
    assert!((x((t1 + t2) / 2.0) - (x1 + 1.0)).abs() < 0.1);
    assert_eq!(x(t2 + 30.0), x2);
}

/// Server-side enforcement through the room: a banned weapon smuggled into
/// a ship never fires, and the rule set never offers it as a pickup.
#[test]
fn banned_weapons_are_enforced_by_the_room() {
    let code = ShareCode::new(77, Sliders::new(40, 30));
    let mut mc = MatchConfig::paint_the_void(code);
    mc.rules
        .weapons
        .insert("bolt_rocket".into(), ls_sim::Availability::Banned);
    mc.rules.pickups.remove("bolt_rocket");
    mc.rules.starting_loadout.secondaries.clear();
    let mut r = Room::new(mc, cfg()).unwrap();
    let rocket = r.world().weapons().find("bolt_rocket").unwrap();
    assert!(r
        .world()
        .resolved_rules()
        .pickup_table
        .iter()
        .all(|e| !matches!(
            e.kind,
            ls_sim::PickupKind::Weapon { weapon, .. } if weapon == rocket
        )));
    let a = r.join(&client_hello("a", None, None)).unwrap();
    let pa = r.player_of(a).unwrap();
    {
        let s = r.world_mut().ship_mut(pa).unwrap();
        s.add_weapon(ls_sim::WeaponState {
            weapon: rocket,
            ammo: 5,
            level: 1,
            shots: 0,
        });
        s.secondary = Some(rocket);
    }
    let fire = |t: u64| {
        Message::Input(InputMsg {
            epoch: 0,
            ack_snapshot: None,
            cmds: vec![input_to_net(
                &InputCmd::idle(t).with_buttons(ls_sim::Buttons::FIRE_SECONDARY),
            )],
        })
    };
    for t in 1..=25u64 {
        r.handle_message(a, fire(t)).unwrap();
        let rep = r.step();
        assert!(!rep.events.iter().any(|e| matches!(e, Event::Fired { .. })));
    }
    for sp in r.world().pickups() {
        if let Some((_, ls_sim::PickupKind::Weapon { weapon, .. })) = sp.content {
            assert_ne!(weapon, rocket);
        }
    }
}

#[test]
fn lag_compensation_undo_follows_swaps_and_rewinds() {
    use super::lagcomp::Displacement;
    use super::restore_offsets;
    use ls_sim::{Cue, CueKind};
    let (a, b, c) = (PlayerId(1), PlayerId(2), PlayerId(3));
    let off = Vec3::new(3.0, 0.0, 0.0);
    let comp = [
        Displacement {
            player: b,
            offset: off,
            projectile: 7,
        },
        Displacement {
            player: c,
            offset: Vec3::new(0.0, 1.0, 0.0),
            projectile: 8,
        },
        Displacement {
            player: c,
            offset: Vec3::new(0.0, 1.0, 0.0),
            projectile: 9,
        },
    ];
    let cue = |kind, source, target| {
        Event::Cue(Cue {
            kind,
            source: Some(source),
            target: Some(target),
            pos: Vec3::ZERO,
            value: 0,
        })
    };
    // No teleport: every compensated ship goes back by its offsets.
    let plain = restore_offsets(&comp, &[]);
    assert_eq!(plain, vec![(b, -off), (c, Vec3::new(0.0, -2.0, 0.0))]);
    // a harpoons the compensated b: a now stands where b appeared (b's
    // present position + off) and takes b's correction; b stands at a's
    // present position and needs none.
    let swapped = restore_offsets(&comp, &[cue(CueKind::Swapped, a, b)]);
    assert_eq!(swapped, vec![(a, -off), (c, Vec3::new(0.0, -2.0, 0.0))]);
    // A rewound ship keeps its recorded position.
    let rewound = restore_offsets(&comp, &[cue(CueKind::Rewound, c, c)]);
    assert_eq!(rewound, vec![(b, -off)]);
}

/// A late joiner's Welcome leaves out the surface-state edits that expired
/// before it joined, keeps the sequence numbers, and builds the same
/// terrain as the server's (hashes at the join tick and later).
#[test]
fn welcome_prunes_expired_surface_edits() {
    use ls_math::terrain::{SurfaceStates, TerrainEdit, TerrainQuery};
    let mut r = room();
    let c = r.terrain().bounds().center();
    let edits = [
        TerrainEdit::surface_for_default(c, 3.0, SurfaceStates::HOT, 0),
        TerrainEdit::SubtractSphere {
            center: c + Vec3::X * 20.0,
            radius: 2.0,
        },
        TerrainEdit::scorch(c + Vec3::Y * 8.0, 2.0, 0, 5),
        TerrainEdit::surface_for_default(c + Vec3::Z * 9.0, 3.0, SurfaceStates::GLOW, 0),
        // Hot for 5 ticks far from everything else: expired before the
        // join, so the Welcome leaves it out entirely.
        TerrainEdit::Surface {
            center: c - Vec3::Z * 30.0,
            radius: 2.0,
            states: SurfaceStates::HOT.bits(),
            until: 5,
        },
    ];
    for e in edits {
        r.terrain_mut().apply(e);
    }
    for _ in 0..20 {
        r.step();
    }
    r.take_outgoing();
    let now = r.tick();
    assert!(now > 10 && now < 600, "{now}");
    let id = r.join(&client_hello("late", None, None)).unwrap();
    let out = r.take_outgoing();
    let w = ls_net::check_welcome(Message::decode(&out[0].bytes).unwrap()).unwrap();
    assert_eq!(out[0].client, id);
    assert_eq!(w.edit_count, 5, "the true count");
    // Hot 10 s from tick 0 still runs; the 5-tick heat of the scorch mark
    // has expired (its scorch stays); the glow runs for 30 s; the short
    // hot edit is gone.
    assert_eq!(w.edit_log.len(), 4, "shortened log");
    assert_eq!(
        w.edit_log[2],
        TerrainEdit::Surface {
            center: c + Vec3::Y * 8.0,
            radius: 2.0,
            states: SurfaceStates::SCORCH.bits(),
            until: 5,
        }
    );
    let s =
        crate::client::ClientSession::from_welcome_with(&w, ls_sim::GameData::builtin()).unwrap();
    let mut s = s;
    assert_eq!(
        s.edit_count(),
        5,
        "sequence numbers line up with the server"
    );
    assert_eq!(s.terrain().state_hash(), r.terrain().state_hash());
    for tick in [now, now + 100, 700, 2000] {
        assert_eq!(
            s.terrain().state_hash_at(tick),
            r.terrain().state_hash_at(tick)
        );
    }
    // Later edits keep both in step (the late joiner applies them on top of
    // the shortened log).
    let later = [
        TerrainEdit::SubtractSphere {
            center: c - Vec3::Z * 30.0,
            radius: 1.5,
        },
        TerrainEdit::scorch(c + Vec3::X * 5.0, 1.0, now, 60),
    ];
    for e in later {
        r.terrain_mut().apply(e);
        s.terrain_mut().apply(e);
    }
    assert_eq!(s.terrain().state_hash(), r.terrain().state_hash());
    for tick in [now + 30, now + 100, 2000] {
        assert_eq!(
            s.terrain().state_hash_at(tick),
            r.terrain().state_hash_at(tick)
        );
    }
}

#[test]
fn welcome_epoch_room_code_lobby_requests_and_stale_inputs() {
    let code = ShareCode::new(77, Sliders::new(40, 30));
    let cfg = RoomConfig { epoch: 5, ..cfg() };
    let mut r = Room::new(MatchConfig::paint_the_void(code), cfg).unwrap();
    r.set_room_code("R-ABC");
    let a = r.join(&client_hello("a", None, None)).unwrap();
    let b = r.join(&client_hello("b", None, None)).unwrap();
    let welcomes: Vec<Welcome> = r
        .take_outgoing()
        .iter()
        .filter_map(|o| ls_net::check_welcome(Message::decode(&o.bytes).ok()?).ok())
        .collect();
    assert_eq!(welcomes.len(), 2);
    for w in &welcomes {
        assert_eq!((w.epoch, w.room_code.as_str()), (5, "R-ABC"));
    }
    // Lobby requests are queued for the host, in order.
    for m in [
        Message::Ready { ready: true },
        Message::PickTeam { team: Some(1) },
        Message::Spectate { spectate: true },
        Message::PickTeam { team: None },
    ] {
        r.handle_packet(a, &m.encode().unwrap()).unwrap();
    }
    assert_eq!(
        r.take_lobby_requests(),
        vec![
            (a, LobbyRequest::Ready(true)),
            (a, LobbyRequest::PickTeam(Some(1))),
            (a, LobbyRequest::Spectate(true)),
            (a, LobbyRequest::PickTeam(None)),
        ]
    );
    assert!(r.take_lobby_requests().is_empty());
    // An input of another epoch (the previous match's) is dropped and does
    // not make the client ready for snapshots.
    let mut stale = input_msg(&[1, 0], Vec3::ZERO, None);
    if let Message::Input(m) = &mut stale {
        m.epoch = 4;
    }
    r.handle_message(b, stale).unwrap();
    assert_eq!(r.stats().stale_epoch_inputs, 1);
    let mut fresh = input_msg(&[2, 1], Vec3::ZERO, None);
    if let Message::Input(m) = &mut fresh {
        m.epoch = 5;
    }
    r.handle_message(a, fresh).unwrap();
    r.step();
    r.step();
    let snaps: Vec<(u8, u8)> = r
        .take_outgoing()
        .iter()
        .filter_map(|o| match Message::decode(&o.bytes) {
            Ok(Message::Snapshot(sm)) => Some((o.client, sm.epoch)),
            _ => None,
        })
        .collect();
    assert_eq!(snaps, vec![(a, 5)], "only the client with a current input");
    // Lobby messages from the host: to one client or to all.
    let lobby = Message::LobbyState(Box::new(ls_net::LobbyStateMsg {
        phase: ls_net::LobbyPhase::Countdown,
        phase_ticks_left: 600,
        ..Default::default()
    }));
    assert!(r.send(b, &lobby));
    assert!(!r.send(30, &lobby), "unknown client");
    r.broadcast(&lobby);
    let to: Vec<u8> = r.take_outgoing().iter().map(|o| o.client).collect();
    assert_eq!(to, vec![b, a, b]);
}

#[test]
fn inventory_carries_shots_and_weapon_state() {
    let mut r = room();
    let a = r.join(&client_hello("a", Some(0), None)).unwrap();
    r.take_outgoing();
    let pa = r.player_of(a).unwrap();
    // Not in this rule set's pickups: carried as if picked up elsewhere.
    let paint = r.world().weapons().find("paint_thrower").unwrap();
    {
        let ship = r.world_mut().ship_mut(pa).unwrap();
        ship.weapons.push(ls_sim::WeaponState {
            weapon: paint,
            ammo: 0,
            level: 1,
            shots: 0,
        });
        ship.weapon_state
            .insert(paint, ls_math::terrain::PAINT_BLUE as u32);
        for w in &mut ship.weapons {
            w.shots = 1000 + w.weapon.0 as u32;
        }
    }
    r.handle_message(a, input_msg(&[1], Vec3::ZERO, None))
        .unwrap();
    r.step();
    r.step();
    let inv = r
        .take_outgoing()
        .iter()
        .find_map(|o| match Message::decode(&o.bytes) {
            Ok(Message::Inventory(inv)) => Some(inv),
            _ => None,
        })
        .expect("inventory");
    let pw = inv.weapons.iter().find(|w| w.weapon == paint.0).unwrap();
    assert_eq!(pw.state, ls_math::terrain::PAINT_BLUE);
    for w in &inv.weapons {
        assert_eq!(w.shots, 1000 + w.weapon as u32);
    }
    // A shot count change alone is reported (the predictor's fan phase).
    r.world_mut().ship_mut(pa).unwrap().weapons[0].shots += 1;
    r.step();
    r.step();
    assert!(r
        .take_outgoing()
        .iter()
        .any(|o| matches!(Message::decode(&o.bytes), Ok(Message::Inventory(_)))));
}

#[test]
fn mode_state_in_snapshots_and_results_once_at_the_end() {
    let mut r = room();
    let a = r.join(&client_hello("a", Some(0), None)).unwrap();
    let welcome = r
        .take_outgoing()
        .iter()
        .find_map(|o| ls_net::check_welcome(Message::decode(&o.bytes).ok()?).ok())
        .unwrap();
    let mut s =
        crate::client::ClientSession::from_welcome_with(&welcome, ls_sim::GameData::builtin())
            .unwrap();
    // Delivers to the client, which acknowledges its newest snapshot.
    let deliver = |r: &mut Room, s: &mut crate::client::ClientSession| {
        for o in r.take_outgoing() {
            s.handle_packet(&o.bytes).unwrap();
        }
        let ack = s.latest_snapshot().map(|l| l.tick);
        let next = r.tick() + 1;
        r.handle_message(a, input_msg(&[next], Vec3::ZERO, ack))
            .unwrap();
    };
    r.handle_message(a, input_msg(&[1], Vec3::ZERO, None))
        .unwrap();
    for _ in 0..4 {
        r.step();
    }
    deliver(&mut r, &mut s);
    let m = s.mode().clone();
    assert_eq!(m.kind, ls_sim::ModeKind::PaintTheVoid.index());
    assert_eq!(m.phase, ls_net::mode_phase::RUNNING);
    assert_eq!(m.teams, 2);
    assert_eq!(m.team_scores.len(), 2);
    assert_eq!(m.players.len(), 1);
    assert_eq!(m.players[0].id, a);
    assert_eq!(m, r.net_mode_state());
    assert!(r.results().is_none());
    // The mode ends early (as a kill target would): results go out once.
    let end = r.world().tick();
    r.world_mut().mode_mut().phase = ls_sim::MatchPhase::Ended { winner: Some(0) };
    r.step();
    let results: Vec<ls_net::MatchResultsMsg> = r
        .take_outgoing()
        .iter()
        .filter_map(|o| match Message::decode(&o.bytes) {
            Ok(Message::MatchResults(m)) => Some(*m),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].winner, 0);
    assert_eq!(results[0].rows.len(), 1);
    assert_eq!(results[0].rows[0].name, "a");
    assert_eq!(Some(&results[0]), r.results());
    for _ in 0..3 {
        r.step();
    }
    deliver(&mut r, &mut s);
    // Snapshots say where the match ended; the client freezes there.
    assert_eq!(s.mode().phase, ls_net::mode_phase::ENDED);
    assert_eq!(s.mode().winner, 0);
    assert_eq!(s.mode().end_tick as u64, end);
    assert_eq!(s.match_end_tick(), Some(end));
    assert!(r
        .take_outgoing()
        .iter()
        .all(|o| !matches!(Message::decode(&o.bytes), Ok(Message::MatchResults(_)))));
    // A player joining the finished match gets the results after the Welcome.
    let b = r.join(&client_hello("b", None, None)).unwrap();
    let kinds: Vec<&'static str> = r
        .take_outgoing()
        .iter()
        .filter(|o| o.client == b)
        .map(|o| Message::decode(&o.bytes).unwrap().name())
        .collect();
    assert_eq!(kinds, vec!["Welcome", "MatchResults"]);
}

/// Only the current architect's requests are queued (bounded, attributed);
/// everyone else's are refused at once, so a runner flooding the room
/// cannot crowd the architect's strokes out of the queue (D1 review).
#[test]
fn architect_actions_are_queued_bounded_and_attributed() {
    let code = ShareCode::new(78, Sliders::new(40, 30));
    let mut mc = MatchConfig::paint_the_void(code);
    mc.rules = ls_sim::RuleSet::default_for(ls_sim::ModeKind::TheArchitect);
    let mut r = Room::new(mc, cfg()).unwrap();
    let a = r.join(&client_hello("a", None, None)).unwrap();
    let b = r.join(&client_hello("b", None, None)).unwrap();
    r.step();
    let pa = r.player_of(a).unwrap();
    assert_eq!(
        r.world().mode().architect.as_deref().unwrap().architect,
        Some(pa)
    );
    let action = |stroke: u8| ls_net::ArchitectActionMsg {
        stroke,
        center: Vec3::new(10.0, 20.0, 30.0),
        radius: 16.0,
    };
    r.handle_packet(a, &Message::ArchitectAction(action(2)).encode().unwrap())
        .unwrap();
    r.handle_packet(b, &Message::ArchitectAction(action(5)).encode().unwrap())
        .unwrap();
    assert_eq!(r.take_architect_actions(), vec![(pa, action(2))]);
    assert_eq!(r.stats().rejected_architect_actions, 1);
    assert!(r.take_architect_actions().is_empty());
    // A runner's flood is refused without touching the queue; the
    // architect's stroke after it is still queued.
    for _ in 0..crate::server::MAX_QUEUED_ARCHITECT_ACTIONS + 10 {
        r.handle_message(b, Message::ArchitectAction(action(1)))
            .unwrap();
    }
    r.handle_message(a, Message::ArchitectAction(action(3)))
        .unwrap();
    assert_eq!(r.take_architect_actions(), vec![(pa, action(3))]);
    assert_eq!(r.stats().dropped_architect_actions, 0);
    assert_eq!(
        r.stats().rejected_architect_actions,
        crate::server::MAX_QUEUED_ARCHITECT_ACTIONS as u64 + 11
    );
    // The architect's own flood between two takes is capped and counted.
    for _ in 0..crate::server::MAX_QUEUED_ARCHITECT_ACTIONS + 10 {
        r.handle_message(a, Message::ArchitectAction(action(1)))
            .unwrap();
    }
    assert_eq!(
        r.take_architect_actions().len(),
        crate::server::MAX_QUEUED_ARCHITECT_ACTIONS
    );
    assert_eq!(r.stats().dropped_architect_actions, 10);
}

/// The Architect (M4 B4): the room hands queued strokes to the world before
/// each tick; the architect's valid stroke is accepted and telegraphed, a
/// runner's forged one is refused and counted.
#[test]
fn architect_strokes_are_validated_counted_and_telegraphed() {
    let code = ShareCode::new(79, Sliders::new(40, 30));
    let mut mc = MatchConfig::paint_the_void(code);
    mc.rules = ls_sim::RuleSet::default_for(ls_sim::ModeKind::TheArchitect);
    let mut r = Room::new(mc, cfg()).unwrap();
    let a = r.join(&client_hello("a", None, None)).unwrap();
    let b = r.join(&client_hello("b", None, None)).unwrap();
    r.step();
    let pa = r.player_of(a).unwrap();
    let state = r.world().mode().architect.as_deref().unwrap();
    assert_eq!(state.architect, Some(pa));
    let centre = r.terrain().bounds().center();
    let palette = ls_net::ArchitectActionMsg {
        stroke: ls_sim::modes::the_architect::StrokeKind::Palette.index(),
        center: centre,
        radius: 16.0,
    };
    // A runner forging the architect's stroke, a bad kind, a bad radius.
    r.handle_packet(b, &Message::ArchitectAction(palette).encode().unwrap())
        .unwrap();
    r.handle_message(
        a,
        Message::ArchitectAction(ls_net::ArchitectActionMsg {
            stroke: 200,
            ..palette
        }),
    )
    .unwrap();
    r.handle_message(
        a,
        Message::ArchitectAction(ls_net::ArchitectActionMsg {
            radius: 100.0,
            ..palette
        }),
    )
    .unwrap();
    r.handle_message(a, Message::ArchitectAction(palette))
        .unwrap();
    let report = r.step();
    assert_eq!(r.stats().rejected_architect_actions, 3);
    assert_eq!(r.stats().architect_strokes, 1);
    let cue = report.events.iter().find_map(|e| match e {
        ls_sim::Event::Cue(c) if c.kind == ls_sim::CueKind::StrokeTelegraph => Some(*c),
        _ => None,
    });
    let cue = cue.expect("the telegraph goes out");
    assert_eq!(cue.source, Some(pa));
    assert_eq!(cue.pos, centre);
    // The budget meter (15 spent) is what the architect's client sees.
    let view = r.net_mode_state();
    let me = view
        .players
        .iter()
        .find(|p| p.flags & ls_net::mode_player_flags::ARCHITECT != 0)
        .expect("the architect's row");
    assert_eq!(me.meter, 85);
    // Sim and wire flags agree.
    assert_eq!(
        ls_sim::mode::player_flags::ARCHITECT,
        ls_net::mode_player_flags::ARCHITECT
    );
    assert_eq!(
        ls_sim::mode::player_flags::FINISHED,
        ls_net::mode_player_flags::FINISHED
    );
}

/// Lag compensation previews the target's movement with the gravity wells
/// (M4; static and dynamic): a target hauled up by a well sits at its
/// rewound position after the displaced step, as the plan promises.
#[test]
fn lag_compensation_previews_targets_in_gravity_wells() {
    use super::lagcomp::{plan_compensation, ShipHistory};
    use ls_math::terrain::TerrainQuery;
    use ls_math::Quat;
    use ls_sim::{Buttons, DynamicWell, ShipType};
    use std::collections::BTreeMap;
    const REWIND: u64 = 15;
    let r = room();
    let t = r.terrain().clone();
    let b = t.bounds();
    // Open space around the spot.
    let c = (0..400)
        .map(|i| {
            let f = |k: u32| ((i * k) % 97) as f32 / 97.0;
            b.min
                + (b.max - b.min).mul_elem(Vec3::new(
                    0.2 + 0.6 * f(13),
                    0.3 + 0.4 * f(29),
                    0.2 + 0.6 * f(53),
                ))
        })
        .find(|&p| {
            [
                Vec3::ZERO,
                Vec3::X * 6.0,
                Vec3::X * -6.0,
                Vec3::Y * 6.0,
                Vec3::Y * -4.0,
                Vec3::new(-3.0, -3.0, 0.0),
                Vec3::new(3.0, 3.0, 0.0),
            ]
            .iter()
            .all(|&o| t.sdf(p + o) > 2.5)
                && t.raycast(p - Vec3::new(9.0, 3.0, 0.0), Vec3::X, 18.0)
                    .is_none()
                && t.raycast(p - Vec3::new(9.0, 0.0, 0.0), Vec3::X, 18.0)
                    .is_none()
        })
        .expect("open space in the level");
    let mut planned = 0;
    let mut worst = 0.0f32;
    // Aim at heights below the target: somewhere the shot misses the
    // climbing target but hits where the shooter saw it.
    for k in 0..12 {
        let mut w = r.world().clone();
        let mut t = t.clone();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let tg = w.add_player(Some(1), ShipType::Tinker).unwrap();
        let place = |w: &mut World, id, pos: Vec3, dir: Vec3| {
            let s = w.ship_mut(id).unwrap();
            s.pos = pos;
            s.vel = Vec3::ZERO;
            s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize());
        };
        let target = c + Vec3::new(3.0, -2.0, 0.0);
        place(&mut w, tg, target, Vec3::X);
        let now = w.tick();
        w.add_well(DynamicWell {
            key: 1,
            pos: target + Vec3::Y * 5.0,
            peak_accel: 9.0,
            range: 16.0,
            start_tick: now + 1,
            end_tick: now + 600,
            owner: None,
        });
        let mut history = ShipHistory::new(REWIND);
        let idle = |k: u64| -> BTreeMap<PlayerId, InputCmd> {
            [(a, InputCmd::idle(k)), (tg, InputCmd::idle(k))].into()
        };
        // The target starts to climb; the shooter waits 8 ticks (the
        // whole test stays below tick 30, see `cfg`).
        for _ in 0..8 {
            place(&mut w, a, target - Vec3::X * 6.0, Vec3::X);
            let k = w.tick() + 1;
            let inputs: Vec<_> = idle(k).into_iter().collect();
            w.step(&mut t, &inputs);
            history.record(k, &w);
        }
        // Fire a Scrap Laser bolt at a point below the target.
        let aim_at = w.ship(tg).unwrap().pos - Vec3::Y * (0.25 * k as f32);
        let from = aim_at - Vec3::X * 6.0;
        place(&mut w, a, from, Vec3::X);
        let k0 = w.tick() + 1;
        let mut inputs = idle(k0);
        inputs.insert(a, InputCmd::idle(k0).with_buttons(Buttons::FIRE_PRIMARY));
        let ev = w.step(&mut t, &inputs.into_iter().collect::<Vec<_>>());
        history.record(k0, &w);
        let Some(pid) = ev.iter().find_map(|e| match e {
            Event::Fired { projectile, .. } => Some(*projectile),
            _ => None,
        }) else {
            panic!("fired");
        };
        let rewinds: BTreeMap<u32, u64> = [(pid, REWIND)].into();
        while w.tick() < 29 {
            let k = w.tick() + 1;
            let inputs = idle(k);
            let plan = plan_compensation(&w, &t, &inputs, &history, &rewinds);
            for d in &plan {
                // The displaced step leaves the target at its rewound spot.
                let rewound = history.position(d.player, k - REWIND).unwrap();
                let mut probe = w.clone();
                probe.ship_mut(d.player).unwrap().pos += d.offset;
                let list: Vec<_> = inputs.clone().into_iter().collect();
                probe.step(&mut t.clone(), &list);
                let err = probe.ship(d.player).unwrap().pos.distance(rewound);
                worst = worst.max(err);
                planned += 1;
            }
            let list: Vec<_> = inputs.into_iter().collect();
            w.step(&mut t, &list);
            history.record(k, &w);
            if !w.projectiles().iter().any(|p| p.id == pid) {
                break;
            }
        }
    }
    assert!(planned >= 1, "some shot needed compensation");
    assert!(worst <= 1e-3, "target {worst} u off its rewound position");
}

/// D1 review: lag compensation never displaces a target onto another ship
/// (their hulls would touch where neither is; Belt Up would bump them apart
/// for real). Control: with the other ship away, the same shot is
/// compensated.
#[test]
fn lag_compensation_never_puts_a_target_onto_another_ship() {
    use super::lagcomp::{plan_compensation, ShipHistory};
    use ls_math::terrain::TerrainQuery;
    use ls_math::Quat;
    use ls_sim::{Buttons, ShipType};
    use std::collections::BTreeMap;
    const REWIND: u64 = 10;
    let r = room();
    let t0 = r.terrain().clone();
    let b = t0.bounds();
    // Open space along a line through the spot.
    let c = (0..400)
        .map(|i| {
            let f = |k: u32| ((i * k) % 97) as f32 / 97.0;
            b.min
                + (b.max - b.min).mul_elem(Vec3::new(
                    0.2 + 0.6 * f(13),
                    0.3 + 0.4 * f(29),
                    0.2 + 0.6 * f(53),
                ))
        })
        .find(|&p| {
            [-8.0f32, -4.0, 0.0, 4.0, 8.0]
                .iter()
                .all(|&x| t0.sdf(p + Vec3::X * x) > 4.0 && t0.sdf(p + Vec3::new(x, 6.0, 0.0)) > 4.0)
        })
        .expect("open space in the level");
    let mut plans = Vec::new();
    for crowd in [false, true] {
        let mut w = r.world().clone();
        let mut t = t0.clone();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let tg = w.add_player(Some(1), ShipType::Tinker).unwrap();
        let o = w.add_player(Some(1), ShipType::Tinker).unwrap();
        let place = |w: &mut World, id, pos: Vec3| {
            let s = w.ship_mut(id).unwrap();
            s.pos = pos;
            s.vel = Vec3::ZERO;
            s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::X);
        };
        let rs = w.tuning_for(ShipType::Tinker).radius;
        let mut history = ShipHistory::new(REWIND);
        let idle = |k: u64| -> BTreeMap<PlayerId, InputCmd> {
            [a, tg, o]
                .iter()
                .map(|&id| (id, InputCmd::idle(k)))
                .collect()
        };
        // The target was at `c` when the shooter saw it...
        for _ in 0..REWIND + 1 {
            place(&mut w, a, c - Vec3::X * 3.5);
            place(&mut w, tg, c);
            place(&mut w, o, c + Vec3::new(0.0, 6.0, 8.0));
            let k = w.tick() + 1;
            w.step(&mut t, &idle(k).into_iter().collect::<Vec<_>>());
            history.record(k, &w);
        }
        // ...and is above it now; the other ship sits next to that spot
        // (off the shot's line) or far away.
        let rewound = history.position(tg, w.tick() + 1 - REWIND).unwrap();
        place(&mut w, tg, rewound + Vec3::Y * 6.0);
        let beside = if crowd {
            rewound + Vec3::Z * (1.6 * rs)
        } else {
            rewound + Vec3::new(0.0, 6.0, 8.0)
        };
        place(&mut w, o, beside);
        place(&mut w, a, rewound - Vec3::X * 3.5);
        let k0 = w.tick() + 1;
        let mut inputs = idle(k0);
        inputs.insert(a, InputCmd::idle(k0).with_buttons(Buttons::FIRE_PRIMARY));
        let ev = w.step(&mut t, &inputs.into_iter().collect::<Vec<_>>());
        history.record(k0, &w);
        let pid = ev
            .iter()
            .find_map(|e| match e {
                Event::Fired { projectile, .. } => Some(*projectile),
                _ => None,
            })
            .expect("fired");
        let p = w
            .projectiles()
            .iter()
            .find(|p| p.id == pid)
            .unwrap()
            .clone();
        assert!(1.6 * rs > rs + p.radius, "the bolt passes the other ship");
        let rewinds: BTreeMap<u32, u64> = [(pid, REWIND)].into();
        let mut planned = Vec::new();
        for _ in 0..REWIND - 1 {
            let k = w.tick() + 1;
            // Everyone holds still where the test put them.
            place(&mut w, tg, rewound + Vec3::Y * 6.0);
            place(&mut w, o, beside);
            let inputs = idle(k);
            planned.extend(plan_compensation(&w, &t, &inputs, &history, &rewinds));
            if !planned.is_empty() {
                break;
            }
            w.step(&mut t, &inputs.into_iter().collect::<Vec<_>>());
            history.record(k, &w);
            if !w.projectiles().iter().any(|p| p.id == pid) {
                break;
            }
        }
        plans.push(planned);
    }
    assert!(
        plans[0].iter().any(|d| d.offset.y < -5.0),
        "control: compensated {:?}",
        plans[0]
    );
    assert!(plans[1].is_empty(), "onto the other ship: {:?}", plans[1]);
}
