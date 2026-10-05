// The effect library (visual-quality-plan.md VQ5): what each effect spawns
// into the particle system. Pure functions over a `VfxSink` (the renderer's
// ParticleSystem, or a recorder in tests) and an RNG, so every effect is
// testable in Node and repeatable in the gallery (seeded RNG).
//
// Colours here are linear HDR radiance (emissive units: 1 is "white paper
// under the sun", the bloom threshold; flashes peak around 20-30). Display
// colours from the data (visual colours, team colours) go through `lin()`.
// Particles are cosmetic and never feed back into the simulation.

import type { Vec3 } from "../math/vec3.ts";
import type { ParticleSpec, Rgb3 } from "../renderer/particles.ts";
import { srgbToLinear } from "../renderer/shaders/frame.ts";
import { SPRITE, type SpriteId } from "../renderer/spriteAtlas.ts";

/** Where effects go: the renderer's ParticleSystem satisfies it. */
export interface VfxSink {
  /** Particle clock (s). */
  now: number;
  /** Spawn-rate factor of the quality preset (1 at "high"). */
  density: number;
  spawn(p: ParticleSpec): void;
  immediate(p: ParticleSpec): void;
  /** Drops the previous frame's immediate sprites (called at the start of every update). */
  beginFrame?(): void;
}

export type Rng = () => number;

// ----- small vector and colour helpers -------------------------------------------

export const v = (x: number, y: number, z: number): Vec3 => ({ x, y, z });
export const add = (a: Vec3, b: Vec3): Vec3 => ({ x: a.x + b.x, y: a.y + b.y, z: a.z + b.z });
export const sub = (a: Vec3, b: Vec3): Vec3 => ({ x: a.x - b.x, y: a.y - b.y, z: a.z - b.z });
export const mul = (a: Vec3, s: number): Vec3 => ({ x: a.x * s, y: a.y * s, z: a.z * s });
export const madd = (a: Vec3, b: Vec3, s: number): Vec3 => ({ x: a.x + b.x * s, y: a.y + b.y * s, z: a.z + b.z * s });
export const len = (a: Vec3): number => Math.hypot(a.x, a.y, a.z);
export const dist = (a: Vec3, b: Vec3): number => Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z);
export function norm(a: Vec3, fallback: Vec3 = { x: 0, y: 0, z: -1 }): Vec3 {
  const l = len(a);
  return l > 1e-9 ? mul(a, 1 / l) : fallback;
}

/** Uniform random direction. */
export function randDir(rng: Rng): Vec3 {
  const z = rng() * 2 - 1;
  const a = rng() * Math.PI * 2;
  const r = Math.sqrt(Math.max(0, 1 - z * z));
  return { x: r * Math.cos(a), y: r * Math.sin(a), z };
}

/** Random direction within `spread` (0 = exactly `dir`, 1 = hemisphere) of `dir`. */
export function randCone(rng: Rng, dir: Vec3, spread: number): Vec3 {
  return norm(madd(dir, randDir(rng), spread), dir);
}

export const range = (rng: Rng, lo: number, hi: number): number => lo + (hi - lo) * rng();

/** Display colour to linear, times an HDR intensity. */
export function lin(c: Rgb3, intensity = 1): Rgb3 {
  return [srgbToLinear(c[0]) * intensity, srgbToLinear(c[1]) * intensity, srgbToLinear(c[2]) * intensity];
}

export const scale3 = (c: Rgb3, s: number): Rgb3 => [c[0] * s, c[1] * s, c[2] * s];
const mix3 = (a: Rgb3, b: Rgb3, t: number): Rgb3 => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
const BLACK: Rgb3 = [0, 0, 0];

/** A colour pushed toward white (hot cores of coloured light). */
export function whiten(c: Rgb3, t: number): Rgb3 {
  const m = Math.max(c[0], c[1], c[2], 1e-6);
  return mix3(c, [m, m, m], t);
}

