// Gallery scenes of The Architect's client (M4 C4, milestone-4-plan.md
// wave C): the architect's free-flight view with the stroke dock and a
// placement preview, the map view with the cut, the stroke wheel and the
// slider panel; three-frame strips (tell, peak, aftermath) of a rock stroke,
// the Palette ink blackout and the Absurdity+ event pop as runners see them;
// the exit gate and the round banners; the results page is in
// attract/galleryPages.ts (`results.architect`). The scene data is pure
// (imported by scenes.ts, also from Node); `prepareArchitectScene` and
// `runArchitectScene` build them in the page through the same code play
// uses (game/architect/, ui/architect.ts, ui/modeHud.ts).

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { ArchitectCamera } from "../game/architect/camera.ts";
import { DEFAULT_CAMERA } from "../game/camera.ts";
import { architectView, ArchitectTracker, DEFAULT_ARCHITECT_RULES, strokeDef, type RoundRecord } from "../game/architect/model.ts";
import { architectLights, architectVolumes, ArchitectFx, type ArchitectSceneInput, type StrokePreview } from "../game/architect/visuals.ts";
import { aimPoint, OVERVIEW_FOG, OVERVIEW_HEADLIGHT, RUNNER_MARKER_LIFT } from "../game/architect/index.ts";
import { emptyModeState, OBJECTIVE, PLAYER_FLAG, VIEW_FLAG, type MatchResults, type ModeState, type Objective } from "../game/modeState.ts";
import { buildScene } from "../game/scene.ts";
import type { ShipState, VisualDef } from "../game/views.ts";
import type { PointLight } from "../renderer/lights.ts";
import type { ParticleSystem } from "../renderer/particles.ts";
import type { Renderer } from "../renderer/renderer.ts";
import { mulberry32 } from "../renderer/spriteAtlas.ts";
import { ArchitectHud, architectHudModel, type ArchitectBanner, type ArchitectToast } from "../ui/architect.ts";
import { ModeHud, modeHudModel, objectiveMarkers, projectMarker, type Marker } from "../ui/modeHud.ts";
import { sideCss } from "../game/sides.ts";
import type { ArchitectSceneKind } from "./architectSceneList.ts";

type Pose = { position: Vec3; orientation: Quat };
type Ray = (o: Vec3, d: Vec3, max: number) => number;

/** The tick the scenes are set at. */
const SCENE_TICK = 36_000;
const OWN = 0;
const ARCHITECT = 4;
const NAMES = new Map<number, string>([
  [0, "Scrapling"],
  [1, "Rivet"],
  [2, "Torque"],
  [3, "Sprocket"],
  [4, "Gasket"],
]);

/** A stroke on the scene's timeline: placed at `placed` s before the scene's moment. */
interface TimedStroke {
  index: number;
  pos: Vec3;
  radius: number;
  /** Seconds before the scene's end it was placed. */
  ago: number;
}

/** The scene's set-up, made before the terrain settles. */
export interface ArchitectPrep {
  pose: Pose;
  strokes: TimedStroke[];
  exit: { pos: Vec3; radius: number };
  edited: boolean;
  /** The map view's camera (overview scenes). */
  overview: ArchitectCamera | null;
  /** The picked tunnel view (the runners fly along it). */
  tunnel: Pose;
}

/** Seconds a stroke telegraphs (default rules). */
const TELEGRAPH = DEFAULT_ARCHITECT_RULES.telegraphSecs;

/** How long a stroke acts (rock 8 ticks, events 1 s, the well and the ink their rule times). */
function activeSecs(index: number): number {
  if (index === 3) return DEFAULT_ARCHITECT_RULES.wellSecs;
  if (index === 6) return DEFAULT_ARCHITECT_RULES.paletteSecs;
  if (index === 5 || index === 7) return 1;
  return 8 / 60;
}

