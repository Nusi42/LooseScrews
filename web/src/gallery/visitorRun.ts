// Builds the visitor event strips of visitorScenes.ts in the gallery page
// (M4 C2): places the scene around the picked camera (a rock wall for the
// gossip face), scripts the visitors' sim states (entities, ships, the mood
// view, outcome cues) over the scene's last seconds, runs the real visitor
// visuals (game/events/visitors.ts) through them so particles, receipts and
// hops have a history, and puts the scene, its lights and the HUD on the
// page. `?visitors=off` draws the same states without C2 (plain shapes).

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { DEFAULT_CAMERA } from "../game/camera.ts";
import type { Cue } from "../game/mechanics.ts";
import { emptyMood, type MoodState } from "../game/modeState.ts";
import { buildScene } from "../game/scene.ts";
import type { EntityState, ShipState, VisualDef } from "../game/views.ts";
import { VisitorVisuals, VISITOR_KINDS } from "../game/events/visitors.ts";
import { CUE_EVENT_OUTCOME, OUTCOME_SHOT_DOWN } from "../game/events/visitorKit.ts";
import { EVENT_IDS } from "../audio/announcer/lines.ts";
import { transformPoint } from "../math/mat4.ts";
import type { PointLight } from "../renderer/lights.ts";
import type { ParticleSystem } from "../renderer/particles.ts";
import type { Renderer } from "../renderer/renderer.ts";
import type { VisitorSceneKind } from "./visitorScenes.ts";

type Pose = { position: Vec3; orientation: Quat };
type Ray = (o: Vec3, d: Vec3, max: number) => number;

const UP: Vec3 = { x: 0, y: 1, z: 0 };
const HZ = 60;
/** Seconds of history run before the shot. */
const HISTORY = 2.6;

/** Pilot names of the strips (the announcer's still caption names "Gasket"). */
export const NAMES: ReadonlyMap<number, string> = new Map([
  [0, "You"],
  [101, "Gasket"],
  [102, "Rivet"],
  [103, "Sprocket"],
  [104, "Cog"],
]);

function look(from: Vec3, at: Vec3): Quat {
  const d = v3.normalize(v3.sub(at, from));
  return q.lookRotation(d, Math.abs(d.y) > 0.95 ? { x: 1, y: 0, z: 0 } : UP);
}

/** The strip's set-up: the camera and, for the gossip face, the rock spot and its outward normal. */
export interface VisitorPrep {
  pose: Pose;
  wall: { at: Vec3; normal: Vec3 } | null;
}

/**
 * A rock wall for the gossip face: along a horizontal direction from the
 * camera a wall 9-22 u away whose surface faces back (its normal from a
 * fan of rays); the camera then stands 10 u in front of it, a little off
 * to the side.
 */
export function prepareVisitorScene(kind: VisitorSceneKind, pose: Pose, ray: Ray): VisitorPrep {
  const prep: VisitorPrep = { pose: { position: { ...pose.position }, orientation: { ...pose.orientation } }, wall: null };
  if (kind.event !== "gossip_wall") return prep;
  let best: { at: Vec3; normal: Vec3; dir: Vec3; score: number } | null = null;
  for (let k = 0; k < 24; k++) {
    const a = (k / 24) * Math.PI * 2;
    const dir = { x: Math.cos(a), y: -0.05, z: Math.sin(a) };
    const t = ray(pose.position, dir, 30);
    if (t < 9 || t > 22) continue;
    const at = v3.addScaled(pose.position, dir, t);
    // The normal from three nearby hits.
    const side = v3.normalize(v3.cross(dir, UP));
    const p1 = v3.addScaled(pose.position, v3.normalize(v3.addScaled(dir, side, 0.06)), ray(pose.position, v3.normalize(v3.addScaled(dir, side, 0.06)), 40));
    const p2 = v3.addScaled(pose.position, v3.normalize(v3.addScaled(dir, UP, 0.06)), ray(pose.position, v3.normalize(v3.addScaled(dir, UP, 0.06)), 40));
    let normal = v3.normalize(v3.cross(v3.sub(p1, at), v3.sub(p2, at)));
    if (v3.dot(normal, dir) > 0) normal = v3.scale(normal, -1);
    const facing = -v3.dot(normal, dir);
    if (!(facing > 0.6) || Math.abs(normal.y) > 0.5) continue;
    const score = facing - Math.abs(t - 14) * 0.03;
    if (!best || score > best.score) best = { at, normal, dir, score };
  }
  if (!best) return prep;
  prep.wall = { at: best.at, normal: best.normal };
  const side = v3.normalize(v3.cross(best.normal, UP));
  const cam = v3.add(v3.addScaled(best.at, best.normal, 10), v3.add(v3.scale(side, 2.2), v3.scale(UP, 0.8)));
  prep.pose = { position: cam, orientation: look(cam, v3.addScaled(best.at, best.normal, 1.2)) };
  return prep;
}