/** Integer particle count for `n` at the sink's density (a fraction spawns with that probability). */
export function count(sink: VfxSink, rng: Rng, n: number): number {
  const x = n * sink.density;
  const whole = Math.floor(x);
  return whole + (rng() < x - whole ? 1 : 0);
}

const SMOKE_SPRITES: readonly SpriteId[] = [SPRITE.smoke0, SPRITE.smoke1, SPRITE.smoke2, SPRITE.smoke3];
export const smokeSprite = (rng: Rng): SpriteId => SMOKE_SPRITES[Math.floor(rng() * 4) % 4] ?? SPRITE.smoke0;

// ----- palettes ---------------------------------------------------------------------

/** Fire: white-yellow core cooling through orange to deep red. */
export const FIRE_HOT: Rgb3 = [2.6, 1.2, 0.33];
export const FIRE_COOL: Rgb3 = [0.3, 0.055, 0.01];
/** Sparks and embers. */
export const EMBER_HOT: Rgb3 = [7, 3.4, 1.0];
export const EMBER_COOL: Rgb3 = [0.8, 0.12, 0.02];
/** Smoke albedo-ish radiance (dark, slightly warm while the fire lights it). */
export const SMOKE_LIT: Rgb3 = [0.08, 0.05, 0.035];
export const SMOKE_DARK: Rgb3 = [0.014, 0.013, 0.012];
/** Rock dust kicked up by impacts. */
export const DUST: Rgb3 = [0.11, 0.09, 0.07];
export const DUST_DARK: Rgb3 = [0.04, 0.035, 0.03];

// ----- explosions -------------------------------------------------------------------

export interface ExplosionOptions {
  /** Velocity the whole blast drifts with (a dying ship's). */
  vel?: Vec3;
  /** Fire (rockets, ships) or plasma (energy orbs, coloured). */
  kind?: "fire" | "plasma";
  /** Plasma colour (display). */
  color?: Rgb3;
  /** Throw rock dust and shards (the blast touched terrain). */
  terrain?: boolean;
  /** Throw hull shards and burning debris (a ship blew up). */
  hull?: boolean;
  /** Start time offset (s, may be negative for effects that began earlier). */
  t0?: number;
}

/**
 * An explosion of visual radius `r` (u): flash, fireball (turbulent flame
 * puffs cooling from white-hot to red), smoke that billows out and lingers,
 * sparks and embers, a shockwave ring, and debris shards.
 */
