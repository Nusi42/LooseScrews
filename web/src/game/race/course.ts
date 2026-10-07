// Belt Up's course for the client (M4 C7, docs/milestone-4-plan.md §3.8):
// the asteroid belt as the HUD and the race visuals read it (ls-wasm
// `Game.beltSite()` JSON), a mirror of the simulation's progress function
// (`ls_sim::modes::belt_up::progress`, checked against a Rust fixture in
// course.test.ts) and the HUD maths built on it: distance to the exit along
// the course, gaps in seconds, the next gate and wrong-way flight. Pure
// functions, no DOM or GPU.

import type { Quat } from "../../math/quat.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { RaceState, Racer } from "../modeState.ts";

/** Wedges (start slots, gates) of the belt. */
export const WEDGES = 8;
/** Angle of one wedge (rad). */
export const WEDGE = Math.PI / 4;
/** Gates in a lap. */
export const LAP_GATES = 8;
/** Simulation rate (ticks per second). */
export const TICK_HZ = 60;

/** One drifting boulder's orbit (`ls_math::level::BoulderOrbit`). */
export interface BoulderOrbit {
  radius: number;
  height: number;
  phase: number;
  angularSpeed: number;
  size: number;
  tumbleAxis: Vec3;
  tumbleSpeed: number;
}

/** The asteroid belt (`ls_math::level::BeltSite`). */
export interface Belt {
  center: Vec3;
  innerRadius: number;
  outerRadius: number;
  halfThickness: number;
  cocoonRadius: number;
  exitRadius: number;
  gateInnerRadius: number;
  gateOuterRadius: number;
  slots: { pos: Vec3; rot: Quat }[];
  pickupPoints: Vec3[];
  boulders: BoulderOrbit[];
}

function num(v: unknown, fallback = 0): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

function vec(v: unknown): Vec3 {
  const a = Array.isArray(v) ? v : [];
  return { x: num(a[0]), y: num(a[1]), z: num(a[2]) };
}

function quat(v: unknown): Quat {
  const a = Array.isArray(v) ? v : [];
  return { x: num(a[0]), y: num(a[1]), z: num(a[2]), w: num(a[3], 1) };
}

function list(v: unknown): unknown[] {
  return Array.isArray(v) ? v : [];
}

function obj(v: unknown): Record<string, unknown> {
  return v !== null && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
}

/** Parses ls-wasm `Game.beltSite()` (null on classic levels or broken JSON). */
export function parseBelt(text: string | undefined | null): Belt | null {
  if (!text) return null;
  let j: unknown;
  try {
    j = JSON.parse(text);
  } catch {
    return null;
  }
  const o = obj(j);
  if (!("center" in o)) return null;
  return {
    center: vec(o["center"]),
    innerRadius: num(o["innerRadius"], 95),
    outerRadius: num(o["outerRadius"], 145),
    halfThickness: num(o["halfThickness"], 12),
    cocoonRadius: num(o["cocoonRadius"], 30),
    exitRadius: num(o["exitRadius"], 6),
    gateInnerRadius: num(o["gateInnerRadius"], 87),
    gateOuterRadius: num(o["gateOuterRadius"], 153),
    slots: list(o["slots"]).map((s) => ({ pos: vec(obj(s)["pos"]), rot: quat(obj(s)["rot"]) })),
    pickupPoints: list(o["pickupPoints"]).map(vec),
    boulders: list(o["boulders"]).map((x) => {
      const b = obj(x);
      return {
        radius: num(b["radius"]),
        height: num(b["height"]),
        phase: num(b["phase"]),
        angularSpeed: num(b["angularSpeed"]),
        size: num(b["size"], 2),
        tumbleAxis: vec(b["tumbleAxis"]),
        tumbleSpeed: num(b["tumbleSpeed"]),
      };
    }),
  };
}

/**
 * A belt from ls-wasm `levelBelt(code)` floats (centre, radii, slots; no
 * boulders or pickups): the gallery and previews without a match.
 */
