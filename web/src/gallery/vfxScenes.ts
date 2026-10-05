// Particle effects for the visual gallery (VQ5): the gallery renders one
// fixed frame, but particles are the sum of what happened before it. So the
// scenes with content are given a short synthetic history (ships flying,
// shots fired and hitting, a ship blowing up), and `simulateVfx` runs the
// normal effect code (game/vfx.ts) over it frame by frame with a seeded RNG
// up to the scene's time. The frame shown is then the same every run.
//
// The positions at the scene's time match gallery.ts `sceneContent`, so the
// meshes and the particles agree.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { NO_SHIP, type Cue } from "../game/mechanics.ts";
import { CUE_HIT, Vfx, type LightEmitter, type VfxFrame } from "../game/vfx.ts";
import type { VfxSink } from "../game/vfxLibrary.ts";
import type { ProjectileState, ShipState, VisualDef } from "../game/views.ts";
import { mulberry32 } from "../renderer/spriteAtlas.ts";
import type { GalleryScene } from "./scenes.ts";

/** Seconds of history simulated before the scene's time. */
export const HISTORY_SECS = 2.5;
/** Simulation step (s). */
export const STEP = 1 / 60;

interface Cam {
  position: Vec3;
  orientation: Quat;
}

/** What exists at time `t` of a scene's synthetic history. */
export interface Moment {
  ships: ShipState[];
  shots: ProjectileState[];
  cues: Cue[];
}

function frameOf(cam: Cam): (fw: number, rt: number, upw: number) => Vec3 {
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  return (fw, rt, upw) => v3.add(cam.position, v3.add(v3.scale(f, fw), v3.add(v3.scale(r, rt), v3.scale(u, upw))));
}

const shapeId = (visuals: readonly VisualDef[], shape: string): number => visuals.findIndex((v) => v.shape === shape);

/** The four hulls of the lineup (as in gallery.ts), hovering. */
function lineupAt(cam: Cam, radii: readonly number[]): Moment {
  const at = frameOf(cam);
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const nose = v3.normalize(v3.add(v3.scale(f, -1), v3.scale(r, 0.9)));
  const ships = radii.map((radius, i) => ({
    id: 100 + i,
    team: i % 2,
    alive: true,
    boosting: i % 2 === 0,
    position: at(4.2, (i - 1.5) * 1.7, -0.35),
    orientation: q.lookRotation(nose, u),
    shipType: i,
    radius,
    gadgetActive: false,
  }));
  return { ships, shots: [], cues: [] };
}

/**
 * The fight (gallery.ts `sceneContent` "fight") at time `t`; the scene's
 * time is `T`: ship A fires bolts at the camera, one bolt splashes on ship
 * B's shield at T - 0.1, the own previous rocket explodes at the rock at
 * T - 0.15, the next rocket left the own ship at T - 0.08.
 */