export function explosion(sink: VfxSink, rng: Rng, at: Vec3, r: number, o: ExplosionOptions = {}): void {
  const t0 = o.t0 ?? sink.now;
  const drift = o.vel ?? v(0, 0, 0);
  const plasma = o.kind === "plasma";
  const tint = o.color ? lin(o.color, 1) : null;
  const hot: Rgb3 = plasma && tint ? scale3(whiten(tint, 0.35), 4) : FIRE_HOT;
  const cool: Rgb3 = plasma && tint ? scale3(tint, 0.3) : FIRE_COOL;
  const big = Math.max(0.4, r);
  // Flash: a short white burst (bloom spreads it) and a star flare.
  sink.spawn({ pos: at, vel: drift, t0, life: 0.1, sprite: SPRITE.glow, color: scale3(whiten(hot, 0.7), 2), color1: BLACK, size: big * 1.5, size1: big * 2.2 });
  sink.spawn({ pos: at, vel: drift, t0, life: 0.06, sprite: SPRITE.flare, color: scale3(whiten(hot, 0.8), 1.5), color1: BLACK, size: big * 2.2, size1: big * 2.8, rot: rng() * 6.28 });
  // Fireball.
  const nFire = count(sink, rng, 10 + 4 * big);
  for (let i = 0; i < nFire; i++) {
    const d = randDir(rng);
    const life = range(rng, 0.45, 0.9) * Math.min(1.4, 0.75 + 0.12 * big);
    sink.spawn({
      pos: madd(at, d, big * range(rng, 0, 0.3)),
      vel: madd(drift, d, big * range(rng, 1.2, 3.6)),
      t0: t0 + range(rng, 0, 0.05),
      life,
      drag: 4,
      sprite: SPRITE.flame,
      hot: true,
      color: scale3(hot, range(rng, 0.45, 1.1)),
      color1: cool,
      size: big * range(rng, 0.3, 0.65),
      size1: big * range(rng, 0.7, 1.1),
      spin: range(rng, -2, 2),
      rot: rng() * 6.28,
    });
  }
  // Smoke billows out from inside the fireball and lingers.
  const nSmoke = count(sink, rng, 8 + 4 * big);
  for (let i = 0; i < nSmoke; i++) {
    const d = randDir(rng);
    // Smoke rises out of the fire after it: born inside, late, fading in.
    sink.spawn({
      pos: madd(at, d, big * range(rng, 0, 0.25)),
      vel: madd(mul(drift, 0.6), d, big * range(rng, 0.2, 1.2)),
      t0: t0 + range(rng, 0.08, 0.3),
      life: range(rng, 1.6, 3.2) * (0.8 + 0.1 * big),
      drag: 1.3,
      sprite: smokeSprite(rng),
      blend: "alpha",
      color: SMOKE_LIT,
      color1: SMOKE_DARK,
      alpha: range(rng, 0.55, 0.8),
      alpha1: 0,
      fadeIn: 0.2,
      size: big * range(rng, 0.45, 0.8),
      size1: big * range(rng, 1.2, 1.8),
      spin: range(rng, -0.6, 0.6),
      rot: rng() * 6.28,
    });
  }
  // Sparks and embers, some fast and short, some slow and glowing on.
  const nSpark = count(sink, rng, 22 + 9 * big);
  for (let i = 0; i < nSpark; i++) {
    const d = randDir(rng);
    const fast = rng() < 0.6;
    sink.spawn({
      pos: madd(at, d, big * 0.2),
      vel: madd(drift, d, (fast ? range(rng, 14, 30) : range(rng, 4, 11)) * (0.6 + 0.2 * big)),
      t0,
      life: fast ? range(rng, 0.25, 0.6) : range(rng, 0.8, 1.6),
      drag: fast ? 2.2 : 1.2,
      sprite: SPRITE.spark,
      orient: "velocity",
      stretch: 0.035,
      color: plasma && tint ? scale3(whiten(tint, 0.5), 7) : EMBER_HOT,
      color1: plasma && tint ? scale3(tint, 0.5) : EMBER_COOL,
      size: range(rng, 0.05, 0.1),
      size1: 0.03,
    });
  }
  // Shockwave ring (a spherical shell reads as a ring from any side).
  sink.spawn({ pos: at, vel: drift, t0, life: 0.32, sprite: SPRITE.ring, color: scale3(whiten(hot, 0.5), 0.08), color1: BLACK, size: big * 0.5, size1: big * 2.8 });
  if (o.terrain) {
    // Dust and shards scale with the blast, up to a point (a huge dust cloud reads as fog).
    rockBurst(sink, rng, at, null, Math.min(big, 2.5), t0);
  }
  if (o.hull) {
    const nShard = count(sink, rng, 10 + 2 * big);
    for (let i = 0; i < nShard; i++) {
      const d = randDir(rng);
      sink.spawn({
        pos: madd(at, d, 0.2),
        vel: madd(drift, d, range(rng, 4, 13)),
        t0,
        life: range(rng, 1.0, 2.0),
        drag: 0.6,
        sprite: SPRITE.shard,
        blend: "alpha",
        color: [0.12, 0.12, 0.13],
        color1: [0.05, 0.05, 0.05],
        alpha: 1,
        alpha1: 0,
        size: range(rng, 0.1, 0.24),
        spin: range(rng, -9, 9),
        rot: rng() * 6.28,
      });
      // A burning trail on some of them.
      if (rng() < 0.5) {
        sink.spawn({
          pos: madd(at, d, 0.2),
          vel: madd(drift, d, range(rng, 4, 13)),
          t0,
          life: range(rng, 0.6, 1.2),
          drag: 0.6,
          sprite: SPRITE.flame,
          hot: true,
          color: [5, 2.2, 0.6],
          color1: [0.4, 0.06, 0.01],
          size: 0.28,
          size1: 0.12,
        });
      }
    }
  }
}

