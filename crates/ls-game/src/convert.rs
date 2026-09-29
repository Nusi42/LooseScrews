//! Conversions between `ls-sim` state and `ls-net` wire types.
//!
//! - Ships: [`NetConv::ship_to_net`] / [`NetConv::apply_net_ship`], with the
//!   private [`ShipStatus`] bytes scaled to each resource's maximum.
//! - Inputs: [`input_to_net`] / [`net_to_input`]. Clients must simulate the
//!   *quantized* command ([`quantize_input`]) so they predict exactly what the
//!   server will simulate.
//! - Projectiles, entities, players and events: [`projectile_to_net`],
//!   [`entity_to_net`], [`NetConv::player_to_net`], [`events_to_net`].
//! - Quantization: [`quant_config_for`] derives the [`QuantConfig`] from the
//!   level bounds (plus [`QUANT_MARGIN`]) and the weapon table.

use ls_math::{Aabb, Quat, Vec3};
use ls_net::{
    buttons, ship_flags, NetEntity, NetEvent, NetInput, NetPlayer, NetProjectile, NetShip,
    QuantConfig, Quantizer, ShipStatus,
};
use ls_sim::{
    Buttons, EntityView, Event, InputCmd, PlayerId, Projectile, ShipState, ShipTable, ShipTuning,
    ShipType, WeaponTable,
};

use crate::GameError;

/// Margin (u) added around the level bounds for position quantization.
pub const QUANT_MARGIN: f32 = 2.0;
/// Wire id for "no ship" (departed projectile owner, kill without killer).
pub const NO_SHIP: u8 = u8::MAX;
/// Wire weapon id for "no weapon" in kill events.
pub const NO_WEAPON: u8 = u8::MAX;

/// Quantization for a level: `bounds` grown by [`QUANT_MARGIN`], ship speed
/// range from `QuantConfig::default()` (64 u/s per axis), projectile range at
/// least the default (320 u/s) and at least the fastest weapon.
pub fn quant_config_for(bounds: Aabb, weapons: &WeaponTable) -> QuantConfig {
    let d = QuantConfig::default();
    let fastest = weapons.iter().map(|(_, w)| w.speed).fold(0.0f32, f32::max);
    QuantConfig {
        bounds: bounds.expand(QUANT_MARGIN),
        max_ship_speed: d.max_ship_speed,
        max_projectile_speed: d.max_projectile_speed.max(fastest),
    }
}

/// `ls-sim` <-> `ls-net` conversion context for one match.
#[derive(Debug, Clone)]
pub struct NetConv {
    quant: Quantizer,
    ships: ShipTable,
}

impl NetConv {
    pub fn new(config: QuantConfig, ships: ShipTable) -> Result<Self, GameError> {
        Ok(NetConv {
            quant: Quantizer::new(config)?,
            ships,
        })
    }

    /// Conversion context for a level with the given bounds.
    pub fn for_level(
        bounds: Aabb,
        weapons: &WeaponTable,
        ships: ShipTable,
    ) -> Result<Self, GameError> {
        Self::new(quant_config_for(bounds, weapons), ships)
    }

    pub fn quantizer(&self) -> &Quantizer {
        &self.quant
    }

    /// Tuning of the standard ship type (Tinker).
    pub fn tuning(&self) -> &ShipTuning {
        self.ships.get(ShipType::Tinker)
    }

    /// Tuning of a ship type.
    pub fn tuning_for(&self, t: ShipType) -> &ShipTuning {
        self.ships.get(t)
    }

    pub fn ships(&self) -> &ShipTable {
        &self.ships
    }

    /// Wire form of a player entry (name, team, ship type).
    pub fn player_to_net(id: u8, name: &str, ship: &ShipState) -> NetPlayer {
        let mut name = name.to_string();
        while name.len() > ls_net::MAX_PLAYER_NAME {
            name.pop();
        }
        NetPlayer {
            id,
            team: ship.team,
            ship_type: ship.ship_type.index(),
            gadget: ship.gadget.map_or(u8::MAX, |g| g.0),
            name,
        }
    }

