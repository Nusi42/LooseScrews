// World visuals of M4 (B5; docs/milestone-4-plan.md §3.3, §3.4): what the
// sliders put into the level, drawn on the client.
//
// - Gravity wells (Gravity Chaos, the Paperweight, Architect strokes,
//   shepherd moons): the world effects pass bends the background around
//   each well, lights a violet glow and the core stone's veins
//   (renderer/wells.ts); an accretion swirl of glowing motes, rock chips and
//   haze circles the core (immediate particles); a violet point light. The
//   own ship's pull feeds the HUD's pull indicator (ui/worldHud.ts). The
//   force law mirrors ls_sim::gravity (data/world/gravity.ron). During the
//   Gravity coffee break event the wells dim and stop swirling.
// - Cave-ins (Instability): the warning cue (`CueKind::CaveInWarning`) opens
//   glowing cracks in the ceiling, trickles dust and pebbles and shakes the
//   view; when it runs out the rock comes down in a dust burst and the
//   rubble keeps a dust coat (pale while fresh, a light coat for good).
// - Pigment (monochrome levels, Palette 80+): pigment pickups are three
//   drops of paint in the primaries circling a glowing core, a death's
//   splash is one big drop in the side's colour; pigmented ships fly in
//   their side's colour through the ink and drip paint, grey ships are ink
//   grey (teammates keep a tint of their team colour). Everything coloured
//   keeps its colour through the ink (Instance.keepColor).
//
// Pure maths (force law, pull, cave-in timeline, ship styles) is exported
// for the unit tests; `WorldFx` holds the per-match state.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { PointLight } from "../renderer/lights.ts";
import type { Rgb, SceneInstances } from "../renderer/renderer.ts";
import { SPRITE } from "../renderer/spriteAtlas.ts";
import type { WorldVolume } from "../renderer/wells.ts";
import type { Cue } from "./mechanics.ts";
import type { MoodState } from "./modeState.ts";
import { moodGrade, moodMotes, MoodFx, type DroneEngine, type MoodIconModel } from "./moodFx.ts";
import type { ColorGrade } from "../renderer/post.ts";
import { teamHull } from "./scene.ts";
import { lin, type VfxSink } from "./vfxLibrary.ts";
import type { EntityState, ShipState } from "./views.ts";
import { WELL_STRIDE } from "../wasm/api.ts";

/** `ls_sim::CueKind` values this module reacts to. */
export const WORLD_CUE = { caveInWarning: 14, pigment: 15 } as const;

/** data/world/gravity.ron: softening length (u) and the ship clamp (share of per-axis thrust). */
export const WELL_SOFTENING = 3;
export const SHIP_PULL_CLAMP = 0.4;
/** `max_r r / (r^2 + 1)^1.5` (ls_sim::gravity PEAK_FACTOR). */
const PEAK_FACTOR = 0.38490018;

/** Per-axis thrust acceleration (u/s^2) by ship type (data/ships/*.ron `thrust_accel`; Skitter, Tinker, Hauler, Tug). */
export const SHIP_THRUST_ACCEL: readonly number[] = [30, 30, 24, 30];

/** Violet of the wells (display colour). */
export const WELL_COLOR: Rgb = [0.66, 0.4, 1.0];
/** Glow of a cave-in's cracks (display colour). */
export const CRACK_COLOR: Rgb = [1.0, 0.5, 0.16];

/** Pigment primaries (display colours) of a pigment pickup's three drops. */
export const PIGMENT_PRIMARIES: readonly Rgb[] = [
  [0.95, 0.14, 0.12],
  [1.0, 0.78, 0.08],
  [0.1, 0.36, 1.0],
];
/** Ink grey of an unpigmented ship in a monochrome level. */
export const INK_GREY: Rgb = [0.44, 0.45, 0.47];
/** Share of the team colour a grey teammate keeps (their tint through the ink). */
export const TEAMMATE_KEEP = 0.6;

/** One well of ls-wasm `Game.wells()`. */
export interface WellInfo {
  position: Vec3;
  range: number;
  peak: number;
  /** Core stone radius (u), 0 for dynamic wells. */
  core: number;
  startTick: number;
  /** -1 = static (always on). */
  endTick: number;
}

export function parseWells(a: ArrayLike<number> | undefined | null): WellInfo[] {
  const out: WellInfo[] = [];
  if (!a) return out;
  for (let i = 0; i + WELL_STRIDE <= a.length; i += WELL_STRIDE) {
    const n = (k: number): number => {
      const x = a[i + k];
      return typeof x === "number" && Number.isFinite(x) ? x : 0;
    };
    out.push({ position: { x: n(0), y: n(1), z: n(2) }, range: n(3), peak: n(4), core: n(5), startTick: n(6), endTick: n(7) });
  }
  return out;
}

/** Whether a well pulls at `tick` (statics always; dynamics from start to end). */
export function wellActive(w: WellInfo, tick: number): boolean {
  return w.endTick < 0 || (w.startTick <= tick && tick < w.endTick);
}

/** Pull (u/s^2) of one well at `p` (ls_sim::gravity::well_accel). */
export function wellPull(w: WellInfo, p: Vec3, s = WELL_SOFTENING): Vec3 {
  if (w.peak <= 0 || w.range <= 0) return { x: 0, y: 0, z: 0 };
  const d = v3.sub(w.position, p);
  const r2 = v3.dot(d, d);
  const range2 = w.range * w.range;
  if (r2 >= range2) return { x: 0, y: 0, z: 0 };
  const s2 = s * s;
  const g = (w.peak * s2) / PEAK_FACTOR;
  const qq = r2 + s2;
  const taper = 1 - r2 / range2;
  return v3.scale(d, (g * taper * taper) / (qq * Math.sqrt(qq)));
}

