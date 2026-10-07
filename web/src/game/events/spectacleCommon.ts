// Shared pieces of M4 C3's spectacle event visuals (Duck parade, Hat rain,
// Cat paw, Disco inferno, Tired announcer; game/events/spectacles.ts runs
// them): the per-frame input, scene helpers and small maths. Pure, so the
// Node tests can drive every module with synthetic states.

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { MeshKind } from "../../renderer/models/index.ts";
import type { Instance, Rgb, SceneInstances } from "../../renderer/renderer.ts";
import type { Cue } from "../mechanics.ts";
import type { MoodState } from "../modeState.ts";
import type { EntityState, ProjectileState, ShipState } from "../views.ts";

/** `ls_sim::CueKind` values of the spectacles (protocol 6). */
export const SPECTACLE_CUE = { eventAnnounce: 16, killCam: 19, eventHit: 20 } as const;

/** `ReplicatedKind` bytes of the props (EventHit cue values). */
export const PROP_KIND = { duck: 12, hat: 13, catPaw: 14, discoBall: 16 } as const;

/** Ship id meaning "none" (cues). */
export const NONE = 255;

/** The own ship as the spectacles see it (alive, flying). */
export interface OwnShip {
  id: number;
  team: number;
  position: Vec3;
  orientation: Quat;
  shipType: number;
  radius: number;
}

/** What the spectacle visuals read each frame (main.ts and the gallery build it). */
export interface SpectacleInput {
  time: number;
  dt: number;
  nowTick: number;
  camera: Vec3;
  orientation: Quat;
  mood: MoodState;
  cues: readonly Cue[];
  /** Other ships (not the own one). */
  ships: readonly ShipState[];
  own: OwnShip | null;
  entities: readonly EntityState[];
  /** Projectiles (the kill-cam records them). */
  shots: readonly ProjectileState[];
  /** ls-wasm `Game.shipExt()`: [id, pigmented, hats] per ship. */
  shipExt: ArrayLike<number> | undefined;
  /** Side colour (display) of a ship id. */
  colorOf: (id: number) => Rgb;
  /** Player name of a ship id ("" if unknown). */
  nameOf: (id: number) => string;
  /** The level's up (unit). */
  up: Vec3;
  /** First rock along a ray (distance, -1 for none within `max`). */
  raycast?: ((o: Vec3, d: Vec3, max: number) => number) | undefined;
  /** Results screen or lobby: loops fade out. */
  quiet: boolean;
}

export function pushInstance(scene: SceneInstances, pass: "opaque" | "translucent", mesh: MeshKind, inst: Instance): void {
  (scene[pass][mesh] ??= []).push(inst);
}

/** Deterministic 0..1 hash of two integers (per-prop variation). */
export function hash01(a: number, b = 0): number {
  const x = Math.sin(a * 127.1 + b * 311.7 + 74.7) * 43758.5453;
  return x - Math.floor(x);
}

export const clamp01 = (x: number): number => Math.min(1, Math.max(0, x));

export function smoothstep(a: number, b: number, x: number): number {
  const t = clamp01((x - a) / (b - a));
  return t * t * (3 - 2 * t);
}

/** Exponential approach of `from` to `to` with time constant `tau` (s). */
export function approach(from: number, to: number, dt: number, tau: number): number {
  if (tau <= 0) return to;
  return to + (from - to) * Math.exp(-dt / tau);
}

/** A unit vector perpendicular to `n`. */
export function perpendicular(n: Vec3): Vec3 {
  const h = Math.abs(n.y) > 0.9 ? { x: 1, y: 0, z: 0 } : { x: 0, y: 1, z: 0 };
  return v3.normalize(v3.cross(n, h));
}

/** Orientation looking along `dir` (-Z) with `up` as close as possible (falls back when parallel). */
export function lookAlong(dir: Vec3, up: Vec3): Quat {
  const d = v3.length(dir) > 1e-9 ? v3.normalize(dir) : { x: 0, y: 0, z: -1 };
  const hint = Math.abs(v3.dot(d, up)) > 0.98 ? perpendicular(up) : up;
  return q.lookRotation(d, hint);
}

/** Orientation whose local +Y is `up` (unit) and whose -Z leans toward `toward`. */
export function uprightToward(up: Vec3, toward: Vec3): Quat {
  const f = v3.sub(toward, v3.scale(up, v3.dot(toward, up)));
  return q.lookRotation(v3.length(f) > 1e-6 ? f : perpendicular(up), up);
}

/** Quaternion slerp (shortest arc). */
export function slerp(a: Quat, b: Quat, t: number): Quat {
  let d = a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w;
  let bx = b.x, by = b.y, bz = b.z, bw = b.w;
  if (d < 0) {
    d = -d;
    bx = -bx;
    by = -by;
    bz = -bz;
    bw = -bw;
  }
  if (d > 0.9995) return q.normalize({ x: a.x + (bx - a.x) * t, y: a.y + (by - a.y) * t, z: a.z + (bz - a.z) * t, w: a.w + (bw - a.w) * t });
  const th = Math.acos(d);
  const s = Math.sin(th);
  const wa = Math.sin((1 - t) * th) / s;
  const wb = Math.sin(t * th) / s;
  return { x: a.x * wa + bx * wb, y: a.y * wa + by * wb, z: a.z * wa + bz * wb, w: a.w * wa + bw * wb };
}

/** Hats per ship id from `Game.shipExt()` ([id, pigmented, hats] triples). */
export function hatCounts(ext: ArrayLike<number> | undefined): Map<number, number> {
  const out = new Map<number, number>();
  if (!ext) return out;
  for (let i = 0; i + 2 < ext.length; i += 3) out.set(ext[i] ?? 0, ext[i + 2] ?? 0);
  return out;
}

/** Seconds left in the mood's current phase (0 when none). */
export function phaseSecsLeft(mood: MoodState, nowTick: number): number {
  return mood.phaseEndTick > 0 ? Math.max(0, (mood.phaseEndTick - nowTick) / 60) : 0;
}

/** The sound engine as the spectacles use it (audio/engine.ts AudioEngine; null = silent). */
export interface SpectacleSoundEngine {
  play(id: string, o?: { position?: Vec3 | null; gain?: number; rate?: number; delay?: number }): boolean;
  loop(id: string, o?: { position?: Vec3 | null; gain?: number; rate?: number }): SpectacleLoop | null;
}

export interface SpectacleLoop {
  readonly playing: boolean;
  setGain(gain: number, ramp?: number): void;
  setPosition(p: Vec3): void;
  stop(fade?: number): void;
}

/** A looping sound that follows a wanted gain (0 = stopped) and a position. */
export class LoopVoice {
  private handle: SpectacleLoop | null = null;
  private readonly id: string;
  constructor(id: string) {
    this.id = id;
  }

  update(engine: SpectacleSoundEngine | null, gain: number, position: Vec3 | null): void {
    if (!engine || gain <= 0.001) {
      this.stop(0.6);
      return;
    }
    // Started at the wanted gain: the engine skips a positional sound that starts
    // (nearly) silent, and `setGain` is relative to the sound's own mix gain.
    if (!this.handle || !this.handle.playing) this.handle = engine.loop(this.id, { position, gain });
    this.handle?.setGain(gain, 0.25);
    if (position) this.handle?.setPosition(position);
  }

  stop(fade = 0.3): void {
    this.handle?.stop(fade);
    this.handle = null;
  }
}
