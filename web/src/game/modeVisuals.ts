// Mode visuals (M3 C1) on the new pipeline, driven by the replicated mode
// state (game/modeState.ts) like the mode HUD:
//
// - The Squeeze: the shell (port of ls_sim's `ShellSchedule`, quantized like
//   the terrain's) glows on the rock while the walls warn and grinds amber
//   while they close (renderer/modeFx.ts WALL), sparks and dust come off the
//   moving wall near the camera and where a ship scrapes it, lights tint the
//   rock next to it; the next zone is a holographic sphere with a dashed
//   contour on the rock.
// - Heist: the core is a glowing crystal (models/modeProps.ts) with a gold
//   beacon; embedded, a contour on the rock marks where to dig; carried, it
//   floats over its carrier, who gets a gold halo and a trail of motes;
//   extraction zones are team-coloured spheres with a beacon and rising
//   motes; a delivery bursts.
// - Hot Reactor: the reactor floats over its holder (red halo pulsing faster
//   as the fuse burns), the fuse radius is a red sphere whose equator ring
//   shows the fuse left; a pass flashes, a detonation blows up.
// - Last Color Standing: the world is graded monochrome (post.ts grade) and
//   ships keep the share of their colour their meter has left (the keep mask
//   in the object shader).
//
// Pure functions build volumes, lights, props and ship styles; `ModeVfx`
// keeps the little state particles and events need.

import * as q from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { PointLight } from "../renderer/lights.ts";
import type { ModeVolume } from "../renderer/modeFx.ts";
import type { ColorGrade } from "../renderer/post.ts";
import type { Instance, Rgb, SceneInstances } from "../renderer/renderer.ts";
import { SPRITE } from "../renderer/spriteAtlas.ts";
import { OBJECTIVE, PLAYER_FLAG, VIEW_FLAG, type ModeState, type Objective } from "./modeState.ts";
import { sideColor } from "./sides.ts";
import type { LightEmitter } from "./vfx.ts";
import {
  add,
  dist,
  DUST,
  explosion,
  glint,
  glowNow,
  lin,
  madd,
  mul,
  norm,
  randCone,
  randDir,
  range,
  ring,
  sparkleBurst,
  smokeSprite,
  sparks,
  sub,
  type Rng,
  type VfxSink,
} from "./vfxLibrary.ts";
import { TINKER_RADIUS, type ShipState } from "./views.ts";

// ----- The Squeeze ------------------------------------------------------------------

/** ls_sim::modes::the_squeeze::SHELL_STEP: the wall moves in steps of this much surface travel (u). */
export const SHELL_STEP = 0.5;
/** ls_sim::modes::the_squeeze::ZONE_WALLS: the safe zone's `state` bit once walls exist. */
export const ZONE_WALLS = 1;

export interface Zone {
  center: Vec3;
  radius: number;
}

/** Whole units, like `the_squeeze::snap` (the marker positions are quantized). */
export function snap(p: Vec3): Vec3 {
  return { x: Math.floor(p.x + 0.5), y: Math.floor(p.y + 0.5), z: Math.floor(p.z + 0.5) };
}

/** `the_squeeze::close_step`: the zone between `from` and `to` at `now`, in SHELL_STEP steps. */
export function closeStep(from: Zone, to: Zone, start: number, end: number, now: number): Zone {
  if (now >= end || end <= start) return to;
  if (now <= start) return from;
  const travel = Math.hypot(from.center.x - to.center.x, from.center.y - to.center.y, from.center.z - to.center.z) + Math.max(0, from.radius - to.radius);
  const steps = Math.max(1, Math.ceil(travel / SHELL_STEP));
  const k = Math.floor(((now - start) * steps) / (end - start));
  const f = k / steps;
  return {
    center: {
      x: from.center.x + (to.center.x - from.center.x) * f,
      y: from.center.y + (to.center.y - from.center.y) * f,
      z: from.center.z + (to.center.z - from.center.z) * f,
    },
    radius: from.radius + (to.radius - from.radius) * f,
  };
}

