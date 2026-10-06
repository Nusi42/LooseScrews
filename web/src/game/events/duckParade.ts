// Duck parade (M4 C3; milestone-4-plan.md §3.2): five giant glossy rubber
// ducks waddle in a convoy along the level's network (sim: B3's
// events/duck_parade.rs, `ReplicatedKind::Duck` props, state [index,
// clearance in tenths of a unit]). The client makes them alive:
//
// - squash and stretch: in a tunnel narrower than the duck (clearance below
//   its radius) the duck squeezes thinner and longer, on a spring, so it
//   pops back with a wobble when the tunnel opens;
// - a waddle (roll, bob and a nod), facing along the route with the level's
//   up as up (the sim's rotation arc rolls freely);
// - the lead duck wears a top hat, the last one sunglasses;
// - a trail of iridescent soap bubbles, a warm glow under each duck;
// - a squeak per projectile it swallows (EventHit cues) and when it shoves a
//   ship, with a squish; the marching kazoo tune follows the convoy.

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { PointLight } from "../../renderer/lights.ts";
import type { Rgb, SceneInstances } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { HAT_COLORS } from "../../renderer/models/spectacleProps.ts";
import type { VfxSink } from "../vfxLibrary.ts";
import { lin } from "../vfxLibrary.ts";
import type { EntityState } from "../views.ts";
import {
  clamp01,
  hash01,
  lookAlong,
  LoopVoice,
  PROP_KIND,
  pushInstance,
  slerp,
  SPECTACLE_CUE,
  type SpectacleInput,
  type SpectacleSoundEngine,
} from "./spectacleCommon.ts";

/** data/events/duck_parade.ron `duck_radius` (u); also the entity radius. */
export const DUCK_RADIUS = 3.5;
/** Model half width of the duck body (spectacleProps.ts): instance scale = radius / this. */
const MODEL_HALF_WIDTH = 0.82;
/** Classic rubber-duck yellow (display). */
export const DUCK_YELLOW: Rgb = [1.0, 0.8, 0.1];
/** Thinnest a duck squeezes (share of its size). */
export const MIN_SQUASH = 0.6;
/** Longest it stretches. */
export const MAX_STRETCH = 1.65;

/**
 * Squash target of a duck with `clearance` u of free space around its
 * centre: 1 in the open, down to MIN_SQUASH where the tunnel is narrow.
 */
export function squashTarget(clearance: number, radius = DUCK_RADIUS): number {
  return Math.min(1, Math.max(MIN_SQUASH, clearance / radius));
}

/** Stretch along the route for a squash `s`: most of the lost width goes into length (capped, so a duck stays a duck). */
export function stretchFor(s: number): number {
  return Math.min(MAX_STRETCH, Math.pow(Math.max(s, 1e-3), -1.6));
}

interface DuckAnim {
  id: number;
  index: number;
  pos: Vec3;
  vel: Vec3;
  facing: Quat;
  /** Spring: squash value and its speed. */
  s: number;
  sv: number;
  /** Last squish pulse (hit or shove), time (s). */
  hitAt: number;
  bubbleClock: number;
  seen: number;
}

/** One frame's spring step of a duck's squash toward `target`. */
export function springStep(s: number, sv: number, target: number, dt: number): [number, number] {
  // Underdamped: overshoots and wobbles when a tunnel ends.
  const k = 90;
  const c = 7;
  const steps = Math.max(1, Math.ceil(dt / (1 / 120)));
  const h = dt / steps;
  for (let i = 0; i < steps; i++) {
    sv += (k * (target - s) - c * sv) * h;
    s += sv * h;
  }
  return [s, sv];
}

export class DuckParadeFx {
  private ducks = new Map<number, DuckAnim>();
  private readonly kazoo = new LoopVoice("duck.kazoo");
  private readonly shoveAt = new Map<string, number>();
  sink: VfxSink | null = null;
  engine: SpectacleSoundEngine | null = null;
  /** Lights of the last update. */
  lights: PointLight[] = [];

  reset(): void {
    this.ducks.clear();
    this.shoveAt.clear();
    this.kazoo.stop(0.2);
  }

  /** Whether any duck is out. */
  get active(): boolean {
    return this.ducks.size > 0;
  }

