// Builds the M4 B5 world scenes of moodScenes.ts in the gallery page
// (gallery.ts calls prepareWorldScene before the terrain settles and
// runWorldScene once it has): picks the camera and the cave-in spot, makes
// the rubble's terrain edits, runs the world visuals (game/worldFx.ts,
// game/moodFx.ts) through the scene's last seconds so particles have a
// history, and puts the result, the synthetic ships and pickups and the HUD
// pieces on the page. `?worldfx=off` draws the same frame without B5.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { Cue } from "../game/mechanics.ts";
import { emptyMood, type MoodId, type MoodState } from "../game/modeState.ts";
import { buildScene } from "../game/scene.ts";
import { DEFAULT_CAMERA } from "../game/camera.ts";
import type { EntityState, ShipState, VisualDef } from "../game/views.ts";
import { parseWells, shipPull, pullModel, WorldVisuals, WORLD_CUE } from "../game/worldFx.ts";
import type { PointLight } from "../renderer/lights.ts";
import type { ParticleSystem } from "../renderer/particles.ts";
import type { Instance, Renderer } from "../renderer/renderer.ts";
import { MoodBadge, PullIndicator } from "../ui/worldHud.ts";
import { NameTags, nameTagModel } from "../ui/nameTags.ts";
import type { GameHandle } from "../wasm/api.ts";
import { BORED_TURN_FRAMES, type WorldSceneKind } from "./moodScenes.ts";

type Pose = { position: Vec3; orientation: Quat };
type Ray = (o: Vec3, d: Vec3, max: number) => number;

const UP: Vec3 = { x: 0, y: 1, z: 0 };

function look(from: Vec3, at: Vec3): Quat {
  const d = v3.normalize(v3.sub(at, from));
  return q.lookRotation(d, Math.abs(d.y) > 0.95 ? { x: 1, y: 0, z: 0 } : UP);
}

/** The scene's set-up: its camera, a cave-in spot and its ceiling, whether terrain was edited. */
export interface WorldPrep {
  pose: Pose;
  spot: { at: Vec3; ceiling: Vec3 } | null;
  edited: boolean;
  /** The first well (x, y, z, core radius) of the level, if any. */
  well: { center: Vec3; core: number } | null;
}

/** Horizontal directions around the camera (16), for the scenes that need a level view. */
function horizontals(): Vec3[] {
  return Array.from({ length: 16 }, (_, i) => ({ x: Math.cos((i / 16) * Math.PI * 2), y: 0, z: Math.sin((i / 16) * Math.PI * 2) }));
}

/**
 * A cave-in spot seen from inside a chamber: along some horizontal direction
 * from `origin`, a point 9-20 u out in open space with a ceiling 3.5-8 u
 * above it and a floor below; the camera stands 9 u back toward the origin.
 */
export function pickCaveInSpot(spawn: Vec3, ray: Ray): { at: Vec3; ceiling: Vec3; camera: Vec3 } | null {
  // From the middle of the room the spawn stands in (spawns sit near the floor).
  const above = ray(spawn, UP, 12);
  const origin = v3.addScaled(spawn, UP, above > 0 ? Math.min(1.5, above * 0.4) : 1.5);
  let best: { at: Vec3; ceiling: Vec3; camera: Vec3; score: number } | null = null;
  for (const dir of horizontals()) {
    const open = ray(origin, dir, 40);
    for (let d = 6; d <= 24; d += 1) {
      if (open > 0 && d > open - 2.5) break;
      const at = v3.addScaled(origin, dir, d);
      const c = ray(at, UP, 24);
      if (c < 1.2 || c > 8) continue;
      const score = Math.abs(c - 3.5) + Math.abs(d - 12) * 0.3;
      if (!best || score < best.score) best = { at, ceiling: v3.addScaled(at, UP, c), camera: v3.addScaled(v3.addScaled(origin, dir, Math.max(0, d - 12)), UP, -0.5), score };
    }
  }
  return best ? { at: best.at, ceiling: best.ceiling, camera: best.camera } : null;
}

/** The most open level view from `origin` (a horizontal direction, tilted down a little). */
export function openView(origin: Vec3, ray: Ray): Quat {
  let best = horizontals()[0]!;
  let far = -1;
  for (const d of horizontals()) {
    const t = ray(origin, d, 30);
    const open = t < 0 ? 30 : t;
    if (open > far) {
      far = open;
      best = d;
    }
  }
  return q.lookRotation(v3.normalize({ x: best.x, y: -0.12, z: best.z }), UP);
}

