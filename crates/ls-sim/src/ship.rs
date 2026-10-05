//! Ship state, flight model and movement resources.
//!
//! The flight model caps thrust **per axis in ship space** (as in Descent): each
//! local axis accelerates towards `input * cap` independently, so flying on all
//! three axes at once is `sqrt(3)` times faster than flying straight. Cruise cap
//! is 12 u/s per axis (21 u/s diagonal), boost cap 16.5 u/s per axis (28.5 u/s
//! diagonal, 95% of the 30 u/s laser).
//!
//! [`simulate_movement`] is a pure function of ship, input, tuning and terrain, so
//! the client can run the exact same code for prediction.
//!
//! Paint Thrower colours (M2 plan) change the contact response, read from
//! the terrain's paint (`TerrainQuery::paint_colors_near`): red paint is
//! bouncy (walls hit at `paint_bounce_restitution`), blue paint is slippery
//! (no damping while touching it, and a wall hit keeps the ship's speed,
//! turned along the wall), yellow paint is sticky (while touching it the
//! caps and the ship's local velocity are limited to `paint_sticky_speed`
//! of the cap). Plain paint has no effect.

use std::collections::BTreeMap;

use ls_math::terrain::{TerrainQuery, PAINT_BLUE, PAINT_RED, PAINT_YELLOW};
use ls_math::{det, Quat, Vec3, DT};
use serde::{Deserialize, Serialize};

use crate::terrain::{sphere_contact, sphere_contact_distance};
use crate::{Buttons, GadgetId, InputCmd, ShipTuning, ShipType, WeaponId, WeaponSlot};

/// Player identifier, assigned by the [`crate::World`] in join order.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct PlayerId(pub u16);

/// One weapon a ship carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeaponState {
    pub weapon: WeaponId,
    /// Ammo for count weapons; unused (0) otherwise.
    pub ammo: u32,
    /// Upgrade level, starting at 1.
    pub level: u8,
    /// Paid shots fired with this weapon so far (wrapping): alternating or
    /// rotating patterns (Scatter Laser, Corkscrew) read it as `Shot::shots`.
    #[serde(default)]
    pub shots: u32,
}

/// A weapon being charged (trigger held; see `WeaponDef::charge`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChargeState {
    pub slot: WeaponSlot,
    pub weapon: WeaponId,
    /// Tick the trigger went down.
    pub start_tick: u64,
}

/// Complete state of one ship.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipState {
    pub player: PlayerId,
    pub team: u8,
    pub ship_type: ShipType,
    pub pos: Vec3,
    pub vel: Vec3,
    pub orientation: Quat,
    /// Body-frame angular velocity applied during the last tick (rad/s).
    pub ang_vel: Vec3,
    pub hull: f32,
    pub energy: f32,
    pub boost: f32,
    pub scrap: f32,
    pub alive: bool,
    /// Tick at which a dead ship respawns.
    pub respawn_tick: u64,
    /// Carried weapons, sorted by weapon id.
    pub weapons: Vec<WeaponState>,
    pub primary: Option<WeaponId>,
    pub secondary: Option<WeaponId>,
    /// First tick at which each trigger may fire again.
    pub primary_ready_tick: u64,
    pub secondary_ready_tick: u64,
    /// Whether the ship boosted during the last tick.
    pub boosting: bool,
    /// Whether the ship was in terrain contact during the last tick.
    pub in_contact: bool,
    pub horizon_assist: bool,
    /// Last player that damaged this ship (for kill credit).
    pub last_attacker: Option<PlayerId>,
    pub kills: u32,
    pub deaths: u32,
    /// Gadget in the ship's one gadget slot.
    pub gadget: Option<GadgetId>,
    /// Toggle gadgets (Shield, Cloak): currently on.
    pub gadget_active: bool,
    /// First tick the gadget may be activated again.
    pub gadget_ready_tick: u64,
    /// Weapon being charged, if any.
    pub charge: Option<ChargeState>,
    /// Buttons of the previous tick (edge detection, e.g. the gadget key).
    pub prev_buttons: Buttons,
    /// Blinded (Flash Pod) until this tick; the client darkens the view.
    pub blind_until: u64,
    /// Weapons cannot fire before this tick (Borrowed Time malus).
    pub weapons_offline_until: u64,
    /// Tick of the last shot (Cloak reveals a ship that just fired).
    pub last_fired_tick: Option<u64>,
    /// Ship type to use at the next respawn (ship choice).
    pub next_ship_type: Option<ShipType>,
    /// Module-owned state per carried weapon (Paint Thrower colour, Leech
    /// Arc lock), set with `Ctx::set_weapon_state`; 0 when absent. Kept
    /// across weapon switches, dropped with the weapon and at respawn.
    #[serde(default)]
    pub weapon_state: BTreeMap<WeaponId, u32>,
    /// Pigmented (monochrome levels, M4) until this tick: +25 % damage,
    /// drawn in colour; 0 = grey. See `crate::pigment`.
    #[serde(default)]
    pub pigment_until: u64,
}

impl ShipState {
    /// A freshly spawned ship at `pos` facing `orientation`, with spawn resources.
    pub fn new(
        player: PlayerId,
        team: u8,
        ship_type: ShipType,
        tuning: &ShipTuning,
        pos: Vec3,
        orientation: Quat,
    ) -> ShipState {
        ShipState {
            player,
            team,
            ship_type,
            pos,
            vel: Vec3::ZERO,
            orientation,
            ang_vel: Vec3::ZERO,
            hull: tuning.hull_max,
            energy: tuning.spawn_energy,
            boost: tuning.spawn_boost,
            scrap: tuning.spawn_scrap,
            alive: true,
            respawn_tick: 0,
            weapons: Vec::new(),
            primary: None,
            secondary: None,
            primary_ready_tick: 0,
            secondary_ready_tick: 0,
            boosting: false,
            in_contact: false,
            horizon_assist: true,
            last_attacker: None,
            kills: 0,
            deaths: 0,
            gadget: None,
            gadget_active: false,
            gadget_ready_tick: 0,
            charge: None,
            prev_buttons: Buttons::NONE,
            blind_until: 0,
            weapons_offline_until: 0,
            last_fired_tick: None,
            next_ship_type: None,
            weapon_state: BTreeMap::new(),
            pigment_until: 0,
        }
    }

    pub fn weapon(&self, w: WeaponId) -> Option<&WeaponState> {
        self.weapons.iter().find(|s| s.weapon == w)
    }

    pub fn weapon_mut(&mut self, w: WeaponId) -> Option<&mut WeaponState> {
        self.weapons.iter_mut().find(|s| s.weapon == w)
    }

