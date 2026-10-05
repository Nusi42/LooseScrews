// Visual gallery page (`?gallery=<scene id>`, docs/visual-quality-plan.md VQ8):
// renders one fixed scene of scenes.ts at a fixed pose and time, for
// before/after screenshots and the GPU budget check driven by
// tools/browser-smoke/gallery.mjs.
//
// The world comes from a deterministic share code (`randomShareCode(seed, 0,
// enclosure, density)`) in offline practice with no dummies; the camera pose
// is picked from the player's spawn point by CPU ray casts against the chunk
// meshes (raycast.ts), so it follows the generator without hand-tuned
// coordinates. Ships, shots and the explosion are synthetic and drawn
// through the normal scene builder and effect timeline (game/scene.ts,
// game/mechanics.ts), so later packages that change those show up here.
//
// Progress and results are published on `window.__gallery` (GalleryStatus).
// The page keeps rendering the same frame after it is ready, so the GPU
// timing window fills.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { boundsCenter, DEFAULT_CAMERA, type CameraParams } from "../game/camera.ts";
import { probeFitRing } from "../game/scaleCues.ts";
import { teamHull } from "../game/scene.ts";
import { FitRing, HIDDEN_RING, type FitRingModel } from "../ui/fitRing.ts";
import { ChunkLoader, LOADING_BUDGET } from "../game/chunks.ts";
import { CUE, EffectsState, NO_SHIP, type Cue, type EffectsContext, type Flash } from "../game/mechanics.ts";
import { LightFx, sceneLights } from "../game/lightFx.ts";
import { buildScene } from "../game/scene.ts";
import type { PointLight } from "../renderer/lights.ts";
import { parsePose, parseVisuals, type ProjectileState, type ShipState, type VisualDef } from "../game/views.ts";
import type { GpuTimings } from "../renderer/gpuTimer.ts";
import { atmosphereFor } from "../renderer/atmosphere.ts";
import { GRADE_PRESETS } from "../renderer/post.ts";
import { hashSeed } from "../renderer/rockTextures.ts";
import { mulberry32 } from "../renderer/spriteAtlas.ts";
import { Renderer, type CameraView, type Rgb, type SceneInstances } from "../renderer/renderer.ts";
import { CONTROLS_LEN, type GameHandle } from "../wasm/api.ts";
import { loadWasm } from "../wasm/load.ts";
import { pickCloseup, pickLongest, pickVista, probeAround, raycast, rayMesh, sphereDirections, type Probe, type RayMesh } from "./raycast.ts";
import { FIT_TUNNEL_RADIUS, sceneById, type GalleryScene } from "./scenes.ts";
import { applyDecor, decorSpots, pickBorder, type DecorSpot } from "./decor.ts";
import { explosionAt, simulateVfx } from "./vfxScenes.ts";
import { pickShadePose, pickSunWallPose } from "./shadePose.ts";
import { chargeAt, cloakAt, CLOAKED } from "./vqpScenes.ts";
import { chargeLight } from "../game/chargeFx.ts";
import { modeMoment, modeScene } from "./modeScenes.ts";
import { colourMeterStyle, monochromeGrade } from "../game/modeVisuals.ts";
import { runPageScene } from "../attract/galleryPages.ts";
import { Announcer } from "../ui/announcer.ts";
import { packById } from "../audio/announcer/packs.ts";
import type { Trigger } from "../audio/announcer/lines.ts";

/** Frames rendered before the page reports ready. */
const WARMUP_FRAMES = 20;
/** Probe range for pose picking (u). */
const PROBE_RANGE = 160;
/** Probe directions around the spawn point. */
const PROBES = 160;

