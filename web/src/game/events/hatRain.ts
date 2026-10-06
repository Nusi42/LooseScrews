// Hat rain (M4 C3; milestone-4-plan.md §3.2): hats drift down along the
// level's up and rest on rock; touching one wears it (sim: B3's
// events/hat_rain.rs, `ReplicatedKind::Hat` props with state [style 0-7,
// phase 0 falling / 1 resting / 2 scattered / 3 scattered and resting], the
// worn count in the ship's `ext` bits). The client draws:
//
// - eight hat styles (renderer/models/spectacleProps.ts), falling like
//   leaves (flutter and spin), resting upright with a glint now and then;
// - the stack on every hull: hats sit on each other and sway on a spring
//   driven by the ship's acceleration (each hat a little more than the one
//   below, so tall stacks whip around); the beanie's propeller spins;
// - a pop and a puff of confetti per hat taken, a golden glint over a
//   stack of three or more while the event runs (everyone sees you);
// - a HUD chip with the own count.

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { PointLight } from "../../renderer/lights.ts";
import { hatMeshKind } from "../../renderer/models/index.ts";
import { HAT_COLORS, HAT_HEIGHTS } from "../../renderer/models/spectacleProps.ts";
import type { SceneInstances } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { lin, type VfxSink } from "../vfxLibrary.ts";
import type { EntityState } from "../views.ts";
import { clamp01, hash01, hatCounts, pushInstance, uprightToward, type SpectacleInput, type SpectacleSoundEngine } from "./spectacleCommon.ts";

/** Drawn size of a loose hat (instance scale: brim radius 0.5 x this, u). */
export const LOOSE_HAT_SCALE = 1.7;
/** A worn hat's scale per unit of hull radius. */
export const WORN_HAT_SCALE = 1.3;
/** Where the stack sits on a hull: this times the hull radius above its centre. */
export const HAT_SEAT = 0.4;
/** data/events/hat_rain.ron `reveal_hats`. */
export const REVEAL_HATS = 3;

/** One worn stack's sway: tilt about the hull's right (x) and nose axes (z), and their speeds. */
interface Sway {
  x: number;
  z: number;
  vx: number;
  vz: number;
}

interface Stack {
  styles: number[];
  sway: Sway;
  lastPos: Vec3 | null;
  lastVel: Vec3 | null;
  prop: number;
}

/** Hat styles a ship wears, given the styles of hats it just touched (else seeded by ship and slot). */
export function growStack(styles: readonly number[], count: number, taken: readonly number[], ship: number): number[] {
  const out = styles.slice(0, Math.max(0, count));
  let k = 0;
  while (out.length < count) {
    const s = taken[k++] ?? Math.floor(hash01(ship * 7 + out.length, 13) * 8);
    out.push(s % 8);
  }
  return out;
}

/** Spring step of a stack's sway toward the tilt the hull's acceleration asks for (ship-local, u/s^2). */
export function swayStep(s: Sway, accRight: number, accFwd: number, dt: number): Sway {
  const k = 55;
  const c = 3.6;
  const lim = 0.7;
  // The hats lag behind the hull: accelerating forward tips them back (+x), right tips them left (+z).
  const tx = Math.max(-lim, Math.min(lim, accFwd * 0.018));
  const tz = Math.max(-lim, Math.min(lim, accRight * 0.018));
  const steps = Math.max(1, Math.ceil(dt / (1 / 120)));
  const h = dt / steps;
  let { x, z, vx, vz } = s;
  for (let i = 0; i < steps; i++) {
    vx += (k * (tx - x) - c * vx) * h;
    vz += (k * (tz - z) - c * vz) * h;
    x += vx * h;
    z += vz * h;
  }
  return { x, z, vx, vz };
}