    /// Wire form of a ship. `with_status` adds the private resource bytes
    /// (sent only to the ship's owner).
    pub fn ship_to_net(&self, id: u8, ship: &ShipState, with_status: bool) -> NetShip {
        let mut flags = 0u16;
        if ship.alive {
            flags |= ship_flags::ALIVE;
        }
        if ship.boosting {
            flags |= ship_flags::BOOSTING;
        }
        if ship.gadget_active {
            flags |= ship_flags::SHIELDED;
        }
        flags = ship_flags::with_team(flags, ship.team);
        let weapon = ship.primary.map_or(0, |w| w.0.min(31));
        flags = ship_flags::with_weapon(flags, weapon);
        NetShip {
            id,
            pos: ship.pos,
            vel: ship.vel,
            rot: ship.orientation,
            flags,
            status: with_status.then(|| self.status(ship)),
        }
    }

    /// Private resources scaled to bytes: `ceil(value / max * 255)` with the
    /// maxima of the ship's type, so any positive amount stays positive on
    /// the wire.
    pub fn status(&self, ship: &ShipState) -> ShipStatus {
        let t = self.ships.get(ship.ship_type);
        ShipStatus {
            hull: to_byte(ship.hull, t.hull_max),
            energy: to_byte(ship.energy, t.energy_max),
            boost: to_byte(ship.boost, t.boost_max),
            scrap: to_byte(ship.scrap, t.scrap_max),
        }
    }

    /// Inverse of [`NetConv::status`] for a Tinker, as `(hull, energy, boost, scrap)`.
    pub fn status_values(&self, s: &ShipStatus) -> (f32, f32, f32, f32) {
        self.status_values_for(ShipType::Tinker, s)
    }

    /// Inverse of [`NetConv::status`] for a ship type.
    pub fn status_values_for(&self, ship_type: ShipType, s: &ShipStatus) -> (f32, f32, f32, f32) {
        let t = self.ships.get(ship_type);
        (
            from_byte(s.hull, t.hull_max),
            from_byte(s.energy, t.energy_max),
            from_byte(s.boost, t.boost_max),
            from_byte(s.scrap, t.scrap_max),
        )
    }

    /// Overwrites the networked parts of `ship` (pose, velocity, alive,
    /// boosting, team and, if present, resources) with `net`. Weapons,
    /// cooldowns and scores are left untouched.
    pub fn apply_net_ship(&self, ship: &mut ShipState, net: &NetShip) {
        ship.pos = net.pos;
        ship.vel = net.vel;
        ship.orientation = net.rot;
        ship.alive = net.flags & ship_flags::ALIVE != 0;
        ship.boosting = net.flags & ship_flags::BOOSTING != 0;
        ship.team = ship_flags::team(net.flags);
        if let Some(s) = &net.status {
            let (hull, energy, boost, scrap) = self.status_values_for(ship.ship_type, s);
            ship.hull = hull;
            ship.energy = energy;
            ship.boost = boost;
            ship.scrap = scrap;
        }
    }

    /// A fresh [`ShipState`] (Tinker) carrying the networked state of `net`.
    pub fn net_to_ship(&self, player: PlayerId, net: &NetShip) -> ShipState {
        self.net_to_ship_as(player, ShipType::Tinker, net)
    }

    /// A fresh [`ShipState`] of `ship_type` carrying the networked state of `net`.
    pub fn net_to_ship_as(
        &self,
        player: PlayerId,
        ship_type: ShipType,
        net: &NetShip,
    ) -> ShipState {
        let mut s = ShipState::new(
            player,
            ship_flags::team(net.flags),
            ship_type,
            self.ships.get(ship_type),
            net.pos,
            net.rot,
        );
        self.apply_net_ship(&mut s, net);
        s
    }

    /// `net` after a pack/unpack round trip: exactly what a peer decodes.
    pub fn quantize_ship(&self, net: &NetShip) -> NetShip {
        self.quant.unpack_ship(&self.quant.pack_ship(net))
    }
}

fn to_byte(v: f32, max: f32) -> u8 {
    if v.is_nan() || max.is_nan() || v <= 0.0 || max <= 0.0 {
        return 0;
    }
    ls_math::det::ceil(v / max * 255.0).clamp(0.0, 255.0) as u8
}