export interface SqueezeView {
  current: Zone;
  /** Walls exist at the current zone. */
  walls: boolean;
  next: Zone | null;
  /** Ticks the next close starts and ends (0 without a next zone). */
  start: number;
  end: number;
  /** The shell at the tick (null before the first close). */
  shell: Zone | null;
  /** The next zone is announced and the walls have not started moving. */
  warning: boolean;
  /** The walls are moving. */
  closing: boolean;
  /** 0..1 of the running close (0 when not closing). */
  progress: number;
}

/** The Squeeze's walls at `tick` from the SAFE_ZONE / NEXT_ZONE markers; null for other modes. */
export function squeezeView(m: ModeState, tick: number): SqueezeView | null {
  if (m.mode !== "the_squeeze") return null;
  const safe = m.objectives.find((o) => o.kind === OBJECTIVE.safeZone);
  if (!safe) return null;
  const nextObj = m.objectives.find((o) => o.kind === OBJECTIVE.nextZone);
  const current: Zone = { center: snap(safe.pos), radius: safe.radius };
  const walls = (safe.state & ZONE_WALLS) !== 0;
  const start = safe.timerEnd;
  const end = nextObj && nextObj.timerEnd > start ? nextObj.timerEnd : 0;
  const next: Zone | null = nextObj && end > 0 ? { center: snap(nextObj.pos), radius: nextObj.radius } : null;
  let shell: Zone | null = walls ? current : null;
  let closing = false;
  let progress = 0;
  if (next && tick >= start) {
    shell = closeStep(current, next, start, end, tick);
    closing = tick < end;
    progress = Math.min(1, (tick - start) / Math.max(1, end - start));
  }
  return { current, walls, next, start, end: end, shell, warning: next !== null && tick < start, closing, progress };
}

// ----- Heist and Hot Reactor states -------------------------------------------------

/** ls_sim::modes::heist::core_state. */
export const CORE_STATE = { embedded: 0, free: 1, carried: 2, dropped: 3, respawning: 4 } as const;
/** ls_sim::modes::hot_reactor STATE_*. */
export const REACTOR_STATE = { waiting: 0, held: 1 } as const;

// ----- colours (display values; HDR glow through intensities) -----------------------

export const WALL_COLOR: Rgb = [0.35, 0.85, 1.0];
export const NEXT_ZONE_COLOR: Rgb = [1.0, 0.82, 0.3];
export const CORE_COLOR: Rgb = [1.0, 0.78, 0.25];
export const REACTOR_COLOR: Rgb = [1.0, 0.3, 0.1];
/** Extraction zones in free-for-all (any carrier delivers there). */
export const FFA_EXTRACTION: Rgb = [0.4, 1.0, 0.55];

// ----- inputs -------------------------------------------------------------------------

export interface ModeSceneInput {
  mode: ModeState;
  /** Server tick estimate (`Game.nowTick()`). */
  nowTick: number;
  /** Seconds (animation). */
  time: number;
  /** Own player id and ship position (null when dead or spectating). */
  own: number;
  ownPos: Vec3 | null;
  /** Remote ships (the own ship is not in the list online). */
  ships: readonly ShipState[];
  camera: Vec3;
  /** Warm-up or countdown: the warm-up room plays Paint the Void; nothing shows. */
  warmup?: boolean;
  /** Seconds of a full reactor fuse (the rule set's; default 20). */
  fuseSecs?: number;
}

const UP: Vec3 = { x: 0, y: 1, z: 0 };

/** Rendered position and radius of a ship id (the own one from its pose). */
export function shipAt(i: ModeSceneInput, id: number): { pos: Vec3; radius: number } | null {
  if (id < 0) return null;
  if (id === i.own) return i.ownPos ? { pos: i.ownPos, radius: ownRadius(i) } : null;
  const s = i.ships.find((x) => x.id === id && x.alive);
  return s ? { pos: s.position, radius: s.radius || TINKER_RADIUS } : null;
}