/** Total pull on a ship of `shipType` at `p` (clamped like ls_sim's `ship_accel`). */
export function shipPull(wells: readonly WellInfo[], p: Vec3, tick: number, shipType = 1): Vec3 {
  let a = { x: 0, y: 0, z: 0 };
  for (const w of wells) if (wellActive(w, tick)) a = v3.add(a, wellPull(w, p));
  const max = SHIP_PULL_CLAMP * (SHIP_THRUST_ACCEL[shipType] ?? 30);
  const l = v3.length(a);
  return l > max ? v3.scale(a, max / l) : a;
}

/** The HUD's pull indicator. */
export interface PullModel {
  /** 0..1: the share of the most a ship can be pulled (SHIP_PULL_CLAMP of its thrust). */
  strength: number;
  /** Pull as a share of the ship's own thrust (0..0.4), for the label. */
  ofThrust: number;
  /** Screen direction of the pull (x right, y up, unit) when it leans sideways; null when it points along the view. */
  screen: { x: number; y: number } | null;
  /** The pull points ahead (true) or behind (false) along the view. */
  ahead: boolean;
}

/** Below this (u/s^2) the indicator hides. */
export const PULL_VISIBLE = 0.25;

export function pullModel(a: Vec3, orientation: Quat, shipType = 1): PullModel | null {
  const l = v3.length(a);
  if (l < PULL_VISIBLE) return null;
  const thrust = SHIP_THRUST_ACCEL[shipType] ?? 30;
  const r = q.right(orientation);
  const u = q.up(orientation);
  const f = q.forward(orientation);
  const x = v3.dot(a, r);
  const y = v3.dot(a, u);
  const side = Math.hypot(x, y);
  return {
    strength: Math.min(1, l / (SHIP_PULL_CLAMP * thrust)),
    ofThrust: l / thrust,
    screen: side > 0.3 * l ? { x: x / side, y: y / side } : null,
    ahead: v3.dot(a, f) >= 0,
  };
}

/** Well dimming of the Gravity coffee break event at `tick`: 1 normal, 0.08 at its quietest (1 s fades). */
export function coffeeDim(mood: MoodState | null, tick: number): number {
  if (!mood || mood.eventId !== "gravity_coffee_break") return 1;
  if (mood.phase === "active") return 1 - 0.92 * Math.min(1, Math.max(0, (tick - mood.activeTick) / 60));
  if (mood.phase === "ending") return 1 - 0.92 * Math.min(1, Math.max(0, (mood.phaseEndTick - tick) / 60));
  return 1;
}

/** How a well looks at full strength (the dim factor scales glow and lens). */
export function wellLook(w: WellInfo, index: number, dim: number): WorldVolume {
  const k = Math.min(1, Math.max(0.3, w.peak / 12));
  const core = w.core;
  const axis = v3.normalize({ x: Math.sin(index * 2.3 + 0.4) * 0.45, y: 1, z: Math.cos(index * 1.7 + 1.1) * 0.45 });
  return {
    kind: "well",
    center: w.position,
    core,
    lens: (core > 0 ? core + 0.55 * k : 1.1 * k) * (0.35 + 0.65 * dim),
    reach: Math.min(w.range * 0.4, core + 7),
    glow: core * 0.6 + 1.0,
    color: WELL_COLOR,
    intensity: 0.85 * dim,
    axis,
    phase: index * 1.37,
  };
}

/** A cave-in in progress: the warning, then the collapse. */
export interface CaveIn {
  /** The spot the warning names (in the tunnel). */
  position: Vec3;
  /** The ceiling above it (first rock along up), where the cracks open. */
  ceiling: Vec3;
  start: number;
  /** Collapse time (s). */
  end: number;
  seed: number;
  collapsed: boolean;
  /** The announcer said it (once per cave-in, when near). */
  said: boolean;
}

/** Fallen rubble: when and where (kept for the match). */
export interface Rubble {
  position: Vec3;
  time: number;
  seed: number;
}

/** Fresh dust share of rubble `age` seconds old (1 at the collapse; the lasting coat is drawn on top). */
export function rubbleFresh(age: number): number {
  return Math.exp(-Math.max(0, age) / 25);
}

/** Screen shake (0..1) of a cave-in at time `t` for a camera `dist` u away. */
export function caveInShake(c: CaveIn, t: number, dist: number): number {
  const near = Math.max(0, 1 - dist / 45);
  if (near <= 0) return 0;
  if (t < c.end) {
    const p = Math.min(1, Math.max(0, (t - c.start) / Math.max(0.01, c.end - c.start)));
    return near * (0.12 + 0.25 * p * p);
  }
  return near * 0.9 * Math.exp(-(t - c.end) / 0.5);
}

/** Pigment state of the ships (ls-wasm `Game.shipExt()`: [id, pigmented, hats] per ship). */
export function pigmentedIds(ext: ArrayLike<number> | undefined | null): Set<number> {
  const out = new Set<number>();
  if (!ext) return out;
  for (let i = 0; i + 3 <= ext.length; i += 3) if ((ext[i + 1] ?? 0) !== 0) out.add(ext[i] ?? -1);
  return out;
}