export interface GalleryStatus {
  scene: string;
  phase: "loading" | "ready" | "error";
  error: string | null;
  shareCode: string;
  camera: { position: Vec3; orientation: Quat } | null;
  frames: number;
  chunksDrawn: number;
  triangles: number;
  instances: number;
  canTime: boolean;
  /** Dynamic lights drawn and offered (VQ6). */
  lights: { lights: number; candidates: number } | null;
  gpu: GpuTimings | null;
  gpuError: string | null;
  /** Live particles in the frame (VQ5), null without particles. */
  particles?: number | null;
  /** VQ1 scenes with a ship: the fit ring shown and the CPU time of its probes (ms per frame, mean of 100). */
  fit: FitRingModel | null;
  probeMs: number | null;
}

/** The pre-VQ1 camera (`?view=legacy`: 75° vertical FOV, no cockpit frame or fit ring) for before shots. */
export const LEGACY_CAMERA: CameraParams = { fovY: (75 * Math.PI) / 180, near: 0.05, far: 600 };

/** `fit` scene: distance from the camera to the rock ball's face (u), the ball's radius and the tunnel depth. */
export const FIT_WALL_DISTANCE = 2.5;
export const FIT_BALL_RADIUS = 3.5;
export const FIT_TUNNEL_DEPTH = 5;

/**
 * Edits of the `fit` scene, in order: a rock ball whose face is
 * FIT_WALL_DISTANCE ahead of `eye`, then a dead-end tunnel of
 * FIT_TUNNEL_RADIUS bored into it along `dir` (overlapping spheres 0.05 u
 * apart, so its wall is smooth to 0.0005 u).
 */
export function fitTunnelEdits(eye: Vec3, dir: Vec3): { add: boolean; center: Vec3; radius: number }[] {
  const out = [{ add: true, center: v3.addScaled(eye, dir, FIT_WALL_DISTANCE + FIT_BALL_RADIUS), radius: FIT_BALL_RADIUS }];
  const from = FIT_WALL_DISTANCE - 1;
  const steps = Math.round((FIT_TUNNEL_DEPTH + 1) / 0.05);
  for (let i = 0; i <= steps; i++) {
    out.push({ add: false, center: v3.addScaled(eye, dir, from + i * 0.05), radius: FIT_TUNNEL_RADIUS });
  }
  return out;
}

declare global {
  interface Window {
    __gallery?: GalleryStatus;
  }
}

export function wantsGallery(loc: { search: string }): boolean {
  return new URLSearchParams(loc.search).has("gallery");
}

const nextFrame = (): Promise<void> => new Promise((r) => requestAnimationFrame(() => r()));

/** Camera pose for `scene` from the probes around `origin` (spawn orientation as the fallback). */
export function pickPose(
  scene: GalleryScene,
  origin: Vec3,
  fallback: Quat,
  probes: readonly Probe[],
  border: Probe | null = null,
  sky: { sun: Vec3; planet: Vec3 | null } | null = null,
): { position: Vec3; orientation: Quat } {
  const up = { x: 0, y: 1, z: 0 };
  const look = (dir: Vec3): Quat => q.lookRotation(dir, Math.abs(dir.y) > 0.95 ? { x: 1, y: 0, z: 0 } : up);
  if (scene.pose === "sun" || scene.pose === "planet") {
    const target = scene.pose === "sun" ? sky?.sun : (sky?.planet ?? sky?.sun);
    if (!target) return { position: origin, orientation: fallback };
    // Put the target up and to the right of the centre (rule of thirds).
    const side = v3.normalize(v3.cross(target, Math.abs(target.y) > 0.95 ? { x: 1, y: 0, z: 0 } : up));
    const dir = v3.normalize(v3.add(target, v3.add(v3.scale(side, -0.32), { x: 0, y: -0.16, z: 0 })));
    return { position: origin, orientation: look(dir) };
  }
  if (scene.pose === "closeup" || scene.pose === "border") {
    const p = (scene.pose === "border" ? border : null) ?? pickCloseup(probes, 9, 5);
    if (!p || !p.hit) return { position: origin, orientation: fallback };
    // Stand 3.5 u off the wall (4.5 u for a border or decor, so both sides
    // show) and look at it obliquely (25° to the side), so the relief and
    // the material borders read.
    const off = scene.pose === "border" || (scene.decor ?? "none") !== "none" ? 4.5 : 3.5;
    const position = v3.addScaled(origin, p.dir, Math.max(1, p.hit.t - off));
    const side = v3.normalize(v3.cross(p.dir, Math.abs(p.dir.y) > 0.95 ? { x: 1, y: 0, z: 0 } : up));
    const a = (25 * Math.PI) / 180;
    const dir = v3.normalize(v3.add(v3.scale(p.dir, Math.cos(a)), v3.scale(side, Math.sin(a))));
    return { position, orientation: look(dir) };
  }
  const p = scene.pose === "vista" ? pickVista(probes, 40, 150) : pickLongest(probes, PROBE_RANGE);
  return p ? { position: origin, orientation: look(p.dir) } : { position: origin, orientation: fallback };
}

