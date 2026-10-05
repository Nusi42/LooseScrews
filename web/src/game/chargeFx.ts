// Charge-up effect of hold-to-charge weapons (Fusion Charger, Polite Laser;
// visual-quality plan VQP): while a ship holds the trigger, energy gathers
// at its muzzle. A white-hot core inside a coloured orb grows with the
// charge, sparks stream in from a shell around it, a ring pulses faster and
// faster, crackling arcs jump once it is half full, a star flare marks full
// charge, and an overcharge (the Fusion Charger past full burns the hull)
// turns the orb red, unstable and spitting sparks. A point light (VQ6)
// grows with it, so a charging ship lights the cave before it fires.
//
// The own charge comes from the shot predictor every frame (Game.charge);
// other ships' charges from their ChargeStart cues (the cue carries the
// weapon, crates/ls-wasm GameCore::on_cue) and end when the ship's next
// projectile of that weapon appears, the ship dies or after a timeout.
// Cosmetic only; Math.random is fine.

import type { Quat } from "../math/quat.ts";
import * as q from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { PointLight } from "../renderer/lights.ts";
import { FRAME_EYES, hullFit } from "../renderer/hullFrameModels.ts";
import type { Rgb3 } from "../renderer/particles.ts";
import { SPRITE } from "../renderer/spriteAtlas.ts";
import { CUE, type ChargeMeter, type Cue } from "./mechanics.ts";
import { add, count, cross, len, lin, madd, mul, norm, randDir, range, scale3, sub, v, whiten, type Rng, type VfxSink } from "./vfxLibrary.ts";
import type { LocalPose, ProjectileState, ShipState, VisualDef } from "./views.ts";

/** One charge to draw this frame. */
export interface ChargeGlow {
  ship: number;
  /** Where the energy gathers: just ahead of the nose (the muzzle). */
  pos: Vec3;
  /** Aim direction (unit). */
  dir: Vec3;
  /** Ship velocity (inflowing sparks move with the ship). */
  vel: Vec3;
  /** 0..1 of a full charge. */
  fraction: number;
  /** Held past full charge (the Fusion Charger burns its hull). */
  overcharge: boolean;
  /** Weapon colour (display value). */
  color: Rgb3;
  /** Size factor: 1 for other ships; smaller for the own ship (seen from the cockpit). */
  size: number;
  /** The local player's own charge. */
  own: boolean;
}

/** Colour of a charge whose weapon visual is unknown (the Fusion Charger's). */
export const CHARGE_FALLBACK_COLOR: Rgb3 = [200 / 255, 90 / 255, 255 / 255];
/** Overcharge colour: red-white heat. */
const OVERCHARGE_COLOR: Rgb3 = [1, 0.32, 0.18];
/** Size of the own charge relative to other ships' (seen from the cockpit, right at the nose). */
export const OWN_CHARGE_SIZE = 0.22;

const eyeCache = new Map<number, number>();

/**
 * Height (u) of the cockpit eye above the ship's centre line for a ship
 * type at hitbox `radius` (renderer/hullFrameModels.ts): the own charge
 * gathers at the nose, below the crosshair, not in the middle of the view.
 */
export function eyeHeight(shipType: number, radius: number): number {
  let h = eyeCache.get(shipType);
  if (h === undefined) {
    const t = shipType >= 0 && shipType < FRAME_EYES.length ? shipType : 1;
    h = (FRAME_EYES[t]?.[1] ?? 0.2) * hullFit(t);
    eyeCache.set(shipType, h);
  }
  return (h * radius) / 0.5;
}
/** Muzzle distance ahead of the ship's centre beyond its radius (u; ls-sim Shot::muzzle with a 0.3 u orb). */
export const MUZZLE_AHEAD = 0.35;
/** A remote charge that never fired ends this long after it would be full (s). */
export const CHARGE_TIMEOUT = 4;

interface RemoteCharge {
  start: number;
  fullSecs: number;
  weapon: number;
  overcharge: boolean;
  /** The ship's projectiles of the weapon when the charge began (a new one = released). */
  shots: number;
}

/** Charges of the ships in view (see the module docs). */
export class ChargeTracker {
  private remote = new Map<number, RemoteCharge>();

  /** Starts other ships' charges from this frame's cues (and their overcharge). */
  apply(cues: readonly Cue[], now: number, player: number, shots: readonly ProjectileState[]): void {
    for (const c of cues) {
      if (c.source === player) continue;
      if (c.kind === CUE.chargeStart) {
        const weapon = c.target;
        this.remote.set(c.source, {
          start: now,
          fullSecs: Math.max(0.1, c.value / 60),
          weapon,
          overcharge: false,
          shots: shotsOf(shots, c.source, weapon),
        });
      } else if (c.kind === CUE.overcharge) {
        const r = this.remote.get(c.source);
        if (r) r.overcharge = true;
      }
    }
  }

