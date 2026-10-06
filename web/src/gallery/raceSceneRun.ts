// Builds the Belt Up gallery scenes of raceScenes.ts in the page (M4 C7):
// prepareRaceScene picks the camera from the belt and probes for open
// space; runRaceScene builds a synthetic race (the own racer, rivals, start
// lights) through the client's race visuals and HUD (game/race/,
// ui/raceHud.ts) with a short history, so particles have one.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { DEFAULT_CAMERA } from "../game/camera.ts";
import { azimuth, type Belt, gateAzimuth, prograde, raceProgress, ringPoint } from "../game/race/course.ts";
import { BELT_ROCK, RaceVisuals } from "../game/race/raceFx.ts";
import type { RaceState, Racer } from "../game/modeState.ts";
import { buildScene, teamHull } from "../game/scene.ts";
import type { ShipState, VisualDef } from "../game/views.ts";
import type { PointLight } from "../renderer/lights.ts";
import type { ParticleSystem } from "../renderer/particles.ts";
import type { Renderer } from "../renderer/renderer.ts";
import { RaceHud, raceHudModel, type RaceHudInput } from "../ui/raceHud.ts";
import type { GameHandle } from "../wasm/api.ts";
import type { RaceSceneKind } from "./raceScenes.ts";

type Pose = { position: Vec3; orientation: Quat };
type Ray = (o: Vec3, d: Vec3, max: number) => number;

const UP: Vec3 = { x: 0, y: 1, z: 0 };

function look(from: Vec3, at: Vec3): Quat {
  const d = v3.normalize(v3.sub(at, from));
  return q.lookRotation(d, Math.abs(d.y) > 0.95 ? { x: 1, y: 0, z: 0 } : UP);
}

const DIRS: Vec3[] = [
  { x: 1, y: 0, z: 0 },
  { x: -1, y: 0, z: 0 },
  { x: 0, y: 1, z: 0 },
  { x: 0, y: -1, z: 0 },
  { x: 0, y: 0, z: 1 },
  { x: 0, y: 0, z: -1 },
  v3.normalize({ x: 1, y: 1, z: 1 }),
  v3.normalize({ x: -1, y: 1, z: -1 }),
  v3.normalize({ x: 1, y: -1, z: -1 }),
  v3.normalize({ x: -1, y: -1, z: 1 }),
];

/** Whether a ball of `r` at `p` is clear of rock (probe rays). */
export function clearAt(p: Vec3, r: number, ray: Ray): boolean {
  return DIRS.every((d) => {
    const t = ray(p, d, r);
    return t < 0 || t > r;
  });
}

/** The nearest clear spot to `p` within a few u (searched outward along a few directions); `p` when none is found. */
export function nearestClear(p: Vec3, r: number, ray: Ray): Vec3 {
  if (clearAt(p, r, ray)) return p;
  for (let k = 1; k <= 12; k++) {
    for (const d of DIRS) {
      const c = v3.addScaled(p, d, k * 0.8);
      if (clearAt(c, r, ray)) return c;
    }
  }
  return p;
}

/** Own racer of a scene: slot 0, gates passed. */
function sceneGates(kind: RaceSceneKind): number {
  switch (kind) {
    case "start":
    case "inside":
    case "ink":
      return 0;
    case "skim":
      return 2;
    case "open":
      return 4;
    case "overview":
      return 3;
    default:
      return 8;
  }
}

