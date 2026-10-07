// Builds the M4 C3 spectacle scenes of spectacleScenes.ts in the gallery page
// (gallery.ts calls prepareSpectacleScene before the terrain settles, for
// the camera and the terrain edits, and runSpectacleScene once it has):
// synthetic props, ships, cues and mood states are driven through the real
// event visuals (game/events/spectacles.ts) over the scene's last seconds,
// so particles, springs, trails and the kill-cam recorder have a history;
// then the frame goes to the renderer and the page (the hat chip, the
// kill-cam window). `?worldfx=off` draws the same moment without C3.

import { showStripCard } from "./eventCard.ts";
import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { Cue } from "../game/mechanics.ts";
import { emptyMood, type MoodState } from "../game/modeState.ts";
import { buildScene, teamHull } from "../game/scene.ts";
import type { EntityState, ProjectileState, ShipState, VisualDef } from "../game/views.ts";
import { SpectacleVisuals } from "../game/events/spectacles.ts";
import { lookAlong, type OwnShip, type SpectacleInput } from "../game/events/spectacleCommon.ts";
import { DUCK_RADIUS } from "../game/events/duckParade.ts";
import { axisDir, CLAW_GAP } from "../game/events/catPaw.ts";
import { hatMeshKind } from "../renderer/models/index.ts";
import { HAT_COLORS } from "../renderer/models/spectacleProps.ts";
import type { PointLight } from "../renderer/lights.ts";
import type { ParticleSystem } from "../renderer/particles.ts";
import { Renderer, type SceneInstances } from "../renderer/renderer.ts";
import type { GameHandle } from "../wasm/api.ts";
import type { SpectacleSceneKind } from "./spectacleScenes.ts";

type Pose = { position: Vec3; orientation: Quat };
type Ray = (o: Vec3, d: Vec3, max: number) => number;
type Edit = (add: boolean, c: Vec3, r: number) => boolean;

const UP: Vec3 = { x: 0, y: 1, z: 0 };
const NAMES = ["Rivet", "Sprocket", "Gasket", "Cog", "Bolt"];

function basis(p: Pose): { f: Vec3; r: Vec3; u: Vec3; at: (fw: number, rt: number, up: number) => Vec3 } {
  const f = q.forward(p.orientation);
  const r = q.right(p.orientation);
  const u = q.up(p.orientation);
  return { f, r, u, at: (fw, rt, up) => v3.add(p.position, v3.add(v3.scale(f, fw), v3.add(v3.scale(r, rt), v3.scale(u, up)))) };
}

function look(from: Vec3, at: Vec3, up: Vec3 = UP): Quat {
  return lookAlong(v3.sub(at, from), up);
}

/** The cat paw scenes' spot: a boundary face, the rock where it begins, and which way along it there is room. */
export interface PawWall {
  face: number;
  /** The face's outward normal. */
  n: Vec3;
  /** Sweep direction along the wall (toward the grazing camera). */
  t: Vec3;
  /** `t x n`. */
  b: Vec3;
  /** Where the wall's rock begins. */
  wall: Vec3;
}

/**
 * A stretch of boundary wall with room in front of it and along it (for the
 * paw's swipe and a camera grazing the wall): sampled over the four side
 * faces with the exact terrain ray (ls-wasm `Game.raycast`).
 */