/** Poses of a worn stack on a hull at `pos` with orientation `rot` and hull radius `radius`. */
export function stackPoses(styles: readonly number[], pos: Vec3, rot: Quat, radius: number, sway: Pick<Sway, "x" | "z">, time: number): Array<{ style: number; position: Vec3; rotation: Quat; scale: number }> {
  const scale = WORN_HAT_SCALE * radius;
  let base = v3.addScaled(pos, q.up(rot), HAT_SEAT * radius);
  let r = rot;
  const out: Array<{ style: number; position: Vec3; rotation: Quat; scale: number }> = [];
  styles.forEach((style, i) => {
    // Each hat tilts a bit more than the one below; a small idle wobble on top.
    const k = 1 + 0.45 * i;
    const wob = 0.04 * Math.sin(time * 2.3 + i * 1.1) * Math.min(1, i * 0.5);
    const tilt = q.mul(q.fromAxisAngle({ x: 1, y: 0, z: 0 }, sway.x * (i === 0 ? 0.5 : 0.35 * k)), q.fromAxisAngle({ x: 0, y: 0, z: 1 }, sway.z * (i === 0 ? 0.5 : 0.35 * k) + wob));
    r = q.normalize(q.mul(r, tilt));
    // A jaunty yaw per hat so the stack is not a neat column.
    const yaw = q.fromAxisAngle({ x: 0, y: 1, z: 0 }, (hash01(style, i) - 0.5) * 0.9);
    const rotation = q.mul(r, yaw);
    out.push({ style, position: base, rotation, scale });
    base = v3.addScaled(base, q.up(r), (HAT_HEIGHTS[style] ?? 0.4) * scale * 0.82);
  });
  return out;
}

export class HatRainFx {
  private stacks = new Map<number, Stack>();
  private loose = new Map<number, { style: number; pos: Vec3 }>();
  /** Where a resting hat's brim touches the rock (one ray along -up when it lands). */
  private seats = new Map<number, Vec3>();
  sink: VfxSink | null = null;
  engine: SpectacleSoundEngine | null = null;
  lights: PointLight[] = [];
  /** The own ship's hats (HUD chip). */
  ownHats = 0;

  reset(): void {
    this.stacks.clear();
    this.loose.clear();
    this.seats.clear();
    this.ownHats = 0;
  }

  update(i: SpectacleInput): void {
    const counts = hatCounts(i.shipExt);
    // Hats that vanished this frame: someone took them (or they expired).
    const now = new Map<number, { style: number; pos: Vec3 }>();
    for (const e of i.entities) if (e.kind === "hat") now.set(e.id, { style: e.state[0], pos: e.position });
    const gone = [...this.loose.entries()].filter(([id]) => !now.has(id)).map(([, h]) => h);
    this.loose = now;
    for (const e of i.entities) {
      if (e.kind !== "hat") continue;
      const resting = e.state[1] === 1 || e.state[1] === 3;
      if (!resting) {
        this.seats.delete(e.id);
        continue;
      }
      if (this.seats.has(e.id)) continue;
      // The sim rests a hat when rock is within its touch radius: put the brim on it.
      const down = v3.scale(i.up, -1);
      const t = i.raycast ? i.raycast(e.position, down, 2) : -1;
      this.seats.set(e.id, v3.addScaled(e.position, down, t >= 0 ? Math.max(0, t - 0.04) : 0.7));
    }
    for (const id of [...this.seats.keys()]) if (!now.has(id)) this.seats.delete(id);

    const poses = new Map<number, { pos: Vec3; rot: Quat; radius: number }>();
    for (const s of i.ships) if (s.alive) poses.set(s.id, { pos: s.position, rot: s.orientation, radius: s.radius });
    if (i.own) poses.set(i.own.id, { pos: i.own.position, rot: i.own.orientation, radius: i.own.radius });

    for (const [id, count] of counts) {
      let st = this.stacks.get(id);
      if (!st) {
        if (count === 0) continue;
        st = { styles: [], sway: { x: 0, z: 0, vx: 0, vz: 0 }, lastPos: null, lastVel: null, prop: 0 };
        this.stacks.set(id, st);
      }
      const pose = poses.get(id);
      if (count > st.styles.length) {
        // The hats it touched: the vanished ones nearest this hull.
        const near = pose ? gone.filter((h) => v3.length(v3.sub(h.pos, pose.pos)) < 4).map((h) => h.style) : [];
        const added = count - st.styles.length;
        st.styles = growStack(st.styles, count, near, id);
        if (pose) {
          const top = v3.addScaled(pose.pos, q.up(pose.rot), pose.radius * (HAT_SEAT + 0.6 + 0.4 * st.styles.length));
          this.pop(top, i.time, added);
          this.engine?.play("hat.pop", { position: id === i.own?.id ? null : top, rate: 0.85 + 0.08 * Math.min(6, st.styles.length) });
        }
      } else if (count < st.styles.length) {
        st.styles = st.styles.slice(0, count);
      }
      // Sway from the hull's acceleration.
      if (pose) {
        const dt = Math.max(1e-3, Math.min(0.1, i.dt));
        const vel = st.lastPos ? v3.scale(v3.sub(pose.pos, st.lastPos), 1 / dt) : { x: 0, y: 0, z: 0 };
        const acc = st.lastVel ? v3.scale(v3.sub(vel, st.lastVel), 1 / dt) : { x: 0, y: 0, z: 0 };
        // Snapshot jitter makes raw accelerations spiky: soften them.
        const soft = (a: number): number => Math.sign(a) * Math.min(60, Math.abs(a));
        st.sway = swayStep(st.sway, soft(v3.dot(acc, q.right(pose.rot))), soft(v3.dot(acc, q.forward(pose.rot))), dt);
        st.lastVel = st.lastVel ? v3.lerp(st.lastVel, vel, 0.5) : vel;
        st.lastPos = pose.pos;
        st.prop += dt * (14 + 1.2 * v3.length(vel));
      }
    }
    for (const id of [...this.stacks.keys()]) if (!counts.has(id) || (counts.get(id) ?? 0) === 0) this.stacks.delete(id);
    this.ownHats = i.own ? (counts.get(i.own.id) ?? 0) : 0;
    this.glints(i, poses);
  }