/** Rock the Enclosure+ stroke grows (a handful of lumps on the walls inside the sphere), as the planner would. */
function rockLumps(center: Vec3, radius: number, ray: Ray, from: Vec3): Array<{ center: Vec3; radius: number }> {
  const rng = mulberry32(31);
  const out: Array<{ center: Vec3; radius: number }> = [];
  for (let i = 0; i < 64 && out.length < 14; i++) {
    const d = v3.normalize({ x: rng() * 2 - 1, y: rng() * 2 - 1, z: rng() * 2 - 1 });
    const t = ray(center, d, radius);
    if (t < 0) continue;
    const at = v3.addScaled(center, d, t);
    // Never over the viewer.
    if (v3.length(v3.sub(at, from)) < 6) continue;
    out.push({ center: v3.addScaled(at, d, -0.6), radius: 2.2 + rng() * 2.4 });
  }
  return out;
}

/**
 * Picks the scene's camera and strokes, and makes the terrain edits of the
 * aftermath frames (call before the terrain loads again).
 */
export function prepareArchitectScene(kind: ArchitectSceneKind, pose: Pose, spawn: Vec3, bounds: ArrayLike<number>, ray: Ray, edit: (add: boolean, c: Vec3, r: number) => boolean): ArchitectPrep {
  const f = q.forward(pose.orientation);
  const ahead = ray(pose.position, f, 80);
  const reach = ahead > 0 ? ahead : 40;
  const at = (d: number): Vec3 => v3.addScaled(pose.position, f, d);
  const exitAt = (d: number): Vec3 => at(Math.min(reach - 4, d));
  let strokes: TimedStroke[] = [];
  // Off to the side, so its marker stays clear of the strokes ahead.
  let exit = { pos: v3.addScaled(exitAt(60), q.right(pose.orientation), 26), radius: 6 };
  let edited = false;
  let overview: ArchitectCamera | null = null;
  let p = pose;
  const strip = /-(a|b|c)$/.exec(kind)?.[1] ?? "";
  // Strip moments: the tell (40 % into the telegraph), the peak (0.12 s after it acts), the aftermath.
  const stripAgo = (index: number): number => (strip === "a" ? TELEGRAPH * 0.4 : strip === "b" ? TELEGRAPH + (index === 6 ? 4 : 0.12) : TELEGRAPH + (index === 6 ? activeSecs(6) - 1.4 : index === 5 ? 2.2 : 2.6));
  if (kind.startsWith("architect-rock")) {
    const d = Math.min(24, Math.max(13, reach - 8));
    const c = at(d);
    strokes = [{ index: 0, pos: c, radius: 12, ago: stripAgo(0) }];
    if (strip !== "a") {
      const lumps = rockLumps(c, 12, ray, pose.position);
      const n = strip === "b" ? Math.ceil(lumps.length / 2) : lumps.length;
      for (const l of lumps.slice(0, n)) edited = edit(true, l.center, l.radius) || edited;
    }
  } else if (kind.startsWith("architect-ink")) {
    strokes = [{ index: 6, pos: at(Math.min(22, Math.max(12, reach - 8))), radius: 13, ago: stripAgo(6) }];
  } else if (kind.startsWith("architect-event")) {
    strokes = [{ index: 5, pos: at(Math.min(22, Math.max(12, reach - 8))), radius: 12, ago: stripAgo(5) }];
  } else if (kind === "architect-exit") {
    exit = { pos: exitAt(Math.max(12, reach * 0.75)), radius: 6 };
  } else if (kind.startsWith("architect-overview")) {
    const cam = new ArchitectCamera(pose);
    cam.toggle(bounds);
    cam.pitch = 1.2;
    // Between the runners' start and the level's middle.
    const mid = { x: ((bounds[0] ?? 0) + (bounds[3] ?? 0)) / 2, z: ((bounds[2] ?? 0) + (bounds[5] ?? 0)) / 2 };
    cam.target = { x: spawn.x + (mid.x - spawn.x) * 0.45, y: spawn.y - 2, z: spawn.z + (mid.z - spawn.z) * 0.45 };
    cam.update(0, idleInput(), 0, bounds);
    overview = cam;
    p = cam.view;
    // The exit and a telegraphed well somewhere on the floor ahead of the target.
    const fw = v3.normalize({ x: q.forward(cam.orientation).x, y: 0, z: q.forward(cam.orientation).z });
    const floorAt = (d: number, side: number): Vec3 => {
      const r = { x: -fw.z, y: 0, z: fw.x };
      const o = v3.add(v3.addScaled(cam.target, fw, d), v3.scale(r, side));
      const down = ray({ ...o, y: cam.target.y + 4 }, { x: 0, y: -1, z: 0 }, 30);
      return down > 0 ? { ...o, y: cam.target.y + 4 - down + 2 } : o;
    };
    exit = { pos: floorAt(52, 14), radius: 6 };
    strokes = [
      { index: 3, pos: floorAt(14, -30), radius: 15, ago: 1.1 },
      { index: 4, pos: floorAt(-14, 30), radius: 13, ago: 0.5 },
    ];
  } else if (kind === "architect-round-over") {
    exit = { pos: at(Math.max(10, Math.min(30, reach - 8))), radius: 6 };
  }
  if (kind === "architect-free" || kind === "architect-radial" || kind === "architect-panel") p = aimAtWall(pose, ray);
  return { pose: p, strokes, exit, edited, overview, tunnel: pose };
}