/** Rock dust and shards thrown off the rock at `at` (along `normal` when known). */
export function rockBurst(sink: VfxSink, rng: Rng, at: Vec3, normal: Vec3 | null, r: number, t0 = sink.now): void {
  const nDust = count(sink, rng, 3 + 3 * r);
  for (let i = 0; i < nDust; i++) {
    const d = normal ? randCone(rng, normal, 0.9) : randDir(rng);
    sink.spawn({
      pos: madd(at, d, r * 0.3),
      vel: mul(d, r * range(rng, 0.4, 1.4)),
      t0: t0 + range(rng, 0, 0.1),
      life: range(rng, 1.2, 2.4),
      drag: 1.6,
      sprite: smokeSprite(rng),
      blend: "alpha",
      color: DUST,
      color1: DUST_DARK,
      alpha: range(rng, 0.25, 0.45),
      alpha1: 0,
      fadeIn: 0.1,
      size: r * range(rng, 0.4, 0.8),
      size1: r * range(rng, 1.2, 2.0),
      spin: range(rng, -0.5, 0.5),
      rot: rng() * 6.28,
    });
  }
  const nShard = count(sink, rng, 4 + 3 * r);
  for (let i = 0; i < nShard; i++) {
    const d = normal ? randCone(rng, normal, 1.0) : randDir(rng);
    sink.spawn({
      pos: madd(at, d, 0.1),
      vel: mul(d, range(rng, 3, 10) * Math.min(2, 0.6 + r * 0.3)),
      t0,
      life: range(rng, 0.7, 1.5),
      drag: 0.9,
      sprite: SPRITE.shard,
      blend: "alpha",
      color: [0.11, 0.095, 0.08],
      color1: [0.05, 0.045, 0.04],
      alpha: 1,
      alpha1: 0,
      size: range(rng, 0.06, 0.16) * Math.min(2, 0.7 + r * 0.25),
      spin: range(rng, -10, 10),
      rot: rng() * 6.28,
    });
  }
}

// ----- impacts ------------------------------------------------------------------------

export type ImpactKind = "sparks" | "plasma" | "splat" | "thud" | "explosion" | "none";

export interface ImpactOptions {
  kind: ImpactKind;
  /** Display colour of the projectile. */
  color: Rgb3;
  /** Size factor (visual scale). */
  size: number;
  /** Direction the projectile was flying. */
  dir: Vec3;
  /** It hit rock (dust, shards) rather than a ship or nothing. */
  terrain: boolean;
  /** Explosion radius for `explosion`. */
  radius?: number;
}