    /// Adds a weapon (kept sorted by id). Returns false if already carried.
    pub fn add_weapon(&mut self, state: WeaponState) -> bool {
        match self
            .weapons
            .binary_search_by(|s| s.weapon.cmp(&state.weapon))
        {
            Ok(_) => false,
            Err(i) => {
                self.weapons.insert(i, state);
                true
            }
        }
    }

    pub fn remove_weapon(&mut self, w: WeaponId) {
        self.weapons.retain(|s| s.weapon != w);
        self.weapon_state.remove(&w);
        if self.primary == Some(w) {
            self.primary = None;
        }
        if self.secondary == Some(w) {
            self.secondary = None;
        }
    }

    /// Module-owned state of weapon `w` (0 if none was set).
    pub fn weapon_state_of(&self, w: WeaponId) -> u32 {
        self.weapon_state.get(&w).copied().unwrap_or(0)
    }

    /// Velocity expressed in ship-local axes.
    pub fn local_velocity(&self) -> Vec3 {
        self.orientation.conjugate().rotate(self.vel)
    }

    pub fn speed(&self) -> f32 {
        self.vel.length()
    }

    /// Whether the ship carries pigment at tick `now` (M4).
    pub fn pigmented(&self, now: u64) -> bool {
        now < self.pigment_until
    }
}

/// What happened during one movement step (used for resources and tests).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MovementReport {
    /// Speed (u/s) removed by hard braking this tick, summed over axes.
    pub hard_braked: f32,
    /// Boost gained from braking and scraping this tick.
    pub boost_gained: f32,
    /// True if the ship touched terrain.
    pub contact: bool,
    /// Largest normal speed lost against terrain this tick.
    pub impact_speed: f32,
}

#[inline]
fn approach(v: f32, target: f32, step: f32) -> f32 {
    if v < target {
        (v + step).min(target)
    } else {
        (v - step).max(target)
    }
}

/// One local axis of the flight model. Returns the new axis speed and the
/// amount of speed removed by active (reverse-thrust) braking. Without
/// `damping` (slippery paint) an axis without input keeps its speed.
fn axis_step(v: f32, input: f32, cap: f32, t: &ShipTuning, damping: bool) -> (f32, f32) {
    axis_step_pulled(v, input, cap, t, damping, 0.0, 0.0)
}

/// [`axis_step`] with a well's pull `g` (u/s^2) along this axis. With
/// input (or without damping) the pull is simply added after the step, so
/// thrust (at least 2.5x the clamped pull) always wins. Without input the
/// dampers aim at the drift speed `g * drift_secs` (at most the cap)
/// instead of 0: the ship falls freely toward the well up to that speed,
/// faster motion along the pull and any motion against it is damped as
/// usual. `g == 0` is exactly [`axis_step`].
fn axis_step_pulled(
    v: f32,
    input: f32,
    cap: f32,
    t: &ShipTuning,
    damping: bool,
    g: f32,
    drift_secs: f32,
) -> (f32, f32) {
    if g != 0.0 {
        if input != 0.0 || !damping {
            let (nv, braked) = axis_step_pulled(v, input, cap, t, damping, 0.0, 0.0);
            return (nv + g * DT, braked);
        }
        let v1 = v + g * DT;
        let drift = (g * drift_secs).clamp(-cap, cap);
        // Falling toward the well, slower than the drift: no damping.
        if v1 * g >= 0.0 && v1.abs() <= drift.abs() {
            return (v1, 0.0);
        }
        let rate = if v1.abs() > cap {
            t.overspeed_decel.max(t.damping_decel)
        } else {
            t.damping_decel
        };
        return (approach(v1, drift, rate * DT), 0.0);
    }
    let target = input * cap;
    if input == 0.0 {
        if !damping {
            return (v, 0.0);
        }
        // Damping; excess speed above the cap bleeds off faster.
        let rate = if v.abs() > cap {
            t.overspeed_decel.max(t.damping_decel)
        } else {
            t.damping_decel
        };
        return (approach(v, 0.0, rate * DT), 0.0);
    }
    if v * input < 0.0 {
        // Thrusting against the motion: brake.
        let nv = approach(v, target, t.brake_accel * DT);
        let removed = if nv * v > 0.0 {
            v.abs() - nv.abs()
        } else {
            v.abs()
        };
        return (nv, removed);
    }
    if v.abs() < target.abs() {
        (approach(v, target, t.thrust_accel * DT), 0.0)
    } else {
        // Faster than this input asks for (partial input, or above the cap).
        let rate = if v.abs() > cap {
            t.overspeed_decel
        } else {
            t.damping_decel
        };
        (approach(v, target, rate * DT), 0.0)
    }
}

/// Applies look, roll and horizon assist (levelling against `world_up`)
/// to the orientation.
fn rotate_ship(ship: &mut ShipState, input: &InputCmd, t: &ShipTuning, world_up: Vec3) {
    let max_turn = t.max_turn_rate * DT;
    let pitch = input.look_pitch.clamp(-max_turn, max_turn);
    let yaw = input.look_yaw.clamp(-max_turn, max_turn);
    let mut roll_rate = input.roll * t.roll_rate;

    if ship.horizon_assist && input.roll == 0.0 {
        // Roll angle relative to the level's up, measured about the forward
        // axis. World +Y (the usual up) keeps the exact M1 arithmetic.
        let fwd = ship.orientation.forward();
        let y = world_up == Vec3::Y;
        let fwd_up = if y { fwd.y } else { fwd.dot(world_up) };
        if fwd_up.abs() < 0.95 {
            let right = ship.orientation.right();
            let up = ship.orientation.up();
            let angle = if y {
                det::atan2(right.y, up.y)
            } else {
                det::atan2(right.dot(world_up), up.dot(world_up))
            };
            let correction =
                (-angle * t.horizon_gain).clamp(-t.horizon_max_rate, t.horizon_max_rate);
            roll_rate += correction;
        }
    }

    let omega = Vec3::new(pitch / DT, yaw / DT, roll_rate);
    ship.ang_vel = omega;
    ship.orientation = ship.orientation.integrate_local(omega, DT);
}

/// How a moving sphere responds to one terrain contact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ContactResponse {
    /// Share of the normal speed reflected (0 = slide, 1 = bounce).
    pub restitution: f32,
    /// Keep the speed: after the normal part is removed, the velocity along
    /// the surface is scaled back up towards the speed before the hit
    /// (slippery; at most doubled, so a head-on hit does not shoot the
    /// ship sideways).
    pub keep_speed: bool,
}

/// Moves a sphere through terrain, sliding along surfaces (and bouncing with
/// `restitution`). Returns `(contact, max normal speed removed)`. Shared by
/// ships and debris.
pub(crate) fn move_sphere<T: TerrainQuery + ?Sized>(
    terrain: &T,
    pos: &mut Vec3,
    vel: &mut Vec3,
    radius: f32,
    restitution: f32,
) -> (bool, f32) {
    let plain = ContactResponse {
        restitution,
        keep_speed: false,
    };
    move_sphere_with(terrain, pos, vel, radius, |_| plain)
}