/** Camera of a race scene from the belt (before the clear-space probe), `shift` rad along the ring. */
export function racePose(kind: RaceSceneKind, b: Belt, shift = 0): Pose {
  const slot0 = b.slots[0]?.pos ?? ringPoint(b, (3 * Math.PI) / 180, 120);
  const h = b.halfThickness;
  const along = (a: number, r: number, y: number, ahead: number, dy = 0, inward = 0): Pose => {
    const p = ringPoint(b, a, r, y);
    const f = prograde(a);
    const target = v3.add(ringPoint(b, a + ahead / r, r - inward, y + dy), { x: 0, y: 0, z: 0 });
    void f;
    return { position: p, orientation: look(p, target) };
  };
  switch (kind) {
    case "start": {
      const a = azimuth(b, slot0);
      const p = v3.add(slot0, { x: 0, y: 0.25, z: 0 });
      const target = v3.add(ringPoint(b, a + 18 / 120, 120, 0), { x: 0, y: 2.6, z: 0 });
      return { position: p, orientation: look(p, target) };
    }
    case "inside":
    case "ink":
      // Approaching the next gate (gate 1) inside the belt.
      return along(gateAzimuth(1) - 22 / 112 - shift * 0.5, 112, 1.5, 30, -1);
    case "skim":
      return along(gateAzimuth(3) - 30 / 125 - shift, 125, h + 4.5, 34, h * -0.4 - 2);
    case "open":
      return along(gateAzimuth(5) - 20 / 132 - shift, 132, h + 26, 40, -18, 6);
    case "dive": {
      const a = gateAzimuth(1) + 0.15 + shift;
      const p = ringPoint(b, a, b.innerRadius - 4, 3);
      return { position: p, orientation: look(p, v3.add(b.center, { x: 0, y: 9, z: 0 })) };
    }
    case "exit":
    case "results": {
      const a = gateAzimuth(2) + 0.4 + shift;
      const p = ringPoint(b, a, 46, 2);
      return { position: p, orientation: look(p, v3.add(b.center, { x: 0, y: 11, z: 0 })) };
    }
    case "overview": {
      // The A1 `belt-overview` camera (levelScenes.ts levelPose "belt"), so before and after match.
      const dir = v3.normalize({ x: -0.62, y: 0, z: 0.78 });
      const position = v3.add(v3.addScaled(b.center, dir, b.outerRadius * 0.95), { x: 0, y: 95, z: 0 });
      const target = v3.addScaled(b.center, dir, -0.15 * b.outerRadius);
      return { position, orientation: look(position, target) };
    }
  }
}

/** A racer of the synthetic race. */
interface SceneRacer {
  id: number;
  slot: number;
  gates: number;
  place: number;
  pos: Vec3;
  fwd: Vec3;
  boosting: boolean;
  shipType: number;
}

/** Synthetic rivals placed relative to the camera for a scene (clear of rock). */
function rivals(kind: RaceSceneKind, cam: Pose, b: Belt, ray: Ray): SceneRacer[] {
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const at = (fw: number, rt: number, up: number): Vec3 => nearestClear(v3.add(cam.position, v3.add(v3.scale(f, fw), v3.add(v3.scale(r, rt), v3.scale(u, up)))), 1, ray);
  const pro = (p: Vec3): Vec3 => prograde(azimuth(b, p));
  const mk = (id: number, slot: number, gates: number, place: number, p: Vec3, fwd: Vec3, boosting: boolean, shipType: number): SceneRacer => ({ id, slot, gates, place, pos: p, fwd, boosting, shipType });
  switch (kind) {
    case "inside":
    case "ink": {
      const a = at(9, -1.4, -0.6);
      const c = at(15, 2.2, 0.8);
      return [mk(1, 7, 1, 1, c, pro(c), true, 0), mk(2, 6, 2, 2, a, pro(a), false, 2)];
    }
    case "skim": {
      const a = at(11, 1.6, -0.9);
      return [mk(1, 1, 3, 1, a, pro(a), true, 1)];
    }
    case "open": {
      const a = at(12, -2.6, -1.5);
      const c = at(26, 4, -6);
      return [mk(1, 1, 5, 1, a, pro(a), true, 3), mk(2, 2, 4, 3, c, pro(c), false, 1)];
    }
    case "dive": {
      const a = at(10, 1.8, -0.4);
      const c = at(22, -3, 1);
      return [mk(1, 4, 8, 1, a, v3.normalize(v3.sub(b.center, a)), true, 0), mk(2, 5, 8, 3, c, v3.normalize(v3.sub(b.center, c)), false, 2)];
    }
    case "exit": {
      const p = v3.add(b.center, v3.scale(v3.normalize(v3.sub(cam.position, b.center)), 11));
      const a = v3.add(p, { x: 2.5, y: 1.2, z: 0 });
      return [mk(1, 3, 8, 1, a, v3.normalize(v3.sub(b.center, a)), true, 1)];
    }
    case "overview": {
      const out: SceneRacer[] = [];
      for (let k = 1; k < 6; k++) {
        const az = gateAzimuth(k) + 0.3;
        const p = nearestClear(ringPoint(b, az, 110 + 8 * k, ((k % 3) - 1) * 4), 1, ray);
        out.push(mk(k, k, 2, k + 1, p, prograde(az), k % 2 === 0, k % 4));
      }
      return out;
    }
    default:
      return [];
  }
}

/** Set-up of a race scene: the camera (clear of rock) and its rivals. */
export interface RacePrep {
  pose: Pose;
  belt: Belt;
}