/**
 * Ship colours in a monochrome level: pigmented ships in their side colour,
 * kept through the ink; grey teammates keep a tint of their team colour;
 * other grey ships are ink grey. Null outside monochrome levels.
 */
export function pigmentStyle(
  monochrome: boolean,
  pigmented: ReadonlySet<number>,
  ownTeam: number,
  teams: number,
): ((s: { id: number; team: number }) => { color: Rgb; keepColor: number }) | null {
  if (!monochrome) return null;
  return (s) => {
    if (pigmented.has(s.id)) return { color: teamHull(s.team, teams), keepColor: 1 };
    if (teams > 0 && s.team === ownTeam) return { color: teamHull(s.team, teams), keepColor: TEAMMATE_KEEP };
    return { color: INK_GREY, keepColor: 0 };
  };
}

/** Small deterministic hash in 0..1. */
export function hash01(a: number, b = 0): number {
  const x = Math.sin(a * 127.1 + b * 311.7 + 74.7) * 43758.5453;
  return x - Math.floor(x);
}

/** A falling drop of paint (pigmented ships, splashes). */
interface Drip {
  pos: Vec3;
  vel: Vec3;
  born: number;
  life: number;
  size: number;
  color: Rgb;
}

/** Everything the world visuals read in one frame. */
export interface WorldFxInput {
  time: number;
  dt: number;
  nowTick: number;
  camera: Vec3;
  /** Camera orientation (pull indicator, sprite facing). */
  orientation: Quat;
  wells: readonly WellInfo[];
  cues: readonly Cue[];
  mood: MoodState | null;
  /** The level's up (Game.up()). */
  up: Vec3;
  /** Own ship (alive, not spectating) and its type, for the pull. */
  own: { position: Vec3; shipType: number } | null;
  ships: readonly ShipState[];
  entities: readonly EntityState[];
  monochrome: boolean;
  pigmented: ReadonlySet<number>;
  teams: number;
  /** First rock along a ray (ls-wasm `Game.raycast`), -1 or null for none. */
  raycast?: ((o: Vec3, d: Vec3, max: number) => number) | undefined;
}

/** What the world visuals hand the renderer and the HUD this frame. */
export interface WorldFxFrame {
  volumes: WorldVolume[];
  lights: PointLight[];
  /** The up to turn the sky to (smoothed). */
  up: Vec3;
  shake: number;
  pull: PullModel | null;
  /** Cave-ins whose warning started this frame (sound, announcer). */
  warnings: CaveIn[];
  /** Cave-ins that came down this frame. */
  collapses: CaveIn[];
  /** Pigment taken (or splashed) this frame: position and display colour. */
  pigmentHits: Array<{ position: Vec3; color: Rgb }>;
}

/** Wells farther than this from the camera get no swirl, light or pass volume (u). */
const WELL_DRAW_RANGE = 150;
/** Wells in the effects pass: the nearest WELL_PASS_MAX within WELL_PASS_RANGE (u); farther ones are a dot of glow (their light and swirl stay). */
export const WELL_PASS_RANGE = 80;
export const WELL_PASS_MAX = 3;
const SWIRL_RANGE = 90;
const RUBBLE_KEEP = 16;

/** The world visuals of one match. */
export class WorldFx {
  private caveIns: CaveIn[] = [];
  private rubble: Rubble[] = [];
  private drips: Drip[] = [];
  private dripClock = new Map<number, number>();
  private lastPos = new Map<number, Vec3>();
  private upNow: Vec3 = { x: 0, y: 1, z: 0 };
  private dimNow = 1;
  /** Particle sink (null: no particles, `?vfx=off`). */
  sink: VfxSink | null = null;
  /** Off: nothing is drawn (`?worldfx=off`, before shots). */
  enabled = true;
  /** Swirl motes per well at "high" (scaled by the sink's density). */
  swirlMotes = 120;

  reset(): void {
    this.caveIns = [];
    this.rubble = [];
    this.drips = [];
    this.dripClock.clear();
    this.lastPos.clear();
    this.upNow = { x: 0, y: 1, z: 0 };
    this.dimNow = 1;
  }

  /** Cave-ins being warned or fallen, for tests and the gallery. */
  get activeCaveIns(): readonly CaveIn[] {
    return this.caveIns;
  }

  get rubbleSites(): readonly Rubble[] {
    return this.rubble;
  }

  /** Adds fallen rubble directly (the gallery's aftermath scenes). */
  addRubble(r: Rubble): void {
    this.rubble.push(r);
    if (this.rubble.length > RUBBLE_KEEP) this.rubble.shift();
  }