function ownRadius(i: ModeSceneInput): number {
  return i.ships.find((s) => s.id === i.own)?.radius || TINKER_RADIUS;
}

/** Seconds from now to a tick (0 when none or past). */
function secsTo(tick: number, now: number): number {
  return tick > 0 ? Math.max(0, (tick - now) / 60) : 0;
}

/** Where an objective is drawn: on its holder when held (the marker lags the rendered ship). */
export function objectivePos(i: ModeSceneInput, o: Objective): Vec3 {
  if (o.holder >= 0) {
    const s = shipAt(i, o.holder);
    if (s) return s.pos;
  }
  return o.pos;
}

/** Extraction zone colour: its team's, or green in free-for-all. */
export function extractionColor(o: Objective, teams: number): Rgb {
  return teams > 0 && o.team >= 0 ? sideColor(o.team, teams) : FFA_EXTRACTION;
}

/** The fuse left as a fraction of `fuseSecs` (1 = fresh). */
export function fuseFraction(o: Objective, nowTick: number, fuseSecs = 20): number {
  return Math.max(0, Math.min(1, secsTo(o.timerEnd, nowTick) / Math.max(1, fuseSecs)));
}

/** Pulse rate (Hz) of the reactor and its holder halo by the seconds left. */
export function reactorPulseHz(secsLeft: number): number {
  return secsLeft < 3 ? 5 : secsLeft < 8 ? 2.6 : 1.2;
}

// ----- volumes (renderer/modeFx.ts) -------------------------------------------------

/** This frame's mode effect volumes, most important first (the pass draws at most 12). */
export function modeVolumes(i: ModeSceneInput): ModeVolume[] {
  const m = i.mode;
  const out: ModeVolume[] = [];
  if (i.warmup || !m.running) return out;
  const sq = squeezeView(m, i.nowTick);
  if (sq) {
    if (sq.shell) {
      const pulse = sq.warning ? 1 : 0;
      out.push({
        kind: "wall",
        center: sq.shell.center,
        radius: sq.shell.radius,
        color: WALL_COLOR,
        intensity: sq.closing ? 1.4 : sq.warning ? 1.4 : 0.6,
        warn: pulse,
        grind: sq.closing ? 1 : 0,
      });
    }
    if (sq.next) {
      out.push({
        kind: "zone",
        center: sq.next.center,
        radius: sq.next.radius,
        color: NEXT_ZONE_COLOR,
        intensity: 1.5,
        film: 0.08,
        grid: 0.25,
        contour: 1.6,
        contourWidth: 0.35,
        pulse: sq.closing ? 0 : 0.8,
        dashSpeed: 2,
      });
    }
  }
  for (const o of m.objectives) {
    if (o.kind === OBJECTIVE.core) {
      if (o.state === CORE_STATE.respawning) continue;
      const at = objectivePos(i, o);
      if (o.state === CORE_STATE.embedded) {
        // Dig here: the knot's outline on the rock, and the beacon from the vein.
        out.push({ kind: "zone", center: at, radius: 2.8, color: CORE_COLOR, intensity: 1.3, film: 0, grid: 0, contour: 1.4, contourWidth: 0.12, pulse: 0.7, dashSpeed: 1.5 });
        out.push({ kind: "beam", center: at, width: 0.3, height: 70, color: CORE_COLOR, intensity: 1.4, core: 0.8 });
      } else if (o.state === CORE_STATE.carried) {
        const s = shipAt(i, o.holder);
        const r = s?.radius ?? TINKER_RADIUS;
        out.push({ kind: "halo", center: at, radius: r * 2.6, color: CORE_COLOR, intensity: 1.2, power: 5 });
        out.push({ kind: "beam", center: at, width: 0.22, height: 45, color: CORE_COLOR, intensity: 1.1, core: 0.8 });
      } else {
        out.push({ kind: "beam", center: at, width: 0.35, height: 70, color: CORE_COLOR, intensity: 1.8, core: 1 });
        out.push({ kind: "halo", center: at, radius: 1.6, color: CORE_COLOR, intensity: 1.0, power: 5 });
      }
    } else if (o.kind === OBJECTIVE.extraction) {
      const c = extractionColor(o, m.teams);
      const r = o.radius > 0 ? o.radius : 6;
      out.push({ kind: "zone", center: o.pos, radius: r, color: c, intensity: 1.3, film: 0.22, grid: 0.35, contour: 1.2, contourWidth: 0.3, dashSpeed: 1 });
      out.push({ kind: "beam", center: o.pos, width: 0.7, height: 60, color: c, intensity: 0.9, core: 0.6 });
    } else if (o.kind === OBJECTIVE.reactor && o.state === REACTOR_STATE.held) {
      const at = objectivePos(i, o);
      const left = secsTo(o.timerEnd, i.nowTick);
      const hz = reactorPulseHz(left);
      const s = shipAt(i, o.holder);
      const r = s?.radius ?? TINKER_RADIUS;
      out.push({ kind: "halo", center: at, radius: r * 2.4, color: REACTOR_COLOR, intensity: 1.0 + 0.8 * (0.5 + 0.5 * Math.sin(i.time * Math.PI * 2 * hz)), power: 5 });
      if (o.radius > 0) {
        out.push({
          kind: "zone",
          center: at,
          radius: o.radius,
          color: REACTOR_COLOR,
          intensity: 1.3,
          film: 0.1,
          grid: 0.12,
          contour: 1.5,
          contourWidth: 0.3,
          ring: fuseFraction(o, i.nowTick, i.fuseSecs),
          pulse: hz,
          dashSpeed: 3,
        });
      }
    }
  }
  return out;
}