/** How open the view from `p` along `f` is: the shortest free distance of a few rays in a cone (u, capped). */
export function viewRoom(p: Vec3, f: Vec3, ray: Ray, max = 60): number {
  const side = v3.normalize(v3.cross(f, Math.abs(f.y) > 0.9 ? { x: 1, y: 0, z: 0 } : UP));
  const up = v3.cross(side, f);
  let room = max;
  for (const [a, b] of [[0, 0], [0.12, 0], [-0.12, 0], [0, 0.09], [0, -0.09]] as const) {
    const d = v3.normalize(v3.add(f, v3.add(v3.scale(side, a), v3.scale(up, b))));
    const t = ray(p, d, max);
    room = Math.min(room, t < 0 ? max : t);
  }
  return room;
}

/**
 * Picks the scene's camera from the belt: the scene's spot, or nearby along
 * the ring (a few degrees either way) where the view ahead is open, nudged
 * into open space.
 */
export function prepareRaceScene(kind: RaceSceneKind, b: Belt, ray: Ray): RacePrep {
  let pose = racePose(kind, b);
  if (kind !== "overview" && kind !== "start") {
    let best = { pose, room: -1 };
    for (const shift of [0, 0.03, -0.03, 0.06, -0.06, 0.09, -0.09, 0.12, 0.15, 0.18]) {
      const cand = racePose(kind, b, shift);
      cand.position = nearestClear(cand.position, 1.6, ray);
      const room = viewRoom(cand.position, q.forward(cand.orientation), ray);
      if (room > best.room + 2) best = { pose: cand, room };
      if (room >= 55) break;
    }
    pose = best.pose;
  }
  return { pose, belt: b };
}

export interface RaceSceneContext {
  kind: RaceSceneKind;
  renderer: Renderer;
  game: GameHandle;
  prep: RacePrep;
  time: number;
  visuals: readonly VisualDef[];
  particles: ParticleSystem | null;
  ui: HTMLElement;
  canvas: HTMLCanvasElement;
  ray: Ray;
  enabled: boolean;
  lights: PointLight[];
  shareCode: string;
}

const NAMES = ["You", "Rivet", "Sprocket", "Gasket", "Cog", "Widget", "Ratchet", "Flange"];

