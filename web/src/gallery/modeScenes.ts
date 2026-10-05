// Mode visuals for the visual gallery (M3 C1): a synthetic mode state, ships
// and history in front of the gallery camera, rendered through the normal
// mode visuals (game/modeVisuals.ts), effects (game/vfx.ts) and scene
// builder, so the shots show what a player sees in each mode.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { emptyModeState, OBJECTIVE, PLAYER_FLAG, VIEW_FLAG, type ModeState, type Objective } from "../game/modeState.ts";
import { addModeProps, CORE_STATE, modeLights, ModeVfx, modeVolumes, REACTOR_STATE, shipStyleFor, squeezeView, ZONE_WALLS, type ModeSceneInput, type Zone } from "../game/modeVisuals.ts";
import { LightFx, sceneLights } from "../game/lightFx.ts";
import { buildScene } from "../game/scene.ts";
import { Vfx, type VfxFrame } from "../game/vfx.ts";
import type { VfxSink } from "../game/vfxLibrary.ts";
import type { ShipState, VisualDef } from "../game/views.ts";
import type { PointLight } from "../renderer/lights.ts";
import type { ModeVolume } from "../renderer/modeFx.ts";
import type { SceneInstances } from "../renderer/renderer.ts";
import { mulberry32 } from "../renderer/spriteAtlas.ts";

/** Mode scenes of the gallery (scenes.ts `mode`). */
export type ModeSceneKind = "squeeze-warn" | "squeeze-close" | "heist-carrier" | "heist-dig" | "reactor" | "last-color" | "teams4";

interface Cam {
  position: Vec3;
  orientation: Quat;
}

/** The tick the scenes are set at. */
export const SCENE_TICK = 36_000;
/** Seconds of history simulated for the particles. */
export const MODE_HISTORY_SECS = 2;
const STEP = 1 / 60;

/** Everything a mode scene puts in front of the camera. */
export interface ModeMoment {
  mode: ModeState;
  nowTick: number;
  ships: ShipState[];
  /** The Squeeze's shell the terrain must carry (Game.practiceShell). */
  shell: Zone | null;
}

function frameOf(cam: Cam): (fw: number, rt: number, upw: number) => Vec3 {
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  return (fw, rt, upw) => v3.add(cam.position, v3.add(v3.scale(f, fw), v3.add(v3.scale(r, rt), v3.scale(u, upw))));
}

function objective(id: number, kind: number, pos: Vec3, extra: Partial<Objective> = {}): Objective {
  return { id, kind, team: -1, holder: -1, state: 0, timerEnd: 0, radius: 0, pos, ...extra };
}

function ship(id: number, team: number, position: Vec3, look: Vec3, up: Vec3, shipType: number, radius: number, boosting = false): ShipState {
  return { id, team, alive: true, boosting, position, orientation: q.lookRotation(v3.normalize(look), up), shipType, radius, gadgetActive: false };
}

/**
 * The mode state and ships of `kind` around `cam` at `t` seconds (ships
 * fly a little so the particle history has motion). `wall` is the distance
 * to the rock straight ahead (the Squeeze scenes put their wall before it).
 */