  update(i: SpectacleInput): void {
    const seen = new Set<number>();
    for (const e of i.entities) {
      if (e.kind !== "duck") continue;
      seen.add(e.id);
      this.track(e, i);
    }
    for (const id of [...this.ducks.keys()]) if (!seen.has(id)) this.ducks.delete(id);

    // Squeaks: shots it swallowed, ships it shoves.
    for (const c of i.cues) {
      if (c.kind !== SPECTACLE_CUE.eventHit || c.value !== PROP_KIND.duck) continue;
      const d = this.nearest(c.position);
      if (d) d.hitAt = i.time;
      this.engine?.play("duck.squeak", { position: c.position, rate: 0.9 + 0.25 * hash01(c.tick, d?.index ?? 0) });
      this.pop(c.position, i.time);
    }
    const ships = i.own ? [...i.ships, { id: i.own.id, alive: true, position: i.own.position, radius: i.own.radius }] : i.ships;
    for (const d of this.ducks.values()) {
      for (const s of ships) {
        if (!s.alive) continue;
        const gap = v3.length(v3.sub(s.position, d.pos)) - DUCK_RADIUS - s.radius;
        if (gap > 0.6) continue;
        const key = `${d.id}:${s.id}`;
        const last = this.shoveAt.get(key) ?? -9;
        if (i.time - last < 0.8) continue;
        this.shoveAt.set(key, i.time);
        d.hitAt = i.time;
        this.engine?.play("duck.squeak", { position: d.pos, rate: 0.75 + 0.2 * hash01(d.index, Math.floor(i.time)) });
      }
    }

    // The kazoo marches with the lead duck.
    let lead: DuckAnim | null = null;
    for (const d of this.ducks.values()) if (!lead || d.index < lead.index) lead = d;
    this.kazoo.update(this.engine, lead && !i.quiet ? 1 : 0, lead?.pos ?? null);

    this.bubbles(i);
    this.lights = [...this.ducks.values()].map((d) => ({
      position: v3.addScaled(d.pos, i.up, -DUCK_RADIUS * 0.6),
      color: [1.0, 0.82, 0.35] as const,
      intensity: 5,
      range: 14,
    }));
  }

  private track(e: EntityState, i: SpectacleInput): void {
    let d = this.ducks.get(e.id);
    const speed = v3.length(e.velocity);
    const along = speed > 0.1 ? e.velocity : d ? q.forward(d.facing) : q.forward(e.rotation);
    const want = lookAlong(along, i.up);
    if (!d) {
      d = { id: e.id, index: e.state[0], pos: e.position, vel: e.velocity, facing: want, s: 1, sv: 0, hitAt: -9, bubbleClock: 0, seen: i.time };
      this.ducks.set(e.id, d);
    }
    d.pos = e.position;
    d.vel = e.velocity;
    d.index = e.state[0];
    d.seen = i.time;
    d.facing = slerp(d.facing, want, clamp01(i.dt * 4));
    const clearance = e.state[1] / 10;
    [d.s, d.sv] = springStep(d.s, d.sv, squashTarget(clearance, e.radius > 0 ? e.radius : DUCK_RADIUS), Math.min(i.dt, 0.1));
  }

  private nearest(p: Vec3): DuckAnim | null {
    let best: DuckAnim | null = null;
    let bd = Infinity;
    for (const d of this.ducks.values()) {
      const dd = v3.length(v3.sub(d.pos, p));
      if (dd < bd) {
        bd = dd;
        best = d;
      }
    }
    return bd < DUCK_RADIUS * 2.5 ? best : null;
  }

  /** A little burst of bubbles and a star where a shot plopped into a duck. */
  private pop(p: Vec3, time: number): void {
    const sink = this.sink;
    if (!sink) return;
    sink.spawn({ pos: p, life: 0.25, sprite: SPRITE.flare, color: lin([1, 0.95, 0.6], 6), size: 1.6, size1: 0.4, blend: "add", rot: time });
    for (let k = 0; k < 6; k++) {
      const dir = { x: Math.sin(k * 2.1 + time), y: Math.cos(k * 1.3), z: Math.cos(k * 2.1 + time) };
      sink.spawn({ pos: p, vel: v3.scale(v3.normalize(dir), 3), life: 1.1, sprite: SPRITE.ring, color: iridescent(k * 0.17 + time), size: 0.35, size1: 0.5, drag: 1.5, blend: "add" });
    }
  }