/** A projectile hit something (or burnt out): sparks, plasma splash, paint splat, thud or explosion. */
export function impact(sink: VfxSink, rng: Rng, at: Vec3, o: ImpactOptions): void {
  const s = Math.max(0.3, o.size);
  const back = mul(o.dir, -1);
  switch (o.kind) {
    case "none":
      return;
    case "explosion":
      explosion(sink, rng, at, o.radius ?? 2.4 * s, { terrain: o.terrain });
      return;
    case "plasma": {
      const c = lin(o.color, 1);
      sink.spawn({ pos: at, life: 0.16, sprite: SPRITE.glow, color: scale3(whiten(c, 0.5), 6), color1: BLACK, size: 1.1 * s, size1: 2 * s });
      sink.spawn({ pos: at, life: 0.22, sprite: SPRITE.ring, color: scale3(c, 3.5), color1: BLACK, size: 0.4 * s, size1: 2.6 * s });
      const n = count(sink, rng, 12 * s);
      for (let i = 0; i < n; i++) {
        const d = o.terrain ? randCone(rng, back, 1.1) : randDir(rng);
        sink.spawn({ pos: at, vel: mul(d, range(rng, 4, 12)), life: range(rng, 0.2, 0.5), drag: 3, sprite: SPRITE.spark, orient: "velocity", stretch: 0.03, color: scale3(whiten(c, 0.6), 10), color1: scale3(c, 0.4), size: 0.07, size1: 0.03 });
      }
      if (o.terrain) rockBurst(sink, rng, at, back, 0.5 * s);
      return;
    }
    case "splat": {
      const c = lin(o.color, 1);
      sink.spawn({ pos: at, life: 0.12, sprite: SPRITE.glow, color: scale3(c, 5), color1: BLACK, size: 1.0 * s, size1: 1.6 * s });
      const n = count(sink, rng, 10 * s);
      for (let i = 0; i < n; i++) {
        const d = randCone(rng, back, 1.2);
        sink.spawn({
          pos: at,
          vel: mul(d, range(rng, 2, 7)),
          life: range(rng, 0.35, 0.7),
          drag: 2.5,
          sprite: SPRITE.smoke0,
          blend: "alpha",
          color: scale3(c, 0.9),
          color1: scale3(c, 0.5),
          alpha: 0.95,
          alpha1: 0,
          size: range(rng, 0.12, 0.25) * s,
          size1: range(rng, 0.25, 0.45) * s,
        });
      }
      return;
    }
    case "thud":
      rockBurst(sink, rng, at, back, 0.5 * s);
      sparks(sink, rng, at, back, [1, 0.85, 0.6], 6 * s, 1);
      return;
    default: {
      // Sparks: a hot point flash, a burst of coloured and white sparks, a puff of dust on rock.
      const c = lin(o.color, 1);
      sink.spawn({ pos: at, life: 0.09, sprite: SPRITE.flare, color: scale3(whiten(c, 0.6), 9), color1: BLACK, size: 0.9 * s, size1: 1.2 * s, rot: rng() * 6.28 });
      sink.spawn({ pos: at, life: 0.18, sprite: SPRITE.glow, color: scale3(c, 4), color1: BLACK, size: 0.8 * s, size1: 1.3 * s });
      sparks(sink, rng, at, o.terrain ? back : randDir(rng), o.color, 10 * s, o.terrain ? 1.0 : 2);
      if (o.terrain) rockBurst(sink, rng, at, back, 0.35 * s);
    }
  }
}

/** A burst of `n` sparks of display colour `color` around `dir` (`spread` 0..2). */
export function sparks(sink: VfxSink, rng: Rng, at: Vec3, dir: Vec3, color: Rgb3, n: number, spread: number, speed = 1, t0 = sink.now): void {
  const c = lin(color, 1);
  const k = count(sink, rng, n);
  for (let i = 0; i < k; i++) {
    const d = randCone(rng, dir, spread);
    const white = rng() < 0.4;
    sink.spawn({
      pos: at,
      vel: mul(d, range(rng, 5, 16) * speed),
      t0,
      life: range(rng, 0.15, 0.45),
      drag: 2.5,
      sprite: SPRITE.spark,
      orient: "velocity",
      stretch: 0.03,
      color: white ? [6, 4.8, 3.6] : scale3(whiten(c, 0.4), 6),
      color1: white ? EMBER_COOL : scale3(c, 0.4),
      size: range(rng, 0.04, 0.08),
      size1: 0.025,
    });
  }
}

// ----- muzzle flashes -------------------------------------------------------------

export type MuzzleKind = "energy" | "kinetic" | "rocket" | "lob" | "none";

