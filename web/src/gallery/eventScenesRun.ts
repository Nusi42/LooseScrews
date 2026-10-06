// Builds the comedy event strips of eventScenes.ts in the gallery page (M4
// C1): a synthetic match around the scene's camera (ships, shots, pickups,
// the level's wells) runs through the event visuals (game/events/) from the
// announcement up to the frame's moment at 60 Hz, so particles and props
// have their history, then the scene, the renderer settings (pixel look,
// cake wave, cutaway terrain, the turned up), the camera override and the
// HUD (banner, hints) are put in place. `?eventfx=off` draws the same match
// without the event visuals (the before shots; the banner stays).

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { CUE_HIT } from "../game/vfx.ts";
import type { Cue } from "../game/mechanics.ts";
import { emptyMood, type MoodState } from "../game/modeState.ts";
import { buildScene, teamHull } from "../game/scene.ts";
import { EventVisuals, wellPoints, type EventOutput } from "../game/events/index.ts";
import { NO_EVENT_FX, TICK_HZ, type EventFx } from "../game/events/fx.ts";
import { PROJ_CHICKEN, type EntityState, type ProjectileState, type ShipState, type VisualDef } from "../game/views.ts";
import { WorldVisuals } from "../game/worldFx.ts";
import type { PointLight } from "../renderer/lights.ts";
import type { ParticleSystem } from "../renderer/particles.ts";
import type { CameraView, Renderer } from "../renderer/renderer.ts";
import { EventHud } from "../ui/eventBanner.ts";
import type { GameHandle } from "../wasm/api.ts";
import { boredUp } from "./worldScenes.ts";
import type { EventSceneSpec } from "./eventScenes.ts";

type Pose = { position: Vec3; orientation: Quat };
type Ray = (o: Vec3, d: Vec3, max: number) => number;

/** Active phase lengths (s) and ending phases of data/events (the strips' timelines). */
const ACTIVE_SECS: Readonly<Record<string, number>> = {
  bored_level: 6,
  level_hiccups: 15,
  gravity_coffee_break: 20,
  wrong_genre: 10,
  big_head_day: 15,
  rubber_chickens: 12,
  moving_day: 2,
  everything_is_cake: 20,
};
const ENDING_SECS: Readonly<Record<string, number>> = { gravity_coffee_break: 1 };

/** The tick the strips' active phases start at. */
const ACTIVE_TICK = 6000;

export interface EventSceneContext {
  spec: EventSceneSpec;
  renderer: Renderer;
  game: GameHandle;
  pose: Pose;
  /** Shader time of the frame (s). */
  time: number;
  visuals: readonly VisualDef[];
  particles: ParticleSystem | null;
  ui: HTMLElement;
  canvas: HTMLCanvasElement;
  ray: Ray;
  enabled: boolean;
  center: Vec3;
  lights: PointLight[];
  /** Distance to the rock straight ahead (u). */
  wallAhead: number;
  /** The gallery's camera (field of view, near and far). */
  view: CameraView;
}

/** The event's fx at clock `t` (seconds since the active start), as the core would report it. */
export function scriptedFx(id: string, t: number, extra: Partial<EventFx>): EventFx {
  const active = ACTIVE_SECS[id] ?? 10;
  const ending = ENDING_SECS[id] ?? 0;
  if (t >= active + ending) return { ...NO_EVENT_FX, now: ACTIVE_TICK + Math.round(t * TICK_HZ) };
  const phase = t < 0 ? "announce" : t < active ? "active" : "ending";
  const fx: EventFx = {
    ...NO_EVENT_FX,
    event: id,
    phase,
    now: ACTIVE_TICK + Math.round(t * TICK_HZ),
    activeTick: ACTIVE_TICK,
    phaseEndTick: phase === "announce" ? ACTIVE_TICK : ACTIVE_TICK + Math.round((phase === "active" ? active : active + ending) * TICK_HZ),
    ...extra,
  };
  if (phase !== "announce") {
    if (id === "rubber_chickens") fx.chicken = true;
    if (id === "big_head_day") fx.hitScale = 3;
    if (id === "gravity_coffee_break") fx.wellScale = 0;
  }
  return fx;
}

/** The mood view the world visuals (B5: well dimming) see at clock `t`. */
function moodAt(fx: EventFx): MoodState {
  return { ...emptyMood(), eventId: fx.event, phase: fx.phase, activeTick: fx.activeTick, phaseEndTick: fx.phaseEndTick };
}