// ----- props (instanced meshes) ------------------------------------------------------

/** Adds the core and reactor props to `scene`. */
export function addModeProps(scene: SceneInstances, i: ModeSceneInput): void {
  const m = i.mode;
  if (i.warmup || !m.running) return;
  const t = i.time;
  for (const o of m.objectives) {
    if (o.kind === OBJECTIVE.core && o.state !== CORE_STATE.respawning) {
      const carried = o.state === CORE_STATE.carried;
      const base = objectivePos(i, o);
      const lift = carried ? 1.15 : 0;
      const bob = Math.sin(t * 2.1) * (carried ? 0.08 : 0.15);
      const position = { x: base.x, y: base.y + lift + bob, z: base.z };
      const glow = 0.85 + 0.15 * Math.sin(t * 3);
      const inst: Instance = {
        position,
        rotation: q.mul(q.fromAxisAngle(UP, t * 0.9), q.fromAxisAngle({ x: 1, y: 0, z: 0 }, 0.25 * Math.sin(t * 0.7))),
        scale: carried ? 0.9 : 1.6,
        // HDR display colour: the crystal glows (PART_GLOW), the cage stays metal.
        color: [2.0 * glow, 1.55 * glow, 0.55 * glow],
        emissive: 0,
      };
      (scene.opaque.core ??= []).push(inst);
    } else if (o.kind === OBJECTIVE.reactor && o.state === REACTOR_STATE.held) {
      const base = objectivePos(i, o);
      const left = secsTo(o.timerEnd, i.nowTick);
      const p = 0.5 + 0.5 * Math.sin(t * Math.PI * 2 * reactorPulseHz(left));
      const k = 1.4 + 1.4 * p;
      (scene.opaque.reactor ??= []).push({
        position: { x: base.x, y: base.y + 1.25 + Math.sin(t * 2.6) * 0.06, z: base.z },
        rotation: q.mul(q.fromAxisAngle(UP, t * 1.6), q.fromAxisAngle({ x: 0, y: 0, z: 1 }, t * 0.7)),
        scale: 1.15,
        color: [k, k * 0.32, k * 0.12],
        emissive: 0,
      });
    }
  }
}