interface Moment {
  entities: EntityState[];
  ships: ShipState[];
  mood: MoodState;
  cues: Cue[];
  /** The own pilot's id (the HUD's point of view). */
  own: number;
  ownPos: Vec3 | null;
  pickupPoints: number[];
}

function ent(id: number, kind: string, position: Vec3, rotation: Quat, state: [number, number, number, number], extra: Partial<EntityState> = {}): EntityState {
  return { id, kind, visual: 0, owner: 255, team: 255, position, velocity: { x: 0, y: 0, z: 0 }, rotation, state, radius: 0.9, ...extra };
}

function ship(id: number, position: Vec3, orientation: Quat, shipType = 1, team = 0): ShipState {
  return { id, team, alive: true, boosting: false, position, orientation, shipType, radius: [0.4, 0.5, 0.65, 0.55][shipType] ?? 0.5, gadgetActive: false };
}

function mood(event: string, phase: string, nowTick: number, secsLeft: number, target = -1, params: number[] = []): MoodState {
  return { ...emptyMood(), mood: "grumpy", eventId: event, phase, target, activeTick: 0, phaseEndTick: nowTick + Math.round(secsLeft * HZ), params };
}

function outcome(event: string, byte: number, position: Vec3, source = 255, target = 255): Cue {
  return { kind: CUE_EVENT_OUTCOME, source, target, position, value: (EVENT_IDS.indexOf(event as (typeof EVENT_IDS)[number]) << 8) | byte, tick: 0 };
}

/**
 * The scripted sim state of a strip at time `t` (the shot is at `time`):
 * every visitor where the sim would put it, cues on their frame.
 */