  update(input: WorldFxInput): WorldFxFrame {
    const out: WorldFxFrame = { volumes: [], lights: [], up: input.up, shake: 0, pull: null, warnings: [], collapses: [], pigmentHits: [] };
    const t = input.time;
    // The up follows the replicated value smoothly (snapshots arrive at 20-30 Hz).
    const k = 1 - Math.exp(-Math.max(0, input.dt) * 12);
    const tgt = v3.normalize(input.up);
    const jump = v3.dot(this.upNow, tgt) < 0.7;
    this.upNow = jump || input.dt <= 0 ? tgt : v3.normalize(v3.lerp(this.upNow, tgt, k));
    out.up = this.upNow;
    const up = this.upNow;
    this.cues(input, out);
    if (!this.enabled) return out;

    // Wells.
    const dimTarget = coffeeDim(input.mood, input.nowTick);
    this.dimNow += (dimTarget - this.dimNow) * (1 - Math.exp(-Math.max(0, input.dt) * 6));
    if (input.dt <= 0) this.dimNow = dimTarget;
    const dim = this.dimNow;
    input.wells.forEach((w, i) => {
      if (!wellActive(w, input.nowTick)) return;
      const dist = v3.length(v3.sub(w.position, input.camera));
      if (dist > WELL_DRAW_RANGE) return;
      // Dynamic wells grow in and fade out over half a second.
      let life = 1;
      if (w.endTick >= 0) {
        life = Math.min(1, (input.nowTick - w.startTick) / 30, (w.endTick - input.nowTick) / 30);
        life = Math.max(0, life);
      }
      const look = wellLook(w, i, dim * life);
      if (dist < WELL_PASS_RANGE + w.core) out.volumes.push(look);
      out.lights.push({ position: w.position, color: WELL_COLOR, intensity: (w.core > 0 ? 7 : 4) * dim * life, range: w.core + 11, priority: 1.2 });
      if (dist < SWIRL_RANGE && this.sink) this.swirl(w, i, t, dim * life, look.kind === "well" ? look.axis : up);
    });

    // The own ship's pull.
    if (input.own) {
      const a = shipPull(input.wells, input.own.position, input.nowTick, input.own.shipType);
      out.pull = pullModel(a, input.orientation, input.own.shipType);
    }

    // Cave-ins: cracks while warned, dust and pebbles, the collapse, rubble.
    for (const c of this.caveIns) {
      const dist = v3.length(v3.sub(c.ceiling, input.camera));
      out.shake = Math.max(out.shake, caveInShake(c, t, dist));
      if (!c.collapsed && t >= c.end) {
        c.collapsed = true;
        out.collapses.push(c);
        const centre = v3.lerp(c.ceiling, c.position, 0.45);
        this.addRubble({ position: centre, time: t, seed: c.seed });
        this.collapseBurst(c, up, t);
      }
      if (!c.collapsed) {
        const p = Math.min(1, Math.max(0, (t - c.start) / Math.max(0.01, c.end - c.start)));
        out.volumes.push({ kind: "crack", center: c.ceiling, radius: 7.5, progress: p, color: CRACK_COLOR, intensity: 1.6, up, seed: c.seed });
        out.lights.push({ position: v3.addScaled(c.ceiling, up, -1), color: CRACK_COLOR, intensity: 2 + 10 * p * p, range: 9, priority: 1.5 });
        this.trickle(c, up, p, input.dt);
      }
    }
    this.caveIns = this.caveIns.filter((c) => t < c.end + 3);
    for (const r of this.rubble) {
      if (v3.length(v3.sub(r.position, input.camera)) > 120) continue;
      out.volumes.push({ kind: "rubble", center: r.position, radius: 7, fresh: rubbleFresh(t - r.time), color: [0.8, 0.74, 0.64], up, seed: r.seed });
    }
    // Nearest first when there are too many for the pass; at most WELL_PASS_MAX wells.
    out.volumes.sort((a, b) => volumeRank(a, input.camera) - volumeRank(b, input.camera));
    let wellsKept = 0;
    out.volumes = out.volumes.filter((v) => v.kind !== "well" || ++wellsKept <= WELL_PASS_MAX);

    // Pigment drips behind pigmented ships.
    this.pigmentDrips(input, up);
    for (const h of out.pigmentHits) this.splash(h.position, h.color, up, t);
    return out;
  }

  /** Cues: cave-in warnings and pigment pickups. */
  private cues(input: WorldFxInput, out: WorldFxFrame): void {
    const t = input.time;
    for (const c of input.cues) {
      if (c.kind === WORLD_CUE.caveInWarning) {
        const secs = Math.max(0.2, c.value / 60);
        if (this.caveIns.some((x) => !x.collapsed && v3.length(v3.sub(x.position, c.position)) < 1)) continue;
        const hit = input.raycast?.(c.position, this.upNow, 24) ?? -1;
        const ceiling = v3.addScaled(c.position, this.upNow, hit > 0 ? hit : 4);
        const site: CaveIn = { position: c.position, ceiling, start: t, end: t + secs, seed: hash01(c.position.x, c.position.z) * 100, collapsed: false, said: false };
        this.caveIns.push(site);
        out.warnings.push(site);
      } else if (c.kind === WORLD_CUE.pigment) {
        const ship = input.ships.find((s) => s.id === c.source);
        const color = ship ? teamHull(ship.team, input.teams) : PIGMENT_PRIMARIES[Math.floor(hash01(t) * 3)] ?? PIGMENT_PRIMARIES[0]!;
        out.pigmentHits.push({ position: c.position, color });
      }
    }
  }