function look(from: Vec3, at: Vec3, up: Vec3 = { x: 0, y: 1, z: 0 }): Quat {
  const d = v3.normalize(v3.sub(at, from));
  return q.lookRotation(d, Math.abs(v3.dot(d, up)) > 0.95 ? { x: 1, y: 0, z: 0 } : up);
}

/** Four hulls ahead of the camera (the gallery's lineup), sides 0 and 1. */
function lineup(cam: Pose, dist = 5.5, spread = 1.9): ShipState[] {
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const radii = [0.4, 0.5, 0.65, 0.55];
  return radii.map((radius, i) => ({
    id: 100 + i,
    team: i % 2,
    alive: true,
    boosting: i % 2 === 0,
    position: v3.add(cam.position, v3.add(v3.scale(f, dist + (i % 2) * 1.2), v3.add(v3.scale(r, (i - 1.5) * spread), v3.scale(u, (i % 2 ? 0.3 : -0.4))))),
    orientation: look({ x: 0, y: 0, z: 0 }, v3.add(v3.scale(f, -1), v3.scale(r, (1.5 - i) * 0.35)), u),
    shipType: i,
    radius,
    gadgetActive: false,
  }));
}

interface ScriptedShot {
  born: number;
  from: Vec3;
  vel: Vec3;
  /** Time it hits (s on the event clock). */
  hit: number;
  visual: number;
  owner: number;
  kind: number;
}