/**
 * Sun pose (VQ7): a camera position a little behind a rock as seen from the
 * sun, so the rock covers the sun and its light shafts fan out past the
 * rock's edges. Tries the probe hits nearest the origin first; null when no
 * rock works (the caller falls back to looking at the sun from the origin).
 */
export function pickSunPose(
  meshes: readonly RayMesh[],
  probes: readonly Probe[],
  origin: Vec3,
  sun: Vec3,
  room: { min: Vec3; max: Vec3 } | null,
): { position: Vec3; orientation: Quat } | null {
  const up = { x: 0, y: 1, z: 0 };
  const side = v3.normalize(v3.cross(sun, Math.abs(sun.y) > 0.95 ? { x: 1, y: 0, z: 0 } : up));
  const lift = v3.cross(side, sun);
  const inside = (p: Vec3): boolean =>
    !room || (p.x > room.min.x + 3 && p.y > room.min.y + 3 && p.z > room.min.z + 3 && p.x < room.max.x - 3 && p.y < room.max.y - 3 && p.z < room.max.z - 3);
  const hits = probes
    .filter((p) => p.hit && p.hit.t > 6 && p.hit.t < 90)
    .map((p) => v3.addScaled(origin, p.dir, (p.hit?.t ?? 0) + 1.0))
    .filter(inside);
  for (const rock of hits.slice(0, 24)) {
    for (const back of [18, 28]) {
      // Slide sideways from behind the rock until the sun just clears its edge.
      for (const dir of [side, lift, v3.scale(side, -1), v3.scale(lift, -1)]) {
        for (let k = 0.5; k <= 6; k += 0.5) {
          const cam = v3.addScaled(v3.addScaled(rock, sun, -back), dir, k);
          if (!inside(cam)) break;
          const hit = raycast(meshes, cam, sun, back + 6);
          if (hit && hit.t < back - 8) break; // something else in the way
          if (hit) continue; // still behind the rock (what lies beyond it does not matter)
          // Not inside rock: open space around the camera.
          const free = sphereDirections(12).every((d) => (raycast(meshes, cam, d, 1.5) ?? { t: 2 }).t > 1.2);
          if (!free) break;
          const look = v3.normalize(v3.add(sun, v3.add(v3.scale(side, -0.22), v3.scale(lift, -0.12))));
          return { position: cam, orientation: q.lookRotation(look, Math.abs(look.y) > 0.95 ? { x: 1, y: 0, z: 0 } : up) };
        }
      }
    }
  }
  return null;
}

function shapeId(visuals: readonly VisualDef[], shape: string): number {
  return visuals.findIndex((v) => v.shape === shape);
}

/** Hitbox radius per ship type (Skitter, Tinker, Hauler, Tug), data/ships. */
export const LINEUP_RADII: readonly number[] = [0.4, 0.5, 0.65, 0.55];

/**
 * Synthetic ships, shots and effects in front of `cam` for the scene's
 * content; their dynamic lights (VQ6, game/lightFx.ts) go into `lights`.
 */