  /** The accretion swirl: motes spiralling in, rock chips, a faint haze (immediate sprites). */
  private swirl(w: WellInfo, index: number, t: number, dim: number, axis: Vec3): void {
    const sink = this.sink;
    if (!sink || dim < 0.05) return;
    const ref = Math.abs(axis.x) > 0.9 ? { x: 0, y: 1, z: 0 } : { x: 1, y: 0, z: 0 };
    const ex = v3.normalize(v3.cross(axis, ref));
    const ey = v3.cross(axis, ex);
    const core = w.core > 0 ? w.core : 0.5;
    const disk = Math.min(w.range * 0.3, 7.5);
    const n = Math.round(this.swirlMotes * Math.max(0.25, sink.density));
    const violet = lin(WELL_COLOR, 2.4 * dim);
    const white = lin([0.92, 0.86, 1.0], 3.2 * dim);
    const at = (r: number, ang: number, lift: number): Vec3 =>
      v3.add(w.position, v3.add(v3.scale(ex, Math.cos(ang) * r), v3.add(v3.scale(ey, Math.sin(ang) * r), v3.scale(axis, lift))));
    for (let i = 0; i < n; i++) {
      const h1 = hash01(index * 97 + i, 1);
      const h2 = hash01(index * 97 + i, 2);
      const h3 = hash01(index * 97 + i, 3);
      const h4 = hash01(index * 97 + i, 4);
      const period = 5 + 7 * h2;
      const f = (t / period + h3) % 1;
      const r0 = core + 0.7 + disk * Math.pow(h1, 0.75);
      const r = r0 * (1 - 0.55 * f);
      const omega = 7.5 / Math.pow(Math.max(r, 0.8), 1.5);
      const ang = h4 * Math.PI * 2 + omega * f * period;
      const lift = (hash01(i, index + 9) - 0.5) * 0.18 * r;
      const pos = at(r, ang, lift);
      const tangent = v3.add(v3.scale(ex, -Math.sin(ang)), v3.scale(ey, Math.cos(ang)));
      const fade = Math.min(1, f / 0.12) * Math.min(1, (1 - f) / 0.2);
      const hot = 1 - f * 0.4;
      const c = h2 < 0.25 ? white : violet;
      sink.immediate({
        pos,
        vel: v3.scale(tangent, omega * r),
        life: 1,
        sprite: SPRITE.glow,
        color: [c[0] * fade * hot, c[1] * fade * hot, c[2] * fade * hot],
        size: 0.05 + 0.08 * h3,
        orient: "velocity",
        stretch: 0.06,
      });
    }
    // Rock chips caught in the swirl.
    const chips = Math.round(16 * Math.max(0.4, sink.density));
    for (let i = 0; i < chips; i++) {
      const h1 = hash01(index * 53 + i, 11);
      const h2 = hash01(index * 53 + i, 12);
      const r = core + 1.0 + disk * 0.8 * h1;
      const omega = 5.5 / Math.pow(r, 1.5);
      const ang = h2 * Math.PI * 2 + omega * t;
      const lift = (hash01(i, index + 21) - 0.5) * 0.3 * r;
      sink.immediate({
        pos: at(r, ang, lift),
        life: 1,
        sprite: SPRITE.shard,
        blend: "alpha",
        color: [0.06, 0.055, 0.05],
        alpha: 0.95,
        alpha1: 0.95,
        size: 0.18 + 0.3 * hash01(i, index + 31),
        rot: t * (0.6 + 2 * h1) + h2 * 6,
      });
    }
    // A faint haze in the disc.
    for (let i = 0; i < 6; i++) {
      const r = core + 1.2 + disk * 0.5 * hash01(i, index + 41);
      const ang = hash01(i, index + 42) * Math.PI * 2 + (2.2 / Math.pow(r, 1.5)) * t;
      sink.immediate({
        pos: at(r, ang, 0),
        life: 1,
        sprite: SPRITE.smoke0,
        blend: "alpha",
        color: lin([0.35, 0.25, 0.5], 0.06 * dim),
        alpha: 0.14 * dim,
        alpha1: 0.14 * dim,
        size: 2.2 + 1.6 * hash01(i, index + 43),
        rot: ang,
      });
    }
  }

  /** Dust and pebbles trickling from the cracking ceiling. */
  private trickle(c: CaveIn, up: Vec3, p: number, dt: number): void {
    const sink = this.sink;
    if (!sink) return;
    const rate = (40 + 160 * p) * sink.density;
    const count = Math.floor(rate * dt + hash01(c.seed, sink.now * 13.7));
    for (let i = 0; i < count; i++) {
      const a = hash01(c.seed + i, sink.now * 3.1) * Math.PI * 2;
      const rr = Math.sqrt(hash01(c.seed - i, sink.now * 5.3)) * 3.5;
      const side = v3.normalize(v3.cross(up, Math.abs(up.x) > 0.9 ? { x: 0, y: 0, z: 1 } : { x: 1, y: 0, z: 0 }));
      const side2 = v3.cross(up, side);
      const from = v3.add(c.ceiling, v3.add(v3.scale(side, Math.cos(a) * rr), v3.add(v3.scale(side2, Math.sin(a) * rr), v3.scale(up, -0.3))));
      const pebble = hash01(i, sink.now) < 0.22;
      sink.spawn(
        pebble
          ? { pos: from, vel: v3.scale(up, -(4 + 4 * hash01(i, 2))), life: 1.4, sprite: SPRITE.shard, blend: "alpha", color: [0.07, 0.06, 0.05], alpha: 1, alpha1: 0.9, size: 0.1 + 0.12 * hash01(i, 3), spin: 6 }
          : { pos: from, vel: v3.scale(up, -(2 + 3 * hash01(i, 4))), life: 2.0, sprite: SPRITE.smoke1, blend: "alpha", color: lin([0.62, 0.57, 0.5], 0.5), alpha: 0.55, alpha1: 0, size: 0.2, size1: 1.4, drag: 0.3, fadeIn: 0.1 },
      );
    }
  }