// ----- lights (VQ6) ---------------------------------------------------------------

/** Lights of the mode visuals beyond `lightFx.objectiveLight` (the moving wall near the camera, zones' beacons). */
export function modeLights(i: ModeSceneInput, out: PointLight[] = []): PointLight[] {
  const m = i.mode;
  if (i.warmup || !m.running) return out;
  const sq = squeezeView(m, i.nowTick);
  if (sq?.shell && (sq.warning || sq.closing)) {
    // The wall nearest the camera and nearest the own ship.
    const points = [i.camera, ...(i.ownPos ? [i.ownPos] : [])];
    for (const p of points) {
      const d = sub(p, sq.shell.center);
      const len = Math.hypot(d.x, d.y, d.z);
      if (len < 1e-3) continue;
      const gap = sq.shell.radius - len;
      if (gap > 30 || gap < -2) continue;
      const at = madd(sq.shell.center, d, (sq.shell.radius - 1.2) / len);
      const flicker = sq.closing ? 0.8 + 0.2 * Math.sin(i.time * 23 + p.x) : 0.75 + 0.25 * Math.sin(i.time * Math.PI * 2 * 0.8);
      out.push({ position: at, color: sq.closing ? [1, 0.45, 0.12] : [0.4, 0.85, 1], intensity: (sq.closing ? 16 : 7) * flicker, range: 22, priority: 2 });
    }
  }
  return out;
}

// ----- Last Color Standing ----------------------------------------------------------

/** Ship colour and keep share in Last Color Standing: the meter left of the side colour. */
export function colourMeterStyle(base: Rgb, meter: number): { color: Rgb; keepColor: number } {
  const k = Math.max(0, Math.min(1, meter / 100));
  const l = 0.2126 * base[0] + 0.7152 * base[1] + 0.0722 * base[2];
  // A drained ship fades toward the grey of the world; a little colour stays visible to the end.
  const s = k > 0 ? 0.15 + 0.85 * k : 0;
  return { color: [l + (base[0] - l) * s, l + (base[1] - l) * s, l + (base[2] - l) * s], keepColor: k > 0 ? 1 : 0 };
}

/** Whether the mode shows the world in monochrome (Last Color Standing). */
export function isMonochrome(m: ModeState): boolean {
  return (m.flags & VIEW_FLAG.monochrome) !== 0;
}

/**
 * The colour grade on top of the world's: monochrome blended in by `amount`
 * (0..1, eased by the caller) with a slightly cold, contrasty look.
 */
export function monochromeGrade(base: ColorGrade, amount: number): ColorGrade {
  const a = Math.max(0, Math.min(1, amount));
  if (a <= 0) return base;
  const mixc = (x: Rgb, y: Rgb): Rgb => [x[0] + (y[0] - x[0]) * a, x[1] + (y[1] - x[1]) * a, x[2] + (y[2] - x[2]) * a];
  return {
    ...base,
    slope: mixc(base.slope, [0.85, 0.88, 0.94]),
    power: mixc(base.power, [1.15, 1.15, 1.15]),
    monochrome: a,
    monoKeep: 0,
  };
}

/** Eases the monochrome in and out (per frame, `dt` s; about 1.2 s to full). */
export function easeMonochrome(current: number, target: number, dt: number): number {
  const step = Math.max(0, dt) / 1.2;
  return target > current ? Math.min(target, current + step) : Math.max(target, current - step);
}

/** Ship style for the scene builder in Last Color Standing (null elsewhere). */
export function shipStyleFor(m: ModeState, teams: number): ((s: ShipState) => { color: Rgb; keepColor: number } | undefined) | null {
  if (!isMonochrome(m)) return null;
  const meters = new Map(m.players.map((p) => [p.id, p]));
  return (s) => {
    const p = meters.get(s.id);
    if (!p || (p.flags & PLAYER_FLAG.spectator) !== 0) return undefined;
    return colourMeterStyle(sideColor(s.team, teams), p.meter);
  };
}

