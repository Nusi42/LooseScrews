// Cinematic camera rig and the synthetic dogfight that flies ahead of it
// (first-impression pass, FI). Both follow a FlightPath (flightPath.ts):
//
// - CinematicCamera glides along the path at a calm speed (easing in from
//   rest), looks a few units ahead with a damped turn and banks into curves
//   like a pilot would.
// - Squadron: four ships (one per hull type, two teams) weave through the
//   tube ahead of the camera, so the view chases their engines. They fire
//   bolts in bursts at the enemy ahead of them (misses spark on the rock and
//   light the cave), now and then a rocket that blows the target up; the
//   destroyed ship comes back a few seconds later far ahead and falls back
//   into the formation. Ships, shots and hit cues are the game's own types, so
//   the normal scene builder, effect library (VQ5) and lights (VQ6) draw them.
//
// Deterministic for a given RNG and step sequence (the visual gallery steps
// it at a fixed rate); cosmetic only, nothing reaches the sim.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { Cue } from "../game/mechanics.ts";
import { CUE_HIT } from "../game/vfx.ts";
import type { ProjectileState, ShipState, VisualDef } from "../game/views.ts";
import type { FlightPath, LevelProbe } from "./flightPath.ts";

const WORLD_UP: Vec3 = { x: 0, y: 1, z: 0 };

/** Normalised linear interpolation of two orientations (short way round). */
export function nlerp(a: Quat, b: Quat, t: number): Quat {
  const d = a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w;
  const s = d < 0 ? -1 : 1;
  return q.normalize({
    x: a.x + (b.x * s - a.x) * t,
    y: a.y + (b.y * s - a.y) * t,
    z: a.z + (b.z * s - a.z) * t,
    w: a.w + (b.w * s - a.w) * t,
  });
}

/** Path frame at a sample: right and up perpendicular to the tangent, up as close to world up as the tangent allows. */
export function pathFrame(tangent: Vec3, upHint: Vec3 = WORLD_UP): { right: Vec3; up: Vec3 } {
  let right = v3.cross(tangent, upHint);
  if (v3.length(right) < 1e-3) right = v3.cross(tangent, { x: 1, y: 0, z: 0 });
  right = v3.normalize(right);
  return { right, up: v3.cross(right, tangent) };
}

export interface CinematicOptions {
  /** Cruise speed (u/s). */
  speed: number;
  /** Seconds to reach cruise speed from rest. */
  easeIn: number;
  /** Look-ahead along the path (u). */
  lookAhead: number;
  /** Turn damping rate (1/s): higher follows the path more tightly. */
  turnRate: number;
  /** Largest bank angle (rad). */
  maxBank: number;
  /**
   * Look this far left of the flight (rad; negative: right), so what lies
   * ahead sits right of the screen centre, clear of a panel docked left.
   */
  frameYaw: number;
}

export const DEFAULT_CINEMATIC: CinematicOptions = { speed: 8.5, easeIn: 2.5, lookAhead: 9, turnRate: 2.2, maxBank: 0.45, frameYaw: 0 };

/** The camera that flies the path. */
export class CinematicCamera {
  readonly path: FlightPath;
  readonly options: CinematicOptions;
  /** Arc length travelled (u). */
  s = 0;
  private elapsed = 0;
  private orientation: Quat | null = null;
  private bank = 0;
  private prevForward: Vec3 | null = null;

  /** `start`: orientation to turn from (default: straight along the path). */
  constructor(path: FlightPath, options: CinematicOptions = DEFAULT_CINEMATIC, start: Quat | null = null) {
    this.path = path;
    this.options = options;
    this.orientation = start;
  }

  /** Current speed (u/s), eased in from rest. */
  get speed(): number {
    const k = Math.min(1, this.elapsed / Math.max(1e-3, this.options.easeIn));
    return this.options.speed * (k * k * (3 - 2 * k));
  }

  /** Advances `dt` seconds (0 holds still) and returns the pose. */
  update(dt: number): { position: Vec3; orientation: Quat } {
    const o = this.options;
    this.elapsed += dt;
    this.s += this.speed * dt;
    this.path.ensure(this.s + o.lookAhead + 40);
    this.path.forget(this.s - 10);
    const here = this.path.sample(this.s);
    const ahead = this.path.sample(this.s + o.lookAhead);
    let forward = v3.sub(ahead.position, here.position);
    forward = v3.length(forward) > 1e-3 ? v3.normalize(forward) : here.tangent;
    // Bank into the turn: the yaw rate about the up axis.
    if (this.prevForward && dt > 0) {
      const turn = v3.dot(v3.cross(this.prevForward, forward), WORLD_UP) / dt;
      const target = Math.max(-o.maxBank, Math.min(o.maxBank, -turn * 0.9));
      this.bank += (target - this.bank) * Math.min(1, dt * 1.5);
    }
    this.prevForward = forward;
    const { right, up } = pathFrame(forward);
    const upHint = v3.add(v3.scale(up, Math.cos(this.bank)), v3.scale(right, Math.sin(this.bank)));
    const target = q.lookRotation(forward, upHint);
    this.orientation = this.orientation && dt > 0 ? nlerp(this.orientation, target, 1 - Math.exp(-o.turnRate * dt)) : target;
    const framed = o.frameYaw !== 0 ? q.mul(this.orientation, q.fromAxisAngle(WORLD_UP, o.frameYaw)) : this.orientation;
    return { position: here.position, orientation: framed };
  }
}