/** Rubble of the `rubble` scene: a hollow in the ceiling and spheres hanging from it (like ls_sim::instability). */
export function rubbleEdits(spot: { at: Vec3; ceiling: Vec3 }): Array<{ add: boolean; center: Vec3; radius: number }> {
  const out: Array<{ add: boolean; center: Vec3; radius: number }> = [];
  out.push({ add: false, center: v3.addScaled(spot.ceiling, UP, 1.6), radius: 3.2 });
  const h = (i: number, k: number): number => {
    const x = Math.sin(i * 91.7 + k * 37.3) * 43758.5453;
    return x - Math.floor(x);
  };
  const drop = v3.length(v3.sub(spot.ceiling, spot.at));
  for (let i = 0; i < 9; i++) {
    const r = 0.9 + 0.9 * h(i, 1);
    const a = h(i, 2) * Math.PI * 2;
    const off = 3.2 * Math.sqrt(h(i, 3));
    const down = Math.min(drop - 0.5, 0.3 + 1.6 * h(i, 4));
    out.push({ add: true, center: { x: spot.ceiling.x + Math.cos(a) * off, y: spot.ceiling.y - down, z: spot.ceiling.z + Math.sin(a) * off }, radius: r });
  }
  return out;
}

export function prepareWorldScene(kind: WorldSceneKind, pose: Pose, wells: ArrayLike<number> | null, ray: Ray, edit: (add: boolean, c: Vec3, r: number) => boolean): WorldPrep {
  const well = wells && wells.length >= 8 ? { center: { x: wells[0] ?? 0, y: wells[1] ?? 0, z: wells[2] ?? 0 }, core: wells[3] ?? 2 } : null;
  const prep: WorldPrep = { pose: { position: { ...pose.position }, orientation: { ...pose.orientation } }, spot: null, edited: false, well };
  if (kind === "wells-debris" && well) {
    const from = v3.addScaled(well.center, v3.normalize({ x: 0.8, y: 0.3, z: 0.55 }), 11);
    prep.pose = { position: from, orientation: look(from, v3.add(well.center, { x: 0, y: -0.6, z: 0 })) };
  } else if (kind === "well-lensing" && well) {
    const from = v3.addScaled(well.center, v3.normalize({ x: 0.35, y: -0.5, z: 0.6 }), 13);
    prep.pose = { position: from, orientation: look(from, well.center) };
  } else if (kind === "pigment-pickup" || kind === "pigmented-vs-grey") {
    prep.pose = { position: pose.position, orientation: openView(pose.position, ray) };
  } else if (kind === "cave-in-warning" || kind === "rubble") {
    const spot = pickCaveInSpot(pose.position, ray);
    if (spot) {
      prep.spot = { at: spot.at, ceiling: spot.ceiling };
      // Look at the rock between the spot and its ceiling.
      prep.pose = { position: spot.camera, orientation: look(spot.camera, v3.lerp(spot.at, spot.ceiling, kind === "rubble" ? 0.4 : 0.6)) };
      if (kind === "rubble") {
        for (const e of rubbleEdits(spot)) prep.edited = edit(e.add, e.center, e.radius) || prep.edited;
      }
    }
  }
  return prep;
}

/**
 * The up of a Bored level frame: +Y turned 90 degrees x progress (eased)
 * about a horizontal axis; of the four axes along and across the view the
 * one that leaves the turned sun lighting what the camera sees.
 */
export function boredUp(progress: number, pose: Pose, sun: Vec3 = { x: 0.35, y: 0.85, z: 0.4 }): Vec3 {
  const p = Math.min(1, Math.max(0, progress));
  const e = p * p * (3 - 2 * p);
  const f = q.forward(pose.orientation);
  const flat = v3.length({ x: f.x, y: 0, z: f.z }) > 1e-3 ? v3.normalize({ x: f.x, y: 0, z: f.z }) : { x: 0, y: 0, z: 1 };
  const across = { x: -flat.z, y: 0, z: flat.x };
  const axes = [flat, v3.scale(flat, -1), across, v3.scale(across, -1)];
  const s0 = v3.normalize(sun);
  let best = axes[0]!;
  let score = -Infinity;
  for (const a of axes) {
    const turned = q.rotate(q.fromAxisAngle(a, Math.PI / 2), s0);
    // Lit faces toward the camera, and the sun not behind the view's edge;
    // turning about the view (a roll) keeps the planet ahead in sight.
    const sc = -v3.dot(turned, f) + 0.3 * turned.y + (a === axes[0] || a === axes[1] ? 2 : 0);
    if (sc > score) {
      score = sc;
      best = a;
    }
  }
  return q.rotate(q.fromAxisAngle(best, (Math.PI / 2) * e), UP);
}