export function findPawWall(bounds: ArrayLike<number>, ray: Ray): PawWall | null {
  const lo = [bounds[0] ?? 0, bounds[1] ?? 0, bounds[2] ?? 0];
  const hi = [bounds[3] ?? 0, bounds[4] ?? 0, bounds[5] ?? 0];
  let best: { w: PawWall; score: number } | null = null;
  for (const face of [0, 1, 4, 5]) {
    const n = axisDir(face);
    const axis = Math.floor(face / 2);
    const plane = face & 1 ? hi[axis]! : lo[axis]!;
    const ta = axis === 0 ? 2 : 0;
    for (const depth of [16, 26, 36]) {
      for (let a = lo[ta]! + 24; a <= hi[ta]! - 24; a += 14) {
        for (let y = lo[1]! + 14; y <= hi[1]! - 14; y += 10) {
          const p = [0, 0, 0];
          p[axis] = plane - (face & 1 ? depth : -depth);
          p[ta] = a;
          p[1] = y;
          const from = { x: p[0]!, y: p[1]!, z: p[2]! };
          const hit = ray(from, n, depth + 6);
          if (hit < 0 || depth + 0.5 - hit > 12) continue;
          // Generated rock, not the bare sealing wall (2 u inside the box), which no crater can open (D5).
          if (depth - hit < 3) continue;
          const wall = v3.addScaled(from, n, hit);
          const front = ray(v3.addScaled(wall, n, -0.6), v3.scale(n, -1), 40);
          if (front >= 0 && front < 22) continue;
          const lane = v3.addScaled(wall, n, -9);
          for (const s of [1, -1]) {
            const t: Vec3 = ta === 0 ? { x: s, y: 0, z: 0 } : { x: 0, y: 0, z: s };
            const along = ray(lane, t, 40);
            const back = ray(lane, v3.scale(t, -1), 30);
            const room = (along < 0 ? 40 : along) + Math.min(back < 0 ? 30 : back, 20);
            const up = ray(lane, UP, 20);
            const down = ray(lane, v3.scale(UP, -1), 20);
            const height = Math.min(up < 0 ? 20 : up, 14) + Math.min(down < 0 ? 20 : down, 14);
            const score = room + height + (front < 0 ? 40 : front) * 0.3;
            if (!best || score > best.score) best = { w: { face, n, t, b: v3.cross(t, n), wall }, score };
          }
        }
      }
    }
  }
  return best?.w ?? null;
}

/** The duck scenes' convoy: the lead duck's position at the frame and the direction it waddles. */
export function duckRoute(kind: SpectacleSceneKind, pose: Pose): { lead: Vec3; dir: Vec3 } {
  const { f, r, at } = basis(pose);
  if (kind === "duck_parade-a") return { lead: at(30, -4, -1), dir: v3.normalize(v3.add(v3.scale(f, -0.8), v3.scale(r, 0.6))) };
  if (kind === "duck_parade-b") return { lead: at(10, 1.5, -0.8), dir: v3.normalize(v3.add(v3.scale(f, -0.55), v3.scale(r, -0.8))) };
  // Aftermath: the last, cool duck waddles past toward the right, the others already leaving.
  const dir = v3.normalize(v3.add(r, v3.scale(f, -0.15)));
  return { lead: v3.addScaled(at(13, -4, -0.5), dir, 36), dir };
}

/** Set-up of a spectacle scene: its camera, terrain edits made, and the paw's wall. */
export interface SpectaclePrep {
  pose: Pose;
  edited: boolean;
  wall: PawWall | null;
}

/** data/events/cat_paw.ron `groove_radius` (u). */
export const GROOVE_RADIUS = 3;
/** data/events/cat_paw.ron `groove_inset` (u): a groove sphere's centre inside the rock its claw scrapes. */
export const GROOVE_INSET = 0.6;

/**
 * Groove carving of the aftermath frame: three chains of craters along the
 * sweep, `gap` apart, each sphere `GROOVE_INSET` inside the rock its claw
 * meets (found with `ray` along the wall's normal, like the sim's claws;
 * without a ray the wall is taken as flat).
 */
export function grooveEdits(w: PawWall, ray?: Ray, gap = CLAW_GAP, radius = GROOVE_RADIUS): Array<{ center: Vec3; radius: number }> {
  const out: Array<{ center: Vec3; radius: number }> = [];
  for (const k of [-1, 0, 1]) {
    for (let s = -26; s <= 12; s += 3.5) {
      const wob = Math.sin(s * 0.13 + k) * 0.3;
      const lane = v3.add(v3.add(w.wall, v3.scale(w.b, k * gap + wob)), v3.scale(w.t, s));
      const from = v3.addScaled(lane, w.n, -6);
      const hit = ray ? ray(from, w.n, 12) : 6;
      if (hit < 0) continue;
      out.push({ center: v3.addScaled(from, w.n, hit + GROOVE_INSET), radius });
    }
  }
  return out;
}