/**
 * A muzzle flash at `at` for a shot flying along `dir`. `scale` < 1 for the
 * own ship (the flash is right in front of the camera).
 */
export function muzzleFlash(sink: VfxSink, rng: Rng, kind: MuzzleKind, at: Vec3, dir: Vec3, color: Rgb3, scale = 1, shipVel: Vec3 = v(0, 0, 0)): void {
  if (kind === "none") return;
  const c = lin(color, 1);
  const s = scale;
  switch (kind) {
    case "energy":
      sink.spawn({ pos: at, vel: shipVel, life: 0.06, sprite: SPRITE.flare, color: scale3(whiten(c, 0.5), 3), color1: BLACK, size: 0.6 * s, size1: 0.75 * s, rot: rng() * 6.28 });
      sink.spawn({ pos: madd(at, dir, 0.15 * s), vel: shipVel, life: 0.08, sprite: SPRITE.glow, color: scale3(c, 2), color1: BLACK, size: 0.5 * s, size1: 0.3 * s });
      sparks(sink, rng, at, dir, color, 3 * s, 0.35, 0.8);
      return;
    case "kinetic":
      sink.spawn({ pos: at, vel: shipVel, life: 0.06, sprite: SPRITE.flare, color: [5, 3.8, 2.1], color1: BLACK, size: 0.6 * s, size1: 0.75 * s, rot: rng() * 6.28 });
      sink.spawn({ pos: madd(at, dir, 0.25 * s), vel: madd(shipVel, dir, 4), life: 0.08, sprite: SPRITE.flame, hot: true, orient: "fixed", stretch: 0.7 * s, color: [3.4, 1.9, 0.7], color1: BLACK, size: 0.35 * s });
      for (let i = 0; i < count(sink, rng, 2 * s); i++) {
        sink.spawn({ pos: madd(at, dir, 0.3 * s), vel: madd(shipVel, randCone(rng, dir, 0.6), range(rng, 1, 3)), life: range(rng, 0.4, 0.8), drag: 2, sprite: smokeSprite(rng), blend: "alpha", color: [0.12, 0.11, 0.1], color1: SMOKE_DARK, alpha: 0.35, alpha1: 0, size: 0.15 * s, size1: 0.6 * s, rot: rng() * 6.28 });
      }
      return;
    case "rocket":
      sink.spawn({ pos: at, vel: shipVel, life: 0.1, sprite: SPRITE.glow, color: [5, 2.5, 0.8], color1: BLACK, size: 1.0 * s, size1: 1.4 * s });
      for (let i = 0; i < count(sink, rng, 6 * s); i++) {
        const d = randCone(rng, mul(dir, -1), 0.7);
        sink.spawn({ pos: at, vel: madd(shipVel, d, range(rng, 2, 6)), life: range(rng, 0.15, 0.3), drag: 4, sprite: SPRITE.flame, hot: true, color: FIRE_HOT, color1: FIRE_COOL, size: 0.3 * s, size1: 0.6 * s, rot: rng() * 6.28 });
      }
      for (let i = 0; i < count(sink, rng, 4 * s); i++) {
        const d = randCone(rng, mul(dir, -1), 0.9);
        sink.spawn({ pos: at, vel: madd(shipVel, d, range(rng, 1, 3)), t0: sink.now + 0.04, life: range(rng, 0.8, 1.5), drag: 1.5, sprite: smokeSprite(rng), blend: "alpha", color: [0.14, 0.12, 0.1], color1: SMOKE_DARK, alpha: 0.5, alpha1: 0, fadeIn: 0.05, size: 0.3 * s, size1: 1.2 * s, rot: rng() * 6.28 });
      }
      return;
    case "lob":
      sink.spawn({ pos: at, vel: shipVel, life: 0.1, sprite: SPRITE.glow, color: scale3(c, 4), color1: BLACK, size: 0.6 * s, size1: 0.9 * s });
      return;
  }
}