function moment(kind: VisitorSceneKind, prep: VisitorPrep, t: number, time: number, dt: number, visuals: readonly VisualDef[]): Moment {
  const cam = prep.pose;
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const at = (fw: number, rt: number, up: number): Vec3 => v3.add(cam.position, v3.add(v3.scale(f, fw), v3.add(v3.scale(r, rt), v3.scale(u, up))));
  const nowTick = Math.round(t * HZ);
  const left = time - t;
  const cueAt = (secsBefore: number): boolean => Math.abs(left - secsBefore) < dt / 2 + 1e-6;
  const m: Moment = { entities: [], ships: [], mood: emptyMood(), cues: [], own: 0, ownPos: cam.position, pickupPoints: [] };
  const across = q.lookRotation(r, u);
  const toward = (from: Vec3, to: Vec3): Quat => q.lookRotation(v3.sub(to, from), u);
  switch (kind.event) {
    case "tax_drone": {
      const leader = at(3.3, -1.25, -0.55);
      if (kind.frame === "a") {
        m.mood = mood("tax_drone", "announce", nowTick, 1.2, 101);
        const d = at(3.7, 0.35, 0.1);
        m.entities.push(ent(1, "drone", d, toward(d, at(-30, -6, 0)), [0, 0, 0, 100]));
      } else if (kind.frame === "b") {
        m.mood = mood("tax_drone", "active", nowTick, 14, 101);
        m.ships.push(ship(101, leader, across));
        // Pressed against the leader's flank, nudging along with it.
        const d = v3.add(at(4.6, 0.15, 0.15), v3.scale(r, Math.sin(t * 2) * 0.08));
        m.entities.push(ent(1, "drone", d, toward(d, leader), [0, 3, 1, 100], { velocity: v3.scale(r, -0.6) }));
        if (cueAt(0.3)) m.cues.push(outcome("tax_drone", 3, d, 255, 101));
      } else {
        m.mood = mood("tax_drone", "ending", nowTick, 1, 101);
        const d = at(4.8, 0.2, 0.2);
        m.ships.push(ship(101, leader, across), ship(103, at(8, 3.0, 0.8), toward(at(8, 3.0, 0.8), d), 0, 1));
        if (left > 0.3) m.entities.push(ent(1, "drone", d, toward(d, leader), [0, 4, 1, 22], { velocity: v3.scale(r, -1) }));
        if (cueAt(0.3)) m.cues.push(outcome("tax_drone", OUTCOME_SHOT_DOWN, d, 103, 101));
        const scrap = visuals.findIndex((v) => v.shape === "scrap");
        if (left <= 0.3) m.entities.push(ent(2, "scrapPile", v3.addScaled(d, u, -0.6), q.identity(), [16, 0, 0, 0], { visual: Math.max(0, scrap), radius: 0 }));
      }
      break;
    }
    case "pizza_delivery": {
      const target = at(5, -0.6, -0.7);
      if (kind.frame === "a") {
        m.mood = mood("pizza_delivery", "announce", nowTick, 1.2, 101);
        const d = at(4.4, -0.6, 0.35);
        m.entities.push(ent(1, "drone", d, toward(d, at(-20, 0, 0)), [1, 0, 0, 100], { owner: 101 }));
      } else {
        m.ships.push(ship(101, target, across, 2));
        const d = v3.add(target, v3.add(v3.scale(r, 1.7), v3.scale(u, 1.5)));
        if (kind.frame === "b") {
          m.mood = mood("pizza_delivery", "active", nowTick, 20, 101);
          const progress = Math.min(255, Math.max(0, Math.round(175 - left * 60)));
          m.entities.push(ent(1, "drone", d, toward(d, target), [1, Math.max(0, progress), 1, 100], { owner: 101 }));
        } else {
          // Delivered 0.45 s before the shot; then it flies off.
          const gone = Math.max(0, 0.45 - left);
          m.mood = left > 0.45 ? mood("pizza_delivery", "active", nowTick, 9, 101) : mood("pizza_delivery", "ending", nowTick, 1.5 - gone, 101);
          const away = v3.add(d, v3.add(v3.scale(u, gone * 3), v3.scale(r, gone * 2)));
          const working = left > 0.45;
          m.entities.push(ent(1, "drone", away, toward(away, target), [1, working ? 250 : 255, working ? 1 : 2, 100], { owner: 101, velocity: working ? { x: 0, y: 0, z: 0 } : v3.add(v3.scale(u, 3), v3.scale(r, 2)) }));
          if (cueAt(0.45)) m.cues.push(outcome("pizza_delivery", 1, target, 255, 101));
        }
      }
      break;
    }
    case "customer_survey": {
      if (kind.frame === "b") {
        // The leader's cockpit: the panel 2.5 u ahead, facing the camera.
        m.own = 101;
        m.mood = mood("customer_survey", "active", nowTick, 9, 101);
        const p = v3.add(cam.position, v3.scale(f, 2.5));
        const rot = q.normalize(q.mul(cam.orientation, q.fromAxisAngle(UP, Math.PI)));
        m.entities.push(ent(1, "drone", p, rot, [2, 0, 1, 100], { radius: 1.25 }));
        m.ships.push(ship(102, at(14, 4, 1), toward(at(14, 4, 1), cam.position), 0, 1));
        break;
      }
      const lead = at(3.4, -1.6, -0.7);
      const leadRot = q.lookRotation(v3.add(v3.scale(r, 0.55), v3.scale(f, 1)), u);
      const panel = v3.addScaled(lead, q.forward(leadRot), 2.5);
      const panelRot = q.normalize(q.mul(leadRot, q.fromAxisAngle(UP, Math.PI)));
      m.ships.push(ship(101, lead, leadRot));
      if (kind.frame === "a") {
        m.mood = mood("customer_survey", "active", nowTick, 11.5, 101);
        if (left < 0.9) m.entities.push(ent(1, "drone", panel, panelRot, [2, 0, 1, 100], { radius: 1.25 }));
      } else {
        m.mood = mood("customer_survey", "ending", nowTick, 1.2, 101);
        m.ships.push(ship(102, at(10, 3.5, 1.5), toward(at(10, 3.5, 1.5), panel), 0, 1));
        if (left > 0.55) m.entities.push(ent(1, "drone", panel, panelRot, [2, 0, 1, 100], { radius: 1.25 }));
        if (cueAt(0.55)) m.cues.push(outcome("customer_survey", 2, panel, 102, 101));
      }
      break;
    }
    case "mimic_pickups": {
      const crate = Math.max(0, visuals.findIndex((v) => v.shape === "crate"));
      const crates = [crate, crate, crate];
      // Three more crate colours if the table has them.
      let n = 0;
      for (let k = 0; k < visuals.length && n < 3; k++) if (visuals[k]?.shape === "crate") crates[n++] = k;
      const spots = [at(5, -1.6, -0.9), at(6.2, 0.2, -0.9), at(5.4, 1.9, -0.9)];
      m.pickupPoints = spots.flatMap((p) => [p.x, p.y, p.z]);
      if (kind.frame === "a") {
        m.mood = mood("mimic_pickups", "announce", nowTick, 1.2, -1, [0, 1, 2, 255, 255, 255, 255, 255]);
        spots.forEach((p, k) => m.entities.push(ent(10 + k, "weaponPickup", p, q.identity(), [0, 0, 0, 0], { visual: crates[k] ?? crate, radius: 0 })));
      } else {
        const prey = at(3.0, 1.5, -0.3);
        m.ships.push(ship(102, prey, q.lookRotation(v3.add(v3.scale(r, -1), v3.scale(f, 0.6)), u), 1, 1));
        if (kind.frame === "b") {
          m.mood = mood("mimic_pickups", "active", nowTick, 12, -1, [0, 1, 2, 255, 255, 255, 255, 255]);
          // One biting at the pilot, one hopping in from the left, one from below the camera.
          const biter = v3.add(prey, v3.add(v3.add(v3.scale(f, 1.45), v3.scale(r, -0.5)), v3.scale(u, -0.3)));
          m.entities.push(ent(20, "mimic", biter, q.lookRotation(v3.sub(prey, biter), u), [0, 0, 2 | (Math.sin(t * 6.5) > 0.2 ? 4 : 0), 70], { visual: crates[0] ?? crate }));
          const hop = (k: number, from: Vec3): void => {
            const s = Math.min(1, Math.max(0, (t - (time - HISTORY)) / HISTORY));
            const p = v3.lerp(from, v3.add(prey, v3.add(v3.scale(f, 2.6 + k), v3.scale(r, -2.2 - 1.3 * k))), s * 0.9);
            m.entities.push(ent(21 + k, "mimic", p, q.lookRotation(v3.sub(prey, p), u), [0, 0, 2, 100], { visual: crates[k + 1] ?? crate, velocity: v3.scale(v3.normalize(v3.sub(prey, from)), 3.5) }));
          };
          hop(0, at(9, -5, -0.9));
          hop(1, at(11, -1.5, -1.1));
        } else {
          m.mood = mood("mimic_pickups", "active", nowTick, 6, -1, [0, 255, 2, 255, 255, 255, 255, 255]);
          const dead = at(4.6, -0.5, -0.5);
          if (left > 0.35) m.entities.push(ent(20, "mimic", dead, q.lookRotation(v3.sub(prey, dead), u), [0, 0, 2, 12], { visual: crates[0] ?? crate }));
          if (cueAt(0.35)) m.cues.push(outcome("mimic_pickups", OUTCOME_SHOT_DOWN, dead, 102));
          if (left <= 0.35) m.entities.push(ent(30, "weaponPickup", dead, q.identity(), [0, 0, 0, 0], { visual: crates[0] ?? crate, radius: 0 }));
          const home = spots[2]!;
          const p = v3.add(home, v3.scale(r, -1.2 + 0.4 * Math.sin(t)));
          m.entities.push(ent(22, "mimic", p, q.lookRotation(v3.sub(prey, p), u), [0, 0, 2, 100], { visual: crates[2] ?? crate, velocity: v3.scale(r, 1.8) }));
        }
      }
      break;
    }
    case "gossip_wall": {
      const wall = prep.wall ?? { at: at(12, 0, 0), normal: v3.scale(f, -1) };
      const faceRot = q.lookRotation(wall.normal, UP);
      const faceAt = v3.addScaled(wall.at, wall.normal, 0.4);
      m.entities.push(ent(1, "gossipFace", faceAt, faceRot, [kind.frame === "b" ? 1 : 0, 0, 0, 0], { radius: 3 }));
      if (kind.frame === "a") {
        m.mood = mood("gossip_wall", "announce", nowTick, 1.3);
      } else if (kind.frame === "b") {
        m.mood = mood("gossip_wall", "active", nowTick, 9);
        // Pilots around: one in view, two hidden behind the rock.
        m.ships.push(ship(102, at(7, 4, 2.2), q.lookRotation(r, u), 0, 1));
        m.ships.push(ship(103, v3.addScaled(wall.at, wall.normal, -14), q.identity(), 2, 1));
        m.ships.push(ship(104, at(-6, 9, -3), q.identity(), 3, 1));
      } else {
        m.mood = mood("gossip_wall", "ending", nowTick, 0.62);
      }
      break;
    }
  }
  return m;
}

