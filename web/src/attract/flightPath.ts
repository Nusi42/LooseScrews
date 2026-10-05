// Cinematic camera path through a generated level (first-impression pass,
// FI): the start screen and the lobby fly a camera along it behind the UI.
//
// The path is planned on the fly from probes against the level (ls-wasm
// `raycast` and `sweepSphere`, the same SDF the sim uses): from the last
// waypoint, candidate directions in a cone around the current heading are
// swept with a sphere of `clearance` (the tube the camera and the ships stay
// inside); the planner scores free distance, the view beyond (open sky and
// big caverns win), keeping the heading (no zig-zag), a level horizon and
// not flying back to where it has been, steps there and nudges the new
// waypoint toward the middle of the passage. A centripetal Catmull-Rom
// spline through the waypoints, sampled by arc length, gives the smooth
// flight; waypoints are added as the camera needs them, so the flight never
// ends. Pure logic over a probe interface: unit tested with an analytic
// level (flightPath.test.ts).

import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";

/** Level probes (ls-wasm Game `raycast` / `sweepSphere`, wrapped). */
export interface LevelProbe {
  /** Distance to the first rock (or the level bounds) along unit `d`; `max` or more when clear. */
  ray(o: Vec3, d: Vec3, max: number): number;
  /** Travel of a sphere of `radius` along unit `d` until it touches rock; `max` when clear. */
  sweep(o: Vec3, d: Vec3, radius: number, max: number): number;
}

export interface PathOptions {
  /** Radius of the tube kept free of rock (u). */
  clearance: number;
  /** Shortest and longest step between waypoints (u). */
  stepMin: number;
  stepMax: number;
  /** Half-angle of the cone of candidate directions around the heading (rad). */
  cone: number;
  /** Candidate directions per waypoint. */
  candidates: number;
  /** How far the view beyond counts (u). */
  vista: number;
  /** A direction the flight leans toward (the planet or the sun, for the view), and how strongly. */
  toward?: Vec3;
  towardWeight?: number;
}

export const DEFAULT_PATH: PathOptions = {
  clearance: 2.2,
  stepMin: 6,
  stepMax: 20,
  cone: 0.75,
  candidates: 30,
  vista: 90,
};

/** Wraps an ls-wasm Game's probes (-1 / empty mean "clear"). */
export function wasmProbe(game: {
  raycast?(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, max: number): number;
  sweepSphere?(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, radius: number, max: number): Float32Array;
}): LevelProbe | null {
  const ray = game.raycast?.bind(game);
  const sweep = game.sweepSphere?.bind(game);
  if (!ray || !sweep) return null;
  return {
    ray: (o, d, max) => {
      const t = ray(o.x, o.y, o.z, d.x, d.y, d.z, max);
      return t < 0 ? max : t;
    },
    sweep: (o, d, radius, max) => {
      const r = sweep(o.x, o.y, o.z, d.x, d.y, d.z, radius, max);
      return r.length > 0 ? Math.max(0, r[0] ?? 0) : max;
    },
  };
}

/** A direction within `angle` of unit `axis`, uniform over the cap (`u`, `v` in [0, 1)). */
export function coneDirection(axis: Vec3, angle: number, u: number, v: number): Vec3 {
  const cosA = 1 - u * (1 - Math.cos(angle));
  const sinA = Math.sqrt(Math.max(0, 1 - cosA * cosA));
  const phi = v * Math.PI * 2;
  const helper = Math.abs(axis.y) < 0.9 ? { x: 0, y: 1, z: 0 } : { x: 1, y: 0, z: 0 };
  const a = v3.normalize(v3.cross(axis, helper));
  const b = v3.cross(axis, a);
  return v3.normalize(v3.add(v3.scale(axis, cosA), v3.add(v3.scale(a, sinA * Math.cos(phi)), v3.scale(b, sinA * Math.sin(phi)))));
}

/** Evenly spread unit directions (Fibonacci sphere). */
export function sphereDirs(n: number): Vec3[] {
  const out: Vec3[] = [];
  const golden = Math.PI * (3 - Math.sqrt(5));
  for (let i = 0; i < n; i++) {
    const y = 1 - (2 * (i + 0.5)) / n;
    const r = Math.sqrt(1 - y * y);
    out.push({ x: Math.cos(golden * i) * r, y, z: Math.sin(golden * i) * r });
  }
  return out;
}

const CENTER_DIRS = sphereDirs(14);
/** Samples per spline segment in the arc-length table. */
const SEGMENT_SAMPLES = 24;