function fightAt(cam: Cam, visuals: readonly VisualDef[], blast: Vec3, t: number, T: number): Moment {
  const at = frameOf(cam);
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const a = at(7, -1.6, 0.35);
  const b = at(13, 0.4, 1.9);
  const ships: ShipState[] = [
    {
      id: 1, team: 1, alive: true, boosting: true, position: a,
      orientation: q.lookRotation(v3.add(v3.normalize(v3.sub(cam.position, a)), v3.scale(r, 1.2)), u),
      shipType: 1, radius: 0.5, gadgetActive: false,
    },
    {
      id: 2, team: 1, alive: true, boosting: false, position: b,
      orientation: q.lookRotation(v3.add(v3.scale(f, -1), v3.scale(r, 0.8)), u), shipType: 2, radius: 0.65, gadgetActive: true,
    },
  ];
  const shots: ProjectileState[] = [];
  const bolt = shapeId(visuals, "bolt");
  const rocket = shapeId(visuals, "rocket");
  const toCam = v3.normalize(v3.sub(cam.position, a));
  if (bolt >= 0) {
    // Bolts at 2.2 and 4.6 u from A at T, flying at 90 u/s.
    for (const d of [2.2, 4.6]) {
      const along = d - 90 * (T - t);
      if (along >= 0.6 && along < v3.length(v3.sub(cam.position, a)) - 1) {
        shots.push({ kind: 0, owner: 1, position: v3.addScaled(a, toCam, along), velocity: v3.scale(toCam, 90), visual: bolt, state: 0 });
      }
    }
    // One at B: it reaches the shield bubble (1.1 u around B) at T - 0.1.
    const toB = v3.normalize(v3.sub(b, a));
    const reach = v3.length(v3.sub(b, a)) - 1.1;
    const along = reach - 90 * (T - 0.1 - t);
    if (along >= 0.6 && along <= reach) shots.push({ kind: 0, owner: 1, position: v3.addScaled(a, toB, along), velocity: v3.scale(toB, 90), visual: bolt, state: 0 });
  }
  if (rocket >= 0) {
    const from = at(3.5, 0.6, -0.5);
    const towards = v3.normalize(v3.sub(blast, from));
    const span = v3.length(v3.sub(blast, from));
    // The previous rocket: launched from beside the camera, gone at T - 0.15 (at the rock).
    const launch1 = T - 0.15 - (span + 3.6) / 45;
    if (t >= launch1 && t < T - 0.15) {
      const along = -3.6 + 45 * (t - launch1);
      shots.push({ kind: 0, owner: 0, position: v3.addScaled(from, towards, Math.min(span, along)), velocity: v3.scale(towards, 45), visual: rocket, state: 0 });
    }
    // The current one: at `from` at T.
    if (t >= T - 0.08) shots.push({ kind: 0, owner: 0, position: v3.addScaled(from, towards, 45 * (t - T)), velocity: v3.scale(towards, 45), visual: rocket, state: 0 });
  }
  return { ships, shots, cues: [] };
}

/**
 * The explosion scene at `t`: a Hauler blows up at T - 0.2 ahead, a hurt
 * Tinker (it took 85 of its 100 hull at the start) trails smoke and fire.
 */
export function explosionAt(cam: Cam, t: number, T: number): Moment {
  const at = frameOf(cam);
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const hauler = at(10, 0.6, 0.5);
  const drift = v3.scale(r, -6);
  // The Tinker crosses from right to left in front of the camera.
  const tinkerPos = v3.add(at(6.5, 1.2, -1.2), v3.scale(drift, t - T));
  const ships: ShipState[] = [
    {
      id: 1, team: 0, alive: t < T - 0.2, boosting: true, position: v3.add(hauler, v3.scale(r, 2 * (t - T))),
      orientation: q.lookRotation(v3.add(v3.scale(f, -1), v3.scale(r, -0.6)), u), shipType: 2, radius: 0.65, gadgetActive: false,
    },
    {
      id: 2, team: 1, alive: true, boosting: true, position: tinkerPos,
      orientation: q.lookRotation(v3.add(v3.normalize(drift), v3.scale(f, -0.25)), u), shipType: 1, radius: 0.5, gadgetActive: false,
    },
  ];
  const cues: Cue[] = [];
  if (Math.abs(t - (T - HISTORY_SECS)) < STEP / 2) {
    cues.push({ kind: CUE_HIT, source: NO_SHIP, target: 2, position: tinkerPos, value: 85, tick: 0 });
  }
  return { ships, shots: [], cues };
}

/** Explosions per simulated frame and bolts in flight in the stress scene. */
export const STRESS_PER_FRAME = 1;
export const STRESS_BOLTS = 120;

const hash = (n: number): number => {
  const x = Math.sin(n * 12.9898 + 78.233) * 43758.5453;
  return x - Math.floor(x);
};

/**
 * The particle budget scene: an explosion every frame scattered 8-40 u
 * ahead (fire, smoke, sparks, rock dust), STRESS_BOLTS bolts with ribbon
 * trails crossing the view (each re-fired every 0.5 s, so they also hit and
 * spark) and the lineup's four hulls at full boost: about 20 k live
 * particles at the scene's time, a fight far busier than a real one.
 */