export function prepareSpectacleScene(kind: SpectacleSceneKind, pose: Pose, bounds: ArrayLike<number>, exactRay: Ray, edit: Edit): SpectaclePrep {
  const prep: SpectaclePrep = { pose: { position: { ...pose.position }, orientation: { ...pose.orientation } }, edited: false, wall: null };
  if (kind.startsWith("cat_paw")) {
    const w = findPawWall(bounds, exactRay);
    if (w) {
      prep.wall = w;
      if (kind === "cat_paw-a") {
        // Face-on to the wall where the paw will come through.
        const cam = v3.add(v3.addScaled(w.wall, w.n, -21), v3.add(v3.scale(w.t, 4), v3.scale(UP, 2)));
        prep.pose = { position: cam, orientation: look(cam, w.wall) };
      } else {
        // Along the wall: the paw swipes toward the camera, beans first; the
        // aftermath looks down the scored wall from the same side (D5).
        const cam = kind === "cat_paw-c"
          ? v3.add(v3.addScaled(w.wall, w.n, -15), v3.add(v3.scale(w.t, 24), v3.scale(UP, 6)))
          : v3.add(v3.addScaled(w.wall, w.n, -15), v3.add(v3.scale(w.t, 30), v3.scale(UP, 7.5)));
        const target = kind === "cat_paw-c" ? v3.add(v3.addScaled(w.wall, w.n, -2), v3.scale(w.t, -6)) : v3.add(v3.addScaled(w.wall, w.n, -7.5), v3.scale(w.t, 2));
        prep.pose = { position: cam, orientation: look(cam, target) };
      }
      if (kind === "cat_paw-c") {
        for (const e of grooveEdits(w, exactRay)) prep.edited = edit(false, e.center, e.radius) || prep.edited;
      }
    }
  } else if (kind === "duck_parade-b") {
    // Two rocks pinch the parade's lane: the second duck squeezes through.
    const { lead, dir } = duckRoute(kind, prep.pose);
    const second = v3.addScaled(lead, dir, -9);
    for (const s of [-1, 1]) prep.edited = edit(true, v3.addScaled(second, UP, s * 7.4), 5.2) || prep.edited;
  }
  return prep;
}

export interface SpectacleSceneContext {
  kind: SpectacleSceneKind;
  renderer: Renderer;
  game: GameHandle;
  prep: SpectaclePrep;
  time: number;
  visuals: readonly VisualDef[];
  particles: ParticleSystem | null;
  ui: HTMLElement;
  ray: Ray;
  enabled: boolean;
  lights: PointLight[];
}

/** One synthetic moment of a scene. */
interface Moment {
  ships: ShipState[];
  entities: EntityState[];
  shots: ProjectileState[];
  mood: MoodState;
  cues: Cue[];
  own: OwnShip | null;
  ext: number[];
}

function ship(id: number, shipType: number, team: number, position: Vec3, orientation: Quat): ShipState {
  return { id, team, alive: true, boosting: id % 2 === 0, position, orientation, shipType, radius: [0.4, 0.5, 0.65, 0.55][shipType] ?? 0.5, gadgetActive: false };
}

function ent(id: number, kind: string, position: Vec3, velocity: Vec3, rotation: Quat, state: [number, number, number, number], radius: number): EntityState {
  return { id, kind, visual: 0, owner: 255, team: 255, position, velocity, rotation, state, radius };
}

function mood(eventId: string, phase: string, nowTick: number, phaseLeftSecs = 10): MoodState {
  return { ...emptyMood(), mood: "giddy", eventId, phase, phaseEndTick: nowTick + Math.round(phaseLeftSecs * 60) };
}