export function modeMoment(kind: ModeSceneKind, cam: Cam, t: number, T: number, wall = 40): ModeMoment {
  const at = frameOf(cam);
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const now = SCENE_TICK + Math.round((t - T) * 60);
  const mode = emptyModeState();
  mode.running = true;
  const ships: ShipState[] = [];
  let shell: Zone | null = null;
  const toward = (p: Vec3): Vec3 => v3.sub(cam.position, p);
  const ahead = Math.min(16, Math.max(9, wall - 4));
  if (kind === "squeeze-warn" || kind === "squeeze-close") {
    mode.mode = "the_squeeze";
    mode.teams = 0;
    mode.stage = 2;
    const R = 60;
    // The wall cuts the cave some way ahead (the tunnel pose looks down the longest view).
    const reach = Math.max(12, Math.min(34, wall * 0.6));
    // The wall's nearest point lies ahead and to the right, so it slices across part of the view.
    const toWall = v3.normalize(v3.add(v3.scale(f, 0.75), v3.scale(r, 0.66)));
    const center = v3.addScaled(cam.position, toWall, reach - R);
    if (kind === "squeeze-warn") {
      // Walls stand at the current zone; the next one is announced, 9 s to the close.
      mode.objectives = [
        objective(0, OBJECTIVE.safeZone, center, { radius: R, timerEnd: SCENE_TICK + 540, state: ZONE_WALLS }),
        objective(1, OBJECTIVE.nextZone, v3.add(v3.addScaled(center, f, 34), v3.scale(r, -5)), { radius: 24, timerEnd: SCENE_TICK + 2340 }),
      ];
      ships.push(ship(2, 2, at(reach * 0.55, 2.4, 0.9 + 0.3 * Math.sin(t)), v3.add(v3.scale(f, -1), v3.scale(r, 0.6)), u, 0, 0.4));
    } else {
      // Halfway through a close from 64 u to 56 u around the same centre: the wall stands at 60 u.
      mode.objectives = [
        objective(0, OBJECTIVE.safeZone, center, { radius: 64, timerEnd: SCENE_TICK - 900, state: ZONE_WALLS }),
        objective(1, OBJECTIVE.nextZone, center, { radius: 56, timerEnd: SCENE_TICK + 900 }),
      ];
      // A Tinker scraping along the moving wall, and a Skitter fleeing it.
      const dir = v3.normalize(v3.sub(at(reach, 2.2, 0.6), center));
      const scrape = v3.addScaled(center, dir, 60 - 0.5 - 0.15);
      ships.push(ship(2, 2, v3.add(scrape, v3.scale(r, 0.3 * (t - T))), v3.add(r, v3.scale(dir, -0.2)), u, 1, 0.5));
      ships.push(ship(3, 3, at(reach * 0.45 - 2 * (T - t), -2, -0.5), toward(at(0, -2, -0.5)), u, 0, 0.4, true));
    }
    shell = squeezeView(mode, SCENE_TICK)?.shell ?? null;
  } else if (kind === "heist-carrier") {
    mode.mode = "heist";
    mode.teams = 2;
    mode.scoreTarget = 3;
    mode.teamScores = [1, 2];
    const carrier = at(ahead * 0.55 - 1.5 * (T - t), -0.8, -0.2);
    ships.push(ship(2, 1, carrier, v3.add(toward(carrier), v3.scale(r, 2.5)), u, 2, 0.65));
    ships.push(ship(3, 1, at(ahead * 0.75 - 1.5 * (T - t), 1.8, 0.8), v3.add(v3.scale(f, -1), v3.scale(r, 0.8)), u, 0, 0.4, true));
    mode.players = [
      { id: 2, side: 1, score: 0, lives: -1, meter: 0, flags: PLAYER_FLAG.carrier },
      { id: 3, side: 1, score: 0, lives: -1, meter: 0, flags: 0 },
    ];
    mode.objectives = [
      objective(0, OBJECTIVE.core, carrier, { holder: 2, state: CORE_STATE.carried, radius: 0.8 }),
      objective(1, OBJECTIVE.extraction, at(ahead * 1.6, 3, -2.5), { team: 1, radius: 7 }),
      objective(2, OBJECTIVE.extraction, at(ahead * 3, -14, 6), { team: 0, radius: 7 }),
    ];
  } else if (kind === "heist-dig") {
    mode.mode = "heist";
    mode.teams = 2;
    // The core 1.5 u inside the rock ahead: the knot's outline glows on the wall.
    const vein = v3.addScaled(cam.position, f, wall + 1.5);
    mode.objectives = [objective(0, OBJECTIVE.core, vein, { state: CORE_STATE.embedded, radius: 0.8 })];
  } else if (kind === "reactor") {
    mode.mode = "hot_reactor";
    mode.teams = 0;
    const holder = at(ahead * 0.6 - 2 * (T - t), 0.6, -0.2);
    ships.push(ship(3, 3, holder, v3.add(toward(holder), v3.scale(r, -2)), u, 3, 0.55));
    ships.push(ship(4, 4, at(ahead * 0.45, -2.6, 0.8), v3.add(v3.scale(f, 1), v3.scale(r, 0.5)), u, 1, 0.5, true));
    mode.players = [
      { id: 3, side: 3, score: 1, lives: 2, meter: 0, flags: PLAYER_FLAG.carrier },
      { id: 4, side: 4, score: 0, lives: 3, meter: 0, flags: 0 },
    ];
    mode.flags = VIEW_FLAG.lives;
    // 6.5 s of a 20 s fuse left.
    mode.objectives = [objective(0, OBJECTIVE.reactor, holder, { holder: 3, state: REACTOR_STATE.held, radius: 15, timerEnd: SCENE_TICK + 390 })];
  } else if (kind === "last-color" || kind === "teams4") {
    const lcs = kind === "last-color";
    mode.mode = lcs ? "last_color_standing" : "team_brawl";
    mode.teams = lcs ? 0 : 4;
    mode.flags = lcs ? VIEW_FLAG.monochrome | VIEW_FLAG.meter : 0;
    const meters = [100, 66, 32, 100];
    const nose = v3.normalize(v3.add(v3.scale(f, -1), v3.scale(r, 0.9)));
    // In front of the painted wall (teams4) the ships fly lower and closer, so the paint shows.
    const d = lcs ? Math.min(4.2, wall - 1.2) : Math.min(2.6, wall - 1.6);
    [0.4, 0.5, 0.65, 0.55].forEach((radius, i) => {
      const id = 100 + i;
      ships.push(ship(id, lcs ? [0, 3, 6, 1][i]! : i, at(d, (i - 1.5) * (lcs ? 1.7 : 1.2), lcs ? -0.35 : -1.05), nose, u, i, radius, i % 2 === 0));
      mode.players.push({ id, side: i, score: 0, lives: -1, meter: meters[i]!, flags: 0 });
    });
  }
  return { mode, nowTick: now, ships, shell };
}