  /** The rock comes down: a dust burst, chips, a lingering cloud. */
  private collapseBurst(c: CaveIn, up: Vec3, t: number): void {
    const sink = this.sink;
    if (!sink) return;
    const centre = v3.lerp(c.ceiling, c.position, 0.5);
    const n = Math.round(46 * Math.max(0.4, sink.density));
    for (let i = 0; i < n; i++) {
      const d = v3.normalize({ x: hash01(i, t) - 0.5, y: hash01(i, t + 1) - 0.5, z: hash01(i, t + 2) - 0.5 });
      sink.spawn({ pos: v3.addScaled(centre, d, 1.5), vel: v3.add(v3.scale(d, 3 + 5 * hash01(i, 7)), v3.scale(up, -2)), life: 4 + 4 * hash01(i, 8), sprite: SPRITE.smoke2, blend: "alpha", color: lin([0.6, 0.55, 0.48], 0.3), alpha: 0.55, alpha1: 0, size: 1.2, size1: 5 + 3 * hash01(i, 9), drag: 0.9, fadeIn: 0.05 });
    }
    for (let i = 0; i < n; i++) {
      const d = v3.normalize({ x: hash01(i, t + 3) - 0.5, y: hash01(i, t + 4) - 0.5, z: hash01(i, t + 5) - 0.5 });
      sink.spawn({ pos: v3.addScaled(centre, d, 0.8), vel: v3.add(v3.scale(d, 6 + 8 * hash01(i, 10)), v3.scale(up, -5)), life: 1.2 + hash01(i, 11), sprite: SPRITE.shard, blend: "alpha", color: [0.07, 0.06, 0.05], alpha: 1, alpha1: 1, size: 0.15 + 0.3 * hash01(i, 12), spin: 8, drag: 0.6 });
    }
  }

  /** Paint dripping off pigmented ships (opaque drops: they keep their colour through the ink). */
  private pigmentDrips(input: WorldFxInput, up: Vec3): void {
    const t = input.time;
    const seen = new Set<number>();
    for (const s of input.ships) {
      if (!s.alive || !input.pigmented.has(s.id)) continue;
      seen.add(s.id);
      const last = this.lastPos.get(s.id);
      this.lastPos.set(s.id, s.position);
      const vel = last && input.dt > 0 ? v3.scale(v3.sub(s.position, last), 1 / input.dt) : { x: 0, y: 0, z: 0 };
      let clock = this.dripClock.get(s.id) ?? t;
      while (clock < t) {
        clock += 0.11;
        const back = v3.addScaled(s.position, q.forward(s.orientation), -s.radius * 0.7);
        const jitter = { x: (hash01(clock, 1) - 0.5) * s.radius, y: (hash01(clock, 2) - 0.5) * s.radius * 0.5, z: (hash01(clock, 3) - 0.5) * s.radius };
        this.drips.push({
          pos: v3.add(back, jitter),
          vel: v3.add(v3.scale(vel, 0.35), v3.scale(up, -1.6)),
          born: clock,
          life: 1.1,
          size: 0.13 + 0.06 * hash01(clock, 4),
          color: teamHull(s.team, input.teams),
        });
      }
      this.dripClock.set(s.id, clock);
    }
    for (const id of [...this.dripClock.keys()]) if (!seen.has(id)) this.dripClock.delete(id);
    this.drips = this.drips.filter((d) => t - d.born < d.life).slice(-400);
  }

  private splash(at: Vec3, color: Rgb, up: Vec3, t: number): void {
    for (let i = 0; i < 14; i++) {
      const d = v3.normalize({ x: hash01(i, t) - 0.5, y: hash01(i, t + 1) - 0.3, z: hash01(i, t + 2) - 0.5 });
      this.drips.push({ pos: at, vel: v3.add(v3.scale(d, 4 + 3 * hash01(i, 5)), v3.scale(up, 1)), born: t, life: 0.9, size: 0.12 + 0.08 * hash01(i, 6), color });
    }
  }