/** Free space around a point (the nearest rock along 14 directions). */
function clearanceAt(p: Vec3, ray: Ray): number {
  const dirs: Vec3[] = [
    { x: 1, y: 0, z: 0 }, { x: -1, y: 0, z: 0 }, { x: 0, y: 1, z: 0 }, { x: 0, y: -1, z: 0 }, { x: 0, y: 0, z: 1 }, { x: 0, y: 0, z: -1 },
    ...[-1, 1].flatMap((a) => [-1, 1].flatMap((b) => [-1, 1].map((c) => v3.normalize({ x: a, y: b, z: c })))),
  ];
  let best = 25.5;
  for (const d of dirs) {
    const t = ray(p, d, 25.5);
    if (t >= 0) best = Math.min(best, t);
  }
  return best;
}

/** The scene's moment at time `t` (the frame is at `T`). */
function momentFor(kind: SpectacleSceneKind, pose: Pose, T: number, ctx: SpectacleSceneContext, cache: Map<string, number>): (t: number) => Moment {
  const { f, r, u, at } = basis(pose);
  const tick = (t: number): number => Math.round(t * 60);
  const none = (t: number): Moment => ({ ships: [], entities: [], shots: [], mood: mood("", "", tick(t)), cues: [], own: null, ext: [] });

  if (kind.startsWith("duck_parade")) {
    const { lead, dir } = duckRoute(kind, pose);
    return (t) => {
      const entities: EntityState[] = [];
      for (let i = 0; i < 5; i++) {
        const p = v3.addScaled(lead, dir, -5 * (T - t) - 9 * i);
        const key = `${i}:${Math.round(t * 10)}`;
        let clear = cache.get(key);
        if (clear === undefined) {
          clear = clearanceAt(p, ctx.ray);
          cache.set(key, clear);
        }
        entities.push(ent(10 + i, "duck", p, v3.scale(dir, 5), lookAlong(dir, UP), [i, Math.min(255, Math.round(clear * 10)), 0, 0], DUCK_RADIUS));
      }
      const ships: ShipState[] = [];
      const cues: Cue[] = [];
      if (kind === "duck_parade-b") {
        // A Tinker caught at the lead duck's side, being shoved.
        const leadPos = entities[0]!.position;
        const side = v3.add(v3.addScaled(leadPos, v3.normalize(v3.add(dir, v3.scale(f, -0.6))), DUCK_RADIUS + 0.8), v3.scale(u, 0.6 + 0.3 * Math.sin(t * 3)));
        ships.push(ship(100, 1, 1, side, q.mul(look(side, v3.add(side, v3.add(v3.scale(f, -1), v3.scale(r, 0.8)))), q.fromAxisAngle({ x: 0, y: 0, z: 1 }, 0.5 + 0.3 * Math.sin(t * 7)))));
        if (Math.abs(t - (T - 0.12)) < 0.5 / 60) cues.push({ kind: 20, source: 255, target: 255, position: v3.addScaled(entities[1]!.position, f, -2.5), value: 12, tick: tick(t) });
      }
      return { ships, entities, shots: [], mood: mood("duck_parade", "active", tick(t), 20), cues, own: null, ext: [] };
    };
  }

  if (kind.startsWith("hat_rain")) {
    const falling: Array<{ id: number; base: Vec3; t0: number; style: number }> = [];
    const resting: Array<{ id: number; pos: Vec3; style: number }> = [];
    const nFall = kind === "hat_rain-a" ? 7 : kind === "hat_rain-b" ? 20 : 4;
    for (let k = 0; k < nFall; k++) {
      const h = (a: number): number => {
        const x = Math.sin(k * 91.7 + a * 37.3) * 43758.5453;
        return x - Math.floor(x);
      };
      const fw = kind === "hat_rain-a" ? 6 + 12 * h(1) : 8 + 30 * h(1);
      const rt = (h(2) - 0.5) * 2 * (6 + fw * 0.6);
      const height = kind === "hat_rain-a" ? 3 + 6 * h(3) : 1 + 14 * h(3);
      falling.push({ id: 200 + k, base: v3.add(at(fw, rt, 0), v3.scale(UP, height)), t0: T, style: k % 8 });
    }
    const nRest = kind === "hat_rain-a" ? 0 : 9;
    for (let k = 0; k < nRest; k++) {
      const a = (k / nRest) * 1.6 - 0.8;
      const p = at(7 + 3 * k, Math.sin(a * 3) * (4 + k), 1);
      const down = ctx.ray(p, v3.scale(UP, -1), 30);
      if (down < 0) continue;
      resting.push({ id: 300 + k, pos: v3.addScaled(p, UP, -(down - 0.75)), style: (k * 3 + 1) % 8 });
    }
    const lineupAt = (i: number, t: number): Pose => {
      const sway = Math.sin(t * 2.2 + i * 1.3) * 0.6;
      const p = at(5.2 + (i % 2) * 0.8, (i - 1.5) * 2.1 + sway, -0.4 + 0.15 * Math.sin(t * 1.7 + i));
      const nose = v3.normalize(v3.add(v3.scale(f, -1), v3.scale(r, 0.7 + 0.25 * Math.cos(t * 2.2 + i * 1.3))));
      return { position: p, orientation: q.lookRotation(nose, u) };
    };
    const stacks: number[][] = kind === "hat_rain-c" ? [[1], [3, 6, 2, 5, 0, 4], [7, 2]] : [[0], [2, 1], [4, 6, 3, 5], [7]];
    return (t) => {
      const entities: EntityState[] = [];
      for (const h of falling) {
        const y = -3 * (t - h.t0);
        entities.push(ent(h.id, "hat", v3.addScaled(h.base, UP, y), { x: 0, y: -3, z: 0 }, q.identity(), [h.style, 0, 0, 0], 0.8));
      }
      for (const h of resting) entities.push(ent(h.id, "hat", h.pos, { x: 0, y: 0, z: 0 }, q.identity(), [h.style, kind === "hat_rain-c" ? 3 : 1, 0, 0], 0.8));
      const ships = stacks.map((_, i) => {
        const p = kind === "hat_rain-c" ? lineupAt(i + 1, t) : lineupAt(i, t);
        return ship(100 + i, kind === "hat_rain-c" && i === 1 ? 2 : i % 4, i % 2, p.position, p.orientation);
      });
      const ext: number[] = [];
      // The announcement: nobody wears a hat yet.
      ships.forEach((s, i) => ext.push(s.id, 0, kind === "hat_rain-a" ? 0 : stacks[i]!.length));
      let own: OwnShip | null = null;
      if (kind === "hat_rain-b") {
        own = { id: 0, team: 0, position: pose.position, orientation: pose.orientation, shipType: 1, radius: 0.5 };
        ext.push(0, 0, 3);
      }
      return { ships, entities, shots: [], mood: mood(kind === "hat_rain-c" ? "" : "hat_rain", kind === "hat_rain-c" ? "" : "active", tick(t)), cues: [], own, ext };
    };
  }

  if (kind.startsWith("cat_paw")) {
    const w = ctx.prep.wall;
    if (!w) return none;
    const pad = 10;
    // Paw frame (ls-sim Sweep::rot): X = the sweep direction (t, toward the grazing camera), Z = toward the shoulder (n).
    const rot = q.lookRotation(v3.scale(w.n, -1), v3.cross(w.n, w.t));
    const lane = v3.addScaled(w.wall, w.n, -(pad + 1));
    return (t) => {
      const entities: EntityState[] = [];
      let m = mood("cat_paw", "active", tick(t), 1);
      if (kind === "cat_paw-a") {
        m = mood("cat_paw", "announce", tick(t), 0.55);
        entities.push(ent(400, "catPaw", v3.addScaled(w.wall, w.n, pad + 4), { x: 0, y: 0, z: 0 }, rot, [0, 0, w.face, 2], pad));
      } else if (kind === "cat_paw-b") {
        const x = 2 - 45 * (T - t);
        const prog = Math.max(0, Math.min(255, Math.round((0.55 - (T - t) / 2) * 255)));
        entities.push(ent(400, "catPaw", v3.addScaled(lane, w.t, x), v3.scale(w.t, 45), rot, [2, prog, w.face, 2], pad));
      } else {
        // The sweep ended 1.3 s ago and the paw has pulled back out: grooves, dust and fur remain.
        const end = T - 1.3;
        if (t < end) {
          const x = 10 - 45 * (end - t);
          const prog = Math.max(0, Math.min(255, Math.round((1 - (end - t) / 2) * 255)));
          entities.push(ent(400, "catPaw", v3.addScaled(lane, w.t, x), v3.scale(w.t, 45), rot, [2, prog, w.face, 2], pad));
        } else if (t < end + 0.5) {
          const k = Math.min(1, (t - end) / 0.5);
          const c = v3.addScaled(v3.addScaled(lane, w.t, 10), w.n, (pad + 4) * k * k);
          entities.push(ent(400, "catPaw", c, v3.scale(w.n, 30 * k), rot, [3, Math.round(k * 255), w.face, 2], pad));
        }
      }
      // A Tinker caught in the peak frame: flung ahead of the paw, tumbling.
      const ships: ShipState[] = [];
      if (kind === "cat_paw-b") {
        const p = v3.add(v3.addScaled(lane, w.t, 2 + pad + 4 + 25 * (t - T + 0.25)), v3.add(v3.scale(w.n, -3), v3.scale(UP, 3)));
        ships.push(ship(100, 1, 1, p, q.mul(look(p, v3.add(p, w.t)), q.fromAxisAngle({ x: 1, y: 0.3, z: 0 }, t * 11))));
      }
      return { ships, entities, shots: [], mood: m, cues: [], own: null, ext: [] };
    };
  }

  if (kind.startsWith("disco_inferno")) {
    const ceil = ctx.ray(at(12, 0, 0), u, 40);
    const height = Math.max(2.5, Math.min(6, (ceil >= 0 ? ceil : 12) - 2.8));
    const ballAt = at(12, 0.5, height);
    const centre = at(13, 0, -0.6);
    const fly = (k: number, t: number): Pose => {
      const w = 0.9 + 0.18 * k;
      const a = t * w + k * 1.6;
      const rad = 4.2 + 1.1 * k;
      const p = v3.add(centre, v3.add(v3.scale(r, Math.cos(a) * rad), v3.add(v3.scale(f, Math.sin(a) * rad * 0.7), v3.scale(u, Math.sin(a * 2 + k) * 0.8))));
      const vel = v3.add(v3.scale(r, -Math.sin(a)), v3.scale(f, Math.cos(a) * 0.7));
      return { position: p, orientation: lookAlong(vel, u) };
    };
    return (t) => {
      const entities: EntityState[] = [];
      let m: MoodState;
      if (kind === "disco_inferno-a") {
        const prog = Math.min(1, 0.72 - (T - t) / 3);
        const p = v3.addScaled(ballAt, u, 15 * (1 - prog) * (1 - prog));
        if (prog > 0) entities.push(ent(500, "discoBall", p, { x: 0, y: 0, z: 0 }, q.fromAxisAngle(u, t * 1.2), [45, Math.round(prog * 255), 0, 0], 1.5));
        m = mood("disco_inferno", "active", tick(t), 18);
      } else if (kind === "disco_inferno-b") {
        entities.push(ent(500, "discoBall", ballAt, { x: 0, y: 0, z: 0 }, q.fromAxisAngle(u, t * 1.2), [8, 255, 0, 0], 1.5));
        m = mood("disco_inferno", "active", tick(t), 9);
      } else {
        const over = T - 0.75;
        if (t < over) entities.push(ent(500, "discoBall", ballAt, { x: 0, y: 0, z: 0 }, q.fromAxisAngle(u, t * 1.2), [8, 255, 0, 0], 1.5));
        m = t < over ? mood("disco_inferno", "active", tick(t), over - t) : mood("", "", tick(t));
      }
      const ships = [0, 1, 2, 3].map((k) => {
        const p = fly(k, t);
        return ship(100 + k, k, k, p.position, p.orientation);
      });
      return { ships, entities, shots: [], mood: m, cues: [], own: null, ext: [] };
    };
  }

  if (kind.startsWith("tired_announcer")) {
    // A dogfight in front of the camera: the killer (100) chases the victim (101) and shoots it down.
    const killTime = kind === "tired_announcer-b" ? T - 2.2 : kind === "tired_announcer-c" ? T - 3.3 : T + 10;
    const bolt = Math.max(0, ctx.visuals.findIndex((v) => v.shape === "bolt"));
    const path = (k: number, t: number): Pose => {
      const a = t * 0.8 - k * 0.42;
      const p = at(10 + Math.sin(a * 1.3) * 3, Math.cos(a) * 5.5, Math.sin(a * 2) * 1.2);
      const n = at(10 + Math.sin((a + 0.01) * 1.3) * 3, Math.cos(a + 0.01) * 5.5, Math.sin((a + 0.01) * 2) * 1.2);
      return { position: p, orientation: lookAlong(v3.sub(n, p), u) };
    };
    return (t) => {
      const ships: ShipState[] = [];
      const killer = path(0.9, t);
      ships.push(ship(100, 1, 0, killer.position, killer.orientation));
      const victim = path(0, t);
      ships.push({ ...ship(101, 2, 1, victim.position, victim.orientation), alive: t < killTime });
      const third = path(2.4, t);
      ships.push(ship(102, 0, 1, third.position, third.orientation));
      const shots: ProjectileState[] = [];
      for (let k = 0; k < 4; k++) {
        const fired = killTime - 0.18 - k * 0.22;
        const age = t - fired;
        if (age < 0 || age > 0.5 || t > killTime) continue;
        const from = path(0.9, fired).position;
        const to = path(0, fired + 0.18).position;
        const vel = v3.scale(v3.normalize(v3.sub(to, from)), 120);
        shots.push({ kind: 0, owner: 100, position: v3.addScaled(from, vel, age), velocity: vel, visual: bolt, state: 0 });
      }
      const cues: Cue[] = [];
      if (Math.abs(t - killTime) < 0.5 / 60) cues.push({ kind: 19, source: 100, target: 101, position: victim.position, value: 0, tick: tick(t) });
      const phase = kind === "tired_announcer-a" ? "announce" : "active";
      return { ships, entities: [], shots, mood: mood("tired_announcer", phase, tick(t), 20), cues, own: null, ext: [] };
    };
  }
  return none;
}