fn from_byte(b: u8, max: f32) -> f32 {
    b as f32 / 255.0 * max
}

/// Wire form of an input command (thrust/roll to 1/127, look to 1/8192 rad).
pub fn input_to_net(cmd: &InputCmd) -> NetInput {
    let mut b = 0u16;
    for (sim, wire) in BUTTON_MAP {
        if cmd.buttons.contains(sim) {
            b |= wire;
        }
    }
    NetInput::from_controls(
        cmd.tick as u32,
        cmd.thrust,
        cmd.look_yaw,
        cmd.look_pitch,
        cmd.roll,
        b,
        cmd.weapon_select,
    )
}

/// Simulation command for a wire input.
pub fn net_to_input(n: &NetInput) -> InputCmd {
    let mut b = Buttons::NONE;
    for (sim, wire) in BUTTON_MAP {
        if n.pressed(wire) {
            b = b.with(sim);
        }
    }
    let (yaw, pitch) = n.look_radians();
    InputCmd {
        tick: n.tick as u64,
        thrust: n.thrust_axes(),
        look_pitch: pitch,
        look_yaw: yaw,
        roll: n.roll_axis(),
        buttons: b,
        weapon_select: n.weapon,
    }
}

/// The command as the server will see it after the wire round trip.
pub fn quantize_input(cmd: &InputCmd) -> InputCmd {
    net_to_input(&input_to_net(cmd))
}

const BUTTON_MAP: [(Buttons, u16); 8] = [
    (Buttons::BOOST, buttons::BOOST),
    (Buttons::FIRE_PRIMARY, buttons::FIRE_PRIMARY),
    (Buttons::FIRE_SECONDARY, buttons::FIRE_SECONDARY),
    (Buttons::SCAVENGE, buttons::SCAVENGE),
    (Buttons::OVERDRIVE, buttons::OVERDRIVE),
    (Buttons::HORIZON_ASSIST, buttons::HORIZON_ASSIST),
    (Buttons::GADGET, buttons::GADGET),
    (Buttons::WEAPON_MODE, buttons::WEAPON_MODE),
];

/// Wire form of a projectile: id truncated to 16 bits (wraps), `kind` is the
/// weapon id, `owner` the shooter's wire id ([`NO_SHIP`] if they left),
/// `state` the replicated projectile flags (`ls_sim::proj_flags`).
pub fn projectile_to_net(p: &Projectile, owner: Option<u8>) -> NetProjectile {
    NetProjectile {
        id: p.id as u16,
        kind: p.weapon.0,
        owner: owner.unwrap_or(NO_SHIP),
        pos: p.pos,
        vel: p.vel,
        state: p.net_state(),
    }
}

/// Wire form of a generic entity: `id & 0x7fff` (unique among live
/// entities, see `ls_sim::entity`), `kind` = `ls_sim::ReplicatedKind`,
/// `owner` the owner's wire id ([`NO_SHIP`] if none). Debris is sent without
/// velocity (drag changes it every tick; clients interpolate positions).
pub fn entity_to_net(v: &EntityView, owner: Option<u8>) -> NetEntity {
    NetEntity {
        id: v.wire_id(),
        kind: v.kind as u8,
        visual: v.visual.0,
        owner: owner.unwrap_or(NO_SHIP),
        team: v.team,
        pos: v.pos,
        vel: if v.kind == ls_sim::ReplicatedKind::Debris {
            Vec3::ZERO
        } else {
            v.vel
        },
        rot: v.rot,
        state: v.state,
    }
}