  private pop(p: Vec3, time: number, n: number): void {
    const sink = this.sink;
    if (!sink) return;
    sink.spawn({ pos: p, life: 0.3, sprite: SPRITE.flare, color: lin([1, 0.9, 0.55], 5), size: 1.4, size1: 0.3, blend: "add", rot: time });
    const confetti: ReadonlyArray<readonly [number, number, number]> = [
      [1, 0.25, 0.35],
      [0.3, 0.7, 1],
      [1, 0.85, 0.2],
      [0.4, 1, 0.45],
      [0.9, 0.4, 1],
    ];
    for (let k = 0; k < 14 * n; k++) {
      const a = hash01(k, time * 13) * Math.PI * 2;
      const b = hash01(k + 5, time * 7) * 2 - 1;
      const r = Math.sqrt(1 - b * b);
      const dir = { x: Math.cos(a) * r, y: b * 0.6 + 0.6, z: Math.sin(a) * r };
      const c = confetti[k % confetti.length]!;
      sink.spawn({ pos: p, vel: v3.scale(dir, 3 + 3 * hash01(k, 3)), life: 0.9 + 0.5 * hash01(k, 9), sprite: SPRITE.shard, color: c, alpha: 1, alpha1: 0, size: 0.16, drag: 2.2, blend: "alpha", spin: 8 * (hash01(k, 4) - 0.5), rot: a });
    }
  }

  /** Resting hats glint; stacks of REVEAL_HATS or more shine while the event runs. */
  private glints(i: SpectacleInput, poses: Map<number, { pos: Vec3; rot: Quat; radius: number }>): void {
    const sink = this.sink;
    this.lights = [];
    const running = i.mood.eventId === "hat_rain" && i.mood.phase === "active";
    for (const [id, st] of this.stacks) {
      if (!running || st.styles.length < REVEAL_HATS) continue;
      const pose = poses.get(id);
      if (!pose || id === i.own?.id) continue;
      const top = stackPoses(st.styles, pose.pos, pose.rot, pose.radius, st.sway, i.time).at(-1);
      if (!top) continue;
      const p = v3.addScaled(top.position, q.up(top.rotation), (HAT_HEIGHTS[top.style] ?? 0.4) * top.scale + 0.35);
      const pulse = 0.6 + 0.4 * Math.sin(i.time * 6 + id);
      sink?.immediate({ pos: p, life: 1, sprite: SPRITE.flare, color: lin([1, 0.82, 0.3], 7 * pulse), size: 1.3 + 0.4 * pulse, blend: "add", rot: i.time * 0.8 });
      this.lights.push({ position: p, color: [1, 0.8, 0.35], intensity: 4 * pulse, range: 8 });
    }
    if (!sink) return;
    for (const e of i.entities) {
      if (e.kind !== "hat" || (e.state[1] !== 1 && e.state[1] !== 3)) continue;
      const cycle = (i.time * 0.45 + hash01(e.id, 2)) % 1;
      if (cycle > 0.06) continue;
      const k = Math.sin((cycle / 0.06) * Math.PI);
      const p = v3.addScaled(e.position, i.up, 0.45 * LOOSE_HAT_SCALE);
      sink.immediate({ pos: p, life: 1, sprite: SPRITE.flare, color: lin([1, 0.95, 0.8], 5 * k), size: 0.9 * k + 0.1, blend: "add", rot: e.id });
    }
  }