/** The hat sheet: the eight styles in two rows, upright, slowly turning. */
function hatSheet(scene: SceneInstances, pose: Pose, time: number): void {
  const { at, u } = basis(pose);
  for (let k = 0; k < 8; k++) {
    const p = at(5.6, ((k % 4) - 1.5) * 1.75, k < 4 ? 0.55 : -1.25);
    const rot = q.mul(lookAlong(v3.scale(q.forward(pose.orientation), -1), u), q.mul(q.fromAxisAngle({ x: 1, y: 0, z: 0 }, 0.32), q.fromAxisAngle({ x: 0, y: 1, z: 0 }, 0.6 + k * 0.4 + time * 0.2)));
    const color = HAT_COLORS[k] ?? [0.6, 0.2, 0.2];
    (scene.opaque[hatMeshKind(k)] ??= []).push({ position: p, rotation: rot, scale: 1.25, color, emissive: 0 });
    if (k === 5) (scene.opaque.hat_prop ??= []).push({ position: v3.addScaled(p, q.up(rot), 0.38 * 1.25), rotation: q.mul(rot, q.fromAxisAngle({ x: 0, y: 1, z: 0 }, 0.7)), scale: 1.25, color, emissive: 0 });
  }
}

export interface SpectacleRunResult {
  particles: number | null;
  /** Called after every main render (the kill-cam window draws then). */
  afterRender: (() => void) | null;
  /** The kill-cam window's GPU time (ms) of the last frame, if any. */
  killCamMs: () => number | null;
}