  /** Remote charges in progress (ship id, weapon, fraction). */
  get active(): number {
    return this.remote.size;
  }

  forget(): void {
    this.remote.clear();
  }

  /**
   * This frame's charges: the own one from `own` (Game.charge) at the own
   * pose, other ships' from their cues. `visualOf(weapon)` gives the
   * weapon's projectile visual (its colour tints the charge).
   */
  glows(input: {
    now: number;
    player: number;
    pose: LocalPose | null;
    ownRadius: number;
    /** Own ship type (the cockpit eye height places the own charge at the nose). */
    ownShipType: number;
    own: ChargeMeter | null;
    ships: readonly ShipState[];
    shots: readonly ProjectileState[];
    visualOf: (weapon: number) => VisualDef | undefined;
  }): ChargeGlow[] {
    const out: ChargeGlow[] = [];
    const color = (w: number): Rgb3 => {
      const c = input.visualOf(w)?.color;
      return c ? [c[0], c[1], c[2]] : CHARGE_FALLBACK_COLOR;
    };
    const p = input.pose;
    if (input.own && p?.alive) {
      const below = madd(p.position, q.up(p.orientation), -eyeHeight(input.ownShipType, input.ownRadius));
      out.push(glowAt(input.player, below, p.orientation, input.ownRadius, p.velocity, input.own.fraction, input.own.overcharge, color(input.own.weapon), OWN_CHARGE_SIZE, true));
    }
    for (const [id, r] of this.remote) {
      const s = input.ships.find((x) => x.id === id);
      const age = input.now - r.start;
      const released = shotsOf(input.shots, id, r.weapon) > r.shots;
      if (!s || !s.alive || released || age > r.fullSecs + CHARGE_TIMEOUT || age < 0) {
        this.remote.delete(id);
        continue;
      }
      const fraction = Math.min(1, age / r.fullSecs);
      out.push(glowAt(id, s.position, s.orientation, s.radius, v(0, 0, 0), fraction, r.overcharge, color(r.weapon), 1, false));
    }
    return out;
  }
}

function shotsOf(shots: readonly ProjectileState[], owner: number, weapon: number): number {
  let n = 0;
  for (const s of shots) if (s.owner === owner && s.kind === weapon) n++;
  return n;
}

/** A charge at the muzzle of a ship at `pos` with `rot` and hitbox `radius`. */
export function glowAt(
  ship: number,
  pos: Vec3,
  rot: Quat,
  radius: number,
  vel: Vec3,
  fraction: number,
  overcharge: boolean,
  color: Rgb3,
  size: number,
  own: boolean,
): ChargeGlow {
  const dir = q.forward(rot);
  return { ship, pos: madd(pos, dir, radius + MUZZLE_AHEAD), dir, vel, fraction: Math.max(0, Math.min(1, fraction)), overcharge, color, size, own };
}

/** Radius of the visible orb (u) at `fraction`, before the size factor. */
export function orbRadius(fraction: number): number {
  return 0.08 + 0.3 * Math.pow(Math.max(0, Math.min(1, fraction)), 0.7);
}

/**
 * Draws one charge at particle time `now` (s): immediate sprites (core,
 * orb, halo, ring, flare) plus spawned inflowing sparks and arcs for `dt`
 * seconds; `carry` holds the fractional spark count between frames (pass
 * the returned value back next frame).
 */
