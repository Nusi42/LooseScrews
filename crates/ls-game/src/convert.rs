//! Conversions between `ls-sim` state and `ls-net` wire types.
//!
//! - Ships: [`NetConv::ship_to_net`] / [`NetConv::apply_net_ship`], with the
//!   private [`ShipStatus`] bytes scaled to each resource's maximum.
//! - Inputs: [`input_to_net`] / [`net_to_input`]. Clients must simulate the
//!   *quantized* command ([`quantize_input`]) so they predict exactly what the
//!   server will simulate.
//! - Projectiles and events: [`projectile_to_net`], [`events_to_net`].
//! - Quantization: [`quant_config_for`] derives the [`QuantConfig`] from the
//!   level bounds (plus [`QUANT_MARGIN`]) and the weapon table.

use ls_math::{Aabb, Quat, Vec3};
use ls_net::{
    buttons, ship_flags, NetEvent, NetInput, NetProjectile, NetShip, QuantConfig, Quantizer,
    ShipStatus,
};
use ls_sim::{
    Buttons, Event, InputCmd, PlayerId, Projectile, ShipState, ShipTuning, ShipType, WeaponTable,
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
/// least the default (256 u/s) and at least the fastest weapon.
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
    tuning: ShipTuning,
}

impl NetConv {
    pub fn new(config: QuantConfig, tuning: ShipTuning) -> Result<Self, GameError> {
        Ok(NetConv {
            quant: Quantizer::new(config)?,
            tuning,
        })
    }

    /// Conversion context for a level with the given bounds.
    pub fn for_level(
        bounds: Aabb,
        weapons: &WeaponTable,
        tuning: ShipTuning,
    ) -> Result<Self, GameError> {
        Self::new(quant_config_for(bounds, weapons), tuning)
    }

    pub fn quantizer(&self) -> &Quantizer {
        &self.quant
    }

    pub fn tuning(&self) -> &ShipTuning {
        &self.tuning
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
        flags = ship_flags::with_team(flags, ship.team);
        let weapon = ship.primary.map_or(0, |w| w.0.min(15));
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

    /// Private resources scaled to bytes: `ceil(value / max * 255)`, so any
    /// positive amount stays positive on the wire.
    pub fn status(&self, ship: &ShipState) -> ShipStatus {
        let t = &self.tuning;
        ShipStatus {
            hull: to_byte(ship.hull, t.hull_max),
            energy: to_byte(ship.energy, t.energy_max),
            boost: to_byte(ship.boost, t.boost_max),
            scrap: to_byte(ship.scrap, t.scrap_max),
        }
    }

    /// Inverse of [`NetConv::status`] as `(hull, energy, boost, scrap)`.
    pub fn status_values(&self, s: &ShipStatus) -> (f32, f32, f32, f32) {
        let t = &self.tuning;
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
            let (hull, energy, boost, scrap) = self.status_values(s);
            ship.hull = hull;
            ship.energy = energy;
            ship.boost = boost;
            ship.scrap = scrap;
        }
    }

    /// A fresh [`ShipState`] (Tinker) carrying the networked state of `net`.
    pub fn net_to_ship(&self, player: PlayerId, net: &NetShip) -> ShipState {
        let mut s = ShipState::new(
            player,
            ship_flags::team(net.flags),
            ShipType::Tinker,
            &self.tuning,
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

const BUTTON_MAP: [(Buttons, u16); 6] = [
    (Buttons::BOOST, buttons::BOOST),
    (Buttons::FIRE_PRIMARY, buttons::FIRE_PRIMARY),
    (Buttons::FIRE_SECONDARY, buttons::FIRE_SECONDARY),
    (Buttons::SCAVENGE, buttons::SCAVENGE),
    (Buttons::OVERDRIVE, buttons::OVERDRIVE),
    (Buttons::HORIZON_ASSIST, buttons::HORIZON_ASSIST),
];

/// Wire form of a projectile: id truncated to 16 bits (wraps), `kind` is the
/// weapon id, `owner` the shooter's wire id ([`NO_SHIP`] if they left).
pub fn projectile_to_net(p: &Projectile, owner: Option<u8>) -> NetProjectile {
    NetProjectile {
        id: p.id as u16,
        kind: p.weapon.0,
        owner: owner.unwrap_or(NO_SHIP),
        pos: p.pos,
        vel: p.vel,
    }
}

/// Converts one tick's simulation events into wire events.
///
/// `Damaged` becomes `Hit` (position = the preceding `Impact` point, damage
/// rounded to whole points, at least 1), `Killed` becomes `Kill`, `Respawned`
/// becomes `Respawn`, `MatchEnded` becomes `MatchOver`. Events about players
/// without a wire id are dropped; the rest (fired, pickups, edits, scores)
/// reach clients through snapshots instead.
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
                        attacker: wire_id(*attacker).unwrap_or(NO_SHIP),
                        victim,
                        weapon: weapon.0,
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
            ShipTuning::tinker(),
        )
        .unwrap()
    }

    #[test]
    fn quant_config_covers_level_and_weapons() {
        let c = conv();
        let q = c.quantizer().config();
        assert_eq!(q.bounds.min, Vec3::splat(-2.0));
        assert_eq!(q.bounds.max, Vec3::new(258.0, 130.0, 258.0));
        assert!(q.max_projectile_speed >= 180.0);
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
                attacker: PlayerId(0),
                weapon: WeaponId(0),
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
            ]
        );
    }
}