/** Builds the race scene in the page; returns the live particle count. */
export function runRaceScene(ctx: RaceSceneContext): number | null {
  const { kind, renderer, prep, time, particles } = ctx;
  const b = prep.belt;
  const cam = prep.pose;
  const ownGates = sceneGates(kind);
  const nowTick = 6000;
  const tickHz = 60;
  // The own racer at slot 0; rivals from the scene.
  const others = rivals(kind, cam, b, ctx.ray);
  const lightsPhase = kind === "start";
  const ownAt = kind === "overview" ? nearestClear(ringPoint(b, gateAzimuth(3) + 0.35, 118, 2), 1, ctx.ray) : cam.position;
  const racers: Racer[] = [
    { id: 0, gates: ownGates, nextGate: ownGates >= 8 ? -1 : (ownGates + 1) % 8, finishTick: kind === "exit" ? nowTick - 150 : kind === "results" ? nowTick - 60 * 8 : 0, place: 0, holdEndTick: 0, slot: 0 },
    ...others.map((o) => ({ id: o.id, gates: o.gates, nextGate: o.gates >= 8 ? -1 : (o.slot + o.gates + 1) % 8, finishTick: 0, place: o.place, holdEndTick: 0, slot: o.slot })),
  ];
  // Fill to 6 racers for the HUD and minimap: the rest of the field strung out behind on the ring.
  const fillers = new Map<number, Vec3>();
  for (let id = racers.length; id < 6; id++) {
    const slot = id;
    const gates = Math.max(0, ownGates - 1);
    const az = gateAzimuth(slot + gates) + 0.35 + 0.1 * id;
    fillers.set(id, ringPoint(b, az, 104 + 7 * id, (id % 3) - 1));
    racers.push({ id, gates, nextGate: (slot + gates + 1) % 8, finishTick: 0, place: id + 1, holdEndTick: 0, slot });
  }
  // Places by the simulation's ordering (gates, then the angle into the sector), as the server sends them.
  const posOf = (id: number): Vec3 => (id === 0 ? ownAt : (others.find((o) => o.id === id)?.pos ?? fillers.get(id) ?? b.center));
  racers
    .map((r) => ({ r, p: raceProgress(b, r, posOf(r.id)) }))
    .sort((x, y) => Number(y.r.finishTick > 0) - Number(x.r.finishTick > 0) || y.p[0] - x.p[0] || y.p[1] - x.p[1] || x.r.id - y.r.id)
    .forEach((x, i) => (x.r.place = i + 1));
  if (kind === "results") {
    // The heat is over: everyone but the last one finished, a second or a few apart.
    racers.forEach((r) => {
      if (r.id === 0) return;
      r.gates = 8;
      r.finishTick = r.id === 5 ? 0 : nowTick - 60 * 8 + [0, -95, 140, 260, 410, 0][r.id]!;
    });
    racers
      .slice()
      .sort((x, y) => (x.finishTick || 1e9) - (y.finishTick || 1e9))
      .forEach((r, i) => (r.place = i + 1));
  }
  const race: RaceState = {
    heat: 1,
    heats: 3,
    phase: lightsPhase ? "lights" : kind === "results" ? "results" : "racing",
    // Start lights: 3 of 5 lit (lamps come on every 0.5 s of the 3 s).
    phaseEndTick: lightsPhase ? nowTick + Math.round(1.25 * tickHz) : nowTick + 120 * tickHz,
    racers,
  };
  const ships: ShipState[] = others.map((o) => ({
    id: o.id,
    team: o.id,
    alive: true,
    boosting: o.boosting,
    position: o.pos,
    orientation: q.lookRotation(o.fwd, UP),
    shipType: o.shipType,
    radius: [0.4, 0.5, 0.65, 0.55][o.shipType] ?? 0.5,
    gadgetActive: false,
  }));
  const ownColor = teamHull(0, 0);
  const doseAt = (p: Vec3): number => ctx.game.doseRateAt?.(p.x, p.y, p.z) ?? 0;
  // The overview looks from far outside the ring: the own racer flies in the belt below.
  // The overview and the exit are a spectator's views (the exit: the own racer has finished).
  const ownPose = kind === "overview" || kind === "exit" || kind === "results" ? null : { position: cam.position, velocity: v3.scale(q.forward(cam.orientation), 28) };
  let frame = null as ReturnType<RaceVisuals["update"]> | null;
  const history = 1.5;
  const steps = Math.round(history * 60);
  let rs = 1;
  const rnd = (): number => {
    rs = (rs * 16807) % 2147483647;
    return rs / 2147483647;
  };
  const fxr = new RaceVisuals(particles, null, rnd);
  for (let i = 0; i <= steps; i++) {
    const t = time - history + i / 60;
    if (particles) {
      particles.now = t;
      particles.beginFrame();
    }
    frame = fxr.update({
      time: t,
      dt: i === 0 ? 0 : 1 / 60,
      nowTick: nowTick - steps + i,
      camera: cam.position,
      orientation: cam.orientation,
      belt: b,
      race,
      own: 0,
      ownPose,
      ownColor,
      ships,
      boulderPoses: ctx.game.boulderPoses ? (tick) => ctx.game.boulderPoses?.(tick) ?? new Float32Array(0) : undefined,
      doseAt,
      quiet: true,
      seed: 21,
    });
  }
  if (!frame) return null;
  const scene = buildScene(ships, [], [], ctx.visuals, (id) => id, time, { vfx: particles !== null, teams: 0 });
  if (ctx.enabled) {
    fxr.addToScene(scene);
    renderer.settings.rockColor = BELT_ROCK;
    renderer.setBelt(frame.look);
    ctx.lights.push(...frame.lights);
  }
  renderer.setScene(scene);
  if (particles) particles.now = time;
  if (!ctx.enabled) return particles?.liveCount() ?? null;

  // The race HUD over the frame.
  const hud = new RaceHud(ctx.ui);
  const names = new Map(racers.map((r) => [r.id, NAMES[r.id] ?? `Pilot ${r.id}`]));
  const input: RaceHudInput = {
    belt: b,
    race,
    own: 0,
    names,
    sides: new Map(racers.map((r) => [r.id, r.id])),
    points: new Map(racers.map((r) => [r.id, [14, 18, 9, 6, 12, 4][r.id] ?? 0])),
    positions: new Map([[0, ownAt], ...others.map((o): [number, Vec3] => [o.id, o.pos]), ...fillers]),
    ownPos: ownAt,
    ownVel: v3.scale(q.forward(cam.orientation), 28),
    nowTick,
    raceStart: lightsPhase ? 0 : nowTick - 40 * tickHz,
    dose: frame.dose,
    hull: kind === "open" ? 46 : 80,
    instagib: false,
    lights: frame.lights5,
    wrongWay: false,
    viewProj: renderer.viewProjection({ position: cam.position, orientation: cam.orientation, ...DEFAULT_CAMERA }),
    width: ctx.canvas.clientWidth,
    height: ctx.canvas.clientHeight,
    time,
    spectating: kind === "exit" || kind === "results",
  };
  hud.update(raceHudModel(input));
  return particles?.liveCount() ?? null;
}