/// [`move_sphere`] with a per-contact response: `response(p)` for the
/// surface point `p` the sphere touches.
pub(crate) fn move_sphere_with<T: TerrainQuery + ?Sized>(
    terrain: &T,
    pos: &mut Vec3,
    vel: &mut Vec3,
    radius: f32,
    response: impl Fn(Vec3) -> ContactResponse,
) -> (bool, f32) {
    let disp = vel.length() * DT;
    // Substep so a single step never moves more than half the radius.
    let steps = det::ceil(disp / (radius * 0.5)).clamp(1.0, 16.0) as u32;
    let dt = DT / steps as f32;
    let mut contact = false;
    let mut impact = 0.0f32;
    for _ in 0..steps {
        *pos += *vel * dt;
        for _ in 0..4 {
            // The field is only a lower bound on the distance: near the rock,
            // contact uses the first-order estimate `d / |grad d|`.
            let Some(c) = sphere_contact(terrain, *pos, radius) else {
                break;
            };
            let (d, n) = (c.distance, c.normal);
            if d >= radius {
                break;
            }
            contact = true;
            let r = response(*pos - n * d);
            *pos += n * (radius - d + 1e-4);
            let vn = vel.dot(n);
            if vn < 0.0 {
                impact = impact.max(-vn);
                let speed = vel.length();
                *vel -= n * (vn * (1.0 + r.restitution));
                let along = vel.length();
                if r.keep_speed && along > 0.0 {
                    *vel *= (speed / along).min(2.0);
                }
            }
        }
    }
    (contact, impact)
}

/// A ship within this distance of the surface (beyond its radius) touches
/// the paint there (sticky and slippery paint).
pub const PAINT_TOUCH_MARGIN: f32 = 0.2;
/// Radius around a contact point in which paint decides the contact
/// response (bouncy and slippery paint).
const PAINT_CONTACT_RADIUS: f32 = 0.5;

/// Paint colour bits (`1 << PAINT_*`) under a ship at `pos`: the painted
/// surface cells overlapping its sphere grown by [`PAINT_TOUCH_MARGIN`], or
/// none when it is further from the surface.
pub fn paint_touching<T: TerrainQuery + ?Sized>(terrain: &T, pos: Vec3, radius: f32) -> u8 {
    let reach = radius + PAINT_TOUCH_MARGIN;
    if sphere_contact_distance(terrain, pos, reach) >= reach {
        return 0;
    }
    terrain.paint_colors_near(pos, reach)
}

/// The level around a ship during one movement step (M4): the well pull
/// at the ship's position and the level's up. [`MoveEnv::DEFAULT`] (no
/// pull, world +Y) is the M1-M3 flight model exactly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveEnv {
    /// Well acceleration (u/s^2) at the ship, already clamped
    /// (`WellField::ship_accel`).
    pub gravity: Vec3,
    /// The level's up (`World::up`; horizon assist levels the roll to it).
    pub up: Vec3,
    /// Dampers aim at the drift speed `gravity * drift_secs` on axes
    /// without input (`GravitySettings::drift_secs`).
    pub drift_secs: f32,
}

impl MoveEnv {
    pub const DEFAULT: MoveEnv = MoveEnv {
        gravity: Vec3::ZERO,
        up: Vec3::Y,
        drift_secs: 0.0,
    };
}

impl Default for MoveEnv {
    fn default() -> Self {
        MoveEnv::DEFAULT
    }
}

/// One tick of rotation, thrust, boost, energy regen, terrain collision and
/// boost refills (hard braking, near-miss scrapes) for a living ship, with
/// the paint effects of the surface it touches (see the module docs).
/// No wells, world +Y up: [`simulate_movement_env`] with [`MoveEnv::DEFAULT`].
pub fn simulate_movement<T: TerrainQuery + ?Sized>(
    ship: &mut ShipState,
    input: &InputCmd,
    tuning: &ShipTuning,
    terrain: &T,
) -> MovementReport {
    simulate_movement_env(ship, input, tuning, terrain, &MoveEnv::DEFAULT)
}