// ----- shield, flashes, beams ----------------------------------------------------

/** A ripple on a shield bubble of radius `r` around `center`, where it was hit toward `at`. */
export function shieldRipple(sink: VfxSink, rng: Rng, center: Vec3, r: number, at: Vec3, shipVel: Vec3 = v(0, 0, 0)): void {
  const n = norm(sub(at, center), randDir(rng));
  const p = madd(center, n, r);
  const c: Rgb3 = [0.7, 1.9, 3.2];
  // The ring lies in the bubble's tangent plane and runs outward from the hit.
  sink.spawn({ pos: p, vel: madd(mul(n, 1e-3), shipVel, 0), life: 0.5, sprite: SPRITE.ring, orient: "plane", color: c, color1: BLACK, size: 0.2 * r, size1: 1.7 * r });
  sink.spawn({ pos: p, vel: madd(mul(n, 1e-3), shipVel, 0), life: 0.35, sprite: SPRITE.ring, orient: "plane", color: scale3(c, 0.6), color1: BLACK, size: 0.1 * r, size1: 1.0 * r, t0: sink.now + 0.08 });
  sink.spawn({ pos: p, vel: shipVel, life: 0.15, sprite: SPRITE.glow, color: [1.5, 3.5, 6], color1: BLACK, size: 0.6 * r, size1: 0.9 * r });
  for (let i = 0; i < count(sink, rng, 8); i++) {
    const t = norm(sub(randDir(rng), mul(n, 0)));
    sink.spawn({ pos: madd(p, t, 0.1 * r), vel: madd(shipVel, randCone(rng, n, 1.2), range(rng, 1, 4)), life: range(rng, 0.2, 0.45), drag: 3, sprite: SPRITE.spark, orient: "velocity", stretch: 0.04, color: [3, 7, 10], color1: [0.2, 0.6, 1], size: 0.05 });
  }
}

/** A soft glow (immediate, this frame only): replaces the translucent flash spheres. */
export function glowNow(sink: VfxSink, at: Vec3, color: Rgb3, size: number, intensity: number, sprite: SpriteId = SPRITE.glow): void {
  if (intensity <= 0 || size <= 0) return;
  sink.immediate({ pos: at, life: 1, sprite, color: lin(color, intensity), size });
}

/**
 * A crackling electric beam from `from` to `to` (immediate, rebuilt every
 * frame with new jitter): two jagged strands of streak sections, a hot glow
 * at both ends.
 */
export function beamNow(sink: VfxSink, rng: Rng, from: Vec3, to: Vec3, color: Rgb3, strength: number): void {
  const d = sub(to, from);
  const length = len(d);
  if (length < 1e-3) return;
  const dir = mul(d, 1 / length);
  const helper = Math.abs(dir.y) > 0.9 ? v(1, 0, 0) : v(0, 1, 0);
  const s1 = norm(cross(dir, helper));
  const s2 = cross(dir, s1);
  const c = lin(color, 1);
  const segs = Math.max(4, Math.min(28, Math.round(length / 0.7)));
  for (let strand = 0; strand < 2; strand++) {
    const amp = (strand === 0 ? 0.22 : 0.4) * Math.min(1, length / 6);
    let prev = from;
    for (let i = 1; i <= segs; i++) {
      const t = i / segs;
      const env = Math.sin(Math.PI * t);
      const p = i === segs ? to : madd(madd(madd(from, d, t), s1, (rng() * 2 - 1) * amp * env), s2, (rng() * 2 - 1) * amp * env);
      const seg = sub(p, prev);
      const sl = len(seg);
      if (sl > 1e-4) {
        const mid = madd(prev, seg, 0.5);
        sink.immediate({ pos: mid, vel: seg, life: 1, sprite: SPRITE.streak, orient: "fixed", stretch: sl * 1.25, color: scale3(whiten(c, strand === 0 ? 0.6 : 0.2), (strand === 0 ? 9 : 4) * strength), size: strand === 0 ? 0.16 : 0.1 });
      }
      prev = p;
    }
  }
  sink.immediate({ pos: from, life: 1, sprite: SPRITE.glow, color: scale3(c, 5 * strength), size: 0.8 });
  sink.immediate({ pos: to, life: 1, sprite: SPRITE.glow, color: scale3(whiten(c, 0.5), 8 * strength), size: 1.2 });
}