/// Converts one tick's simulation events into wire events.
///
/// `Damaged` becomes `Hit` (position = the preceding `Impact` point, damage
/// rounded to whole points, at least 1), `Killed` becomes `Kill`, `Respawned`
/// becomes `Respawn`, `MatchEnded` becomes `MatchOver`, `Cue` becomes `Cue`.
/// Events about players without a wire id are dropped; the rest (fired,
/// pickups, edits, scores) reach clients through snapshots instead.
pub fn events_to_net(
    events: &[Event],
    tick: u64,
    wire_id: impl Fn(PlayerId) -> Option<u8>,
) -> Vec<NetEvent> {
    let tick = tick as u32;
    let mut out = Vec::new();
    let mut last_impact = Vec3::ZERO;
    for e in events {
        match e {
            Event::Impact { point, .. } => last_impact = *point,
            Event::Damaged {
                target,
                attacker,
                weapon,
                amount,
                ..
            } => {
                if let Some(victim) = wire_id(*target) {
                    out.push(NetEvent::Hit {
                        tick,
                        attacker: attacker.and_then(&wire_id).unwrap_or(NO_SHIP),
                        victim,
                        weapon: weapon.map_or(NO_WEAPON, |w| w.0),
                        damage: ls_math::det::round(*amount).clamp(1.0, u16::MAX as f32) as u16,
                        pos: last_impact,
                    });
                }
            }
            Event::Killed {
                victim,
                killer,
                weapon,
            } => {
                if let Some(v) = wire_id(*victim) {
                    out.push(NetEvent::Kill {
                        tick,
                        killer: killer.and_then(&wire_id).unwrap_or(NO_SHIP),
                        victim: v,
                        weapon: weapon.map_or(NO_WEAPON, |w| w.0),
                    });
                }
            }
            Event::Respawned { player, pos } => {
                if let Some(ship) = wire_id(*player) {
                    out.push(NetEvent::Respawn {
                        tick,
                        ship,
                        pos: *pos,
                    });
                }
            }
            Event::MatchEnded { winner, .. } => out.push(NetEvent::MatchOver {
                tick,
                winner_team: *winner,
            }),
            Event::Cue(c) => out.push(NetEvent::Cue {
                tick,
                kind: c.kind as u8,
                source: c.source.and_then(&wire_id).unwrap_or(NO_SHIP),
                target: c.target.and_then(&wire_id).unwrap_or(NO_SHIP),
                pos: c.pos,
                value: c.value,
            }),
            _ => {}
        }
    }
    out
}

