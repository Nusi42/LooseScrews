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
    let first = inventories(&mut r);
    assert_eq!(first.len(), 1, "sent once the client is ready");
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
    let snap = out.iter().find_map(|o| match Message::decode(&o.bytes) {
        Ok(Message::Snapshot(sm)) => Some(r.conv().quantizer().unpack_snapshot(&sm.snapshot)),
        _ => None,
    });
    r.step();
    let out2 = r.take_outgoing();
    let snap = snap.or_else(|| {
        out2.iter().find_map(|o| match Message::decode(&o.bytes) {
            Ok(Message::Snapshot(sm)) => Some(r.conv().quantizer().unpack_snapshot(&sm.snapshot)),
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