export function chargeNow(sink: VfxSink, rng: Rng, g: ChargeGlow, now: number, dt: number, carry = 0): number {
  const k = g.fraction;
  const s = g.size;
  const r = orbRadius(k) * s;
  const tint = g.overcharge ? OVERCHARGE_COLOR : g.color;
  const c = lin(tint, 1);
  // Unstable when overcharged, a slight shimmer otherwise.
  const flicker = g.overcharge ? 0.65 + 0.7 * rng() : 1 + 0.08 * Math.sin(now * 37 + g.ship) * k;
  // Pulse rate rises from 2 Hz to 9 Hz as the charge fills.
  const pulse = 0.5 + 0.5 * Math.sin(now * Math.PI * 2 * (2 + 7 * k) + g.ship);

  // Halo, orb and the white-hot core (additive; bloom spreads them). The
  // orb keeps the weapon's colour; only the small core goes white.
  sink.immediate({ pos: g.pos, vel: g.vel, life: 1, sprite: SPRITE.glow, color: scale3(c, (0.12 + 0.4 * k) * flicker), size: r * 3.4 });
  sink.immediate({ pos: g.pos, vel: g.vel, life: 1, sprite: SPRITE.glow, color: scale3(whiten(c, 0.12), (0.7 + 2.0 * k) * flicker), size: r * 1.9 });
  sink.immediate({ pos: g.pos, vel: g.vel, life: 1, sprite: SPRITE.glow, color: scale3(whiten(c, 0.75), (1.6 + 5 * k) * flicker), size: r * (0.6 + 0.15 * pulse) });
  // A ring around the aim axis, pulsing.
  sink.immediate({
    pos: g.pos,
    vel: g.dir,
    life: 1,
    sprite: SPRITE.ring,
    orient: "plane",
    color: scale3(whiten(c, 0.1), (0.4 + 1.4 * k) * (0.35 + 0.65 * pulse)),
    size: r * (2.1 + 0.7 * pulse),
  });
  if (k >= 0.999) {
    // Full: a slowly turning star flare and a second, steady ring.
    sink.immediate({ pos: g.pos, vel: g.vel, life: 1, sprite: SPRITE.flare, color: scale3(whiten(c, 0.4), 1.4 * flicker), size: r * 4, rot: now * 1.3 + g.ship });
    sink.immediate({ pos: g.pos, vel: g.dir, life: 1, sprite: SPRITE.ring, orient: "plane", color: scale3(c, 1.1), size: r * 3.3 });
  }

  // Sparks streaming in from a shell around the orb (they arrive at the core).
  let n = carry + dt * (25 + 110 * k) * sink.density * (g.own ? 0.6 : 1);
  const shell = r * 3 + 0.35 * s;
  while (n >= 1) {
    n -= 1;
    const d = randDir(rng);
    const from = madd(g.pos, d, shell * range(rng, 0.8, 1.25));
    const life = range(rng, 0.16, 0.3);
    sink.spawn({
      pos: from,
      vel: add(mul(sub(g.pos, from), 1 / life), g.vel),
      life,
      sprite: SPRITE.spark,
      orient: "velocity",
      stretch: 0.045,
      color: scale3(whiten(c, 0.3), 1.5),
      color1: scale3(whiten(c, 0.7), 6),
      size: 0.03 * s,
      size1: 0.018 * s,
      fadeIn: 0.4,
    });
  }
  // Crackling arcs once half full: two-segment streaks from the core.
  if (k > 0.5 && rng() < dt * (6 + 18 * (k - 0.5)) * sink.density) {
    const d = norm(cross(g.dir, randDir(rng)), randDir(rng));
    let prev = g.pos;
    for (let i = 1; i <= 3; i++) {
      const p = madd(madd(g.pos, d, r * (0.9 + 0.7 * i)), randDir(rng), r * 0.35);
      const seg = sub(p, prev);
      const l = len(seg);
      if (l > 1e-4) sink.spawn({ pos: madd(prev, seg, 0.5), vel: seg, life: 0.06, sprite: SPRITE.streak, orient: "fixed", stretch: l * 1.2, color: scale3(whiten(c, 0.7), 10), color1: scale3(c, 2), size: 0.05 * s });
      prev = p;
    }
  }
  // Overcharge: sparks and embers spitting out of the unstable orb.
  if (g.overcharge) {
    for (let i = 0; i < count(sink, rng, dt * 70); i++) {
      const d = randDir(rng);
      sink.spawn({
        pos: madd(g.pos, d, r),
        vel: madd(g.vel, d, range(rng, 2, 6) * s),
        life: range(rng, 0.2, 0.45),
        drag: 2.5,
        sprite: SPRITE.spark,
        orient: "velocity",
        stretch: 0.05,
        color: [7, 3, 1.2],
        color1: [0.8, 0.12, 0.02],
        size: 0.05 * s,
      });
    }
  }
  return n;
}

/** The dynamic light of a charge (VQ6): grows with it, flickers when overcharged. */
export function chargeLight(g: ChargeGlow, rng: Rng = Math.random): PointLight {
  const tint = g.overcharge ? OVERCHARGE_COLOR : g.color;
  const k = g.fraction;
  const flicker = g.overcharge ? 0.6 + 0.8 * rng() : 1;
  return {
    position: g.pos,
    color: [(tint[0] + 0.35) / 1.35, (tint[1] + 0.35) / 1.35, (tint[2] + 0.35) / 1.35],
    intensity: (1.2 + 5 * k) * flicker * (g.own ? 0.5 : 1),
    range: 3 + 5 * k,
  };
}