export function beltFromLevel(f: ArrayLike<number>): Belt | null {
  if (f.length < 34) return null;
  const at = (i: number): number => f[i] ?? 0;
  const center = { x: at(0), y: at(1), z: at(2) };
  const slots: Belt["slots"] = [];
  for (let k = 0; k < WEDGES; k++) {
    const pos = { x: at(10 + 3 * k), y: at(11 + 3 * k), z: at(12 + 3 * k) };
    // Facing prograde: yaw about +Y by the slot's azimuth (lookRotation of the tangent).
    const a = azimuthAbout(center, pos);
    slots.push({ pos, rot: { x: 0, y: Math.sin(a / 2), z: 0, w: Math.cos(a / 2) } });
  }
  return {
    center,
    innerRadius: at(3),
    outerRadius: at(4),
    halfThickness: at(5),
    cocoonRadius: at(6),
    exitRadius: at(7),
    gateInnerRadius: at(8),
    gateOuterRadius: at(9),
    slots,
    pickupPoints: [],
    boulders: [],
  };
}

/** `ls_math::det::wrap_angle`: `a` wrapped to [-pi, pi). */
export function wrapAngle(a: number): number {
  const r = a - 2 * Math.PI * Math.floor((a + Math.PI) / (2 * Math.PI));
  return r >= Math.PI ? r - 2 * Math.PI : r;
}

function azimuthAbout(c: Vec3, p: Vec3): number {
  const a = Math.atan2(-(p.z - c.z), p.x - c.x);
  return a < 0 ? a + 2 * Math.PI : a;
}

/** Azimuth of `p` about the belt axis (rad, 0..2 pi, prograde positive; `BeltSite::azimuth`). */
export function azimuth(b: Belt, p: Vec3): number {
  return azimuthAbout(b.center, p);
}

/** Distance of `p` from the belt axis (u). */
export function axisDistance(b: Belt, p: Vec3): number {
  return Math.hypot(p.x - b.center.x, p.z - b.center.z);
}

/** Azimuth of gate `k` (rad). */
export function gateAzimuth(k: number): number {
  return (((k % WEDGES) + WEDGES) % WEDGES) * WEDGE;
}

/** A point of the ring at azimuth `a`, axis distance `r` and height `h` above the belt plane. */
export function ringPoint(b: Belt, a: number, r: number, h = 0): Vec3 {
  return { x: b.center.x + r * Math.cos(a), y: b.center.y + h, z: b.center.z - r * Math.sin(a) };
}

/** Unit prograde direction (counter-clockwise seen from above) at azimuth `a`. */
export function prograde(a: number): Vec3 {
  return { x: -Math.sin(a), y: 0, z: -Math.cos(a) };
}

/** Racer fields the progress needs. */
export type RacerProgress = Pick<Racer, "gates" | "slot">;

/**
 * `ls_sim::modes::belt_up::progress`: gates passed, then a position key in
 * the current sector (the angle past the last gate in wedges, -1..2), or
 * once the lap is done, minus the distance to the exit centre.
 */
export function raceProgress(b: Belt, r: RacerProgress, pos: Vec3): [number, number] {
  if (r.gates >= LAP_GATES) {
    return [r.gates, -Math.hypot(pos.x - b.center.x, pos.y - b.center.y, pos.z - b.center.z)];
  }
  const last = (r.slot + r.gates) % WEDGES;
  const a = wrapAngle(azimuth(b, pos) - gateAzimuth(last));
  return [r.gates, Math.min(2, Math.max(-1, a / WEDGE))];
}

/** Gate a racer passes next (`slot + gates + 1`), -1 once the lap is done. */
export function nextGate(r: RacerProgress): number {
  return r.gates >= LAP_GATES ? -1 : (r.slot + r.gates + 1) % WEDGES;
}

/**
 * Distance (u) left to the exit along the course: the rest of the lap as an
 * arc along the middle of the belt, then the dive from there to the exit
 * sphere; after the lap the straight distance to the exit sphere. Measured
 * on the middle line (not the racer's own line), so it falls with the
 * simulation's progress (gates, then the angle into the sector) and the
 * HUD's gaps agree with the places.
 */
export function courseLeft(b: Belt, r: RacerProgress, pos: Vec3): number {
  const [gates, key] = raceProgress(b, r, pos);
  if (gates >= LAP_GATES) return Math.max(0, -key - b.exitRadius);
  const mid = 0.5 * (b.innerRadius + b.outerRadius);
  const wedges = Math.max(0, LAP_GATES - gates - Math.min(1, Math.max(0, key)));
  return wedges * WEDGE * mid + Math.max(0, mid - b.exitRadius);
}