// ----- particles and events ---------------------------------------------------------

/** What the particle side of the mode visuals reads each frame. */
export interface ModeVfxFrame extends ModeSceneInput {
  dt: number;
}

interface HeldReactor {
  holder: number;
  pos: Vec3;
  timerEnd: number;
}

/**
 * Particles and one-shot events of the mode visuals: grinding walls, core
 * sparkles and carrier trail, extraction motes, reactor embers, passes,
 * detonations and deliveries. Call `update` once per frame after
 * `Vfx.update` (which clears the immediate sprites).
 */
export class ModeVfx {
  private readonly sink: VfxSink;
  private readonly rng: Rng;
  /** Dynamic-light pulses of events (VQ6 `LightFx`); null = none. */
  lightSink: LightEmitter | null = null;
  private carry = new Map<string, number>();
  private reactor: HeldReactor | null = null;
  private coreState = -1;
  private corePos: Vec3 | null = null;
  /** Events so far (tests, debug). */
  readonly counts = { grind: 0, scrape: 0, passes: 0, detonations: 0, deliveries: 0 };

  constructor(sink: VfxSink, rng: Rng = Math.random) {
    this.sink = sink;
    this.rng = rng;
  }

  /** Forgets the tracked state (a new match). */
  reset(): void {
    this.carry.clear();
    this.reactor = null;
    this.coreState = -1;
    this.corePos = null;
  }

  /** Whole particles for `rate` per second this frame (fractions carry over), scaled by the sink's density. */
  private emitCount(key: string, rate: number, dt: number): number {
    const c = (this.carry.get(key) ?? 0) + rate * dt * this.sink.density;
    const n = Math.floor(c);
    this.carry.set(key, c - n);
    return n;
  }

  update(f: ModeVfxFrame): void {
    const m = f.mode;
    if (f.warmup || !m.running) {
      this.reactor = null;
      this.coreState = -1;
      return;
    }
    this.squeeze(f);
    for (const o of m.objectives) {
      if (o.kind === OBJECTIVE.core) this.core(f, o);
      else if (o.kind === OBJECTIVE.extraction) this.extraction(f, o);
    }
    this.hotReactor(f);
  }

  private squeeze(f: ModeVfxFrame): void {
    const sq = squeezeView(f.mode, f.nowTick);
    if (!sq?.shell || !(sq.warning || sq.closing)) return;
    const sink = this.sink;
    const rng = this.rng;
    const { center, radius } = sq.shell;
    const toCam = norm(sub(f.camera, center), { x: 0, y: 1, z: 0 });
    // Grinding: sparks and dust off the wall around the camera's nearest wall point.
    const n = this.emitCount("grind", sq.closing ? 70 : 10, f.dt);
    for (let k = 0; k < n; k++) {
      const d = norm(randCone(rng, toCam, 0.9));
      const at = madd(center, d, radius - 0.15);
      if (dist(at, f.camera) > 55) continue;
      const inward = mul(d, -1);
      this.counts.grind++;
      if (sq.closing) {
        if (rng() < 0.55) sparks(sink, rng, at, inward, [1, 0.55, 0.2], 3, 0.9, 0.8);
        else {
          sink.spawn({
            pos: at,
            vel: madd(mul(inward, range(rng, 0.6, 2.2)), randDir(rng), 0.4),
            life: range(rng, 1.2, 2.4),
            drag: 0.8,
            sprite: smokeSprite(rng),
            blend: "alpha",
            color: DUST,
            color1: DUST,
            alpha: 0.5,
            alpha1: 0,
            size: range(rng, 0.5, 1.1),
            size1: range(rng, 1.6, 2.6),
            rot: rng() * 6.28,
            spin: range(rng, -0.6, 0.6),
            fadeIn: 0.15,
          });
        }
      } else {
        glint(sink, rng, at, WALL_COLOR, range(rng, 0.25, 0.5), 5, mul(inward, 0.4));
      }
    }
    if (!sq.closing) return;
    // Ships scraping the moving wall.
    const ships: Array<{ id: number; pos: Vec3; radius: number }> = f.ships.filter((s) => s.alive).map((s) => ({ id: s.id, pos: s.position, radius: s.radius || TINKER_RADIUS }));
    if (f.ownPos) ships.push({ id: f.own, pos: f.ownPos, radius: TINKER_RADIUS });
    for (const s of ships) {
      const d = sub(s.pos, center);
      const len = Math.hypot(d.x, d.y, d.z);
      if (len < 1e-3 || dist(s.pos, f.camera) > 120) continue;
      const gap = radius - len - s.radius;
      if (gap > 1.0) continue;
      const out = mul(d, 1 / len);
      const contact = madd(center, out, radius - 0.05);
      const k = this.emitCount(`scrape${s.id}`, 90, f.dt);
      for (let j = 0; j < k; j++) sparks(sink, rng, contact, mul(out, -1), [1, 0.6, 0.25], 2, 1.4, 1.3);
      if (k > 0) this.counts.scrape++;
      glowNow(sink, contact, [1, 0.5, 0.15], 2.2, 2.5);
    }
  }