  /**
   * Pigment pickups, splashes and drips as instances (opaque, colour kept);
   * the scene builder leaves pigment pickups to this (SceneExtras.skipKinds).
   */
  addToScene(scene: SceneInstances, entities: readonly EntityState[], time: number, teams: number, lights: PointLight[]): void {
    if (!this.enabled) return;
    const drops = (scene.opaque.pigment ??= []);
    const glows = (scene.opaque.orb ??= []);
    for (const e of entities) {
      if (e.kind !== "pigmentPickup") continue;
      const ph = hash01(e.id) * Math.PI * 2;
      const c = { x: e.position.x, y: e.position.y + Math.sin(time * 2 + ph) * 0.15, z: e.position.z };
      const splash = e.state[0] !== 0;
      if (splash) {
        const color = teamHull(e.state[1], teams);
        const wob = 1 + 0.08 * Math.sin(time * 7 + ph);
        drops.push({ position: c, rotation: q.fromAxisAngle({ x: 0, y: 1, z: 0 }, time * 1.2 + ph), scale: 0.95 * wob, color, emissive: 0.25, keepColor: 1 });
        for (let k = 0; k < 2; k++) {
          const a = time * 2.4 + ph + k * Math.PI;
          drops.push({ position: { x: c.x + Math.cos(a) * 0.75, y: c.y + 0.15 * Math.sin(a * 2), z: c.z + Math.sin(a) * 0.75 }, rotation: q.identity(), scale: 0.38, color, emissive: 0.3, keepColor: 1 });
        }
        lights.push({ position: c, color, intensity: 3, range: 6 });
        continue;
      }
      // Three drops in the primaries circling a glowing white core.
      for (let k = 0; k < 3; k++) {
        const a = time * 1.8 + ph + (k * Math.PI * 2) / 3;
        // A ring tilted 30 degrees, so the three drops never hide behind each other.
        const ox = Math.cos(a) * 0.5;
        const oz = Math.sin(a) * 0.5;
        const pos = { x: c.x + ox, y: c.y + oz * 0.5 + 0.08 * Math.sin(time * 2.6 + k * 2.1), z: c.z + oz * 0.87 };
        // Tip leaning along the orbit.
        const rot = q.mul(q.fromAxisAngle({ x: 0, y: 1, z: 0 }, -a), q.fromAxisAngle({ x: 1, y: 0, z: 0 }, 0.5));
        drops.push({ position: pos, rotation: rot, scale: 0.4, color: PIGMENT_PRIMARIES[k]!, emissive: 0.22, keepColor: 1 });
      }
      glows.push({ position: c, rotation: q.identity(), scale: 0.22 + 0.025 * Math.sin(time * 5 + ph), color: [1, 0.96, 0.88], emissive: 1, keepColor: 1 });
      const hue = PIGMENT_PRIMARIES[Math.floor(((time * 0.7 + ph) / (Math.PI * 2)) * 3) % 3] ?? PIGMENT_PRIMARIES[0]!;
      lights.push({ position: c, color: hue, intensity: 3.5, range: 7 });
    }
    for (const d of this.drips) {
      const age = time - d.born;
      if (age < 0 || age > d.life) continue;
      const k = 1 - age / d.life;
      drops.push({ position: v3.addScaled(d.pos, d.vel, age), rotation: q.identity(), scale: d.size * (0.4 + 0.6 * k), color: d.color, emissive: 0.2, keepColor: 1 });
    }
  }
}

/** The audio engine's part the world sounds use (audio/engine.ts). */
export interface WorldSoundEngine {
  play(id: string, o?: { position?: Vec3 | null }): boolean;
  loop(id: string, o?: { position?: Vec3 | null }): { readonly playing: boolean; setGain(g: number, ramp?: number): void; setPosition(p: Vec3): void; stop(fade?: number): void } | null;
}

/** Distance (u) within which the nearest well hums. */
export const WELL_HUM_RANGE = 16;
/** The announcer calls a cave-in this close to the own ship (u). */
export const CAVE_IN_SAY_RANGE = 40;

/**
 * World sounds (audio/sounds/world.ts): the cave-in warning and crash at
 * their spot, the pigment splash, the nearest well's hum (a loop that
 * follows it and swells as you get closer), and the announcer's "caveIn"
 * line when a cave-in is near you.
 */
export class WorldAudio {
  private hum: ReturnType<WorldSoundEngine["loop"]> = null;
  engine: WorldSoundEngine | null = null;

  update(f: WorldFxFrame, wells: readonly WellInfo[], tick: number, camera: Vec3, own: Vec3 | null, say: (trigger: "caveIn") => void, quiet = false): void {
    const e = this.engine;
    if (!e) return;
    for (const c of f.warnings) {
      e.play("cavein.warning", { position: c.ceiling });
      if (own && !c.said && v3.length(v3.sub(c.position, own)) < CAVE_IN_SAY_RANGE) {
        c.said = true;
        say("caveIn");
      }
    }
    for (const c of f.collapses) e.play("cavein.crash", { position: v3.lerp(c.ceiling, c.position, 0.5) });
    for (const h of f.pigmentHits) e.play("pigment.splash", { position: h.position });
    let near: WellInfo | null = null;
    let best = WELL_HUM_RANGE;
    for (const w of wells) {
      if (!wellActive(w, tick)) continue;
      const d = v3.length(v3.sub(w.position, camera));
      if (d < best) {
        best = d;
        near = w;
      }
    }
    if (!near || quiet) {
      this.hum?.stop(0.6);
      this.hum = null;
      return;
    }
    if (!this.hum || !this.hum.playing) this.hum = e.loop("well.hum", { position: near.position });
    this.hum?.setPosition(near.position);
    this.hum?.setGain(Math.min(1, 1.3 * (1 - best / WELL_HUM_RANGE)), 0.3);
  }

  stop(): void {
    this.hum?.stop(0.3);
    this.hum = null;
  }
}

/** `?worldfx=off`: no B5 world visuals (before shots and comparisons). */
export function wantsWorldFx(loc: { search: string } | undefined): boolean {
  return !loc || new URLSearchParams(loc.search).get("worldfx") !== "off";
}

/** What main.ts hands the world visuals each frame (raw ls-wasm views). */
export interface WorldFrameInput {
  time: number;
  dt: number;
  nowTick: number;
  camera: Vec3;
  orientation: Quat;
  /** ls-wasm `Game.wells()`, `Game.up()`, `Game.shipExt()`, `Game.monochrome`. */
  wells: ArrayLike<number> | undefined;
  up: ArrayLike<number> | undefined;
  shipExt: ArrayLike<number> | undefined;
  monochrome: boolean;
  /** Last Color Standing runs its own ship colours. */
  modeStyles: boolean;
  cues: readonly Cue[];
  mood: MoodState;
  own: { id: number; team: number; position: Vec3; shipType: number } | null;
  ownTeam: number;
  ships: readonly ShipState[];
  entities: readonly EntityState[];
  teams: number;
  raycast?: ((o: Vec3, d: Vec3, max: number) => number) | undefined;
  /** Results screen or lobby: drones and hums fade. */
  quiet: boolean;
}