/// Angle (radians) between two orientations, ignoring the quaternion sign.
pub fn rotation_error(a: Quat, b: Quat) -> f32 {
    let d = a.dot(b).abs().min(1.0);
    2.0 * ls_math::det::acos(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_sim::WeaponId;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn conv() -> NetConv {
        NetConv::for_level(
            Aabb::new(Vec3::ZERO, Vec3::new(256.0, 128.0, 256.0)),
            &WeaponTable::builtin(),
            ShipTable::builtin(),
        )
        .unwrap()
    }

    #[test]
    fn quant_config_covers_level_and_weapons() {
        let c = conv();
        let q = c.quantizer().config();
        assert_eq!(q.bounds.min, Vec3::splat(-2.0));
        assert_eq!(q.bounds.max, Vec3::new(258.0, 130.0, 258.0));
        assert!(q.max_projectile_speed >= 270.0, "Railspike");
        // Matches the ls-net default for the default level.
        assert_eq!(*q, QuantConfig::default());
    }

    #[test]
    fn input_round_trip_is_idempotent() {
        let cmd = InputCmd {
            tick: 77,
            thrust: Vec3::new(0.3, -1.0, 0.77),
            look_pitch: 0.0123,
            look_yaw: -0.2,
            roll: 0.5,
            buttons: Buttons::BOOST | Buttons::FIRE_SECONDARY | Buttons::HORIZON_ASSIST,
            weapon_select: 3,
        };
        let q = quantize_input(&cmd);
        assert_eq!(quantize_input(&q), q);
        assert_eq!(q.tick, 77);
        assert_eq!(q.buttons, cmd.buttons);
        assert_eq!(q.weapon_select, 3);
        assert!((q.thrust - cmd.thrust).length() < 0.01);
        assert!((q.look_pitch - cmd.look_pitch).abs() < 1e-3);
        assert_eq!(input_to_net(&q), input_to_net(&cmd));
    }

    #[test]
    fn ship_round_trip_within_quantization() {
        let c = conv();
        let t = ShipTuning::tinker();
        let mut s = ShipState::new(
            PlayerId(3),
            1,
            ShipType::Tinker,
            &t,
            Vec3::new(100.3, 20.7, 51.1),
            Quat::from_axis_angle(Vec3::new(0.3, 1.0, 0.2).normalize(), 1.1),
        );
        s.vel = Vec3::new(3.0, -12.5, 20.0);
        s.boost = 0.01;
        s.energy = t.energy_max * 0.5;
        s.primary = Some(WeaponId(2));
        let net = c.quantize_ship(&c.ship_to_net(3, &s, true));
        let back = c.net_to_ship(PlayerId(3), &net);
        assert!((back.pos - s.pos).length() < 0.01);
        assert!((back.vel - s.vel).length() < 0.01);
        assert!(rotation_error(back.orientation, s.orientation) < 0.01);
        assert!(back.alive && back.team == 1);
        assert!(back.boost > 0.0, "tiny boost must stay positive");
        assert!((back.energy - s.energy).abs() <= t.energy_max / 255.0 + 1e-4);
        assert_eq!(ship_flags::weapon(net.flags), 2);
        assert!(c.ship_to_net(3, &s, false).status.is_none());
    }

    #[test]
    fn events_map_to_wire_events() {
        let id = |p: PlayerId| (p.0 < 4).then_some(p.0 as u8 + 10);
        let events = vec![
            Event::Impact {
                projectile: 1,
                weapon: WeaponId(0),
                point: Vec3::new(1.0, 2.0, 3.0),
                target: Some(PlayerId(1)),
            },
            Event::Damaged {
                target: PlayerId(1),
                attacker: Some(PlayerId(0)),
                weapon: Some(WeaponId(0)),
                amount: 7.6,
                direct: true,
            },
            Event::Killed {
                victim: PlayerId(1),
                killer: None,
                weapon: None,
            },
            Event::Respawned {
                player: PlayerId(9),
                pos: Vec3::ZERO,
            },
            Event::MatchEnded {
                winner: Some(1),
                team_cells: [1, 2],
            },
            Event::Cue(ls_sim::Cue {
                kind: ls_sim::CueKind::PoliteWarning,
                source: Some(PlayerId(0)),
                target: Some(PlayerId(3)),
                pos: Vec3::X,
                value: 90,
            }),
        ];
        let net = events_to_net(&events, 5, id);
        assert_eq!(
            net,
            vec![
                NetEvent::Hit {
                    tick: 5,
                    attacker: 10,
                    victim: 11,
                    weapon: 0,
                    damage: 8,
                    pos: Vec3::new(1.0, 2.0, 3.0),
                },
                NetEvent::Kill {
                    tick: 5,
                    killer: NO_SHIP,
                    victim: 11,
                    weapon: NO_WEAPON,
                },
                NetEvent::MatchOver {
                    tick: 5,
                    winner_team: Some(1),
                },
                NetEvent::Cue {
                    tick: 5,
                    kind: 0,
                    source: 10,
                    target: 13,
                    pos: Vec3::X,
                    value: 90,
                },
            ]
        );
    }

    #[test]
    fn status_uses_the_ship_type_maxima() {
        let c = conv();
        let t = ShipTable::builtin();
        let mut s = ShipState::new(
            PlayerId(0),
            0,
            ShipType::Hauler,
            t.get(ShipType::Hauler),
            Vec3::new(10.0, 10.0, 10.0),
            Quat::IDENTITY,
        );
        s.hull = 150.0;
        s.gadget_active = true;
        let net = c.ship_to_net(0, &s, true);
        assert_eq!(net.status.unwrap().hull, 255);
        assert!(net.flags & ship_flags::SHIELDED != 0);
        let back = c.net_to_ship_as(PlayerId(0), ShipType::Hauler, &c.quantize_ship(&net));
        assert_eq!(back.hull, 150.0);
        let p = NetConv::player_to_net(0, &"x".repeat(40), &s);
        assert_eq!((p.ship_type, p.name.len()), (2, 32));
    }

    /// Bytes per 30 Hz snapshot of a busy fight with real ship physics: 32
    /// ships of all four types flown by `ls_sim::simulate_movement` in a
    /// level-sized room, every pilot thrusting (a third boosting) and
    /// turning at up to `turn` rad/s plus rolling, with inputs changing
    /// every quarter to half second; 64 debris pieces drifting from an
    /// explosion, 48 resting pickups and wrecks, 32 players. Every snapshot
    /// goes through `Message` against the one `age` ticks older and is
    /// checked against the simulation within the quantization bounds.
    /// Returns (mean total, worst total, mean ship section) in bytes.
    fn busy_fight_bytes(seed: u64, turn: f32, age: u32) -> (f64, usize, f64) {
        use ls_math::terrain::BoxRoomTerrain;
        use ls_math::Pcg32;
        use ls_net::{Message, NetEntity, PackedSnapshot, Snapshot, SnapshotHistory, SnapshotMsg};
        use ls_sim::simulate_movement;

        let level = Aabb::new(Vec3::ZERO, Vec3::new(256.0, 128.0, 256.0));
        let room = BoxRoomTerrain::new(level);
        let c = conv();
        let q = *c.quantizer();
        let mut rng = Pcg32::new(seed, 0);
        let mut ships: Vec<(ShipState, InputCmd, u32)> = (0..32u16)
            .map(|i| {
                let ty = ShipType::ALL[i as usize % 4];
                let pos = Vec3::new(
                    rng.range_f32(20.0, 236.0),
                    rng.range_f32(20.0, 108.0),
                    rng.range_f32(20.0, 236.0),
                );
                let axis = Vec3::new(
                    rng.range_f32(-1.0, 1.0),
                    rng.range_f32(-1.0, 1.0),
                    rng.range_f32(0.1, 1.0),
                )
                .normalize();
                let rot = Quat::from_axis_angle(axis, rng.range_f32(-3.0, 3.0));
                let s = ShipState::new(PlayerId(i), (i % 2) as u8, ty, c.tuning_for(ty), pos, rot);
                (s, InputCmd::idle(0), 0)
            })
            .collect();
        let players: Vec<_> = ships
            .iter()
            .map(|(s, _, _)| NetConv::player_to_net(s.player.0 as u8, "pilot", s))
            .collect();
        let centre = Vec3::new(128.0, 64.0, 128.0);
        let mut debris: Vec<(NetEntity, Vec3)> = (0..64u16)
            .map(|i| {
                let dir = Vec3::new(
                    rng.range_f32(-1.0, 1.0),
                    rng.range_f32(-1.0, 1.0),
                    rng.range_f32(-1.0, 1.0),
                )
                .normalize_or_zero();
                let e = NetEntity {
                    id: 100 + i * 3,
                    kind: 0,
                    visual: 0,
                    owner: NO_SHIP,
                    team: NO_SHIP,
                    pos: centre + dir * rng.range_f32(1.0, 8.0),
                    vel: Vec3::ZERO,
                    rot: Quat::IDENTITY,
                    state: [3, 0, 8, 0],
                };
                (e, dir * rng.range_f32(0.5, 6.0))
            })
            .collect();
        let resting: Vec<NetEntity> = (0..48u16)
            .map(|i| NetEntity {
                id: 400 + i,
                kind: if i < 40 { 1 } else { 5 },
                visual: 12 + i % 20,
                owner: NO_SHIP,
                team: NO_SHIP,
                pos: Vec3::new(5.0 * i as f32, 30.0, 200.0),
                vel: Vec3::ZERO,
                rot: Quat::IDENTITY,
                state: [i as u8 % 27, 10, 0, 0],
            })
            .collect();
        let msg = |snapshot: PackedSnapshot, base: u32| {
            Message::Snapshot(Box::new(SnapshotMsg {
                baseline: Some(base),
                snapshot,
            }))
        };
        let mut history = SnapshotHistory::default();
        let (mut total, mut worst, mut ship_total, mut n) = (0usize, 0usize, 0usize, 0usize);
        for tick in 1..=240u32 {
            for (s, input, hold) in &mut ships {
                if *hold == 0 {
                    *hold = 15 + rng.below(16);
                    *input = InputCmd {
                        tick: tick as u64,
                        thrust: Vec3::new(
                            rng.range_f32(-1.0, 1.0),
                            rng.range_f32(-1.0, 1.0),
                            rng.range_f32(-1.0, -0.3),
                        ),
                        look_pitch: rng.range_f32(-turn, turn) / 60.0,
                        look_yaw: rng.range_f32(-turn, turn) / 60.0,
                        roll: rng.range_f32(-1.0, 1.0),
                        buttons: if rng.chance(0.33) {
                            Buttons::BOOST
                        } else {
                            Buttons::NONE
                        },
                        weapon_select: 0,
                    };
                }
                *hold -= 1;
                let t = c.tuning_for(s.ship_type).clone();
                simulate_movement(s, input, &t, &room);
            }
            for (e, v) in &mut debris {
                e.pos += *v * (1.0 / 60.0);
            }
            if tick % 2 != 0 {
                continue;
            }
            let net: Vec<_> = ships
                .iter()
                .map(|(s, _, _)| c.ship_to_net(s.player.0 as u8, s, false))
                .collect();
            let mut entities: Vec<NetEntity> = debris.iter().map(|(e, _)| *e).collect();
            entities.extend(resting.iter().copied());
            let snap = q.pack_snapshot(&Snapshot {
                tick,
                ack_input: tick,
                ships: net.clone(),
                entities,
                players: players.clone(),
                scores: vec![31_000, 29_500],
                match_ticks_left: 9_000,
                ..Default::default()
            });
            if let Some(base) = history.get(tick.wrapping_sub(age)) {
                let bytes = msg(snap.clone(), base.tick).encode_with(&history).unwrap();
                let Message::Snapshot(back) = Message::decode_with(&bytes, &history).unwrap()
                else {
                    panic!("not a snapshot");
                };
                assert_eq!(back.snapshot, snap);
                // Quantization bounds hold through the delta encoding.
                let half_pos = q.position_step() * 0.5 + Vec3::splat(1e-4);
                for (orig, got) in net.iter().zip(&back.snapshot.ships) {
                    let got = q.unpack_ship(got);
                    let e = (got.pos - orig.pos).abs();
                    assert!(e.x <= half_pos.x && e.y <= half_pos.y && e.z <= half_pos.z);
                    let ev = (got.vel - orig.vel).abs().max_element();
                    assert!(ev <= q.ship_velocity_step() * 0.5 + 1e-5);
                    assert!(rotation_error(got.rot, orig.rot) < 0.3f32.to_radians());
                }
                // The ship section: the same pair of snapshots without ships.
                let mut no_ships = SnapshotHistory::default();
                let mut b = base.clone();
                b.ships.clear();
                no_ships.push(b);
                let mut s = snap.clone();
                s.ships.clear();
                let rest = msg(s, base.tick).encode_with(&no_ships).unwrap().len();
                total += bytes.len();
                worst = worst.max(bytes.len());
                ship_total += bytes.len() - rest;
                n += 1;
            }
            history.push(snap);
        }
        let n = n as f64;
        (total as f64 / n, worst, ship_total as f64 / n)
    }

    /// Bandwidth with real ship physics: a busy fight (32 thrusting,
    /// turning ships, 64 drifting debris, 48 resting items) stays within
    /// the 17 KB/s budget at 30 Hz (576 B per snapshot) for baselines up to
    /// 6 ticks (100 ms) old. Protocol 2 needed about 515 B for the busy
    /// ships alone at age 2.
    #[test]
    fn busy_fight_fits_the_bandwidth_budget() {
        // The documented budget: 32 ships x 18 B x 30 Hz = 17,280 B/s.
        const PER_SNAPSHOT: usize = 17_280 / 30;
        for (age, turn) in [(2, 4.0), (4, 4.0), (6, 4.0), (2, 9.0)] {
            let (mean, worst, ships) = busy_fight_bytes(90 + age as u64, turn, age);
            println!(
                "busy fight, turning up to {turn} rad/s, baseline {age} ticks old: mean {mean:.0} B, worst {worst} B ({} B/s), ships {ships:.0} B",
                worst * 30
            );
            assert!(worst <= PER_SNAPSHOT, "age {age}, turn {turn}: {worst} B");
        }
    }
}