  private core(f: ModeVfxFrame, o: Objective): void {
    const sink = this.sink;
    const rng = this.rng;
    const at = objectivePos(f, o);
    const prev = this.coreState;
    this.coreState = o.state;
    // A delivery: carried -> respawning, at the carrier's last position.
    if (prev === CORE_STATE.carried && o.state === CORE_STATE.respawning && this.corePos) {
      this.counts.deliveries++;
      const p = this.corePos;
      ring(sink, p, CORE_COLOR, 6, 1, 14, 0.7);
      ring(sink, p, [1, 1, 1], 4, 0.5, 7, 0.4);
      sparkleBurst(sink, rng, p, CORE_COLOR, 60, 9);
      this.lightSink?.emit("bigExplosion", p, sink.now, { color: [1, 0.85, 0.4] });
    }
    // Taken: a flash where the core was.
    if ((prev === CORE_STATE.free || prev === CORE_STATE.dropped) && o.state === CORE_STATE.carried && this.corePos) {
      ring(sink, this.corePos, CORE_COLOR, 5, 0.6, 5, 0.45);
      sparkleBurst(sink, rng, this.corePos, CORE_COLOR, 24, 5, true);
      this.lightSink?.emit("pickup", this.corePos, sink.now, { color: [1, 0.85, 0.4], scale: 2 });
    }
    // Mined free: a burst at the vein.
    if (prev === CORE_STATE.embedded && o.state === CORE_STATE.free) {
      ring(sink, at, CORE_COLOR, 5, 0.8, 9, 0.6);
      sparkleBurst(sink, rng, at, CORE_COLOR, 40, 7);
    }
    this.corePos = o.state === CORE_STATE.carried ? { x: at.x, y: at.y + 1.15, z: at.z } : at;
    if (o.state === CORE_STATE.respawning || dist(at, f.camera) > 150) return;
    if (o.state === CORE_STATE.carried) {
      const p = this.corePos;
      glowNow(sink, p, CORE_COLOR, 1.1, 1.3);
      const n = this.emitCount("carrier", 26, f.dt);
      for (let k = 0; k < n; k++) glint(sink, rng, madd(p, randDir(rng), 0.5), CORE_COLOR, range(rng, 0.12, 0.26), 6, mul(randDir(rng), 0.3));
    } else if (o.state !== CORE_STATE.embedded) {
      glowNow(sink, at, CORE_COLOR, 3.2, 2.0);
      const n = this.emitCount("core", 14, f.dt);
      for (let k = 0; k < n; k++) glint(sink, rng, madd(at, randDir(rng), range(rng, 0.6, 1.6)), CORE_COLOR, range(rng, 0.15, 0.3), 6, { x: 0, y: range(rng, 0.4, 1.2), z: 0 });
    }
  }