/** Length of the whole course (u) along the middle of the belt. */
export function courseLength(b: Belt): number {
  const mid = 0.5 * (b.innerRadius + b.outerRadius);
  return LAP_GATES * WEDGE * mid + mid - b.exitRadius;
}

/** Share of the course done (0..1), for the ring minimap's progress arc. */
export function courseDone(b: Belt, r: RacerProgress, pos: Vec3): number {
  const total = courseLength(b);
  return total > 0 ? Math.min(1, Math.max(0, 1 - courseLeft(b, r, pos) / total)) : 0;
}

/** Tick the current heat's lights went out (the client remembers it; see RaceClock). */
export class RaceClock {
  private heat = -1;
  private start = 0;

  /**
   * Start tick of the current heat's race: the lights phase's end tick,
   * remembered once the race runs; joined mid-race, estimated from the
   * heat limit (`phaseEndTick - limitSecs`).
   */
  update(race: RaceState | null, limitSecs = 180): number {
    if (!race) {
      this.heat = -1;
      return 0;
    }
    if (race.heat !== this.heat) {
      this.heat = race.heat;
      this.start = 0;
    }
    if (race.phase === "lights") this.start = race.phaseEndTick;
    else if (this.start === 0 && race.phase === "racing") this.start = Math.max(1, race.phaseEndTick - limitSecs * TICK_HZ);
    return this.start;
  }
}

/** One racer's standing for the HUD. */
export interface Standing {
  id: number;
  place: number;
  /** Course left (u); 0 once finished. */
  left: number;
  finishTick: number;
}

/**
 * Seconds between the own racer and another one: positive when the other
 * is ahead. Running racers: their course difference over the own average
 * speed so far (at least `minSpeed`); a finished one ahead: the time since
 * it finished plus the own course left at that speed.
 */
export function gapSecs(own: Standing, other: Standing, avgSpeed: number, nowTick: number, minSpeed = 12): number {
  const v = Math.max(minSpeed, avgSpeed);
  if (own.finishTick > 0 && other.finishTick > 0) return (own.finishTick - other.finishTick) / TICK_HZ;
  if (other.finishTick > 0) return Math.max(0, nowTick - other.finishTick) / TICK_HZ + own.left / v;
  if (own.finishTick > 0) return -(other.left / v + Math.max(0, nowTick - own.finishTick) / TICK_HZ);
  return (own.left - other.left) / v;
}

/** Average speed (u/s) along the course since the start: course done over the race time. */
export function averageSpeed(b: Belt, left: number, startTick: number, nowTick: number): number {
  const secs = (nowTick - startTick) / TICK_HZ;
  if (!(startTick > 0) || secs < 1) return 0;
  const lineDone = courseLength(b) - left;
  return Math.max(0, lineDone / secs);
}

/** Watches the own flight direction: wrong way after `holdSecs` of retrograde flight (lap not done). */
export class WrongWay {
  private since = -1;

  /** Forgets the hold (outside a running heat, a new match). */
  reset(): void {
    this.since = -1;
  }

  /** True while the own ship has flown backwards faster than `minSpeed` for `holdSecs`. */
  update(b: Belt, r: RacerProgress | null, pos: Vec3 | null, vel: Vec3 | null, time: number, minSpeed = 3, holdSecs = 0.8): boolean {
    if (!r || !pos || !vel || r.gates >= LAP_GATES) {
      this.since = -1;
      return false;
    }
    const t = prograde(azimuth(b, pos));
    const along = t.x * vel.x + t.z * vel.z;
    if (along < -minSpeed) {
      if (this.since < 0) this.since = time;
      return time - this.since >= holdSecs;
    }
    if (along > 0) this.since = -1;
    return this.since >= 0 && time - this.since >= holdSecs;
  }
}

/** The target the HUD's next-gate arrow points at: the next gate's middle, or the exit after the lap. */
export function nextTarget(b: Belt, r: RacerProgress): Vec3 {
  const g = nextGate(r);
  if (g < 0) return { ...b.center };
  return ringPoint(b, gateAzimuth(g), 0.5 * (b.innerRadius + b.outerRadius));
}

/** Boulder centre at a (fractional) tick, the orbit maths of `BoulderOrbit::pos_at` in f64 (presentation). */
export function boulderAt(b: Belt, o: BoulderOrbit, tick: number): Vec3 {
  const a = wrapAngle(o.phase + (o.angularSpeed * tick) / TICK_HZ);
  return ringPoint(b, a, o.radius, o.height);
}