export function sceneContent(
  scene: GalleryScene,
  cam: { position: Vec3; orientation: Quat },
  visuals: readonly VisualDef[],
  impact: Vec3 | null = null,
  lights: PointLight[] = [],
  vfx = false,
  wallAhead = 12,
): SceneInstances {
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const at = (fw: number, rt: number, upw: number): Vec3 =>
    v3.add(cam.position, v3.add(v3.scale(f, fw), v3.add(v3.scale(r, rt), v3.scale(u, upw))));
  const ships: ShipState[] = [];
  const shots: ProjectileState[] = [];
  const time = scene.time;
  let gadgets = new Set<number>();
  let cloaked: ReadonlySet<number> = new Set<number>();
  let flashes: readonly Flash[] = [];
  const lightFx = new LightFx();
  lightFx.random = () => 0.5;
  if (scene.content === "shadow") {
    // Ship shadows (D5): three hulls hovering 1.2-2.4 u in front of the sunlit rock ahead.
    const d = Math.max(4, wallAhead);
    [
      { type: 2, at: at(d - 1.6, -1.2, 0.2), team: 0 },
      { type: 0, at: at(d - 1.2, 1.3, -0.7), team: 1 },
      { type: 1, at: at(d - 2.4, 0.4, 1.0), team: 0 },
    ].forEach((s, i) => {
      ships.push({
        id: 200 + i,
        team: s.team,
        alive: true,
        boosting: false,
        position: s.at,
        orientation: q.lookRotation(v3.normalize(v3.add(r, v3.scale(f, 0.35 - 0.3 * i))), u),
        shipType: s.type,
        radius: LINEUP_RADII[s.type] ?? 0.5,
        gadgetActive: false,
      });
    });
  } else if (scene.content === "lineup") {
    LINEUP_RADII.forEach((radius, i) => {
      // Three-quarter view: noses toward the camera and to the right.
      const nose = v3.normalize(v3.add(v3.scale(f, -1), v3.scale(r, 0.9)));
      ships.push({
        id: 100 + i,
        team: i % 2,
        alive: true,
        boosting: i % 2 === 0,
        position: at(4.2, (i - 1.5) * 1.7, -0.35),
        orientation: q.lookRotation(nose, u),
        shipType: i,
        radius,
        gadgetActive: false,
      });
    });
  } else if (scene.content === "fight") {
    const a = at(7, -1.6, 0.35);
    const b = at(13, 0.4, 1.9);
    ships.push({
      id: 1, team: 1, alive: true, boosting: true, position: a,
      // Banking toward the camera, its side in view.
      orientation: q.lookRotation(v3.add(v3.normalize(v3.sub(cam.position, a)), v3.scale(r, 1.2)), u),
      shipType: 1, radius: 0.5, gadgetActive: false,
    });
    ships.push({
      id: 2, team: 1, alive: true, boosting: false, position: b,
      orientation: q.lookRotation(v3.add(v3.scale(f, -1), v3.scale(r, 0.8)), u), shipType: 2, radius: 0.65, gadgetActive: true,
    });
    gadgets = new Set([2]);
    const bolt = shapeId(visuals, "bolt");
    const rocket = shapeId(visuals, "rocket");
    const toCam = v3.normalize(v3.sub(cam.position, a));
    if (bolt >= 0) {
      for (const d of [2.2, 4.6]) {
        shots.push({ kind: 0, owner: 1, position: v3.addScaled(a, toCam, d), velocity: v3.scale(toCam, 90), visual: bolt, state: 0 });
      }
    }
    // The own rocket on its way, and the previous one exploding against the
    // rock ahead and to the right (0.15 s ago); ship A's muzzle flash is fresh.
    const blast = impact ?? at(16, 4, -1);
    if (rocket >= 0) {
      const from = at(3.5, 0.6, -0.5);
      const towards = v3.normalize(v3.sub(blast, from));
      shots.push({ kind: 0, owner: 0, position: from, velocity: v3.scale(towards, 45), visual: rocket, state: 0 });
    }
    const effects = new EffectsState();
    const ctx: EffectsContext = {
      player: 0,
      camera: cam.position,
      shipPos: (id) => ships.find((s) => s.id === id)?.position,
    };
    const cue = (kind: number, source: number, position: Vec3, value: number): Cue => ({
      kind, source, target: NO_SHIP, position, value, tick: 0,
    });
    effects.apply([cue(CUE.shockwave, 0, blast, 5)], time - 0.15, ctx);
    effects.apply([cue(CUE.muzzleFlash, 1, v3.addScaled(a, toCam, 0.7), 0)], time - 0.06, ctx);
    flashes = effects.frame(time, ctx).flashes;
    lightFx.emit("explosion", blast, time - 0.15);
    lightFx.emit("muzzle", v3.addScaled(a, toCam, 0.7), time - 0.03);
  } else if (scene.content === "lights") {
    // A volley down the tunnel: bolts, orbs and pellets in every colour,
    // two rockets, one bursting against the rock, and two ships' engines.
    const shapes = ["bolt", "orb", "pellet", "spike", "arc", "blob"];
    const colors: Rgb[] = [
      [0.3, 1, 0.4], [1, 0.3, 0.2], [0.3, 0.6, 1], [1, 0.8, 0.2], [0.8, 0.3, 1], [0.2, 1, 1],
    ];
    const ids = shapes.map((shape) => shapeId(visuals, shape)).filter((i) => i >= 0);
    let n = 0;
    const count = scene.lights ?? 32;
    for (let k = 0; ids.length > 0 && n < count; k++) {
      // A loose spiral down the tunnel, 4-60 u ahead.
      const d = 4 + (k * 56) / count;
      const ang = k * 2.4;
      const rad = 1.2 + 0.25 * (k % 5);
      const pos = at(d, Math.cos(ang) * rad * 1.6, Math.sin(ang) * rad);
      const vis = ids[k % ids.length]!;
      shots.push({ kind: 0, owner: 1 + (k % 3), position: pos, velocity: v3.scale(f, 70), visual: vis, state: 0 });
      n++;
    }
    // Override the shot colours so the cave lights in many hues (the visual
    // table only has the weapons' own colours).
    const tinted = visuals.map((v) => ({ ...v }));
    shots.forEach((s, i) => {
      const c = colors[i % colors.length]!;
      tinted.push({ ...visuals[s.visual]!, color: c });
      s.visual = tinted.length - 1;
    });
    visuals = tinted;
    ships.push({
      id: 7, team: 0, alive: true, boosting: true, position: at(6, -1.3, -0.6),
      orientation: q.lookRotation(f, u), shipType: 0, radius: 0.4, gadgetActive: false,
    });
    ships.push({
      id: 8, team: 1, alive: true, boosting: false, position: at(9, 1.5, 0.5),
      orientation: q.lookRotation(v3.add(f, v3.scale(r, 0.4)), u), shipType: 3, radius: 0.55, gadgetActive: false,
    });
    lightFx.emit("explosion", impact ?? at(18, 3, -1), time - 0.12);
  } else if (scene.content === "explosion") {
    // VQ5: a ship blowing up (gone at this time), a burning one; the effects are particles (vfxScenes.ts).
    ships.push(...explosionAt(cam, time, time).ships);
  } else if (scene.content === "charge") {
    // VQP: Fusion Charger charges (the glow is particles, vfxScenes.ts; the light grows with it).
    const m = chargeAt(cam, time, time, scene.ship === undefined ? null : (LINEUP_RADII[scene.ship] ?? 0.5));
    ships.push(...m.ships);
    for (const g of m.charges) lights.push(chargeLight(g, () => 0.5));
  } else if (scene.content === "cloak") {
    ships.push(...cloakAt(cam));
    cloaked = CLOAKED;
  }
  const gadgetOf = (id: number): string => (gadgets.has(id) ? "shield" : cloaked.has(id) ? "cloak" : "");
  sceneLights({ ships, shots, visuals, flashes, time, gadgetOf }, lights);
  lightFx.frame(time, lights);
  const teamOf = (id: number): number | undefined => (id === 0 ? 0 : ships.find((s) => s.id === id)?.team);
  return buildScene(ships, shots, [], visuals, teamOf, time, {
    gadgetOf,
    flashes,
    vfx,
    // `?cloak=flat`: the pre-VQP translucent cloak (before shots).
    refractCloak: typeof location === "undefined" || new URLSearchParams(location.search).get("cloak") !== "flat",
  });
}

