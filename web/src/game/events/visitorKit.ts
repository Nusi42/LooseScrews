// Shared pieces of the visitor events' looks (M4 C2, game/events/visitors.ts):
// the event outcome cue, poses of attached parts, drawing helpers, the
// drones' hover and bank, damage smoke and the frame context every event
// file draws into. Pure maths and data; no WebGPU or DOM.

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { PointLight } from "../../renderer/lights.ts";
import type { MeshKind } from "../../renderer/models/index.ts";
import type { Instance, Rgb, SceneInstances } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { EVENT_IDS } from "../../audio/announcer/lines.ts";
import type { Cue } from "../mechanics.ts";
import { lin, type VfxSink } from "../vfxLibrary.ts";
import type { MoodState } from "../modeState.ts";
import type { EntityState, VisualDef } from "../views.ts";

/** `ls_sim::CueKind::EventOutcome`. */
export const CUE_EVENT_OUTCOME = 18;
/** Outcome byte of a drone or mimic shot down (`events::drone::OUTCOME_SHOT_DOWN`). */
export const OUTCOME_SHOT_DOWN = 0xf0;
/** Ship id meaning "none" in cues and entities. */
export const NONE = 255;
/** Simulation ticks per second. */
export const HZ = 60;

/** Drone roles (`state[0]`) and phases (`state[2]`) of `ReplicatedKind::Drone` (`events::drone`). */
export const ROLE = { tax: 0, pizza: 1, survey: 2 } as const;
export const DRONE_PHASE = { waiting: 0, working: 1, leaving: 2 } as const;

/** An event outcome cue, decoded: the event id and its outcome byte. */
export interface Outcome {
  event: string;
  byte: number;
  source: number;
  target: number;
  position: Vec3;
}

/** Decodes `CueKind::EventOutcome` cues (`value` = event index << 8 | byte); other cues are skipped. */
export function outcomes(cues: readonly Cue[]): Outcome[] {
  const out: Outcome[] = [];
  for (const c of cues) {
    if (c.kind !== CUE_EVENT_OUTCOME) continue;
    const event = EVENT_IDS[(c.value >> 8) & 0xff];
    if (!event) continue;
    out.push({ event, byte: c.value & 0xff, source: c.source, target: c.target, position: c.position });
  }
  return out;
}

/** A pose of a drawn part: position, rotation and uniform scale. */
export interface Pose {
  position: Vec3;
  rotation: Quat;
  scale: number;
}

/** The pose of a part attached to `parent` at `offset` (parent space, scaled with it), turned by `rot` relative to it. */
export function attach(parent: Pose, offset: Vec3, rot: Quat = q.identity(), scale = 1): Pose {
  return {
    position: v3.add(parent.position, q.rotate(parent.rotation, v3.scale(offset, parent.scale))),
    rotation: q.normalize(q.mul(parent.rotation, rot)),
    scale: parent.scale * scale,
  };
}

/** Rotation turning a Z-up model (renderer/models/visitors.ts `zUp`) upright under `rot`. */
export function fromZUp(rot: Quat): Quat {
  return q.normalize(q.mul(rot, q.fromAxisAngle({ x: 1, y: 0, z: 0 }, -Math.PI / 2)));
}

/** What every visitor draws into this frame. */
export interface DrawCtx {
  scene: SceneInstances;
  time: number;
  lights: PointLight[];
  /** Particles (null without VQ5 effects). */
  sink: VfxSink | null;
  /** The level's up. */
  up: Vec3;
}

/** Pushes an opaque instance of `mesh` at `pose`. */
export function put(ctx: DrawCtx, mesh: MeshKind, pose: Pose, color: Rgb, extra: Partial<Instance> = {}): void {
  (ctx.scene.opaque[mesh] ??= []).push({ position: pose.position, rotation: pose.rotation, scale: pose.scale, color, emissive: 0, ...extra });
}

/** Pushes a translucent instance. */
export function putGlass(ctx: DrawCtx, mesh: MeshKind, pose: Pose, color: Rgb, opacity: number, emissive = 1): void {
  (ctx.scene.translucent[mesh] ??= []).push({ position: pose.position, rotation: pose.rotation, scale: pose.scale, color, emissive, opacity });
}

/**
 * The drawn pose of a hovering drone: the sim's pose with a slow bob along
 * the level's up and a bank into its velocity (nose down when it speeds
 * ahead, rolled into sideways motion), at most about 25 degrees.
 */
export function hoverPose(e: Pick<EntityState, "id" | "position" | "rotation" | "velocity">, time: number, up: Vec3, scale = 1): Pose {
  const ph = (e.id * 2.39996) % (Math.PI * 2);
  const bob = Math.sin(time * 2.1 + ph) * 0.06 + Math.sin(time * 3.7 + ph * 2) * 0.025;
  // Drones fly level: the sim's facing (which also pitches toward the target) turned about the up only.
  const fwd = q.forward(e.rotation);
  const flat = v3.sub(fwd, v3.scale(up, v3.dot(fwd, up)));
  const rot = v3.length(flat) > 0.05 ? q.lookRotation(flat, up) : e.rotation;
  const local = q.rotate(q.conjugate(rot), e.velocity);
  const tilt = (x: number): number => Math.max(-0.45, Math.min(0.45, x * 0.03));
  // Moving along -Z (ahead) pitches the nose down; along +X rolls to the right.
  const bank = q.mul(q.fromAxisAngle({ x: 1, y: 0, z: 0 }, tilt(local.z)), q.fromAxisAngle({ x: 0, y: 0, z: 1 }, -tilt(local.x)));
  return { position: v3.addScaled(e.position, up, bob), rotation: q.normalize(q.mul(rot, bank)), scale };
}