/** One synthetic pilot of the squadron. */
interface Pilot {
  /** Ship id; a new one per life (the effect library would play a respawn burst for the same id). */
  id: number;
  /** First id of this pilot (ids step by LIFE_ID_STEP). */
  baseId: number;
  team: number;
  shipType: number;
  radius: number;
  /** Rest distance ahead of the camera along the path (u). */
  ahead: number;
  /** Current distance ahead (eases back to `ahead` after a respawn). */
  slot: number;
  phase: number;
  alive: boolean;
  /** Time of death (respawn after RESPAWN_SECS). */
  diedAt: number;
  boosting: boolean;
  boostUntil: number;
  /** Next shot time, shots left in the burst. */
  nextShot: number;
  burst: number;
  /** Visual id of its bolts. */
  visual: number;
  hits: number;
  position: Vec3;
  orientation: Quat;
  /** Sideways and vertical weave amplitude (u), following the room around the path. */
  swingX: number;
  swingY: number;
  swingTargetX: number;
  swingTargetY: number;
  nextProbe: number;
}

interface Shot {
  owner: number;
  target: number;
  visual: number;
  position: Vec3;
  velocity: Vec3;
  /** Time it vanishes (hits rock or runs out). */
  until: number;
  rocket: boolean;
}

/** Ship types (Skitter, Tinker, Hauler, Tug) and hitbox radii (data/ships). */
const RADII = [0.4, 0.5, 0.65, 0.55];
/** Pilot setup: team and rest distance ahead of the camera, front to back. */
const LINEUP: ReadonlyArray<{ shipType: number; team: number; ahead: number }> = [
  { shipType: 3, team: 0, ahead: 19 },
  { shipType: 1, team: 1, ahead: 14 },
  { shipType: 0, team: 0, ahead: 10 },
  { shipType: 2, team: 1, ahead: 7.5 },
];
export const RESPAWN_SECS = 4.5;
/** Ship id step per life. */
const LIFE_ID_STEP = 16;
/** Seconds between room probes per pilot. */
const PROBE_SECS = 0.35;
/** Weave amplitude limits (u). */
const SWING_MIN = 0.5;
const SWING_MAX = 4.5;
const BOLT_SPEED = 80;
const ROCKET_SPEED = 34;
/** Hits that destroy a ship (besides a rocket). */
const HITS_TO_KILL = 9;

export interface ActionFrame {
  ships: ShipState[];
  shots: ProjectileState[];
  cues: Cue[];
}

/** The dogfight ahead of the camera. */
export class Squadron {
  private readonly path: FlightPath;
  private readonly probe: LevelProbe;
  private readonly rng: () => number;
  private readonly pilots: Pilot[];
  private shots: Shot[] = [];
  private readonly rocketVisual: number;
  private nextRocket: number;
  private time = 0;
  /** Kills so far (tests). */
  kills = 0;
  shotsFired = 0;

  constructor(path: FlightPath, probe: LevelProbe, visuals: readonly VisualDef[], rng: () => number) {
    this.path = path;
    this.probe = probe;
    this.rng = rng;
    const bolts = visuals.flatMap((v, i) => (["bolt", "orb", "spike", "blob"].includes(v.shape) ? [i] : []));
    this.rocketVisual = visuals.findIndex((v) => v.shape === "rocket");
    this.pilots = LINEUP.map((l, i) => ({
      id: 100 + i,
      baseId: 100 + i,
      team: l.team,
      shipType: l.shipType,
      radius: RADII[l.shipType] ?? 0.5,
      ahead: l.ahead,
      slot: l.ahead,
      phase: rng() * 6.28,
      alive: true,
      diedAt: -1e9,
      boosting: false,
      boostUntil: 0,
      nextShot: 1 + rng() * 2,
      burst: 0,
      visual: bolts.length > 0 ? bolts[(i * 3 + 1) % bolts.length]! : -1,
      hits: 0,
      position: { x: 0, y: 0, z: 0 },
      orientation: q.identity(),
      swingX: SWING_MIN,
      swingY: SWING_MIN,
      swingTargetX: SWING_MIN,
      swingTargetY: SWING_MIN,
      nextProbe: 0,
    }));
    this.nextRocket = 6 + rng() * 3;
  }