/** Centripetal Catmull-Rom point between p1 and p2 at t in [0, 1]. */
export function catmullRom(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: number): Vec3 {
  const knot = (a: Vec3, b: Vec3): number => Math.max(1e-3, Math.sqrt(v3.length(v3.sub(b, a))));
  const t0 = 0;
  const t1 = t0 + knot(p0, p1);
  const t2 = t1 + knot(p1, p2);
  const t3 = t2 + knot(p2, p3);
  const u = t1 + (t2 - t1) * t;
  const lerp = (a: Vec3, b: Vec3, ta: number, tb: number): Vec3 => v3.add(v3.scale(a, (tb - u) / (tb - ta)), v3.scale(b, (u - ta) / (tb - ta)));
  const a1 = lerp(p0, p1, t0, t1);
  const a2 = lerp(p1, p2, t1, t2);
  const a3 = lerp(p2, p3, t2, t3);
  const b1 = lerp(a1, a2, t0, t2);
  const b2 = lerp(a2, a3, t1, t3);
  return lerp(b1, b2, t1, t2);
}

export interface PathSample {
  position: Vec3;
  /** Unit direction of travel. */
  tangent: Vec3;
}

/**
 * An endless flight path. `sample(s)` returns the point `s` u along it;
 * call `ensure(s)` first (it plans waypoints until the path is known that
 * far). Old waypoints are dropped by `forget(s)` so memory stays bounded.
 */
export class FlightPath {
  readonly options: PathOptions;
  private readonly probe: LevelProbe;
  private readonly rng: () => number;
  /** Waypoints; `points[0]` sits at arc length `base` of the whole flight. */
  private points: Vec3[] = [];
  /** Arc length table per segment k (points[k] -> points[k+1]): cumulative lengths of SEGMENT_SAMPLES + 1 samples. */
  private tables: Float64Array[] = [];
  /** Arc length at points[0]. */
  private base = 0;
  /** Recently visited waypoints (revisit penalty). */
  private readonly visited: Vec3[] = [];
  /** Waypoints planned (tests, debug). */
  planned = 0;
  /** Times the planner had to turn around (no way on within the cone). */
  turnarounds = 0;

  constructor(probe: LevelProbe, start: Vec3, heading: Vec3, rng: () => number, options: PathOptions = DEFAULT_PATH) {
    this.probe = probe;
    this.rng = rng;
    this.options = options;
    const h = v3.normalize(heading);
    // A phantom point behind the start gives the first segment its tangent.
    this.points.push(v3.addScaled(start, h, -options.stepMin), start);
    this.base = -options.stepMin;
    this.tables.push(new Float64Array(0));
    this.visited.push(start);
  }

  /** Arc length up to which `sample` is valid without planning more. */
  get known(): number {
    // The spline between points[k] and points[k+1] needs points[k+2].
    let s = this.base;
    for (let k = 0; k + 2 < this.points.length; k++) s += this.segmentLength(k);
    return s;
  }

  /** Waypoints currently held (tests). */
  get waypoints(): readonly Vec3[] {
    return this.points;
  }

  /** Plans waypoints until the path is known to arc length `s`. */
  ensure(s: number): void {
    for (let guard = 0; this.known < s && guard < 64; guard++) this.plan();
  }

  /** Drops waypoints wholly behind arc length `s` (keeps the two needed for the spline there). */
  forget(s: number): void {
    while (this.points.length > 6) {
      const first = this.segmentLength(0);
      const second = this.segmentLength(1);
      if (this.base + first + second >= s - 1) break;
      this.base += first;
      this.points.shift();
      this.tables.shift();
    }
  }

  /** Point and direction `s` u along the flight (clamped to what is known). */
  sample(s: number): PathSample {
    const position = this.at(s);
    const ahead = this.at(s + 0.5);
    const behind = this.at(s - 0.5);
    const d = v3.sub(ahead, behind);
    const len = v3.length(d);
    return { position, tangent: len > 1e-6 ? v3.scale(d, 1 / len) : this.heading() };
  }

  private at(s: number): Vec3 {
    let rest = s - this.base;
    const n = this.points.length;
    if (rest <= 0 || n < 3) return this.points[Math.min(1, n - 1)] ?? { x: 0, y: 0, z: 0 };
    for (let k = 0; k + 2 < n; k++) {
      const table = this.table(k);
      const len = table[SEGMENT_SAMPLES] ?? 0;
      if (rest <= len || k + 3 >= n) {
        rest = Math.min(rest, len);
        // Binary search the sample, then interpolate the parameter.
        let lo = 0;
        let hi = SEGMENT_SAMPLES;
        while (hi - lo > 1) {
          const mid = (lo + hi) >> 1;
          if ((table[mid] ?? 0) <= rest) lo = mid;
          else hi = mid;
        }
        const a = table[lo] ?? 0;
        const b = table[hi] ?? a;
        const t = (lo + (b > a ? (rest - a) / (b - a) : 0)) / SEGMENT_SAMPLES;
        return this.curve(k, t);
      }
      rest -= len;
    }
    return this.points[n - 2]!;
  }

