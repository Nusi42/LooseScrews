// Everything is cake (M4 C1, milestone-4-plan.md §3.2): for 20 s every crater
// is 1.44x as wide and crater debris repairs twice its scrap. The terrain
// turns into cake in a wave from the tell's position (the living ships'
// centroid at the announcement, `Game.eventFx` cake origin):
// - the tell: a fountain of sprinkles and a growing pink glow at the origin;
// - at the start a party horn and a confetti pop, then the wave runs out at
//   CAKE_WAVE_SPEED (the terrain shader's cake branch, shaders/cake.ts:
//   sponge layers, cream and jam, pink frosting on up-facing rock dripping
//   down the walls, rainbow sprinkles, a bright frosting front);
// - crumbs and sprinkles fly where shots hit (cake crumbs instead of rock
//   dust), crater debris is sponge-coloured;
// - at the end the cake recedes into the origin.

import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { CakeWave, Rgb } from "../../renderer/renderer.ts";
import { SPRINKLES } from "../../renderer/shaders/cake.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { lin } from "../vfxLibrary.ts";
import { hash01, smooth, type EventClock } from "./fx.ts";
import { ShotTracker } from "./shots.ts";
import type { EventEnv, EventInput, EventOutput, EventVisual, ParticleSink } from "./types.ts";

/** Speed of the cake wave (u/s) and the radius it reaches (beyond every level box). */
export const CAKE_WAVE_SPEED = 32;
export const CAKE_MAX_RADIUS = 480;
/** Seconds the cake takes to recede after the end. */
export const CAKE_RECEDE_SECS = 2.5;
/** Debris colour while it is cake (sponge). */
export const CAKE_DEBRIS: Rgb = [0.93, 0.66, 0.32];

/** The wave `t` seconds into the event (or `over` seconds after it), null without cake. */
export function cakeWave(origin: Vec3, up: Vec3, t: number, over: number | null): CakeWave | null {
  if (over !== null) {
    if (over >= CAKE_RECEDE_SECS) return null;
    const k = over / CAKE_RECEDE_SECS;
    return { origin, up, radius: CAKE_MAX_RADIUS * (1 - k) * (1 - k), amount: 1 - smooth(0.7, 1, k) };
  }
  if (t <= 0) return null;
  // A quick start (it pops out of the origin), then the steady wave.
  const r = Math.min(CAKE_MAX_RADIUS, CAKE_WAVE_SPEED * t + 6 * (1 - Math.exp(-t * 4)));
  return { origin, up, radius: r, amount: 1 };
}

/** Crumbs where a shot hit cake: sponge chunks, frosting bits and sprinkles. */
export function crumbBurst(sink: ParticleSink, at: Vec3, seed: number, n = 12): void {
  for (let i = 0; i < n; i++) {
    const a = hash01(seed, i) * Math.PI * 2;
    const b = Math.acos(2 * hash01(seed, i + 30) - 1);
    const d = { x: Math.sin(b) * Math.cos(a), y: Math.cos(b), z: Math.sin(b) * Math.sin(a) };
    const kind = i % 4;
    const color: Rgb = kind === 3 ? (SPRINKLES[i % SPRINKLES.length] as Rgb) : kind === 2 ? [1, 0.72, 0.84] : [0.93, 0.66, 0.32];
    sink.spawn({
      pos: v3.addScaled(at, d, 0.2),
      vel: v3.scale(d, 3 + 6 * hash01(seed, i + 60)),
      life: 1.1 + 0.8 * hash01(seed, i + 90),
      sprite: kind === 3 ? SPRITE.spark : SPRITE.shard,
      blend: "alpha",
      color: lin(color, kind === 3 ? 0.9 : 0.55),
      alpha: 1,
      alpha1: 0.2,
      size: kind === 3 ? 0.12 : 0.1 + 0.12 * hash01(seed, i + 120),
      drag: 1.6,
      spin: (hash01(seed, i + 150) - 0.5) * 10,
    });
  }
  sink.spawn({ pos: at, life: 0.9, sprite: SPRITE.smoke2, blend: "alpha", color: lin([1, 0.85, 0.65], 0.4), alpha: 0.45, alpha1: 0, size: 0.4, size1: 2.2, drag: 1.2 });
}