/** Runs the scene; returns the live particle count and the camera to render instead of the picked one. */
export function runEventScene(ctx: EventSceneContext): { particles: number | null; view: CameraView | null } {
  const { spec, renderer, game, particles, visuals } = ctx;
  const id = spec.id;
  const cam: Pose = { position: { ...ctx.pose.position }, orientation: { ...ctx.pose.orientation } };
  const wells = game.wells?.() ?? null;
  const wellPts = wellPoints(wells);
  const worldUp: Vec3 = { x: 0, y: 1, z: 0 };
  // Coffee break: frame the nearest well's core and the mug on top of it.
  if (id === "gravity_coffee_break") {
    const w = wellPts.filter((x) => !x.dynamic && x.core > 0).sort((p0, p1) => v3.length(v3.sub(p0.center, cam.position)) - v3.length(v3.sub(p1.center, cam.position)))[0];
    if (w) {
      const toCore = v3.normalize(v3.sub(w.center, cam.position));
      const flat = v3.normalize({ x: toCore.x, y: 0, z: toCore.z });
      const back = v3.normalize(v3.add(v3.scale(flat, -1), { x: 0, y: -0.12, z: 0 }));
      // From the core's surface outward (the core itself is rock).
      const room = ctx.ray(v3.addScaled(w.center, back, w.core + 0.2), back, 16);
      const dist = Math.min(w.core * 4.2, (room > 0 ? room + w.core : 16) - 0.9);
      cam.position = v3.addScaled(w.center, back, Math.max(w.core * 2.2, dist));
      cam.orientation = look(cam.position, v3.addScaled(w.center, worldUp, w.core * 0.75));
    }
  }
  // Bored level: horizon assist rolls the pilot with the turning up (the turn's frames).
  if (id === "bored_level" && spec.frame !== "a") {
    const progress = spec.over !== undefined ? 1 : Math.min(1, Math.max(0, spec.t / 5));
    const turned = boredUp(progress, cam, renderer.settings.sunDirection);
    cam.orientation = q.lookRotation(q.forward(cam.orientation), turned);
  }
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const at = (fw: number, rt: number, up: number): Vec3 => v3.add(cam.position, v3.add(v3.scale(f, fw), v3.add(v3.scale(r, rt), v3.scale(u, up))));
  /** `at`, pulled back toward the camera when rock is in the way (so props float in open space). */
  const free = (fw: number, rt: number, up: number, clearance = 1.4): Vec3 => {
    const target = at(fw, rt, up);
    const d = v3.length(v3.sub(target, cam.position));
    const dir = v3.normalize(v3.sub(target, cam.position));
    const hit = ctx.ray(cam.position, dir, d + clearance);
    return hit > 0 && hit < d + clearance ? v3.addScaled(cam.position, dir, Math.max(1.5, hit - clearance)) : target;
  };
  const visualOf = (shape: string): number => Math.max(0, visuals.findIndex((v) => v.shape === shape));
  const hud = new EventHud(ctx.ui);
  const events = new EventVisuals({ particles, sound: null, hud, announcer: null, enabled: ctx.enabled });
  const world = new WorldVisuals(particles, true, null);
  const camView = (): CameraView => ({ ...ctx.view, position: cam.position, orientation: cam.orientation });

  // The synthetic match.
  let ships: ShipState[] = [];
  const shots: ScriptedShot[] = [];
  let own: { position: Vec3; orientation: Quat; shipType: number } | null = null;
  const pickupMoves: Array<{ id: number; from: Vec3; to: Vec3; kind: string; visual: number }> = [];
  let cakeOrigin: Vec3 | null = null;
  const hits: Array<{ t: number; ship: number }> = [];
  if (id === "big_head_day") ships = lineup(cam, 8, 3.2);
  if (id === "level_hiccups") ships = lineup(cam, 6.5, 2.0).map((s, i) => ({ ...s, position: free(6 + (i % 2) * 2.5, (i - 1.5) * 2.0, -0.4 + (i % 2) * 0.5, 1.2) }));
  if (id === "big_head_day") hits.push({ t: 4.0, ship: 101 }, { t: 2.0, ship: 103 });
  if (id === "rubber_chickens") {
    // Three hulls on the left fire across the view at the rock on the right: bolts, rockets, one giant egg.
    ships = [0, 1, 2].map((i) => ({
      id: 100 + i,
      team: i % 2,
      alive: true,
      boosting: true,
      position: free(8 + i * 1.6, -6.5 - i * 0.6, -1.4 + i * 1.1),
      orientation: look({ x: 0, y: 0, z: 0 }, v3.normalize(v3.add(r, v3.scale(f, 0.25))), u),
      shipType: i,
      radius: [0.4, 0.5, 0.65][i] ?? 0.5,
      gadgetActive: false,
    }));
    const volley = (k: number, born: number, speed: number, shape: string): void => {
      const s = ships[k % 3]!;
      const aim = v3.normalize(v3.add(r, v3.add(v3.scale(f, 0.2 + (0.1 * ((k * 37) % 5)) / 5), v3.scale(u, (((k * 53) % 7) / 7) * 0.16 - 0.08))));
      const from = v3.addScaled(s.position, aim, 1);
      const reach = ctx.ray(from, aim, 60);
      const dist = Math.max(4, (reach > 0 ? reach : 30) - 0.4);
      shots.push({ born, from, vel: v3.scale(aim, speed), hit: born + dist / speed, visual: visualOf(shape), owner: s.id, kind: shape === "charge" ? 9 : shape === "rocket" ? 3 : 1 });
    };
    for (let k = 0; k < 30; k++) {
      const shape = k % 6 === 4 ? "rocket" : k % 5 === 1 ? "spike" : "bolt";
      volley(k, 2.4 + k * 0.055, shape === "rocket" ? 12 : 16, shape);
    }
    // The giant egg, lobbed slowly, in the middle of the frame at the peak.
    volley(1, 2.6, 4.2, "charge");
    if (spec.frame === "c") for (let k = 0; k < 8; k++) volley(k, 5.3 + k * 0.05, 18, "bolt");
  }
  if (id === "wrong_genre") {
    // The own ship in the middle of the view, two rivals in other planes, shots flying in x.
    const ownPos = free(6, 0, 0);
    const facing = q.lookRotation({ x: Math.sign(r.x) || 1, y: 0, z: 0 }, { x: 0, y: 1, z: 0 });
    own = { position: ownPos, orientation: facing, shipType: 1 };
    ships = [
      { id: 101, team: 1, alive: true, boosting: true, position: v3.add(ownPos, { x: 7, y: 2.2, z: -7 }), orientation: q.lookRotation({ x: -1, y: 0, z: 0 }, { x: 0, y: 1, z: 0 }), shipType: 2, radius: 0.65, gadgetActive: false },
      { id: 102, team: 1, alive: true, boosting: false, position: v3.add(ownPos, { x: -6, y: -1.6, z: 5 }), orientation: q.lookRotation({ x: 1, y: 0, z: 0 }, { x: 0, y: 1, z: 0 }), shipType: 0, radius: 0.4, gadgetActive: false },
    ];
    for (let k = 0; k < 8; k++) {
      const fromShip = k % 3 === 0 ? ships[0]! : k % 3 === 1 ? ships[1]! : { id: 0, position: ownPos };
      const dir = fromShip.id === 101 ? -1 : 1;
      const born = 1.5 + k * 0.22;
      shots.push({ born, from: v3.add(fromShip.position, { x: dir * 1.2, y: 0, z: 0 }), vel: { x: dir * 14, y: 0, z: 0 }, hit: born + 1.8, visual: visualOf("bolt"), owner: fromShip.id, kind: 1 });
    }
  }
  if (id === "moving_day") {
    const points = [free(7, -5, -2.2), free(10, 3.5, -2.0), free(13.5, -1.2, -1.2), free(10.5, 8, -1.6)];
    const shapes: Array<[string, string]> = [["weaponPickup", "crate"], ["upgradePickup", "upgrade"], ["gadgetPickup", "gadget"], ["weaponPickup", "crate"]];
    points.forEach((p, i) => {
      const [kind, shape] = shapes[i]!;
      pickupMoves.push({ id: 500 + i, from: p, to: points[(i + 1) % points.length]!, kind, visual: visualOf(shape) });
    });
  }
  if (id === "everything_is_cake") {
    cakeOrigin = spec.frame === "a" ? free(11, 1.5, 0.5) : spec.frame === "b" ? at(58, 2, 0) : at(4, 0, 0);
    ships = spec.frame === "c" ? lineup(cam, 6, 2.4).slice(1, 3) : [];
    if (spec.frame === "c") {
      for (let k = 0; k < 5; k++) {
        const s = ships[k % 2]!;
        const aim = v3.normalize(v3.add(f, v3.add(v3.scale(r, (k - 2) * 0.12), v3.scale(u, -0.05 - (k % 2) * 0.08))));
        const from = v3.addScaled(s.position, aim, 1);
        const reach = ctx.ray(from, aim, 60);
        const dist = Math.max(3, (reach > 0 ? reach : 20) - 0.3);
        const born = 8.2 + k * 0.12 - dist / 30;
        shots.push({ born, from, vel: v3.scale(aim, 30), hit: born + dist / 30, visual: visualOf("bolt"), owner: s.id, kind: 1 });
      }
    }
  }

  // The event clock of the frame and where the run starts.
  const active = ACTIVE_SECS[id] ?? 10;
  const ending = ENDING_SECS[id] ?? 0;
  const tFrame = spec.over !== undefined ? active + ending + spec.over : spec.t;
  const tStart = id === "moving_day" || id === "rubber_chickens" ? -3.2 : Math.max(-3.2, tFrame - 4.5);
  const steps = Math.max(1, Math.round((tFrame - tStart) * TICK_HZ));
  let out: EventOutput | null = null;
  let up: Vec3 = { x: 0, y: 1, z: 0 };
  let lastShips = ships;
  let lastShots: ProjectileState[] = [];
  let lastEntities: EntityState[] = [];
  let ownNow: { position: Vec3; orientation: Quat; velocity: Vec3 } | null = null;
  for (let k = 0; k <= steps; k++) {
    const t = tStart + k / TICK_HZ;
    const time = ctx.time - (tFrame - t);
    if (particles) {
      particles.now = time;
      particles.beginFrame();
    }
    // Scripted state at t.
    const extra: Partial<EventFx> = {};
    if (id === "bored_level") {
      const turn = Math.min(1, Math.max(0, t / 5));
      up = boredUp(turn, cam, renderer.settings.sunDirection);
      extra.up = t >= 0 ? up : null;
      extra.bored = { from: { x: 0, y: 1, z: 0 }, axis: { x: 1, y: 0, z: 0 }, turnDegrees: 90, turnSecs: 5 };
    }
    if (id === "level_hiccups") extra.hiccups = { ticks: [3.0, 6.2, 9.1, 12.0].map((s) => ACTIVE_TICK + Math.round(s * TICK_HZ)), liftTicks: 15, up: { x: 0, y: 1, z: 0 } };
    if (cakeOrigin) extra.cake = { origin: cakeOrigin };
    const fx = scriptedFx(id, t, extra);
    const tick = ACTIVE_TICK + t * TICK_HZ;
    // Ships bob a little (googly eyes need motion); hiccups lift them.
    const shipsNow = ships.map((s, i) => {
      let p = v3.addScaled(s.position, u, Math.sin(time * 1.3 + i * 1.7) * 0.18);
      p = v3.addScaled(p, r, Math.sin(time * 0.9 + i) * 0.15);
      if (id === "level_hiccups") for (const h of [3.0, 6.2, 9.1, 12.0]) if (t >= h) p = v3.addScaled(p, up, Math.min(1, (t - h) / 0.25) * 2 * 0.25);
      return { ...s, position: p };
    });
    const shotsNow: ProjectileState[] = shots
      .filter((s) => t >= s.born && t < s.hit)
      .map((s) => ({ kind: s.kind, owner: s.owner, position: v3.addScaled(s.from, s.vel, t - s.born), velocity: s.vel, visual: s.visual, state: fx.chicken || (id === "rubber_chickens" && s.born >= 0) ? PROJ_CHICKEN : 0 }));
    const ents: EntityState[] = pickupMoves.map((m) => ({ id: m.id, kind: m.kind, visual: m.visual, owner: 255, team: 255, position: t >= 0 ? m.to : m.from, velocity: { x: 0, y: 0, z: 0 }, rotation: q.identity(), state: [0, 0, 0, 0], radius: 0 }));
    const cues: Cue[] = [];
    for (const h of hits) if (Math.abs(t - h.t) < 0.5 / TICK_HZ) cues.push({ kind: CUE_HIT, source: 100, target: h.ship, position: shipsNow.find((s) => s.id === h.ship)?.position ?? cam.position, value: 10, tick: Math.round(tick) });
    if (own) {
      const bob = Math.sin(time * 2) * 0.4;
      const pos = v3.add(own.position, { x: t * 0.8, y: bob, z: 0 });
      ownNow = { position: pos, orientation: own.orientation, velocity: { x: 0.8, y: Math.cos(time * 2) * 0.8, z: 0 } };
    }
    const mood = moodAt(fx);
    out = events.update({
      time,
      dt: k === 0 ? 0 : 1 / TICK_HZ,
      tick,
      fx,
      mood,
      camera: camView(),
      own: ownNow ? { id: 0, team: 0, position: ownNow.position, orientation: ownNow.orientation, velocity: ownNow.velocity, shipType: own?.shipType ?? 1, radius: 0.5 } : null,
      ships: shipsNow,
      shots: shotsNow,
      entities: ents,
      visuals,
      cues,
      wells: wellPts,
      up,
      colorOf: (sid) => teamHull(shipsNow.find((s) => s.id === sid)?.team ?? 0, 2),
      raycast: ctx.ray,
      quiet: false,
      screenHeight: ctx.canvas.height || 1080,
    });
    world.update(
      { time, dt: k === 0 ? 0 : 1 / TICK_HZ, nowTick: tick, camera: cam.position, orientation: cam.orientation, wells: wells ?? undefined, up: [up.x, up.y, up.z], shipExt: undefined, monochrome: false, modeStyles: false, cues: [], mood, own: null, ownTeam: 0, ships: shipsNow, entities: [], teams: 2, raycast: ctx.ray, quiet: false },
      () => {},
    );
    lastShips = shipsNow;
    lastShots = shotsNow;
    lastEntities = ents;
  }
  if (!out) return { particles: null, view: null };
  const o = out;
  // The scene of the last step.
  const drawnShips: ShipState[] = [...events.ships(lastShips, o)];
  if (o.thirdPerson && ownNow) drawnShips.push({ id: 0, team: 0, alive: true, boosting: true, position: ownNow.position, orientation: ownNow.orientation, shipType: own?.shipType ?? 1, radius: 0.5, gadgetActive: false });
  const shownShots = events.shots(lastShots, o);
  const hide = events.hideShot(undefined, o);
  const scene = buildScene(drawnShips, hide ? shownShots.filter((p) => !hide(p)) : shownShots, lastEntities.filter((e) => !o.hideEntities.has(e.id)), visuals, (sid) => drawnShips.find((s) => s.id === sid)?.team, ctx.time, { vfx: false, teams: 2 });
  const lights = ctx.lights;
  const wf = world.update(
    { time: ctx.time, dt: 0, nowTick: ACTIVE_TICK + tFrame * TICK_HZ, camera: cam.position, orientation: cam.orientation, wells: wells ?? undefined, up: [up.x, up.y, up.z], shipExt: undefined, monochrome: false, modeStyles: false, cues: [], mood: moodAt(scriptedFx(id, tFrame, {})), own: null, ownTeam: 0, ships: lastShips, entities: [], teams: 2, raycast: ctx.ray, quiet: false },
    () => {},
  );
  lights.push(...wf.lights);
  world.addToScene(scene, [], ctx.time, 2, lights);
  events.addToScene(scene, lights, o);
  renderer.setScene(scene);
  renderer.setWorldVolumes(wf.volumes);
  renderer.setWorldUp(up, ctx.center);
  events.apply(renderer, o);
  if (new URLSearchParams(location.search).get("eventpixel") === "off") renderer.settings.post.pixel = { cell: 1, palette: 0, dither: 0 };
  if (particles) {
    particles.now = ctx.time;
    particles.moteColor = wf.motes.color;
    particles.moteRainbow = wf.motes.rainbow;
  }
  return { particles: particles?.liveCount() ?? null, view: o.view ?? camView() };
}