  /** Soap bubbles streaming from every duck's tail. */
  private bubbles(i: SpectacleInput): void {
    const sink = this.sink;
    if (!sink) return;
    for (const d of this.ducks.values()) {
      d.bubbleClock += i.dt * 5 * sink.density;
      while (d.bubbleClock >= 1) {
        d.bubbleClock -= 1;
        const back = q.rotate(d.facing, { x: 0, y: 0, z: 1 });
        const r1 = hash01(d.id, i.time * 61);
        const r2 = hash01(d.id + 3, i.time * 37);
        const side = q.rotate(d.facing, { x: r1 * 2 - 1, y: r2 * 1.4 - 0.2, z: 0 });
        const pos = v3.add(d.pos, v3.add(v3.scale(back, DUCK_RADIUS * d.s * stretchFor(d.s) * 0.9), v3.scale(side, DUCK_RADIUS * 0.45)));
        const vel = v3.add(v3.scale(back, 0.8), v3.scale(i.up, 0.5 + r2 * 0.6));
        const size = 0.3 + 0.6 * r1;
        sink.spawn({ pos, vel, life: 1.6 + r2, sprite: SPRITE.ring, color: iridescent(r1 + i.time * 0.2), color1: iridescent(r1 + 0.4), size, size1: size * 1.15, drag: 0.6, blend: "add", fadeIn: 0.1 });
        sink.spawn({ pos, vel, life: 1.6 + r2, sprite: SPRITE.glow, color: lin([0.6, 0.8, 1.0], 0.12), size: size * 0.9, drag: 0.6, blend: "add", fadeIn: 0.1 });
      }
    }
  }

  addToScene(scene: SceneInstances, time: number, up: Vec3): void {
    for (const d of this.ducks.values()) {
      const pose = duckPose(d, time, up);
      const scale = (DUCK_RADIUS / MODEL_HALF_WIDTH) * pose.scale;
      pushInstance(scene, "opaque", "rubber_duck", { position: pose.position, rotation: pose.rotation, scale, stretch: pose.stretch, color: DUCK_YELLOW, emissive: 0 });
      // The last duck of five is the cool one; the lead wears the top hat.
      if (d.index === 4) pushInstance(scene, "opaque", "duck_shades", { position: pose.position, rotation: pose.rotation, scale, stretch: pose.stretch, color: [0, 0, 0], emissive: 0 });
      if (d.index === 0) {
        const head = q.rotate(pose.rotation, { x: 0, y: 1.0 * scale, z: -0.48 * scale * pose.stretch });
        const tilt = q.mul(pose.rotation, q.mul(q.fromAxisAngle({ x: 1, y: 0, z: 0 }, 0.12), q.fromAxisAngle({ x: 0, y: 0, z: 1 }, -0.16)));
        pushInstance(scene, "opaque", "hat_0", { position: v3.add(pose.position, head), rotation: tilt, scale: scale * 0.78, color: HAT_COLORS[0] ?? [0.7, 0.1, 0.1], emissive: 0 });
      }
    }
  }
}

/** A duck's drawn pose: waddle, nod, squish and squash on top of the sim's position. */
export function duckPose(d: Pick<DuckAnim, "pos" | "facing" | "s" | "hitAt" | "index">, time: number, up: Vec3): { position: Vec3; rotation: Quat; scale: number; stretch: number } {
  const ph = time * Math.PI * 2 * 1.1 + d.index * 1.7;
  const roll = 0.11 * Math.sin(ph);
  const nod = 0.05 * Math.sin(ph * 2 + 0.6);
  const rotation = q.mul(d.facing, q.mul(q.fromAxisAngle({ x: 0, y: 0, z: 1 }, roll), q.fromAxisAngle({ x: 1, y: 0, z: 0 }, nod)));
  const bob = 0.18 * Math.abs(Math.sin(ph));
  // A squish when hit or shoving: a quick dip in size that springs back.
  const since = time - d.hitAt;
  const squish = since >= 0 && since < 0.5 ? 0.12 * Math.exp(-since * 9) * Math.cos(since * 30) : 0;
  const s = Math.max(MIN_SQUASH * 0.9, d.s - squish);
  return { position: v3.addScaled(d.pos, up, bob), rotation, scale: s, stretch: stretchFor(s) * (1 + squish * 0.6) };
}

/** A soap-bubble colour cycling through the rainbow (linear HDR, faint). */
export function iridescent(t: number): readonly [number, number, number] {
  const a = t * Math.PI * 2;
  return [0.55 + 0.45 * Math.cos(a), 0.55 + 0.45 * Math.cos(a + 2.1), 0.55 + 0.45 * Math.cos(a + 4.2)].map((c) => c * 0.9) as unknown as readonly [number, number, number];
}