/** Confetti pop at the origin (the party horn). */
function confetti(sink: ParticleSink, at: Vec3, seed: number): void {
  for (let i = 0; i < 70; i++) {
    const a = hash01(seed, i) * Math.PI * 2;
    const b = Math.acos(2 * hash01(seed, i + 300) - 1);
    const d = { x: Math.sin(b) * Math.cos(a), y: Math.cos(b), z: Math.sin(b) * Math.sin(a) };
    sink.spawn({
      pos: at,
      vel: v3.scale(d, 6 + 10 * hash01(seed, i + 600)),
      life: 2.6 + hash01(seed, i + 900),
      sprite: i % 2 ? SPRITE.feather : SPRITE.shard,
      blend: "alpha",
      color: lin(SPRINKLES[i % SPRINKLES.length] as Rgb, 0.7),
      alpha: 1,
      alpha1: 0,
      size: 0.18 + 0.12 * hash01(seed, i + 1200),
      drag: 1.3,
      spin: (hash01(seed, i + 1500) - 0.5) * 12,
    });
  }
  sink.spawn({ pos: at, life: 0.35, sprite: SPRITE.ring, color: lin([1, 0.8, 0.9], 3), size: 1, size1: 9 });
}

class EverythingIsCake implements EventVisual {
  private readonly env: EventEnv;
  private readonly tracker = new ShotTracker();
  private horn = false;
  private origin: Vec3 | null = null;
  private tellAt = 0;

  constructor(env: EventEnv) {
    this.env = env;
  }

  update(i: EventInput, clock: EventClock, out: EventOutput): boolean {
    const sink = this.env.particles;
    const sound = i.quiet ? null : this.env.sound;
    const origin = i.fx.cake?.origin ?? this.origin;
    if (origin) this.origin = origin;
    if (!origin) return clock.over === null;
    const wave = cakeWave(origin, i.up, clock.t, clock.over);
    out.cake = wave;
    // The tell: sprinkles fountain and a pink glow growing at the origin.
    if (clock.over === null && clock.t < 0) {
      const k = smooth(-3, 0, clock.t);
      out.lights.push({ position: origin, color: [1, 0.55, 0.75], intensity: 4 + 26 * k, range: 10 + 20 * k });
      if (sink) {
        sink.immediate({ pos: origin, life: 1, sprite: SPRITE.glow, color: lin([1, 0.6, 0.8], 1.5 + 4 * k), size: 1.5 + 5 * k });
        this.tellAt += i.dt * (20 + 60 * k);
        while (this.tellAt >= 1) {
          this.tellAt -= 1;
          const s = Math.floor(i.time * 997) + Math.floor(this.tellAt * 31);
          const a = hash01(s, 1) * 6.283;
          const d = v3.normalize({ x: Math.cos(a), y: 1.2 + hash01(s, 2), z: Math.sin(a) });
          sink.spawn({ pos: origin, vel: v3.scale(d, 4 + 4 * hash01(s, 3)), life: 1.4, sprite: SPRITE.spark, blend: "alpha", color: lin(SPRINKLES[s % SPRINKLES.length] as Rgb, 1), alpha: 1, alpha1: 0, size: 0.13, drag: 0.6, spin: 5 });
        }
      }
    }
    if (clock.t >= 0 && clock.over === null && !this.horn) {
      this.horn = true;
      sound?.play("cake.horn");
      if (sink) confetti(sink, origin, 17);
    }
    // Crumbs where shots end inside the cake.
    const frame = this.tracker.update(i.shots, i.dt, i.time);
    if (wave && sink) {
      let n = 0;
      for (const g of frame.gone) {
        if (v3.length(v3.sub(g.position, origin)) > wave.radius) continue;
        if (n++ > 6) break;
        crumbBurst(sink, g.position, g.id);
      }
      out.debrisColor = CAKE_DEBRIS;
    }
    return clock.over === null || clock.over < CAKE_RECEDE_SECS;
  }
}

export const everythingIsCake = (env: EventEnv): EventVisual => new EverythingIsCake(env);