/** What a mode scene hands the renderer. */
export interface ModeSceneFrame {
  scene: SceneInstances;
  volumes: ModeVolume[];
  lights: PointLight[];
  monochrome: boolean;
}

/**
 * Builds the mode scene at time T: runs the effects and the mode particles
 * over MODE_HISTORY_SECS (seeded), then the scene, volumes and lights of
 * the last frame.
 */
export function modeScene(kind: ModeSceneKind, cam: Cam, T: number, visuals: readonly VisualDef[], sink: VfxSink | null, wall: number): ModeSceneFrame {
  const lightFx = new LightFx();
  lightFx.random = mulberry32(5);
  const input = (m: ModeMoment, t: number): ModeSceneInput => ({
    mode: m.mode,
    nowTick: m.nowTick,
    time: t,
    own: 0,
    ownPos: null,
    ships: m.ships,
    camera: cam.position,
  });
  if (sink) {
    const vfx = new Vfx(sink, mulberry32(20261005));
    const modeVfx = new ModeVfx(sink, mulberry32(42));
    vfx.lightSink = lightFx;
    modeVfx.lightSink = lightFx;
    const steps = Math.round(MODE_HISTORY_SECS / STEP);
    for (let i = 0; i <= steps; i++) {
      const t = T - MODE_HISTORY_SECS + i * STEP;
      const m = modeMoment(kind, cam, t, T, wall);
      const frame: VfxFrame = {
        time: t,
        dt: STEP,
        camera: cam.position,
        cameraOrientation: cam.orientation,
        player: 0,
        pose: null,
        ships: m.ships,
        shots: [],
        entities: [],
        visuals,
        cues: [],
        beams: [],
        flashes: [],
        hullOf: () => 100,
      };
      vfx.update(frame);
      modeVfx.update({ ...input(m, t), dt: STEP });
    }
  }
  const m = modeMoment(kind, cam, T, T, wall);
  const i = input(m, T);
  const scene = buildScene(m.ships, [], [], visuals, () => undefined, T, {
    teams: m.mode.teams,
    vfx: sink !== null,
    shipStyle: shipStyleFor(m.mode, m.mode.teams),
  });
  addModeProps(scene, i);
  const lights: PointLight[] = [];
  sceneLights({ ships: m.ships, objectives: m.mode.objectives, nowTick: m.nowTick, time: T }, lights);
  modeLights(i, lights);
  lightFx.frame(T, lights);
  return { scene, volumes: modeVolumes(i), lights, monochrome: (m.mode.flags & VIEW_FLAG.monochrome) !== 0 };
}