  private curve(k: number, t: number): Vec3 {
    const p = this.points;
    const p1 = p[k]!;
    const p2 = p[k + 1]!;
    const p0 = p[k - 1] ?? v3.sub(v3.scale(p1, 2), p2);
    const p3 = p[k + 2] ?? v3.sub(v3.scale(p2, 2), p1);
    return catmullRom(p0, p1, p2, p3, t);
  }

  private segmentLength(k: number): number {
    return this.table(k)[SEGMENT_SAMPLES] ?? 0;
  }

  private table(k: number): Float64Array {
    let t = this.tables[k];
    if (!t || t.length === 0) {
      t = new Float64Array(SEGMENT_SAMPLES + 1);
      let prev = this.curve(k, 0);
      for (let i = 1; i <= SEGMENT_SAMPLES; i++) {
        const p = this.curve(k, i / SEGMENT_SAMPLES);
        t[i] = (t[i - 1] ?? 0) + v3.length(v3.sub(p, prev));
        prev = p;
      }
      // A segment's shape depends on the point after it: only cache once that exists.
      if (k + 2 < this.points.length) this.tables[k] = t;
    }
    return t;
  }

  private heading(): Vec3 {
    const n = this.points.length;
    const d = v3.sub(this.points[n - 1]!, this.points[n - 2]!);
    const len = v3.length(d);
    return len > 1e-6 ? v3.scale(d, 1 / len) : { x: 0, y: 0, z: -1 };
  }

  /** Adds one waypoint. */
  private plan(): void {
    const o = this.options;
    const from = this.points[this.points.length - 1]!;
    const heading = this.heading();
    let best = this.bestStep(from, heading, o.cone, o.clearance);
    if (!best) {
      // Dead end: look all around (turn around), then accept a tighter tube.
      this.turnarounds++;
      best = this.bestStep(from, heading, Math.PI, o.clearance) ?? this.bestStep(from, heading, Math.PI, o.clearance * 0.5);
    }
    let next: Vec3;
    if (best) {
      next = this.centre(v3.addScaled(from, best.dir, best.step));
    } else {
      // Boxed in (should not happen in a playable level): drift up slowly.
      next = v3.addScaled(from, { x: 0, y: 1, z: 0 }, o.stepMin);
    }
    this.points.push(next);
    this.tables.push(new Float64Array(0));
    // The previous segment's table changes with the new point.
    if (this.tables.length >= 3) this.tables[this.tables.length - 3] = new Float64Array(0);
    this.visited.push(next);
    if (this.visited.length > 48) this.visited.shift();
    this.planned++;
  }

  private bestStep(from: Vec3, heading: Vec3, cone: number, clearance: number): { dir: Vec3; step: number } | null {
    const o = this.options;
    let best: { dir: Vec3; step: number; score: number } | null = null;
    for (let i = 0; i < o.candidates; i++) {
      const dir = i === 0 && cone < Math.PI ? heading : coneDirection(heading, cone, this.rng(), this.rng());
      const free = this.probe.sweep(from, dir, clearance, o.stepMax + 8);
      // Stop well before the wall ahead (room to turn).
      const step = Math.min(o.stepMax, free - 4);
      if (step < o.stepMin) continue;
      const far = Math.min(o.vista, this.probe.ray(from, dir, o.vista));
      const end = v3.addScaled(from, dir, step);
      let revisit = 0;
      for (const p of this.visited) {
        const d = v3.length(v3.sub(p, end));
        if (d < 14) revisit = Math.max(revisit, 1 - d / 14);
      }
      const score =
        step / o.stepMax +
        0.8 * (far / o.vista) +
        0.6 * v3.dot(dir, heading) -
        0.7 * Math.abs(dir.y) -
        1.2 * revisit +
        (o.toward ? (o.towardWeight ?? 0.4) * v3.dot(dir, o.toward) : 0) +
        0.25 * this.rng();
      if (!best || score > best.score) best = { dir, step, score };
    }
    return best;
  }

  /** Moves a waypoint toward the middle of the passage (away from rock within 9 u), if the way there is clear. */
  private centre(p: Vec3): Vec3 {
    const reach = 9;
    let push = { x: 0, y: 0, z: 0 };
    for (const d of CENTER_DIRS) {
      const t = this.probe.ray(p, d, reach);
      if (t < reach) push = v3.addScaled(push, d, -(reach - t) / reach);
    }
    const len = v3.length(push);
    if (len < 1e-3) return p;
    const dir = v3.scale(push, 1 / len);
    const want = Math.min(3, len * 1.2);
    const free = this.probe.sweep(p, dir, this.options.clearance * 0.5, want + 1);
    return v3.addScaled(p, dir, Math.max(0, Math.min(want, free - 1)));
  }
}