export interface WorldFrame extends WorldFxFrame {
  wells: WellInfo[];
  pigmented: ReadonlySet<number>;
  /** Ship colours in a monochrome level (null: none; Last Color Standing uses its own). */
  shipStyle: ((s: { id: number; team: number }) => { color: Rgb; keepColor: number }) | null;
  /** The own cockpit frame's colour and keep in a monochrome level. */
  ownStyle: { color: Rgb; keepColor: number } | null;
  mood: MoodIconModel | null;
  /** Dust mote colour of the mood. */
  motes: { color: [number, number, number]; rainbow: number };
}

const PIGMENT_KINDS: ReadonlySet<string> = new Set(["pigmentPickup"]);

/**
 * Everything of B5 behind one call per frame (main.ts): wells, cave-ins,
 * pigment (WorldFx), mood presentation (MoodFx) and their sounds.
 */
export class WorldVisuals {
  readonly fx = new WorldFx();
  readonly mood = new MoodFx();
  readonly audio = new WorldAudio();
  private last: WorldFrame | null = null;

  constructor(sink: VfxSink | null, enabled: boolean, engine: (WorldSoundEngine & DroneEngine) | null = null) {
    this.fx.sink = sink;
    this.fx.enabled = enabled;
    if (enabled) {
      this.audio.engine = engine;
      this.mood.engine = engine;
    }
  }

  get enabled(): boolean {
    return this.fx.enabled;
  }

  /** Entity kinds the scene builder leaves to the world visuals. */
  get skipKinds(): ReadonlySet<string> | undefined {
    return this.fx.enabled ? PIGMENT_KINDS : undefined;
  }

  reset(): void {
    this.fx.reset();
    this.mood.reset();
    this.audio.stop();
    this.last = null;
  }

  update(i: WorldFrameInput, say: (trigger: "caveIn") => void): WorldFrame {
    const wells = parseWells(i.wells);
    const pigmented = i.monochrome ? pigmentedIds(i.shipExt) : new Set<number>();
    const up = i.up && i.up.length >= 3 ? { x: i.up[0] ?? 0, y: i.up[1] ?? 1, z: i.up[2] ?? 0 } : i.mood.up;
    const f = this.fx.update({
      time: i.time,
      dt: i.dt,
      nowTick: i.nowTick,
      camera: i.camera,
      orientation: i.orientation,
      wells,
      cues: i.cues,
      mood: i.mood,
      up,
      own: i.own ? { position: i.own.position, shipType: i.own.shipType } : null,
      ships: i.ships,
      entities: i.entities,
      monochrome: i.monochrome,
      pigmented,
      teams: i.teams,
      raycast: i.raycast,
    });
    this.audio.update(f, wells, i.nowTick, i.camera, i.own?.position ?? null, say, i.quiet);
    if (this.fx.enabled) this.mood.update(i.mood.mood, i.time, i.dt, i.quiet);
    const style = this.fx.enabled && !i.modeStyles ? pigmentStyle(i.monochrome, pigmented, i.ownTeam, i.teams) : null;
    const frame: WorldFrame = {
      ...f,
      wells,
      pigmented,
      shipStyle: style,
      ownStyle: style ? style({ id: i.own?.id ?? -1, team: i.ownTeam }) : null,
      mood: this.fx.enabled ? this.mood.icon(i.time) : null,
      motes: this.fx.enabled ? moodMotes(this.mood.weights) : { color: [...DEFAULT_MOTES] as [number, number, number], rainbow: 0 },
    };
    // The own ship's colour follows its own pigment (it is a teammate of itself).
    if (frame.ownStyle && i.own && pigmented.has(i.own.id)) frame.ownStyle = { color: teamHull(i.ownTeam, i.teams), keepColor: 1 };
    this.last = frame;
    return frame;
  }

  /** The world's grade with the mood's tint (unchanged with the world visuals off). */
  grade(base: ColorGrade): ColorGrade {
    return this.fx.enabled ? moodGrade(base, this.mood.weights) : base;
  }

  /** Pigment pickups, splashes and drips into the scene; their lights into `lights`. */
  addToScene(scene: SceneInstances, entities: readonly EntityState[], time: number, teams: number, lights: PointLight[]): void {
    this.fx.addToScene(scene, entities, time, teams, lights);
  }

  get lastFrame(): WorldFrame | null {
    return this.last;
  }

  dispose(): void {
    this.audio.stop();
    this.mood.stopDrones();
  }
}

const DEFAULT_MOTES: readonly [number, number, number] = [0.85, 0.78, 0.66];

function volumeRank(v: WorldVolume, cam: Vec3): number {
  const d = v3.length(v3.sub(v.center, cam));
  // Cave-in warnings matter most, then wells, then rubble.
  return d + (v.kind === "crack" ? -1000 : v.kind === "rubble" ? 60 : 0);
}

/** Whether an entity is drawn by the world visuals rather than the scene builder. */
export function drawnByWorldFx(e: Pick<EntityState, "kind">): boolean {
  return e.kind === "pigmentPickup";
}