export function stressAt(cam: Cam, visuals: readonly VisualDef[], t: number, T: number): Moment {
  const at = frameOf(cam);
  const step = Math.round((t - (T - HISTORY_SECS)) / STEP);
  const cues: Cue[] = [];
  for (let k = 0; k < STRESS_PER_FRAME; k++) {
    const n = (step * STRESS_PER_FRAME + k) * 3;
    cues.push({ kind: 5, source: NO_SHIP, target: NO_SHIP, position: at(8 + 32 * hash(n), (hash(n + 1) - 0.5) * 16, (hash(n + 2) - 0.5) * 9), value: 10, tick: 0 });
  }
  const shots: ProjectileState[] = [];
  const bolt = shapeId(visuals, "bolt");
  if (bolt >= 0) {
    for (let i = 0; i < STRESS_BOLTS; i++) {
      const period = 0.5;
      const age = (t + hash(i * 7 + 1) * period) % period;
      if (age < STEP) continue;
      const from = at(6 + 30 * hash(i * 7 + 2), (hash(i * 7 + 3) - 0.5) * 30, (hash(i * 7 + 4) - 0.5) * 14);
      const dir = v3.normalize(v3.sub(at(6 + 30 * hash(i * 7 + 5), (hash(i * 7 + 6) - 0.5) * 30, 0), from));
      shots.push({ kind: 0, owner: 50 + (i % 8), position: v3.addScaled(from, dir, 90 * age), velocity: v3.scale(dir, 90), visual: bolt, state: 0 });
    }
  }
  const ships = lineupAt(cam, [0.4, 0.5, 0.65, 0.55]).ships.map((s) => ({ ...s, boosting: true }));
  return { ships, shots, cues };
}

/** The scene's synthetic state at `t` (null for scenes without particles). */
export function momentAt(
  scene: GalleryScene,
  cam: Cam,
  visuals: readonly VisualDef[],
  impact: Vec3,
  radii: readonly number[],
  t: number,
): Moment | null {
  switch (scene.content) {
    case "lineup":
      return lineupAt(cam, radii);
    case "fight":
      return fightAt(cam, visuals, impact, t, scene.time);
    case "explosion":
      return explosionAt(cam, t, scene.time);
    case "stress":
      return stressAt(cam, visuals, t, scene.time);
    default:
      return null;
  }
}

/**
 * Runs the effect library over the scene's last HISTORY_SECS with a fixed
 * seed and step, ending at the scene's time. Returns the Vfx (counts for
 * tests) or null for a scene without content.
 */
export function simulateVfx(
  sink: VfxSink,
  scene: GalleryScene,
  cam: Cam,
  visuals: readonly VisualDef[],
  impact: Vec3,
  radii: readonly number[],
  gadgetOf: (id: number) => string = () => "",
  lights: LightEmitter | null = null,
): Vfx | null {
  if (momentAt(scene, cam, visuals, impact, radii, scene.time) === null) return null;
  const vfx = new Vfx(sink, mulberry32(20261005));
  vfx.lightSink = lights;
  const steps = Math.round(HISTORY_SECS / STEP);
  for (let i = 0; i <= steps; i++) {
    const t = scene.time - HISTORY_SECS + i * STEP;
    const m = momentAt(scene, cam, visuals, impact, radii, t)!;
    const frame: VfxFrame = {
      time: t,
      dt: STEP,
      camera: cam.position,
      cameraOrientation: cam.orientation,
      player: 0,
      pose: { position: cam.position, orientation: cam.orientation, velocity: { x: 0, y: 0, z: 0 }, alive: true },
      ships: m.ships,
      shots: m.shots,
      entities: [],
      visuals,
      cues: m.cues,
      beams: [],
      flashes: [],
      gadgetOf,
      hullOf: () => 100,
    };
    // The last frame's immediate sprites (cores, flames) stay for the fixed-frame render.
    vfx.update(frame);
  }
  return vfx;
}