/**
 * The architect looks at rock 16-30 u away (where the preview lands), turned
 * as little as possible from the tunnel view.
 */
function aimAtWall(pose: Pose, ray: Ray): Pose {
  const up = q.up(pose.orientation);
  const right = q.right(pose.orientation);
  let best: { o: Quat; cost: number } | null = null;
  for (let yaw = -45; yaw <= 45; yaw += 5) {
    for (let pitch = -30; pitch <= 15; pitch += 5) {
      const o = q.normalize(q.mul(q.fromAxisAngle(right, (pitch * Math.PI) / 180), q.mul(q.fromAxisAngle(up, (yaw * Math.PI) / 180), pose.orientation)));
      const t = ray(pose.position, q.forward(o), 60);
      if (t < 16 || t > 30) continue;
      const cost = Math.abs(yaw) + Math.abs(pitch) * 1.2;
      if (!best || cost < best.cost) best = { o, cost };
    }
  }
  return best ? { position: pose.position, orientation: best.o } : pose;
}

function idleInput(): Parameters<ArchitectCamera["update"]>[1] {
  return {
    thrustForward: 0, thrustRight: 0, thrustUp: 0, roll: 0, lookX: 0, lookY: 0, boost: false, firePrimary: false, fireSecondary: false, scavenge: false, overdrive: false,
    toggleHorizonAssist: false, weaponKey: 0, cyclePrimary: 0, cycleSecondary: false, gadget: false, cyclePaintColor: false, toggleShipSelect: false, pointerLocked: true,
    scoreboard: false, toggleDebug: false, toggleReady: false, cycleTeam: false, toggleSpectate: false,
  };
}

function objective(id: number, kind: number, pos: Vec3, extra: Partial<Objective> = {}): Objective {
  return { id, kind, team: -1, holder: -1, state: 0, timerEnd: 0, radius: 0, pos, ...extra };
}