/** Rotor spin about the part's up (rad) at `time`; drones' rotors turn fast with a per-rotor phase. */
export function rotorSpin(time: number, i: number): Quat {
  return q.fromAxisAngle({ x: 0, y: 1, z: 0 }, time * (31 + (i % 3) * 3) * (i % 2 === 0 ? 1 : -1) + i * 1.3);
}

/** Integrity in 0..1 from a drone's or mimic's hp percent byte (`state[3]`). */
export function integrity(e: Pick<EntityState, "state">): number {
  return Math.max(0, Math.min(1, (e.state[3] ?? 100) / 100));
}

/**
 * Smoke and sparks of a damaged visitor (below 60 % integrity smoke, below
 * 30 % sparks too), spawned at `rate` puffs per second of `dt`.
 */
export function damageSmoke(sink: VfxSink | null, at: Vec3, health: number, dt: number, rnd: () => number): void {
  if (!sink || health >= 0.6 || dt <= 0) return;
  const heavy = 1 - health / 0.6;
  if (rnd() < dt * (4 + 10 * heavy) * sink.density) {
    sink.spawn({
      pos: v3.add(at, { x: (rnd() - 0.5) * 0.4, y: (rnd() - 0.5) * 0.3, z: (rnd() - 0.5) * 0.4 }),
      vel: { x: (rnd() - 0.5) * 0.6, y: 0.8 + rnd() * 0.6, z: (rnd() - 0.5) * 0.6 },
      life: 1.2 + rnd() * 0.8,
      sprite: (SPRITE.smoke0 + Math.floor(rnd() * 4)) as 2 | 3 | 4 | 5,
      color: [0.09, 0.085, 0.08],
      alpha: 0.5 + 0.3 * heavy,
      alpha1: 0,
      size: 0.35,
      size1: 1.2,
      drag: 0.8,
      blend: "alpha",
      spin: (rnd() - 0.5) * 1.5,
    });
  }
  if (health < 0.3 && rnd() < dt * 6 * sink.density) {
    for (let i = 0; i < 3; i++) {
      sink.spawn({
        pos: at,
        vel: { x: (rnd() - 0.5) * 6, y: rnd() * 4, z: (rnd() - 0.5) * 6 },
        life: 0.25 + rnd() * 0.2,
        sprite: SPRITE.spark,
        color: lin([1, 0.7, 0.3], 6),
        size: 0.06,
        orient: "velocity",
        stretch: 0.03,
        drag: 2,
      });
    }
  }
}

/** A burst of `n` particles (scaled by the preset's density; shot-down visitors, deliveries). */
export function burst(sink: VfxSink | null, n: number, spec: (i: number, rnd: () => number) => Parameters<VfxSink["spawn"]>[0], rnd: () => number): void {
  if (!sink) return;
  const count = Math.max(1, Math.round(n * sink.density));
  for (let i = 0; i < count; i++) sink.spawn(spec(i, rnd));
}

/** Smoothstep. */
export function smooth01(x: number): number {
  const t = Math.max(0, Math.min(1, x));
  return t * t * (3 - 2 * t);
}

/** Small deterministic RNG (mulberry32) for the visuals' jitter. */
export function rng(seed: number): () => number {
  let a = seed >>> 0 || 0x9e3779b9;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Seconds from `nowTick` to the mood view's phase end (0 when past). */
export function secsLeft(phaseEndTick: number, nowTick: number): number {
  return Math.max(0, (phaseEndTick - nowTick) / HZ);
}

/** A ship as the visitors see it. */
export interface VisitorShip {
  id: number;
  alive: boolean;
  position: Vec3;
}

/** What the visitor events read each frame (game/events/visitors.ts builds it). */
export interface VisitorInput {
  time: number;
  dt: number;
  nowTick: number;
  camera: Vec3;
  orientation: Quat;
  mood: MoodState;
  outcomes: readonly Outcome[];
  entities: readonly EntityState[];
  /** Every ship the client knows, the own one included. */
  ships: readonly VisitorShip[];
  own: number;
  name: (id: number) => string;
  visuals: readonly VisualDef[];
  /** The match's pickup points in the world's order (Mimic pickups' spawn indices). */
  pickupPoints: readonly Vec3[];
}

/** A short HUD line (ui/visitorHud.ts shows it for a few seconds). */
export interface Toast {
  event: string;
  text: string;
  /** About the own pilot (drawn larger). */
  own: boolean;
  /** A star rating (1-5) shown after the text (Customer survey). */
  stars?: number;
}

/** A sound request: a one-shot, or a loop kept alive under `loop` while requested every frame. */
export interface SoundCall {
  id: string;
  position: Vec3 | null;
  loop?: string;
  gain?: number;
}

/** What the events hand back besides instances, lights and particles. */
export interface VisitorOut {
  toasts: Toast[];
  sounds: SoundCall[];
}

/** The ship `id` if the client knows it alive. */
export function shipOf(i: Pick<VisitorInput, "ships">, id: number): VisitorShip | undefined {
  return i.ships.find((s) => s.id === id && s.alive);
}