  addToScene(scene: SceneInstances, entities: readonly EntityState[], ships: SpectacleInput["ships"], time: number, up: Vec3): void {
    for (const e of entities) {
      if (e.kind !== "hat") continue;
      const pose = looseHatPose(e, time, up);
      const seat = this.seats.get(e.id);
      if (seat) pose.position = seat;
      this.pushHat(scene, e.state[0], pose.position, pose.rotation, LOOSE_HAT_SCALE * pose.scale, time + e.id);
    }
    for (const s of ships) {
      const st = this.stacks.get(s.id);
      if (!st || !s.alive) continue;
      for (const h of stackPoses(st.styles, s.position, s.orientation, s.radius, st.sway, time)) this.pushHat(scene, h.style, h.position, h.rotation, h.scale, st.prop);
    }
  }

  private pushHat(scene: SceneInstances, style: number, position: Vec3, rotation: Quat, scale: number, prop: number): void {
    const color = HAT_COLORS[style % 8] ?? [0.6, 0.2, 0.2];
    pushInstance(scene, "opaque", hatMeshKind(style), { position, rotation, scale, color, emissive: 0 });
    if (style % 8 === 5) {
      const top = v3.addScaled(position, q.up(rotation), 0.38 * scale);
      pushInstance(scene, "opaque", "hat_prop", { position: top, rotation: q.mul(rotation, q.fromAxisAngle({ x: 0, y: 1, z: 0 }, prop)), scale, color, emissive: 0 });
    }
  }

  /** Sets the styles a ship wears (the gallery's curated stacks; the count still comes from `shipExt`). */
  presetStack(id: number, styles: readonly number[]): void {
    this.stacks.set(id, { styles: [...styles], sway: { x: 0, z: 0, vx: 0, vz: 0 }, lastPos: null, lastVel: null, prop: 0 });
  }

  /** Styles worn by a ship (tests, kill-cam). */
  stackOf(id: number): readonly number[] {
    return this.stacks.get(id)?.styles ?? [];
  }
}

/** A loose hat's drawn pose: fluttering down like a leaf, resting upright, tumbling out of a wreck. */
export function looseHatPose(e: Pick<EntityState, "id" | "position" | "state">, time: number, up: Vec3): { position: Vec3; rotation: Quat; scale: number } {
  const yawDir = { x: Math.cos(e.id * 2.4), y: 0.13, z: Math.sin(e.id * 2.4) };
  const base = uprightToward(up, yawDir);
  const phase = e.state[1];
  if (phase === 0) {
    // Falling: rocks side to side, swings and spins slowly.
    const t = time * 1.9 + e.id * 0.7;
    const rock = q.mul(q.fromAxisAngle({ x: 1, y: 0, z: 0 }, 0.42 * Math.sin(t)), q.fromAxisAngle({ x: 0, y: 0, z: 1 }, 0.3 * Math.sin(t * 0.7 + 1)));
    const spin = q.fromAxisAngle({ x: 0, y: 1, z: 0 }, time * 0.6 + e.id);
    const swing = v3.scale(q.rotate(base, { x: Math.sin(t) * 0.35, y: 0, z: Math.cos(t * 0.7) * 0.2 }), 1);
    return { position: v3.add(e.position, swing), rotation: q.mul(base, q.mul(spin, rock)), scale: 1 };
  }
  if (phase === 2) {
    // Flying out of a wreck: tumbling.
    const axis = v3.normalize({ x: hash01(e.id, 1) - 0.5, y: hash01(e.id, 2) - 0.5, z: hash01(e.id, 3) - 0.5 });
    return { position: e.position, rotation: q.mul(base, q.fromAxisAngle(axis, time * 9)), scale: 1 };
  }
  // Resting (also scattered and resting: a slow pulse says "grab me before I go").
  const pulse = phase === 3 ? 1 + 0.06 * Math.sin(time * 7 + e.id) : 1;
  return { position: e.position, rotation: q.mul(base, q.fromAxisAngle({ x: 0, y: 0, z: 1 }, (hash01(e.id, 5) - 0.5) * 0.25)), scale: pulse * clamp01(1) };
}