/** The mode state at tick `now` with the prep's strokes on their timeline. */
function modeAt(kind: ArchitectSceneKind, prep: ArchitectPrep, now: number, end: number): ModeState {
  const m = emptyModeState();
  m.mode = "the_architect";
  m.kind = 6;
  m.teams = 0;
  m.running = true;
  m.stage = 0;
  m.flags = VIEW_FLAG.lives;
  const over = kind === "architect-round-over";
  const exitState = (3 << 4) | (over ? 3 : 0);
  m.objectives = [objective(0, OBJECTIVE.exit, prep.exit.pos, { holder: ARCHITECT, state: exitState, radius: prep.exit.radius, timerEnd: end + (over ? 4 * 60 : 151 * 60) })];
  prep.strokes.forEach((s, n) => {
    const placed = end - Math.round(s.ago * 60);
    const apply = placed + Math.round(TELEGRAPH * 60);
    const stop = apply + Math.max(1, Math.round(activeSecs(s.index) * 60));
    if (now < placed || now >= stop) return;
    const active = now >= apply;
    m.objectives.push(objective(1 + n, OBJECTIVE.stroke, s.pos, { holder: ARCHITECT, radius: s.radius, state: s.index | ((active ? 1 : 0) << 4), timerEnd: active ? stop : apply }));
  });
  const architectView = kind === "architect-free" || kind.startsWith("architect-overview") || kind === "architect-radial" || kind === "architect-panel";
  const budget = kind.startsWith("architect-overview") ? 72 : 135;
  m.players = [
    { id: 0, side: 0, score: over ? 2 : 1, lives: 2, meter: 0, flags: architectView ? 0 : over ? PLAYER_FLAG.finished : 0 },
    { id: 1, side: 1, score: 1, lives: 1, meter: 0, flags: over ? PLAYER_FLAG.finished : 0 },
    { id: 2, side: 2, score: 0, lives: 2, meter: 0, flags: 0 },
    { id: 3, side: 3, score: 0, lives: 0, meter: 0, flags: PLAYER_FLAG.eliminated },
    { id: ARCHITECT, side: 4, score: 2, lives: 2, meter: budget, flags: PLAYER_FLAG.architect },
  ];
  return m;
}

export interface ArchitectSceneContext {
  kind: ArchitectSceneKind;
  renderer: Renderer;
  prep: ArchitectPrep;
  time: number;
  visuals: readonly VisualDef[];
  particles: ParticleSystem | null;
  ui: HTMLElement;
  canvas: HTMLCanvasElement;
  ray: Ray;
  bounds: ArrayLike<number>;
  lights: PointLight[];
}