  /** Where a pilot flies at time `t` with the camera at arc length `camS`. */
  private placeAt(p: Pilot, camS: number, t: number): Vec3 {
    const s = camS + p.slot + 2.2 * Math.sin(t * 0.21 + p.phase);
    const at = this.path.sample(s);
    const { right, up } = pathFrame(at.tangent);
    // Near the camera the weave narrows, so the ship stays in the view cone.
    const cone = 0.4 * Math.max(2, p.slot);
    const x = Math.min(p.swingX, cone) * Math.sin(t * 0.47 + p.phase * 1.3);
    const y = Math.min(p.swingY, cone * 0.6) * Math.sin(t * 0.33 + p.phase * 0.7);
    return v3.add(at.position, v3.add(v3.scale(right, x), v3.scale(up, y)));
  }

  /** Advances to time `t` (s since the start), the camera at arc length `camS` moving at `camSpeed`. */
  update(t: number, dt: number, camS: number, camPos: Vec3): ActionFrame {
    this.time = t;
    const rng = this.rng;
    const cues: Cue[] = [];
    this.path.ensure(camS + 40);
    for (const p of this.pilots) {
      if (!p.alive && t - p.diedAt >= RESPAWN_SECS) {
        // Back far ahead (never through the camera); it falls back into its slot.
        p.alive = true;
        p.id += LIFE_ID_STEP;
        if (p.id >= p.baseId + LIFE_ID_STEP * 64) p.id = p.baseId;
        p.hits = 0;
        p.slot = p.ahead + 22;
        p.boosting = false;
        p.boostUntil = t + 3;
      }
      p.slot += (p.ahead - p.slot) * Math.min(1, dt * (p.slot < p.ahead ? 0.55 : 0.3));
      if (t > p.boostUntil) {
        p.boosting = false;
        if (rng() < dt * 0.12) {
          p.boosting = true;
          p.boostUntil = t + 1 + rng() * 1.5;
        }
      }
      if (!p.alive) continue;
      this.swing(p, camS, t, dt);
      const now = this.placeAt(p, camS, t);
      const soon = this.placeAt(p, camS + 0.3 * 8.5, t + 0.3);
      const before = this.placeAt(p, camS - 0.3 * 8.5, t - 0.3);
      let forward = v3.sub(soon, now);
      forward = v3.length(forward) > 1e-4 ? v3.normalize(forward) : this.path.sample(camS + p.slot).tangent;
      // Bank with the sideways acceleration.
      const acc = v3.sub(v3.add(soon, before), v3.scale(now, 2));
      const { right, up } = pathFrame(forward);
      const bank = Math.max(-0.9, Math.min(0.9, v3.dot(acc, right) * 1.4));
      p.position = now;
      p.orientation = q.lookRotation(forward, v3.add(v3.scale(up, Math.cos(bank)), v3.scale(right, Math.sin(bank))));
    }
    this.fire(t);
    this.moveShots(t, dt, cues, camPos);
    const ships: ShipState[] = this.pilots.map((p) => ({
      id: p.id,
      team: p.team,
      alive: p.alive,
      boosting: p.boosting,
      position: p.position,
      orientation: p.orientation,
      shipType: p.shipType,
      radius: p.radius,
      gadgetActive: false,
    }));
    const shots: ProjectileState[] = this.shots.map((s) => ({ kind: 0, owner: s.owner, position: s.position, velocity: s.velocity, visual: s.visual, state: 0 }));
    return { ships, shots, cues };
  }

  /** Follows the room around the pilot's spot on the path: wide weaves in the open, tight ones in tunnels. */
  private swing(p: Pilot, camS: number, t: number, dt: number): void {
    if (t >= p.nextProbe) {
      p.nextProbe = t + PROBE_SECS;
      const at = this.path.sample(camS + p.slot);
      const { right, up } = pathFrame(at.tangent);
      const reach = SWING_MAX + 3;
      const free = (d: Vec3): number => this.probe.ray(at.position, d, reach);
      const margin = p.radius + 1.2;
      const fit = (a: number, b: number): number => Math.max(SWING_MIN, Math.min(SWING_MAX, Math.min(a, b) - margin));
      p.swingTargetX = fit(free(right), free(v3.scale(right, -1)));
      p.swingTargetY = fit(free(up), free(v3.scale(up, -1))) * 0.6;
    }
    // Shrink fast (a wall comes), widen slowly.
    const ease = (cur: number, target: number): number => cur + (target - cur) * Math.min(1, dt * (target < cur ? 3 : 0.5));
    p.swingX = ease(p.swingX, p.swingTargetX);
    p.swingY = ease(p.swingY, p.swingTargetY);
  }