async function loadAround(
  loader: ChunkLoader,
  game: GameHandle,
  renderer: Renderer,
  view: CameraView,
  status: GalleryStatus,
): Promise<void> {
  for (let i = 0; i < 2000; i++) {
    const s = loader.update(game, view.position, LOADING_BUDGET);
    const stats = renderer.render(view);
    status.chunksDrawn = stats.chunksDrawn;
    if (s.pending === 0) return;
    await nextFrame();
  }
  throw new Error("terrain did not finish loading");
}

export async function runGallery(canvas: HTMLCanvasElement, ui: HTMLElement, loc: { search: string }): Promise<void> {
  const id = new URLSearchParams(loc.search).get("gallery") ?? "";
  const status: GalleryStatus = {
    scene: id,
    phase: "loading",
    error: null,
    shareCode: "",
    camera: null,
    frames: 0,
    chunksDrawn: 0,
    triangles: 0,
    instances: 0,
    canTime: false,
    lights: null,
    gpu: null,
    gpuError: null,
    fit: null,
    probeMs: null,
  };
  window.__gallery = status;
  ui.replaceChildren();
  try {
    const scene = sceneById(id);
    if (!scene) throw new Error(`unknown gallery scene "${id}"`);
    // First-impression pages (start screen, lobby, ship bay) over the live backdrop.
    if (scene.page) {
      await runPageScene(canvas, ui, scene, status);
      return;
    }
    const api = await loadWasm();
    const w = scene.world;
    status.shareCode = api.randomShareCode(w.seed, 0, w.enclosure, w.density);
    const game = api.Game.practice(status.shareCode, 0);
    game.frame(0, new Float32Array(CONTROLS_LEN));
    const spawn = parsePose(game.localPose());
    const origin = spawn?.position ?? boundsCenter(game.bounds());
    const renderer = await Renderer.create(canvas);
    renderer.detailedTiming = true;
    renderer.timeOverride = scene.time;
    renderer.settings.terrainSeed = hashSeed(status.shareCode);
    // The world's sky, fog and grade (VQ7) before its chunks arrive.
    // `?atmosphere=off`: the pre-VQ7 look (flat fog, no sky, neutral grade) for before shots.
    const atmosphere = atmosphereFor(status.shareCode, game.bounds());
    if (new URLSearchParams(loc.search).get("atmosphere") !== "off") renderer.setAtmosphere(atmosphere);
    // `?grade=<preset>`: try a colour grade (post.ts GRADE_PRESETS).
    const grade = new URLSearchParams(loc.search).get("grade");
    if (grade && grade in GRADE_PRESETS) renderer.settings.post.grade = GRADE_PRESETS[grade as keyof typeof GRADE_PRESETS];
    status.canTime = renderer.canTime;

    const rayMeshes = new Map<string, RayMesh>();
    // Paint and surface states of the scene's decor (set once the pose is known).
    let spots: DecorSpot[] = [];
    const loader = new ChunkLoader((key, mesh) => {
      renderer.setChunk(key, mesh ? applyDecor(mesh, spots) : mesh);
      if (mesh && mesh.indices.length > 0) rayMeshes.set(key, rayMesh(mesh));
      else rayMeshes.delete(key);
    });
    const legacy = new URLSearchParams(loc.search).get("view") === "legacy";
    let view: CameraView = { position: origin, orientation: spawn?.orientation ?? q.identity(), ...(legacy ? LEGACY_CAMERA : DEFAULT_CAMERA) };
    await loadAround(loader, game, renderer, view, status);

    const near = [...rayMeshes.values()].filter((m) => {
      const dx = Math.max(m.min.x - origin.x, 0, origin.x - m.max.x);
      const dy = Math.max(m.min.y - origin.y, 0, origin.y - m.max.y);
      const dz = Math.max(m.min.z - origin.z, 0, origin.z - m.max.z);
      return Math.hypot(dx, dy, dz) < PROBE_RANGE;
    });
    const probes = probeAround(near, origin, sphereDirections(PROBES), PROBE_RANGE);
    const border = scene.pose === "border" ? pickBorder(probes, near, origin) : null;
    const sunPose = scene.pose === "sun" ? pickSunPose(near, probes, origin, atmosphere.sunDirection, atmosphere.room) : null;
    const shadePose = scene.pose === "shade" ? pickShadePose([...rayMeshes.values()], probes, origin, atmosphere.sunDirection, atmosphere.room) : null;
    const wallPose = scene.pose === "sunwall" ? pickSunWallPose([...rayMeshes.values()], probes, origin, atmosphere.sunDirection, atmosphere.room) : null;
    const pose = sunPose ?? shadePose ?? wallPose ?? pickPose(scene, origin, view.orientation, probes, border, { sun: atmosphere.sunDirection, planet: atmosphere.planet?.direction ?? null });
    if (scene.belowCeiling !== undefined && atmosphere.room) {
      pose.position = { ...pose.position, y: atmosphere.room.max.y - scene.belowCeiling };
    }
    if (scene.pitch) {
      // Tilt the picked view up (or down) about its right axis.
      const tilt = q.fromAxisAngle(q.right(pose.orientation), (scene.pitch * Math.PI) / 180);
      pose.orientation = q.normalize(q.mul(tilt, pose.orientation));
    }
    // `?roomfade=off`: rock cut off sharply at the hidden walls (pre-VQP).
    renderer.roomFade = new URLSearchParams(loc.search).get("roomfade") !== "off";
    view = { ...view, ...pose };
    status.camera = pose;
    if ((scene.decor ?? "none") !== "none") {
      // Decor goes where the camera looks.
      const ahead = raycast(rayMeshes.values(), pose.position, q.forward(pose.orientation), 30);
      if (ahead) {
        const target = v3.addScaled(pose.position, q.forward(pose.orientation), ahead.t);
        spots = decorSpots(scene.decor ?? "none", target, ahead.normal);
        for (const [key, rm] of rayMeshes) renderer.setChunk(key, applyDecor(rm.mesh, spots));
      }
    }
    // Mode scenes (M3 C1): the distance to the rock ahead places the content; The Squeeze puts its shell on the terrain.
    const wallAhead = raycast(rayMeshes.values(), pose.position, q.forward(pose.orientation), 120)?.t ?? 120;
    if (scene.content === "mode" && scene.mode) {
      const shell = modeMoment(scene.mode, pose, scene.time, scene.time, wallAhead).shell;
      if (shell && game.practiceShell?.(shell.center.x, shell.center.y, shell.center.z, shell.radius)) {
        // A shell put on at once (in play it closes in steps): mesh every chunk again.
        loader.reset(game);
      }
    }
    if (scene.content === "fit") {
      const f = q.forward(pose.orientation);
      for (const e of fitTunnelEdits(pose.position, f)) game.practiceEdit?.(e.add, e.center.x, e.center.y, e.center.z, e.radius);
    }
    await loadAround(loader, game, renderer, view, status);

    // VQ1: the own ship's cockpit frame and the fit ring (not in the legacy view).
    if (scene.ship !== undefined && !legacy) {
      const radius = LINEUP_RADII[scene.ship] ?? 0.5;
      renderer.hullFrame.state = { shipType: scene.ship, radius, color: teamHull(0), glow: 0 };
      const hud = document.createElement("div");
      hud.className = "hud";
      const crosshair = document.createElement("div");
      crosshair.className = "hud-crosshair";
      hud.appendChild(crosshair);
      ui.appendChild(hud);
      const ring = new FitRing(hud);
      const probe = (): FitRingModel => probeFitRing(game, pose.position, pose.orientation, radius, view.fovY, canvas.clientHeight || canvas.height);
      const fit = probe();
      const t0 = performance.now();
      for (let i = 0; i < 100; i++) probe();
      status.probeMs = (performance.now() - t0) / 100;
      status.fit = fit;
      ring.update(fit.visible ? fit : HIDDEN_RING);
    }

    const visuals = parseVisuals(game.visuals(), api.visualShapeNames());
    // Fight: the explosion sits where a ray ahead and to the right meets rock.
    const f = q.forward(pose.orientation);
    const rt = q.right(pose.orientation);
    const impactDir = v3.normalize(v3.add(f, v3.add(v3.scale(rt, 0.35), v3.scale(q.up(pose.orientation), -0.08))));
    const hit = raycast(rayMeshes.values(), pose.position, impactDir, 40);
    const impact = hit ? v3.addScaled(pose.position, impactDir, Math.max(4, hit.t - 0.6)) : null;
    const lights: PointLight[] = [];
    const particles = renderer.particles;
    // The light scenes (VQ6) have no VQ5 history (their tinted volley is not
    // simulated), so their shots keep the mesh cores.
    if (scene.content === "mode" && scene.mode) {
      const m = modeScene(scene.mode, pose, scene.time, visuals, particles, wallAhead);
      renderer.setScene(m.scene);
      renderer.setModeVolumes(m.volumes);
      lights.push(...m.lights);
      if (m.monochrome) renderer.settings.post.grade = monochromeGrade(renderer.settings.post.grade, 1);
      // The own cockpit frame keeps its meter's colour too (D5; main.ts does the same in play).
      const own = renderer.hullFrame.state;
      if (m.monochrome && own) {
        const style = colourMeterStyle(own.color, 80);
        renderer.hullFrame.state = { ...own, color: style.color, keepColor: style.keepColor };
      }
      if (particles) status.particles = particles.liveCount();
    } else {
      renderer.setScene(sceneContent(scene, pose, visuals, impact, lights, particles !== null && scene.content !== "lights", wallAhead));
    }
    if (particles && scene.content !== "mode") {
      // VQ5: the particles of the scene's synthetic history up to its time (vfxScenes.ts).
      // Scenes whose effects come only from VQ5 also take its effect lights (VQ6
      // pulses); the fight and lights scenes emit theirs in sceneContent.
      const fallback = v3.addScaled(pose.position, impactDir, 16);
      const effectLights = scene.content === "explosion" || scene.content === "stress" ? new LightFx() : undefined;
      if (effectLights) effectLights.random = mulberry32(7);
      simulateVfx(particles, scene, pose, visuals, impact ?? fallback, LINEUP_RADII, (id) => (scene.content === "fight" && id === 2 ? "shield" : scene.content === "cloak" && CLOAKED.has(id) ? "cloak" : ""), effectLights);
      effectLights?.frame(scene.time, lights);
      status.particles = particles.liveCount();
    }
    // `?lights=0`: the same frame without dynamic lights (cost comparison).
    renderer.setLights(new URLSearchParams(loc.search).get("lights") === "0" ? [] : lights);

    const frame = (): void => {
      const stats = renderer.render(view);
      status.frames++;
      status.chunksDrawn = stats.chunksDrawn;
      status.triangles = stats.triangles;
      status.instances = stats.instances;
      status.gpu = renderer.gpuTimings();
      status.lights = renderer.lightStats;
      status.gpuError = renderer.lastError;
      if (status.frames === WARMUP_FRAMES) status.phase = "ready";
      if (!renderer.isLost) requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
    // M4 A6: an announcer caption over the frame (no sound; the first line of the trigger).
    const pack = scene.caption ? packById(scene.caption.pack) : undefined;
    if (scene.caption && pack) new Announcer(ui, null, pack, () => 0).still(scene.caption.trigger as Trigger, { killer: "Rivet", victim: "Sprocket", target: "Gasket" });
  } catch (err) {
    status.phase = "error";
    status.error = err instanceof Error ? err.message : String(err);
    console.error("gallery:", status.error);
  }
}