/// [`simulate_movement`] in a level with wells and a turned up (M4): the
/// pull is applied per local axis inside the flight model (the dampers do
/// not fight it; see `axis_step_pulled`), and horizon assist levels the
/// roll against `env.up`. The server and the client predictor call this
/// with the same environment.
pub fn simulate_movement_env<T: TerrainQuery + ?Sized>(
    ship: &mut ShipState,
    input: &InputCmd,
    tuning: &ShipTuning,
    terrain: &T,
    env: &MoveEnv,
) -> MovementReport {
    let input = input.sanitized();
    let t = tuning;
    ship.horizon_assist = input.buttons.contains(Buttons::HORIZON_ASSIST);
    rotate_ship(ship, &input, t, env.up);

    let touching = paint_touching(terrain, ship.pos, t.radius);
    let sticky = touching & (1 << PAINT_YELLOW) != 0;
    let slippery = touching & (1 << PAINT_BLUE) != 0;

    let thrusting = input.thrust != Vec3::ZERO;
    let boosting = input.buttons.contains(Buttons::BOOST) && ship.boost > 0.0 && thrusting;
    ship.boosting = boosting;
    let mut cap = if boosting {
        t.boost_speed
    } else {
        t.cruise_speed
    };
    if sticky {
        cap *= t.paint_sticky_speed;
    }

    let mut report = MovementReport::default();
    let local = ship.orientation.conjugate().rotate(ship.vel);
    let pull = if env.gravity == Vec3::ZERO {
        None
    } else {
        Some(ship.orientation.conjugate().rotate(env.gravity))
    };
    let mut new_local = [0.0f32; 3];
    for (i, out) in new_local.iter_mut().enumerate() {
        let v = local[i];
        let (nv, braked) = match pull {
            None => axis_step(v, input.thrust[i], cap, t, !slippery),
            Some(g) => {
                axis_step_pulled(v, input.thrust[i], cap, t, !slippery, g[i], env.drift_secs)
            }
        };
        if braked > 0.0 && v.abs() > t.hard_brake_min_speed {
            report.hard_braked += braked;
        }
        *out = nv;
    }
    if sticky {
        // Sticky paint holds the ship to a share of its (per-axis) caps at
        // once. The whole vector is scaled, so a diagonal keeps its direction
        // and gets the same share of the speed free flight allows that way.
        let m = new_local.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        if m > cap {
            let k = cap / m;
            for v in &mut new_local {
                *v *= k;
            }
        }
    }
    ship.vel = ship.orientation.rotate(Vec3::from_array(new_local));

    // Resources that depend on boosting.
    if boosting {
        ship.boost = (ship.boost - t.boost_drain * DT).max(0.0);
    } else {
        ship.energy = (ship.energy + t.energy_regen * DT).min(t.energy_max);
    }

    let (contact, impact) =
        move_sphere_with(terrain, &mut ship.pos, &mut ship.vel, t.radius, |p| {
            let colors = terrain.paint_colors_near(p, PAINT_CONTACT_RADIUS);
            if colors & (1 << PAINT_RED) != 0 {
                ContactResponse {
                    restitution: t.restitution.max(t.paint_bounce_restitution),
                    keep_speed: false,
                }
            } else {
                let slippery = colors & (1 << PAINT_BLUE) != 0;
                ContactResponse {
                    restitution: if slippery { 0.0 } else { t.restitution },
                    keep_speed: slippery,
                }
            }
        });
    ship.in_contact = contact;
    report.contact = contact;
    report.impact_speed = impact;

    // Boost refills: hard braking and near-miss wall scrapes (not actual contact).
    let mut gained = report.hard_braked * t.brake_refill_per_speed;
    if !contact && t.scrape_distance > 0.0 {
        let reach = t.radius + t.scrape_distance;
        let gap = sphere_contact_distance(terrain, ship.pos, reach) - t.radius;
        if gap > 0.0 && gap < t.scrape_distance && ship.vel.length() >= t.scrape_min_speed {
            gained += t.scrape_refill * (1.0 - gap / t.scrape_distance) * DT;
        }
    }
    let before = ship.boost;
    ship.boost = (ship.boost + gained).min(t.boost_max);
    report.boost_gained = ship.boost - before;
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::terrain::{BoxRoomTerrain, PlaneTerrain};
    use ls_math::Aabb;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn big_room() -> BoxRoomTerrain {
        BoxRoomTerrain::centered(Vec3::splat(2000.0))
    }

    fn ship() -> ShipState {
        ShipState::new(
            PlayerId(0),
            0,
            ShipType::Tinker,
            &ShipTuning::tinker(),
            Vec3::ZERO,
            Quat::IDENTITY,
        )
    }

    fn fly(ship: &mut ShipState, input: InputCmd, ticks: u32) {
        let t = ShipTuning::tinker();
        let room = big_room();
        for _ in 0..ticks {
            simulate_movement(ship, &input, &t, &room);
        }
    }

    const FWD: Vec3 = Vec3::new(0.0, 0.0, -1.0);
    const DIAG: Vec3 = Vec3::new(1.0, 1.0, -1.0);

    #[test]
    fn straight_cruise_is_12() {
        let mut s = ship();
        fly(&mut s, InputCmd::idle(0).with_thrust(FWD), 120);
        assert_eq!(s.speed(), 12.0);
        assert!(s.vel.z < 0.0, "moves forward (-Z)");
    }

    #[test]
    fn straight_boost_is_16_5() {
        let mut s = ship();
        s.boost = 100.0;
        fly(
            &mut s,
            InputCmd::idle(0)
                .with_thrust(FWD)
                .with_buttons(Buttons::BOOST),
            90,
        );
        assert_eq!(s.speed(), 16.5);
    }

    #[test]
    fn diagonal_speeds_match_design_doc() {
        let laser_speed = 30.0f32;
        let mut s = ship();
        fly(&mut s, InputCmd::idle(0).with_thrust(DIAG), 120);
        // Design doc: 3-axis diagonal cruise ~21 u/s (12 * sqrt 3 = 20.78).
        assert!((s.speed() - 21.0).abs() < 0.25, "{}", s.speed());
        assert!((s.speed() - 12.0 * 3.0f32.sqrt()).abs() < 1e-4);

        let mut s = ship();
        s.boost = 100.0;
        fly(
            &mut s,
            InputCmd::idle(0)
                .with_thrust(DIAG)
                .with_buttons(Buttons::BOOST),
            90,
        );
        // Design doc: 28.5 u/s, 95% of laser speed, but always slower than a laser.
        assert!((s.speed() - 28.5).abs() < 0.1, "{}", s.speed());
        assert!((s.speed() / laser_speed - 0.95).abs() < 0.01);
        assert!(s.speed() < laser_speed);
        assert!((ShipTuning::tinker().top_speed() - s.speed()).abs() < 1e-4);
    }

    #[test]
    fn caps_are_in_ship_space() {
        // Yawed 37 degrees and pitched: forward thrust still caps at 12 u/s total.
        let mut s = ship();
        s.orientation = Quat::from_axis_angle(Vec3::Y, 0.65) * Quat::from_axis_angle(Vec3::X, 0.3);
        fly(&mut s, InputCmd::idle(0).with_thrust(FWD), 120);
        assert!((s.speed() - 12.0).abs() < 1e-4, "{}", s.speed());
        let dir = s.vel.normalize();
        assert!((dir - s.orientation.forward()).length() < 1e-4);
    }

    #[test]
    fn damping_stops_the_ship() {
        let mut s = ship();
        fly(&mut s, InputCmd::idle(0).with_thrust(FWD), 60);
        let v = s.speed();
        fly(&mut s, InputCmd::idle(0), 30);
        assert!(s.speed() < v);
        fly(&mut s, InputCmd::idle(0), 200);
        assert_eq!(s.speed(), 0.0);
    }

    #[test]
    fn overspeed_bleeds_off_after_boost() {
        let mut s = ship();
        s.boost = 100.0;
        fly(
            &mut s,
            InputCmd::idle(0)
                .with_thrust(FWD)
                .with_buttons(Buttons::BOOST),
            90,
        );
        // Keep thrusting without boost: speed falls back to the cruise cap.
        fly(&mut s, InputCmd::idle(0).with_thrust(FWD), 60);
        assert_eq!(s.speed(), 12.0);
    }

    #[test]
    fn look_is_clamped_to_turn_rate() {
        let mut s = ship();
        let input = InputCmd {
            look_yaw: 10.0,
            ..InputCmd::idle(0)
        };
        fly(&mut s, input, 1);
        let max = ShipTuning::tinker().max_turn_rate * DT;
        let angle = s.orientation.angle_between(Quat::IDENTITY);
        assert!((angle - max).abs() < 1e-4, "{angle} vs {max}");
    }

    #[test]
    fn horizon_assist_levels_roll() {
        let rolled = Quat::from_axis_angle(Vec3::Z, 0.8);
        let roll_of = |q: Quat| det::atan2(q.right().y, q.up().y).abs();

        let mut with = ship();
        with.orientation = rolled;
        fly(
            &mut with,
            InputCmd::idle(0).with_buttons(Buttons::HORIZON_ASSIST),
            240,
        );
        assert!(
            roll_of(with.orientation) < 0.05,
            "{}",
            roll_of(with.orientation)
        );

        let mut without = ship();
        without.orientation = rolled;
        fly(&mut without, InputCmd::idle(0), 240);
        assert!((roll_of(without.orientation) - 0.8).abs() < 1e-3);
    }

    #[test]
    fn energy_regens_only_while_not_boosting() {
        let t = ShipTuning::tinker();
        let mut s = ship();
        s.energy = 10.0;
        s.boost = 100.0;
        fly(
            &mut s,
            InputCmd::idle(0)
                .with_thrust(FWD)
                .with_buttons(Buttons::BOOST),
            60,
        );
        assert_eq!(s.energy, 10.0, "no regen while boosting");
        assert!((s.boost - (100.0 - t.boost_drain)).abs() < 1e-3);
        fly(&mut s, InputCmd::idle(0).with_thrust(FWD), 60);
        assert!((s.energy - (10.0 + t.energy_regen)).abs() < 1e-3);
    }

    #[test]
    fn boost_without_boost_meter_is_cruise() {
        let mut s = ship();
        s.boost = 0.0;
        fly(
            &mut s,
            InputCmd::idle(0)
                .with_thrust(FWD)
                .with_buttons(Buttons::BOOST),
            120,
        );
        assert_eq!(s.speed(), 12.0);
        assert!(!s.boosting);
    }

    #[test]
    fn hard_braking_refills_boost() {
        let mut s = ship();
        s.boost = 0.0;
        fly(&mut s, InputCmd::idle(0).with_thrust(FWD), 60);
        assert_eq!(s.boost, 0.0, "cruising alone does not refill boost");
        fly(&mut s, InputCmd::idle(0).with_thrust(-FWD), 20);
        assert!(s.boost > 2.0, "{}", s.boost);
        // Gentle coasting (damping) does not refill.
        let mut c = ship();
        c.boost = 0.0;
        fly(&mut c, InputCmd::idle(0).with_thrust(FWD), 60);
        fly(&mut c, InputCmd::idle(0), 120);
        assert_eq!(c.boost, 0.0);
    }

    #[test]
    fn near_miss_scrape_refills_boost_but_contact_does_not() {
        let t = ShipTuning::tinker();
        let bounds = Aabb::new(Vec3::splat(-500.0), Vec3::splat(500.0));
        let floor = PlaneTerrain::floor(0.0, bounds);
        // Skim 0.2 u above the floor at cruise speed.
        let mut s = ship();
        s.pos = Vec3::new(0.0, t.radius + 0.2, 0.0);
        s.boost = 0.0;
        s.vel = Vec3::new(0.0, 0.0, -12.0);
        let input = InputCmd::idle(0).with_thrust(FWD);
        for _ in 0..30 {
            simulate_movement(&mut s, &input, &t, &floor);
        }
        assert!(s.boost > 1.0, "{}", s.boost);

        // Far from walls: nothing.
        let mut far = ship();
        far.pos = Vec3::new(0.0, 50.0, 0.0);
        far.boost = 0.0;
        far.vel = Vec3::new(0.0, 0.0, -12.0);
        for _ in 0..30 {
            simulate_movement(&mut far, &input, &t, &floor);
        }
        assert_eq!(far.boost, 0.0);
    }

    #[test]
    fn terrain_collision_stops_penetration() {
        let t = ShipTuning::tinker();
        let bounds = Aabb::new(Vec3::splat(-500.0), Vec3::splat(500.0));
        let floor = PlaneTerrain::floor(0.0, bounds);
        let mut s = ship();
        s.pos = Vec3::new(0.0, 3.0, 0.0);
        // Dive straight down fast (e.g. after knockback).
        s.vel = Vec3::new(0.0, -60.0, 0.0);
        let mut touched = false;
        for _ in 0..60 {
            let r = simulate_movement(&mut s, &InputCmd::idle(0), &t, &floor);
            touched |= r.contact;
            assert!(s.pos.y >= t.radius - 1e-3, "{:?}", s.pos);
        }
        assert!(touched);
        // Level bounds are solid too.
        let mut b = ship();
        b.pos = Vec3::new(498.0, 50.0, 0.0);
        b.vel = Vec3::new(60.0, 0.0, 0.0);
        for _ in 0..30 {
            simulate_movement(&mut b, &InputCmd::idle(0), &t, &floor);
        }
        assert!(b.pos.x <= 500.0 - t.radius + 1e-3, "{:?}", b.pos);
    }

    /// A terrain whose field is `scale` times an exact one: a lower bound on
    /// the distance, like the cave generator's noise (about 0.3 of it).
    struct Bound<T> {
        inner: T,
        scale: f32,
    }

    impl<T: TerrainQuery> TerrainQuery for Bound<T> {
        fn sdf(&self, p: Vec3) -> f32 {
            self.inner.sdf(p) * self.scale
        }
        fn bounds(&self) -> Aabb {
            self.inner.bounds()
        }
    }

    /// A thin horizontal slab of rock (`|y| < half`), exact.
    struct Slab {
        half: f32,
        bounds: Aabb,
    }

    impl TerrainQuery for Slab {
        fn sdf(&self, p: Vec3) -> f32 {
            p.y.abs() - self.half
        }
        fn bounds(&self) -> Aabb {
            self.bounds
        }
    }

    #[test]
    fn contact_uses_the_distance_estimate_on_bound_fields() {
        let t = ShipTuning::tinker();
        let bounds = Aabb::new(Vec3::splat(-500.0), Vec3::splat(500.0));
        let floor = Bound {
            inner: PlaneTerrain::floor(0.0, bounds),
            scale: 0.3,
        };
        // The ship settles on the visible floor, not on a shell r / 0.3 away.
        let mut s = ship();
        s.pos = Vec3::new(0.0, 3.0, 0.0);
        s.vel = Vec3::new(0.0, -20.0, 0.0);
        let mut lowest = f32::INFINITY;
        for _ in 0..60 {
            if simulate_movement(&mut s, &InputCmd::idle(0), &t, &floor).contact {
                lowest = lowest.min(s.pos.y);
            }
        }
        // (After the push the bounce moves it on within the same tick.)
        assert!(
            lowest >= t.radius - 0.01 && lowest < t.radius + 0.1,
            "{lowest}"
        );
        // Paint touching and scrapes measure the same estimated gap.
        let skim = Vec3::new(0.0, t.radius + 0.1, 0.0);
        assert!(sphere_contact_distance(&floor, skim, t.radius + 1.0) > t.radius + 0.09);

        // No tunneling through a thin wall that reports a bound distance.
        let slab = Bound {
            inner: Slab { half: 0.2, bounds },
            scale: 0.3,
        };
        let mut d = ship();
        d.pos = Vec3::new(0.0, 3.0, 0.0);
        d.vel = Vec3::new(0.0, -60.0, 0.0);
        for _ in 0..60 {
            simulate_movement(&mut d, &InputCmd::idle(0), &t, &slab);
            assert!(d.pos.y >= 0.2 + t.radius - 0.02, "{:?}", d.pos);
        }
    }

    /// Ships and debris bouncing around a cratered room with a bound field:
    /// the contact estimate (gradient, `sqrt`, division) gives the same bits
    /// on every platform (also run on wasm32, see `CLAUDE.md`).
    #[test]
    fn bound_field_contact_golden_hash() {
        let mut room = BoxRoomTerrain::centered(Vec3::new(30.0, 16.0, 30.0));
        for (c, r) in [
            (Vec3::new(0.0, -8.0, 0.0), 5.0),
            (Vec3::new(15.0, 0.0, 3.0), 4.0),
            (Vec3::new(-6.0, 8.0, -15.0), 6.0),
        ] {
            room.apply(ls_math::terrain::TerrainEdit::SubtractSphere {
                center: c,
                radius: r,
            });
        }
        room.apply(ls_math::terrain::TerrainEdit::AddSphere {
            center: Vec3::new(3.0, 0.0, 4.0),
            radius: 3.0,
        });
        let terrain = Bound {
            inner: room,
            scale: 0.3,
        };
        let table = crate::ShipTable::builtin();
        let mut ships: Vec<ShipState> = ShipType::ALL
            .iter()
            .enumerate()
            .map(|(i, &ty)| {
                let mut s = ShipState::new(
                    PlayerId(i as u16),
                    0,
                    ty,
                    table.get(ty),
                    Vec3::new(-8.0 + 5.0 * i as f32, 2.0, -6.0),
                    Quat::from_axis_angle(Vec3::Y, i as f32),
                );
                s.vel = Vec3::new(9.0, -4.0 + i as f32, 7.0 - 3.0 * i as f32);
                s
            })
            .collect();
        let mut debris = (Vec3::new(5.0, 4.0, 5.0), Vec3::new(-14.0, -6.0, 11.0));
        let mut h = crate::StateHasher::new();
        for tick in 0..900u64 {
            for (i, s) in ships.iter_mut().enumerate() {
                let yaw = if (tick / 40 + i as u64) % 2 == 0 {
                    0.03
                } else {
                    -0.02
                };
                let mut input = InputCmd::idle(tick).with_thrust(Vec3::new(0.3, -0.2, -1.0));
                input.look_yaw = yaw;
                input.look_pitch = 0.01;
                simulate_movement(s, &input, table.get(s.ship_type), &terrain);
            }
            move_sphere(&terrain, &mut debris.0, &mut debris.1, 0.6, 0.3);
        }
        for s in &ships {
            for v in [s.pos, s.vel] {
                for c in v.to_array() {
                    h.write_bytes(&c.to_bits().to_le_bytes());
                }
            }
            h.write_bytes(&s.boost.to_bits().to_le_bytes());
        }
        for c in debris.0.to_array() {
            h.write_bytes(&c.to_bits().to_le_bytes());
        }
        let hash = h.finish();
        assert_eq!(
            hash, 0x75cf_cfbd_ba14_3858,
            "bound-field contact hash {hash:#018x}"
        );
    }

    /// Design doc ship table, flown: the top speed each type reaches
    /// (boosting on the 3-axis diagonal) relative to the 30 u/s laser, and
    /// how fast each turns (Skitter fastest, Hauler slowest).
    #[test]
    fn ship_types_fly_at_their_design_speeds_and_turn_rates() {
        let table = crate::ShipTable::builtin();
        let room = big_room();
        let mut turned = Vec::new();
        for (ty, ratio) in [
            (ShipType::Skitter, 0.98),
            (ShipType::Tinker, 0.95),
            (ShipType::Hauler, 0.85),
            (ShipType::Tug, 0.90),
        ] {
            let t = table.get(ty);
            let mut s = ShipState::new(PlayerId(0), 0, ty, t, Vec3::ZERO, Quat::IDENTITY);
            s.boost = t.boost_max;
            let input = InputCmd::idle(0)
                .with_thrust(DIAG)
                .with_buttons(Buttons::BOOST);
            for _ in 0..90 {
                simulate_movement(&mut s, &input, t, &room);
            }
            let top = s.speed();
            assert!((top / 30.0 - ratio).abs() < 0.006, "{ty:?}: {top} u/s");
            assert!(top < 30.0, "{ty:?} never outruns a laser");
            // A sixth of a second of full yaw input (under half a turn).
            let mut s = ShipState::new(PlayerId(0), 0, ty, t, Vec3::ZERO, Quat::IDENTITY);
            let yaw = InputCmd {
                look_yaw: 1.0,
                ..InputCmd::idle(0)
            };
            for _ in 0..10 {
                simulate_movement(&mut s, &yaw, t, &room);
            }
            let f = s.orientation.forward();
            let angle = det::atan2(-f.x, -f.z);
            let expected = t.max_turn_rate * 10.0 / 60.0;
            assert!(
                (angle - expected).abs() < 1e-3,
                "{ty:?}: {angle} vs {expected}"
            );
            turned.push((ty, angle));
        }
        let angle = |ty| turned.iter().find(|(t, _)| *t == ty).unwrap().1;
        assert!(angle(ShipType::Skitter) > angle(ShipType::Tinker));
        assert!(angle(ShipType::Tinker) > angle(ShipType::Tug));
        assert!(angle(ShipType::Tug) > angle(ShipType::Hauler));
    }

    /// Design doc: the Tug passes scrap to teammates (hull first, at
    /// `scrap_pass_rate`), never to enemies or ships out of reach; other
    /// types cannot pass scrap.
    #[test]
    fn tug_passes_scrap_only_to_teammates_in_reach() {
        use crate::{MatchSetup, SpawnPoint, World, WorldConfig};
        let spawn = |x: f32, team| SpawnPoint {
            pos: Vec3::new(x, 0.0, 0.0),
            team,
            orientation: Quat::IDENTITY,
        };
        let setup = MatchSetup {
            spawns: vec![spawn(-5.0, 0), spawn(5.0, 1)],
            pickup_points: vec![],
            instability: 0,
            heist: None,
            ..Default::default()
        };
        let mut w = World::new(WorldConfig::paint_the_void(3, setup)).unwrap();
        let mut room = BoxRoomTerrain::centered(Vec3::splat(60.0));
        let tug = w.add_player(Some(0), ShipType::Tug).unwrap();
        let enemy = w.add_player(Some(1), ShipType::Tinker).unwrap();
        let far_mate = w.add_player(Some(0), ShipType::Hauler).unwrap();
        let rate = crate::ShipTable::builtin()
            .get(ShipType::Tug)
            .scrap_pass_rate;
        let reach = crate::ShipTable::builtin()
            .get(ShipType::Tug)
            .scavenge_radius;
        assert!(rate > 0.0);
        let set = |w: &mut World, id, pos: Vec3, hull: f32, scrap: f32| {
            let s = w.ship_mut(id).unwrap();
            s.pos = pos;
            s.vel = Vec3::ZERO;
            s.hull = hull;
            s.scrap = scrap;
        };
        set(&mut w, tug, Vec3::ZERO, 110.0, 50.0);
        set(&mut w, enemy, Vec3::new(1.5, 0.0, 0.0), 50.0, 0.0);
        set(
            &mut w,
            far_mate,
            Vec3::new(0.0, 0.0, reach + 1.0),
            50.0,
            0.0,
        );
        let scavenge = [(tug, InputCmd::idle(0).with_buttons(Buttons::SCAVENGE))];
        for _ in 0..30 {
            w.step(&mut room, &scavenge);
        }
        assert_eq!(w.ship(tug).unwrap().scrap, 50.0, "nobody to pass to");
        assert_eq!(w.ship(enemy).unwrap().hull, 50.0);
        // The teammate comes into reach: hull first, then its scrap store.
        set(
            &mut w,
            far_mate,
            Vec3::new(0.0, 0.0, reach - 0.5),
            145.0,
            0.0,
        );
        for _ in 0..60 {
            w.step(&mut room, &scavenge);
        }
        let m = w.ship(far_mate).unwrap();
        assert!((w.ship(tug).unwrap().scrap - (50.0 - rate)).abs() < 1e-3);
        assert!((m.hull - 150.0).abs() < 1e-3, "{}", m.hull);
        assert!((m.scrap - (rate - 5.0)).abs() < 1e-3, "{}", m.scrap);
    }

    /// A room whose walls are charged from tick 0 to `until` (a stand-in for
    /// Leech Arc surface states, which only the real chunk store keeps).
    struct ChargedRoom {
        inner: BoxRoomTerrain,
        until: u64,
    }

    impl TerrainQuery for ChargedRoom {
        fn sdf(&self, p: Vec3) -> f32 {
            self.inner.sdf(p)
        }
        fn bounds(&self) -> Aabb {
            self.inner.bounds()
        }
    }

    impl crate::SimTerrain for ChargedRoom {
        fn apply_edit(&mut self, _edit: &ls_math::terrain::TerrainEdit) {}
        fn surface_states_near(
            &self,
            center: Vec3,
            radius: f32,
            now: u64,
        ) -> ls_math::terrain::SurfaceStates {
            if now < self.until && self.inner.sdf(center) <= radius {
                ls_math::terrain::SurfaceStates::CHARGED
            } else {
                ls_math::terrain::SurfaceStates::NONE
            }
        }
    }

    /// M2 plan: charged surfaces drain 5 energy/s from ships scraping them
    /// (on top of the normal regen); ships in open space are not drained.
    #[test]
    fn scraping_a_charged_surface_drains_five_energy_per_second() {
        use crate::{MatchSetup, SpawnPoint, World, WorldConfig};
        let spawn = |x: f32, team| SpawnPoint {
            pos: Vec3::new(x, 0.0, 0.0),
            team,
            orientation: Quat::IDENTITY,
        };
        let setup = MatchSetup {
            spawns: vec![spawn(-5.0, 0), spawn(5.0, 1)],
            pickup_points: vec![],
            instability: 0,
            heist: None,
            ..Default::default()
        };
        let mut w = World::new(WorldConfig::paint_the_void(3, setup)).unwrap();
        let mut t = ChargedRoom {
            inner: BoxRoomTerrain::centered(Vec3::new(40.0, 20.0, 40.0)),
            until: 120,
        };
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        // a sits on the floor (y = -10), pressing into it; b floats free.
        let floor_y = -10.0 + ShipTuning::tinker().radius;
        for (id, pos) in [(a, Vec3::new(0.0, floor_y, 0.0)), (b, Vec3::ZERO)] {
            let s = w.ship_mut(id).unwrap();
            s.pos = pos;
            s.energy = 50.0;
        }
        let press_down = InputCmd::idle(0).with_thrust(Vec3::NEG_Y);
        let inputs = [(a, press_down), (b, InputCmd::idle(0))];
        for _ in 0..60 {
            w.step(&mut t, &inputs);
        }
        let ea = w.ship(a).unwrap().energy;
        let eb = w.ship(b).unwrap().energy;
        assert!(w.ship(a).unwrap().in_contact);
        // Regen 10/s for both; a loses 5/s to the charged floor.
        assert!((eb - 60.0).abs() < 0.05, "{eb}");
        assert!((ea - 55.0).abs() < 0.05, "{ea}");
        // After the charge expires (tick 120) the drain stops.
        for _ in 0..60 {
            w.step(&mut t, &inputs);
        }
        let (ea, eb) = (w.ship(a).unwrap().energy, w.ship(b).unwrap().energy);
        assert!(
            (ea - 60.0).abs() < 0.1 && (eb - 70.0).abs() < 0.1,
            "{ea} {eb}"
        );
        for _ in 0..60 {
            w.step(&mut t, &inputs);
        }
        let (ea2, eb2) = (w.ship(a).unwrap().energy, w.ship(b).unwrap().energy);
        assert!((ea2 - ea - 10.0).abs() < 0.05, "{ea} -> {ea2}");
        assert!((eb2 - eb - 10.0).abs() < 0.05, "{eb} -> {eb2}");
    }

    /// A 40 x 20 x 40 room (floor at y = -10) with one colour painted on
    /// the floor around the origin (`None`: unpainted).
    fn painted_floor(color: Option<u8>) -> crate::PaintedTerrain<BoxRoomTerrain> {
        let room = BoxRoomTerrain::centered(Vec3::new(40.0, 20.0, 40.0));
        let mut t = crate::PaintedTerrain::new(room, 1.0);
        if let Some(color) = color {
            crate::SimTerrain::apply_edit(
                &mut t,
                &ls_math::terrain::TerrainEdit::Paint {
                    center: Vec3::new(0.0, -10.0, 0.0),
                    radius: 8.0,
                    team: 0,
                    color,
                },
            );
        }
        t
    }

    /// Ship `ticks` later (idle input unless `input` is given).
    fn fly_on<T: TerrainQuery>(
        t: &T,
        pos: Vec3,
        vel: Vec3,
        input: InputCmd,
        ticks: u32,
    ) -> ShipState {
        let mut s = ship();
        s.pos = pos;
        s.vel = vel;
        for _ in 0..ticks {
            simulate_movement(&mut s, &input, &ShipTuning::tinker(), t);
        }
        s
    }

    /// M2 plan: red paint is bouncy (ships rebound at 0.9 restitution),
    /// plain paint and bare rock are not.
    #[test]
    fn red_paint_bounces_ships_off() {
        use ls_math::terrain::{PAINT_PLAIN, PAINT_RED};
        let t = ShipTuning::tinker();
        let start = Vec3::new(0.0, -8.0, 0.0);
        let down = Vec3::new(0.0, -12.0, 0.0);
        // Up speed right after the hit (the ship reaches the floor within 15 ticks).
        let rebound = |color| {
            let floor = painted_floor(color);
            let mut s = ship();
            s.pos = start;
            s.vel = down;
            for _ in 0..30 {
                let r = simulate_movement(&mut s, &InputCmd::idle(0), &t, &floor);
                if r.contact {
                    return s.vel.y;
                }
            }
            panic!("never hit the floor");
        };
        let bare = rebound(None);
        let red = rebound(Some(PAINT_RED));
        assert!(bare < 3.0, "restitution 0.2: {bare}");
        assert_eq!(
            rebound(Some(PAINT_PLAIN)),
            bare,
            "plain paint has no effect"
        );
        let impact = -down.y - 8.0 * 0.25; // damping on the way down, about
        assert!(
            red > 0.85 * impact && red < impact,
            "0.9 restitution: {red}"
        );
        assert_eq!(t.paint_bounce_restitution, 0.9);
    }

    /// M2 plan: blue paint is slippery: no wall friction (a ship gliding
    /// over it keeps its speed without input) and a glancing hit keeps the
    /// speed, turned along the wall.
    #[test]
    fn blue_paint_is_slippery() {
        use ls_math::terrain::PAINT_BLUE;
        let t = ShipTuning::tinker();
        let on_floor = Vec3::new(-3.0, -10.0 + t.radius + 0.05, 0.0);
        let glide = Vec3::new(8.0, 0.0, 0.0);
        let bare = fly_on(&painted_floor(None), on_floor, glide, InputCmd::idle(0), 30);
        let blue = fly_on(
            &painted_floor(Some(PAINT_BLUE)),
            on_floor,
            glide,
            InputCmd::idle(0),
            30,
        );
        assert!(bare.speed() < 5.0, "damped: {}", bare.speed());
        assert!((blue.speed() - 8.0).abs() < 1e-3, "kept: {}", blue.speed());
        // Off the paint (high above it) the damping is back.
        let high = fly_on(
            &painted_floor(Some(PAINT_BLUE)),
            on_floor + Vec3::Y * 5.0,
            glide,
            InputCmd::idle(0),
            30,
        );
        assert!((high.speed() - bare.speed()).abs() < 1e-4);
        // A glancing hit: the plain floor eats the normal speed, blue paint
        // turns it along the floor.
        let dive = Vec3::new(8.0, -6.0, 0.0);
        let start = Vec3::new(-2.0, -9.0, 0.0);
        let hit = |color| {
            let floor = painted_floor(color);
            let mut s = ship();
            s.pos = start;
            s.vel = dive;
            for _ in 0..30 {
                let before = s.speed();
                if simulate_movement(&mut s, &InputCmd::idle(0), &t, &floor).contact {
                    return (before, s.speed(), s.vel.y);
                }
            }
            panic!("never hit the floor");
        };
        let (b0, b1, _) = hit(None);
        let (s0, s1, vy) = hit(Some(PAINT_BLUE));
        assert!(b1 < b0 * 0.9, "{b0} -> {b1}");
        assert!(s1 > s0 * 0.99, "{s0} -> {s1}");
        assert!(vy.abs() < 1e-3, "no bounce: {vy}");
    }

    /// M2 plan: yellow paint is sticky: ships touching it drop to 40% of
    /// their speed caps at once, and cannot thrust faster.
    #[test]
    fn yellow_paint_is_sticky() {
        use ls_math::terrain::PAINT_YELLOW;
        let t = ShipTuning::tinker();
        let on_floor = Vec3::new(-3.0, -10.0 + t.radius + 0.05, 0.0);
        let yellow = painted_floor(Some(PAINT_YELLOW));
        // Arriving at 12 u/s: one tick later at most 40% of the cruise cap.
        let s = fly_on(
            &yellow,
            on_floor,
            Vec3::new(12.0, 0.0, 0.0),
            InputCmd::idle(0),
            1,
        );
        let cap = t.cruise_speed * t.paint_sticky_speed;
        assert!(s.speed() <= cap + 1e-4, "{}", s.speed());
        // Full boost thrust along the floor: capped at 40% of the boost cap.
        let thrust = InputCmd::idle(0)
            .with_thrust(Vec3::NEG_Z)
            .with_buttons(Buttons::BOOST);
        let s = fly_on(&yellow, on_floor, Vec3::ZERO, thrust, 60);
        assert!(s.speed() <= t.boost_speed * t.paint_sticky_speed + 1e-4);
        assert!(s.speed() > t.boost_speed * t.paint_sticky_speed * 0.95);
        let free = fly_on(&painted_floor(None), on_floor, Vec3::ZERO, thrust, 60);
        assert!(free.speed() > t.boost_speed * 0.95, "{}", free.speed());
        // Diagonal: the direction is kept and no axis exceeds 40% of its cap
        // (the caps are per axis, so free flight goes sqrt(3) x faster too).
        let diag = Vec3::new(12.0, 0.0, 4.0);
        let s = fly_on(&yellow, on_floor, diag, InputCmd::idle(0), 1);
        assert!(
            s.vel.x.abs() <= cap + 1e-4 && s.vel.z.abs() <= cap + 1e-4,
            "{:?}",
            s.vel
        );
        assert!(
            s.vel.normalize().dot(diag.normalize()) > 0.99,
            "{:?}",
            s.vel
        );
        let thrust3 = InputCmd::idle(0)
            .with_thrust(Vec3::new(1.0, 0.0, -1.0))
            .with_buttons(Buttons::BOOST);
        let s = fly_on(&yellow, on_floor, Vec3::ZERO, thrust3, 60);
        let free = fly_on(&painted_floor(None), on_floor, Vec3::ZERO, thrust3, 60);
        let share = s.speed() / free.speed();
        assert!(
            share <= t.paint_sticky_speed + 0.01 && share > 0.35,
            "{share}"
        );
        // Pulling away from the paint frees the ship again.
        let up = InputCmd::idle(0).with_thrust(Vec3::Y);
        let s = fly_on(&yellow, on_floor, Vec3::ZERO, up, 60);
        assert!(s.pos.y > on_floor.y + 3.0, "{:?}", s.pos);
    }
}