/** Builds the scene into the renderer and the page; returns the live particle count. */
export function runArchitectScene(ctx: ArchitectSceneContext): number | null {
  const { kind, renderer, prep, time, particles } = ctx;
  const cam = prep.pose;
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const at = (fw: number, rt: number, up: number): Vec3 => v3.add(cam.position, v3.add(v3.scale(f, fw), v3.add(v3.scale(r, rt), v3.scale(u, up))));
  const asArchitect = kind === "architect-free" || kind.startsWith("architect-overview") || kind === "architect-radial" || kind === "architect-panel";
  const own = asArchitect ? ARCHITECT : OWN;
  const end = SCENE_TICK;
  const rules = DEFAULT_ARCHITECT_RULES;

  // Runners in view: two ahead in the tunnel (or on the map), flying away from the camera.
  const ship = (id: number, pos: Vec3, dir: Vec3, type: number, radius: number): ShipState => ({ id, team: id, alive: true, boosting: id === 1, position: pos, orientation: q.lookRotation(v3.normalize(dir), u), shipType: type, radius, gadgetActive: false });
  let ships: ShipState[] = [];
  if (kind === "architect-free" || kind === "architect-radial" || kind === "architect-panel") {
    // Along the tunnel the camera was picked for (it then turned to aim at the rock).
    const tf = q.forward(prep.tunnel.orientation);
    const tr = q.right(prep.tunnel.orientation);
    const tu = q.up(prep.tunnel.orientation);
    const tWall = ctx.ray(cam.position, tf, 60);
    const tReach = tWall > 0 ? tWall : 40;
    const tAt = (fw: number, rt: number, up: number): Vec3 => v3.add(cam.position, v3.add(v3.scale(tf, fw), v3.add(v3.scale(tr, rt), v3.scale(tu, up))));
    ships = [ship(1, tAt(Math.min(13, tReach * 0.5), 1.4, -0.5), tf, 0, 0.4), ship(2, tAt(Math.min(20, tReach * 0.75), -1.8, 0.3), v3.add(tf, v3.scale(tr, -0.3)), 1, 0.5)];
  } else if (kind.startsWith("architect-overview") && prep.overview) {
    const t = prep.overview.target;
    ships = [ship(1, { x: t.x + 6, y: t.y + 1.5, z: t.z - 4 }, f, 0, 0.4), ship(2, { x: t.x - 9, y: t.y + 2, z: t.z + 7 }, f, 1, 0.5), ship(0, { x: t.x + 15, y: t.y + 1, z: t.z + 12 }, f, 3, 0.55)];
  } else if (kind === "architect-exit") {
    ships = [ship(2, at(9, 2.5, -0.8), v3.sub(prep.exit.pos, at(9, 2.5, -0.8)), 1, 0.5)];
  }

  // Preview (architect scenes): Enclosure+ on the rock ahead, Gravity+ for the wheel.
  let preview: StrokePreview | null = null;
  const armed = kind === "architect-radial" ? 3 : kind.startsWith("architect-overview") ? 1 : 0;
  if (asArchitect) {
    const def = strokeDef(armed);
    if (def) {
      let origin = cam.position;
      const cut = prep.overview?.cutaway(ctx.bounds) ?? null;
      if (cut && f.y < -1e-3 && origin.y > cut.max.y) {
        const s = (cut.max.y - origin.y) / f.y;
        origin = v3.addScaled(origin, f, s);
      }
      const hit = ctx.ray(origin, f, 260);
      const radius = kind.startsWith("architect-overview") ? 20 : 16;
      preview = { def, center: aimPoint(origin, f, hit, armed, radius, ctx.bounds), radius, ok: true };
    }
  }

  // History: the tracker and the stroke particles over the scene's last seconds.
  const fx = particles ? new ArchitectFx(particles, mulberry32(20261006)) : null;
  const tracker = new ArchitectTracker();
  const history = Math.max(3, ...prep.strokes.map((s) => Math.min(6, s.ago + 0.5)));
  const steps = Math.round(history * 60);
  let input: ArchitectSceneInput | null = null;
  let mode = modeAt(kind, prep, end, end);
  for (let i = 0; i <= steps; i++) {
    const t = time - history + i / 60;
    const now = end - steps + i;
    if (particles) {
      particles.now = t;
      particles.beginFrame();
    }
    mode = modeAt(kind, prep, now, end);
    const view = architectView(mode);
    if (!view) break;
    const events = tracker.update(view, own, now, rules);
    input = {
      view,
      rules,
      nowTick: now,
      time: t,
      amArchitect: asArchitect,
      preview,
      runners: asArchitect ? ships.map((s) => ({ id: s.id, side: s.team, pos: s.position })) : [],
      overviewTarget: prep.overview ? prep.overview.target : null,
      camera: cam.position,
    };
    fx?.update(input, events, i === 0 ? 0 : 1 / 60);
  }
  if (!input) return null;
  renderer.cutaway = prep.overview?.cutaway(ctx.bounds) ?? null;
  if (prep.overview) {
    renderer.settings.fogDensity *= OVERVIEW_FOG;
    renderer.settings.headlight *= OVERVIEW_HEADLIGHT;
  }
  renderer.setScene(buildScene(ships, [], [], ctx.visuals, (id) => id, time, { teams: 0, vfx: particles !== null }));
  renderer.setModeVolumes(architectVolumes(input));
  architectLights(input, ctx.lights);

  // The HUD: the mode bar and alerts, the architect's dock or the runner's plate, markers.
  const modeHud = new ModeHud(ctx.ui);
  const ownPos = asArchitect ? null : cam.position;
  modeHud.update(modeHudModel({ mode, stage: "match", own, ownSide: own, nowTick: end, timeLeft: 151, names: NAMES, ownPos, spectating: asArchitect || kind === "architect-round-over", following: kind === "architect-round-over" ? 1 : null }));
  const w = ctx.canvas.clientWidth || ctx.canvas.width;
  const h = ctx.canvas.clientHeight || ctx.canvas.height;
  const viewProj = renderer.viewProjection({ ...DEFAULT_CAMERA, ...cam });
  const markers: Marker[] = objectiveMarkers(mode, own, NAMES, end, viewProj, cam.position, w, h, own);
  if (asArchitect) {
    for (const s of ships) {
      const p = projectMarker(viewProj, { ...s.position, y: s.position.y + RUNNER_MARKER_LIFT }, w, h);
      const lives = mode.players.find((x) => x.id === s.id)?.lives ?? 0;
      markers.push({ id: 1000 + s.id, kind: -1, label: NAMES.get(s.id) ?? "", detail: `${Math.round(v3.length(v3.sub(s.position, cam.position)))} u · ${lives} ${lives === 1 ? "life" : "lives"}`, color: sideCss(s.team, 0), icon: "runner", ...p });
    }
  }
  modeHud.updateMarkers(markers);
  const hud = new ArchitectHud(ctx.ui, { arm: () => {} });
  const view = input.view;
  const banner: ArchitectBanner | null =
    kind === "architect-exit"
      ? { kicker: "Round 1 of 3", title: "Reach the exit", sub: "Gasket is the architect. Watch the glowing spheres.", tone: "runner" }
      : kind === "architect-round-over"
        ? { kicker: "Round 1 over", title: "The runners win the round", sub: "2 of 4 runners escaped", tone: "win" }
        : null;
  const toast: ArchitectToast | null = kind === "architect-round-over" ? { text: "You made it out!", tone: "good" } : null;
  hud.update(
    architectHudModel({
      view,
      rules,
      own,
      nowTick: end,
      names: NAMES,
      armed: asArchitect ? armed : -1,
      radius: preview?.radius ?? 16,
      problem: null,
      aimDistance: preview ? v3.length(v3.sub(preview.center, cam.position)) : null,
      cooldownUntil: 0,
      verticalityUsed: false,
      mood: mode.mood,
      overview: prep.overview !== null,
      pointerLocked: kind !== "architect-panel",
      radial: { open: kind === "architect-radial", selected: kind === "architect-radial" ? 3 : -1 },
      banner,
      toast,
      waiting: false,
      sliders: { enclosure: 50, density: 50, verticality: 50, gravityChaos: 0, palette: 0, absurdity: 20, instability: 30 },
    }),
  );
  hud.show(true);
  if (!asArchitect && kind !== "architect-round-over") {
    // The runner's cockpit HUD would be here; the crosshair is enough for the shot.
    const cross = document.createElement("div");
    cross.className = "hud";
    const c = document.createElement("div");
    c.className = "hud-crosshair";
    cross.appendChild(c);
    ctx.ui.appendChild(cross);
  }
  return particles ? particles.liveCount() : null;
}