export function cross(a: Vec3, b: Vec3): Vec3 {
  return { x: a.y * b.z - a.z * b.y, y: a.z * b.x - a.x * b.z, z: a.x * b.y - a.y * b.x };
}

// ----- smoke, sparkle, rings ----------------------------------------------------------

/** One puff of dark damage smoke from a hurt hull at `at`. */
export function damageSmoke(sink: VfxSink, rng: Rng, at: Vec3, shipVel: Vec3, heavy: boolean): void {
  sink.spawn({
    pos: madd(at, randDir(rng), 0.12),
    vel: madd(mul(shipVel, 0.35), randDir(rng), range(rng, 0.3, 0.9)),
    life: range(rng, 1.0, 1.8),
    drag: 1.4,
    sprite: smokeSprite(rng),
    blend: "alpha",
    color: heavy ? [0.03, 0.026, 0.024] : [0.08, 0.075, 0.07],
    color1: SMOKE_DARK,
    alpha: heavy ? 0.8 : 0.55,
    alpha1: 0,
    fadeIn: 0.08,
    size: 0.28,
    size1: range(rng, 1.1, 1.8),
    spin: range(rng, -0.8, 0.8),
    rot: rng() * 6.28,
  });
  if (heavy && rng() < 0.5) {
    sink.spawn({ pos: at, vel: madd(shipVel, randDir(rng), 0.6), life: range(rng, 0.15, 0.3), drag: 3, sprite: SPRITE.flame, hot: true, color: [6, 2.6, 0.7], color1: [0.5, 0.08, 0.01], size: 0.22, size1: 0.35, rot: rng() * 6.28 });
  }
}

/** A tiny star glint (pickup sparkle, cloak shimmer). */
export function glint(sink: VfxSink, rng: Rng, at: Vec3, color: Rgb3, size: number, intensity: number, vel: Vec3 = v(0, 0, 0)): void {
  sink.spawn({ pos: at, vel, life: range(rng, 0.25, 0.5), sprite: SPRITE.flare, color: lin(color, intensity), color1: BLACK, size, size1: size * 0.3, rot: rng() * 6.28, fadeIn: 0.3 });
}

/** An expanding ring of `color` from `size` to `size1` (respawn, pickups, gadgets). */
export function ring(sink: VfxSink, at: Vec3, color: Rgb3, intensity: number, size: number, size1: number, life: number, vel: Vec3 = v(0, 0, 0)): void {
  sink.spawn({ pos: at, vel, life, sprite: SPRITE.ring, color: lin(color, intensity), color1: BLACK, size, size1 });
}

/** A burst of sparkles flying outward (pickup taken, swap, rewind). */
export function sparkleBurst(sink: VfxSink, rng: Rng, at: Vec3, color: Rgb3, n: number, speed: number, inward = false): void {
  const c = lin(color, 1);
  for (let i = 0; i < count(sink, rng, n); i++) {
    const d = randDir(rng);
    const sp = range(rng, 0.5, 1) * speed;
    const life = range(rng, 0.35, 0.7);
    // Inward: start out on the shell and fly to the centre.
    const pos = inward ? madd(at, d, sp * life) : at;
    sink.spawn({ pos, vel: mul(d, inward ? -sp : sp), life, drag: inward ? 0 : 2, sprite: SPRITE.spark, orient: "velocity", stretch: 0.05, color: scale3(whiten(c, 0.5), 8), color1: scale3(c, 0.5), size: 0.07, size1: 0.03 });
  }
}