  private extraction(f: ModeVfxFrame, o: Objective): void {
    const r = o.radius > 0 ? o.radius : 6;
    if (dist(o.pos, f.camera) > r + 90) return;
    const c = extractionColor(o, f.mode.teams);
    const n = this.emitCount(`extract${o.id}`, 24, f.dt);
    for (let k = 0; k < n; k++) {
      const d = randDir(this.rng);
      const at = madd(o.pos, d, r * Math.cbrt(this.rng()) * 0.9);
      this.sink.spawn({
        pos: at,
        vel: { x: 0, y: range(this.rng, 0.8, 2.0), z: 0 },
        life: range(this.rng, 1.2, 2.2),
        sprite: SPRITE.glow,
        color: lin(c, 2.5),
        color1: [0, 0, 0],
        size: range(this.rng, 0.12, 0.22),
        size1: 0.04,
        fadeIn: 0.3,
      });
    }
  }

  private hotReactor(f: ModeVfxFrame): void {
    const sink = this.sink;
    const rng = this.rng;
    const o = f.mode.objectives.find((x) => x.kind === OBJECTIVE.reactor);
    const held = o && o.state === REACTOR_STATE.held && o.holder >= 0 ? o : null;
    const prev = this.reactor;
    if (held) {
      const at = objectivePos(f, held);
      const top = { x: at.x, y: at.y + 1.1, z: at.z };
      if (prev && prev.holder !== held.holder) {
        // Passed: a red arc of sparks from the old holder to the new one.
        this.counts.passes++;
        const from = prev.pos;
        const steps = 14;
        for (let k = 0; k <= steps; k++) {
          const p = add(mul(from, 1 - k / steps), mul(top, k / steps));
          glint(sink, rng, p, REACTOR_COLOR, 0.35, 7);
        }
        ring(sink, top, REACTOR_COLOR, 6, 0.5, 4.5, 0.4);
        sparkleBurst(sink, rng, top, [1, 0.5, 0.2], 26, 6, true);
        this.lightSink?.emit("muzzle", top, sink.now, { color: [1, 0.4, 0.15], scale: 2 });
      }
      this.reactor = { holder: held.holder, pos: top, timerEnd: held.timerEnd };
      if (dist(top, f.camera) < 140) {
        const left = secsTo(held.timerEnd, f.nowTick);
        const p = 0.5 + 0.5 * Math.sin(f.time * Math.PI * 2 * reactorPulseHz(left));
        glowNow(sink, top, REACTOR_COLOR, 2.4 + 1.2 * p, 1.6 + 1.6 * p);
        const n = this.emitCount("embers", left < 5 ? 40 : 16, f.dt);
        for (let k = 0; k < n; k++) {
          sink.spawn({
            pos: madd(top, randDir(rng), 0.3),
            vel: madd({ x: 0, y: range(rng, 0.6, 1.8), z: 0 }, randDir(rng), 1.2),
            life: range(rng, 0.5, 1.1),
            drag: 1.2,
            sprite: SPRITE.spark,
            orient: "velocity",
            stretch: 0.05,
            color: [6, 1.8, 0.4],
            color1: [0.6, 0.06, 0.01],
            size: range(rng, 0.05, 0.09),
            size1: 0.02,
          });
        }
      }
    } else if (prev) {
      // Gone while held: it blew up at the fuse's end (else the holder left).
      if (f.nowTick >= prev.timerEnd - 30) {
        this.counts.detonations++;
        explosion(sink, rng, prev.pos, 3.2, { kind: "plasma", color: [1, 0.35, 0.12] });
        ring(sink, prev.pos, REACTOR_COLOR, 8, 1, 20, 0.6);
        ring(sink, prev.pos, [1, 0.8, 0.5], 6, 0.5, 10, 0.35);
        this.lightSink?.emit("bigExplosion", prev.pos, sink.now, { color: [1, 0.35, 0.12], scale: 1.4 });
      }
      this.reactor = null;
    }
  }
}