/** A finished Architect match for the results page (`results.architect`): four pilots, three rounds. */
export function sampleArchitectResults(): { results: MatchResults; architect: { rounds: RoundRecord[]; mode: ModeState } } {
  const names = ["Scrapling", "Rivet", "Gasket", "Torque"];
  const scores = [4, 3, 3, 1];
  const built = [2, 0, 3, 0];
  const results: MatchResults = {
    tick: 48_000,
    kind: 6,
    mode: "the_architect",
    teams: 0,
    winner: 0,
    teamScores: [],
    rows: names.map((name, id) => ({ id, side: id, kills: 0, deaths: [1, 2, 0, 4][id]!, score: scores[id]!, name })),
    awards: [],
  };
  const mode = emptyModeState();
  mode.mode = "the_architect";
  mode.running = false;
  mode.players = names.map((_, id) => ({ id, side: id, score: scores[id]!, lives: 0, meter: built[id]!, flags: 0 }));
  const rounds: RoundRecord[] = [
    { round: 0, architect: 0, runners: 3, escaped: 1, runnersWon: false },
    { round: 1, architect: 1, runners: 3, escaped: 3, runnersWon: true },
    { round: 2, architect: 2, runners: 3, escaped: 0, runnersWon: false },
  ];
  return { results, architect: { rounds, mode } };
}