/** Builds the scene into the renderer and the page. */
export function runSpectacleScene(ctx: SpectacleSceneContext): SpectacleRunResult {
  const { kind, renderer, prep, time, particles } = ctx;
  const cam = prep.pose;
  const say = (): void => {};
  const fx = new SpectacleVisuals(ctx.ui, particles, ctx.enabled, null, say);
  const cache = new Map<string, number>();
  const moment = momentFor(kind, cam, time, ctx, cache);
  if (kind === "hat_rain-b") {
    fx.hats.presetStack(100, [0]);
    fx.hats.presetStack(101, [2, 1]);
    fx.hats.presetStack(102, [4, 6, 3, 5]);
    fx.hats.presetStack(103, [7]);
    fx.hats.presetStack(0, [3, 1, 6]);
  } else if (kind === "hat_rain-c") {
    fx.hats.presetStack(100, [1]);
    fx.hats.presetStack(101, [3, 6, 2, 5, 0, 4]);
    fx.hats.presetStack(102, [7, 2]);
  }
  const history = kind.startsWith("tired_announcer") ? 6.5 : kind === "cat_paw-c" ? 3.4 : kind.startsWith("cat_paw") ? 2.6 : 3.2;
  const steps = Math.round(history * 60);
  let last: Moment | null = null;
  let frame = null as ReturnType<SpectacleVisuals["update"]> | null;
  const colorOf = (id: number): readonly [number, number, number] => teamHull(id === 0 ? 0 : (id - 100) % 4, 4);
  for (let i = 0; i <= steps; i++) {
    const t = time - history + i / 60;
    if (particles) {
      particles.now = t;
      particles.beginFrame();
    }
    const m = moment(t);
    last = m;
    const input: SpectacleInput = {
      time: t,
      dt: i === 0 ? 0 : 1 / 60,
      nowTick: Math.round(t * 60),
      camera: cam.position,
      orientation: cam.orientation,
      mood: m.mood,
      cues: m.cues,
      ships: m.ships,
      own: m.own,
      entities: m.entities,
      shots: m.shots,
      shipExt: m.ext,
      colorOf,
      nameOf: (id) => NAMES[id % 5] ?? "Bolt",
      up: UP,
      raycast: (o, d, max) => ctx.ray(o, d, max),
      quiet: false,
    };
    frame = fx.update(input);
  }
  if (!last || !frame) return { particles: null, afterRender: null, killCamMs: () => null };
  const m = last;
  const scene = buildScene(m.ships.filter((s) => s.alive), m.shots, m.entities, ctx.visuals, (id) => m.ships.find((s) => s.id === id)?.team, time, {
    vfx: particles !== null,
    teams: 4,
    skipKinds: fx.skipKinds,
  });
  fx.addToScene(scene);
  if (kind === "hat-sheet" && ctx.enabled) hatSheet(scene, cam, time);
  renderer.setScene(scene);
  renderer.setWorldVolumes(frame.volumes);
  renderer.settings.lightLevel = frame.lightLevel;
  renderer.settings.headlight *= frame.headlight;
  ctx.lights.push(...frame.lights);
  // Engines and shots light the cave a little, as in play.
  for (const s of m.ships) if (s.alive) ctx.lights.push({ position: s.position, color: [0.55, 0.75, 1], intensity: 3, range: 8 });
  if (particles) particles.now = time;
  // The event card or pill, as C1's hub draws it in play (M4 D5).
  if (kind !== "hat-sheet") showStripCard(ctx.ui, m.mood, Math.round(time * 60), m.mood.target >= 0 ? (NAMES[m.mood.target % 5] ?? "") : "");
  const visuals = ctx.visuals;
  const afterRender = kind.startsWith("tired_announcer") && ctx.enabled
    ? () =>
        fx.renderKillCam(renderer, {
          visuals,
          teamOf: (id) => m.ships.find((s) => s.id === id)?.team,
          teams: 4,
          create: (c, device) => Renderer.create(c, { device, quality: "low", particles: false, shadows: false }),
        })
    : null;
  return { particles: particles?.liveCount() ?? null, afterRender, killCamMs: () => fx.killCamGpuMs };
}