function moodOf(kind: WorldSceneKind): MoodId | null {
  return kind.startsWith("mood-") ? (kind.slice(5) as MoodId) : null;
}

/** Lineup positions (the gallery's `lineup` content): four hulls 4.2 u ahead. */
function lineup(cam: Pose, styleOf: (i: number) => { team: number } = (i) => ({ team: i % 2 })): ShipState[] {
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const radii = [0.4, 0.5, 0.65, 0.55];
  const nose = v3.normalize(v3.add(v3.scale(f, -1), v3.scale(r, 0.9)));
  return radii.map((radius, i) => ({
    id: 100 + i,
    team: styleOf(i).team,
    alive: true,
    boosting: i % 2 === 0,
    position: v3.add(cam.position, v3.add(v3.scale(f, 4.2), v3.add(v3.scale(r, (i - 1.5) * 1.7), v3.scale(u, -0.35)))),
    orientation: q.lookRotation(nose, u),
    shipType: i,
    radius,
    gadgetActive: false,
  }));
}

export interface WorldSceneContext {
  kind: WorldSceneKind;
  renderer: Renderer;
  game: GameHandle;
  prep: WorldPrep;
  time: number;
  visuals: readonly VisualDef[];
  particles: ParticleSystem | null;
  ui: HTMLElement;
  canvas: HTMLCanvasElement;
  ray: Ray;
  enabled: boolean;
  /** Level centre (the turn's pivot). */
  center: Vec3;
  /** The scene's lights go here (the gallery hands them to the renderer). */
  lights: PointLight[];
}