export interface VisitorSceneContext {
  kind: VisitorSceneKind;
  renderer: Renderer;
  prep: VisitorPrep;
  time: number;
  visuals: readonly VisualDef[];
  particles: ParticleSystem | null;
  ui: HTMLElement;
  canvas: HTMLCanvasElement;
  enabled: boolean;
  lights: PointLight[];
}

/** Builds the strip frame into the renderer and the page; returns the live particle count. */
export function runVisitorScene(ctx: VisitorSceneContext): number | null {
  const { kind, renderer, prep, time, particles } = ctx;
  const cam = prep.pose;
  const visitors = new VisitorVisuals(particles, ctx.enabled, null, ctx.enabled ? ctx.ui : null);
  const steps = Math.round(HISTORY * HZ);
  let last: ReturnType<VisitorVisuals["update"]> | null = null;
  let m: Moment | null = null;
  const toasts: ReturnType<VisitorVisuals["update"]>["hud"]["toasts"][number][] = [];
  for (let k = 0; k <= steps; k++) {
    const t = time - HISTORY + k / HZ;
    if (particles) {
      particles.now = t;
      particles.beginFrame();
    }
    m = moment(kind, prep, t, time, 1 / HZ, ctx.visuals);
    const ships = m.ships.map((s) => ({ id: s.id, alive: s.alive, position: s.position }));
    last = visitors.update({
      time: t,
      dt: k === 0 ? 0 : 1 / HZ,
      nowTick: Math.round(t * HZ),
      camera: cam.position,
      orientation: cam.orientation,
      up: UP,
      mood: m.mood,
      cues: m.cues,
      entities: m.entities,
      ships,
      own: m.own,
      ownPosition: m.ownPos,
      names: NAMES,
      visuals: ctx.visuals,
      pickupPoints: m.pickupPoints,
      quiet: false,
    });
    // Toasts of the last 3 s stay up in the shot.
    if (time - t < 3) toasts.push(...last.hud.toasts);
  }
  if (!last || !m) return null;
  const moment0 = m;
  const shown = last.hidden.size > 0 ? moment0.entities.filter((e) => !last.hidden.has(e.id)) : moment0.entities;
  // Without C2 the visitors fall back to the scene builder's shapes (debris rocks, the pickup's look).
  const scene = buildScene(moment0.ships, [], ctx.enabled ? shown : shown.map((e) => (VISITOR_KINDS.has(e.kind) ? { ...e, visual: e.kind === "mimic" ? e.visual : 0 } : e)), ctx.visuals, (id) => moment0.ships.find((s) => s.id === id)?.team, time, {
    vfx: particles !== null,
    teams: 2,
    skipKinds: visitors.skipKinds,
  });
  visitors.addToScene(scene);
  ctx.lights.push(...last.lights);
  renderer.setScene(scene);
  if (particles) particles.now = time;
  if (!ctx.enabled) return particles?.liveCount() ?? null;
  const vp = renderer.viewProjection({ position: cam.position, orientation: cam.orientation, ...DEFAULT_CAMERA });
  const w = ctx.canvas.clientWidth || ctx.canvas.width;
  const h = ctx.canvas.clientHeight || ctx.canvas.height;
  visitors.updateHud({ ...last.hud, toasts }, (p) => {
    const [x, y, , cw] = transformPoint(vp, p);
    return { x, y, w: cw };
  }, w, h, time);
  return particles?.liveCount() ?? null;
}