  /** The nearest living enemy ahead of `p` (or null). */
  private targetOf(p: Pilot): Pilot | null {
    let best: Pilot | null = null;
    for (const o of this.pilots) {
      if (o.team === p.team || !o.alive || o.slot <= p.slot) continue;
      if (!best || o.slot < best.slot) best = o;
    }
    return best;
  }

  private fire(t: number): void {
    const rng = this.rng;
    for (const p of this.pilots) {
      if (!p.alive || p.visual < 0 || t < p.nextShot || p.slot < 2) continue;
      const target = this.targetOf(p);
      const fwd = q.forward(p.orientation);
      // Lead the target a little; spread so most bolts miss and hit the rock.
      let dir = fwd;
      if (target) {
        const to = v3.sub(v3.addScaled(target.position, q.forward(target.orientation), 2), p.position);
        const aim = v3.normalize(to);
        if (v3.dot(aim, fwd) > 0.6) dir = aim;
      }
      const spread = 0.035;
      dir = v3.normalize(v3.add(dir, { x: (rng() - 0.5) * spread, y: (rng() - 0.5) * spread, z: (rng() - 0.5) * spread }));
      const muzzle = v3.addScaled(p.position, fwd, p.radius * 1.3);
      const reach = this.probe.ray(muzzle, dir, BOLT_SPEED * 1.5);
      this.shots.push({ owner: p.id, target: target?.id ?? -1, visual: p.visual, position: muzzle, velocity: v3.scale(dir, BOLT_SPEED), until: t + reach / BOLT_SPEED, rocket: false });
      this.shotsFired++;
      if (p.burst > 0) {
        p.burst--;
        p.nextShot = t + 0.11 + rng() * 0.05;
      } else {
        p.burst = 2 + Math.floor(rng() * 4);
        p.nextShot = t + 0.9 + rng() * 1.6;
      }
    }
    if (t >= this.nextRocket && this.rocketVisual >= 0) {
      this.nextRocket = t + 7 + rng() * 5;
      // A pursuer with a target fires a rocket that seeks it.
      const shooters = this.pilots.filter((p) => p.alive && p.slot > 2 && this.targetOf(p));
      const p = shooters[Math.floor(rng() * shooters.length) % Math.max(1, shooters.length)];
      const target = p ? this.targetOf(p) : null;
      if (p && target) {
        const fwd = q.forward(p.orientation);
        this.shots.push({
          owner: p.id,
          target: target.id,
          visual: this.rocketVisual,
          position: v3.addScaled(p.position, fwd, p.radius * 1.5),
          velocity: v3.scale(fwd, ROCKET_SPEED),
          until: t + 6,
          rocket: true,
        });
        this.shotsFired++;
      }
    }
  }

  private moveShots(t: number, dt: number, cues: Cue[], camPos: Vec3): void {
    const keep: Shot[] = [];
    for (const s of this.shots) {
      const target = s.target >= 0 ? this.pilots.find((p) => p.id === s.target && p.alive) : undefined;
      if (s.rocket && target) {
        // Seek: turn the velocity toward the target.
        const to = v3.normalize(v3.sub(target.position, s.position));
        const v = v3.normalize(v3.add(v3.normalize(s.velocity), v3.scale(to, Math.min(1, dt * 4))));
        s.velocity = v3.scale(v, ROCKET_SPEED);
        const step = this.probe.ray(s.position, v, ROCKET_SPEED * dt + 0.3);
        if (step < ROCKET_SPEED * dt + 0.3) s.until = t; // hits rock
      }
      s.position = v3.addScaled(s.position, s.velocity, dt);
      if (target && v3.length(v3.sub(target.position, s.position)) < target.radius + (s.rocket ? 0.6 : 0.35)) {
        cues.push({ kind: CUE_HIT, source: s.owner, target: target.id, position: s.position, value: s.rocket ? 100 : 11, tick: 0 });
        target.hits += s.rocket ? HITS_TO_KILL : 1;
        if (target.hits >= HITS_TO_KILL) this.kill(target, t);
        continue;
      }
      // Gone far behind the camera: drop it quietly (no impact effect needed there).
      if (t >= s.until || v3.length(v3.sub(s.position, camPos)) > 160) continue;
      keep.push(s);
    }
    this.shots = keep;
  }

  private kill(p: Pilot, t: number): void {
    p.alive = false;
    p.diedAt = t;
    this.kills++;
  }

  /** Living pilots (tests). */
  get alive(): number {
    return this.pilots.filter((p) => p.alive).length;
  }

  /** Current time (s). */
  get now(): number {
    return this.time;
  }
}