/** Builds the scene into the renderer and the page; returns the live particle count. */
export function runWorldScene(ctx: WorldSceneContext): number | null {
  const { kind, renderer, game, prep, time, particles } = ctx;
  const cam = prep.pose;
  const world = new WorldVisuals(particles, ctx.enabled, null);
  const mood: MoodState = { ...emptyMood(), mood: moodOf(kind) ?? "calm" };
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const at = (fw: number, rt: number, up: number): Vec3 => v3.add(cam.position, v3.add(v3.scale(f, fw), v3.add(v3.scale(r, rt), v3.scale(u, up))));

  let ships: ShipState[] = [];
  const entities: EntityState[] = [];
  let monochrome = false;
  let pigmented: number[] = [];
  const extra: Instance[] = [];
  if (kind === "pigment-pickup") {
    monochrome = true;
    const ent = (id: number, pos: Vec3, splash: boolean, side: number): EntityState => ({
      id, kind: "pigmentPickup", visual: 0, owner: 255, team: splash ? side : 255, position: pos, velocity: { x: 0, y: 0, z: 0 }, rotation: q.identity(), state: [splash ? 1 : 0, side, 0, 0], radius: 0,
    });
    entities.push(ent(1, at(4.6, -1.1, -0.2), false, 0), ent(2, at(6.2, 1.9, 0.3), true, 1));
    ships = [{ id: 7, team: 0, alive: true, boosting: false, position: at(9, -2.6, 0.9), orientation: q.lookRotation(v3.normalize(v3.add(r, v3.scale(f, 0.3))), u), shipType: 1, radius: 0.5, gadgetActive: false }];
  } else if (kind === "pigmented-vs-grey") {
    monochrome = true;
    ships = lineup(cam);
    pigmented = [100, 103];
  } else if (moodOf(kind)) {
    ships = lineup(cam);
  }
  const ext: number[] = [];
  for (const s of ships) ext.push(s.id, pigmented.includes(s.id) ? 1 : 0, 0);

  // Cues of the cave-in scenes, placed in the scene's history.
  const caveCueAt = kind === "cave-in-warning" ? time - 1.0 : kind === "rubble" ? time - 2.6 - 1.5 : null;
  const spot = prep.spot;
  if (kind === "mood-calm" || kind === "mood-bored" || kind === "mood-grumpy" || kind === "mood-giddy") world.mood.settle(mood.mood, time - 1.5);
  const wells = game.wells?.();
  const turn = BORED_TURN_FRAMES[kind];
  const up = turn !== undefined ? boredUp(turn, cam, renderer.settings.sunDirection) : UP;
  const history = kind === "rubble" ? 5 : 3.2;
  const steps = Math.round(history * 60);
  let last = null as ReturnType<WorldVisuals["update"]> | null;
  for (let i = 0; i <= steps; i++) {
    const t = time - history + i / 60;
    if (particles) {
      particles.now = t;
      particles.beginFrame();
    }
    const cues: Cue[] = [];
    if (caveCueAt !== null && spot && Math.abs(t - caveCueAt) < 0.5 / 60) {
      cues.push({ kind: WORLD_CUE.caveInWarning, source: 255, target: 255, position: spot.at, value: 90, tick: 0 });
    }
    last = world.update(
      {
        time: t,
        dt: i === 0 ? 0 : 1 / 60,
        nowTick: Math.round(t * 60),
        camera: cam.position,
        orientation: cam.orientation,
        wells,
        up: [up.x, up.y, up.z],
        shipExt: ext,
        monochrome,
        modeStyles: false,
        cues,
        mood,
        own: { id: 0, team: 0, position: cam.position, shipType: 1 },
        ownTeam: 0,
        ships,
        entities,
        teams: 2,
        raycast: (o, d, max) => ctx.ray(o, d, max),
        quiet: false,
      },
      () => {},
    );
  }
  if (!last) return null;
  const frame = last;
  // The cave-in's ceiling follows the scene's spot (the CPU ray against the meshes).
  const scene = buildScene(ships, [], entities, ctx.visuals, (id) => ships.find((s) => s.id === id)?.team, time, {
    vfx: particles !== null,
    teams: 2,
    shipStyle: frame.shipStyle,
    skipKinds: world.skipKinds,
  });
  const lights = ctx.lights;
  lights.push(...frame.lights);
  world.addToScene(scene, entities, time, 2, lights);
  if (kind === "wells-debris" && prep.well && ctx.enabled) {
    // Debris pulled into the swirl (the sim's debris, as it would orbit).
    for (let i = 0; i < 9; i++) {
      const a = i * 0.71 + time * 0.3;
      const rr = prep.well.core + 2.2 + (i % 4) * 1.3;
      const pos = v3.add(prep.well.center, { x: Math.cos(a) * rr, y: Math.sin(i * 1.9) * 0.6, z: Math.sin(a) * rr });
      extra.push({ position: pos, rotation: q.fromAxisAngle(v3.normalize({ x: Math.sin(i), y: 1, z: Math.cos(i) }), i * 1.3 + time), scale: 0.5 + 0.35 * ((i * 7) % 5) / 4, color: [0.42, 0.39, 0.36], emissive: 0 });
    }
  }
  if (extra.length > 0) (scene.opaque.debris ??= []).push(...extra);
  renderer.setScene(scene);
  renderer.setWorldVolumes(frame.volumes);
  renderer.setWorldUp(frame.up, ctx.center);
  renderer.settings.post.grade = world.grade(renderer.settings.post.grade);
  if (particles) {
    particles.now = time;
    particles.moteColor = frame.motes.color;
    particles.moteRainbow = frame.motes.rainbow;
  }
  if (!ctx.enabled) return particles?.liveCount() ?? null;

  // HUD pieces.
  const hud = document.createElement("div");
  hud.className = "hud";
  ctx.ui.appendChild(hud);
  if (kind === "wells-close") {
    const a = shipPull(parseWells(wells), cam.position, Math.round(time * 60), 1);
    new PullIndicator(hud).update(pullModel(a, cam.orientation, 1));
  }
  if (moodOf(kind)) new MoodBadge(hud).update(frame.mood);
  if (kind === "pigmented-vs-grey" || kind === "pigment-pickup") {
    const tags = new NameTags(ctx.ui);
    const names = ["Rivet", "Sprocket", "Gasket", "Cog"];
    const players = ships.map((s, i) => ({ id: s.id, team: s.team, shipType: s.shipType, gadget: 255, name: names[i % 4] ?? "Bolt" }));
    const vp = renderer.viewProjection({ position: cam.position, orientation: cam.orientation, ...DEFAULT_CAMERA });
    const model = nameTagModel(ships, players, cam.position, vp, ctx.canvas.clientWidth, ctx.canvas.clientHeight, 0, {
      pigmented: new Set(pigmented),
      inSight: (p) => {
        const d = v3.sub(p, cam.position);
        const len = v3.length(d);
        const hit = ctx.ray(cam.position, v3.scale(d, 1 / len), len);
        return hit < 0 || hit > len - 1;
      },
    });
    tags.update(model, 2);
  }
  return particles?.liveCount() ?? null;
}
