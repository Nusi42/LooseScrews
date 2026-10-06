// Wrong genre (M4 C1, milestone-4-plan.md §3.2): for 10 s every ship is
// locked to the plane through its position perpendicular to z, facing +x or
// -x; shots fly in the plane and hit ships by their x/y distance. The client
// turns the match into a side-scroller from the 80s:
// - the tell: the picture glitches into chunky pixels for a few frames
//   while the event is announced;
// - at the start the camera swings out to the side of the own ship's plane
//   (a long lens 22 u out, its near plane 1.4 u in front of the plane, so the
//   rock in between is cut away; the terrain draws the cut as a brick fill,
//   `Renderer.terrainCutaway`), follows the ship like a platformer camera
//   and the own ship is drawn; the frame drops to a quarter of the
//   resolution in 16 colours (renderer/pixelPost.ts) in steps like an old
//   console powering on, with a "WORLD 1-1" card and a power-on bleep;
// - other ships and shots are drawn in the own plane (hits ignore depth, so
//   what you see is what can hit);
// - a chiptune loop plays; at the end the camera swings back, the picture
//   regains its pixels and a "1-UP" jingle plays.

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import { pixelLookAt, quarterCell } from "../../renderer/pixelPost.ts";
import type { CameraView } from "../../renderer/renderer.ts";
import { hash01, smooth, type EventClock } from "./fx.ts";
import type { EventEnv, EventInput, EventOutput, EventVisual, OwnShip } from "./types.ts";

/** Side camera: distance from the plane (u), vertical field of view, how far in front of the plane the cut is (u). */
export const SIDE_DISTANCE = 22;
export const SIDE_FOV = (34 * Math.PI) / 180;
export const CUT_MARGIN = 1.4;
/** Seconds the switch takes in and out. */
export const SWITCH_SECS = 0.55;

/** How far into the side view (0..1) `t` seconds into the event, or `over` seconds after it. */
export function sideAmount(t: number, over: number | null): number {
  if (over !== null) return 1 - smooth(0, SWITCH_SECS, over);
  return smooth(0, SWITCH_SECS, t);
}

/** The announcement's glitch: short bursts of pixels in its last 1.4 s (0..1). */
export function glitch(t: number): number {
  if (t < -1.4 || t >= 0) return 0;
  const k = Math.floor((t + 1.4) * 9);
  return hash01(k, 3) < 0.3 + 0.35 * smooth(-1.4, 0, t) ? 0.35 + 0.4 * smooth(-1.4, 0, t) : 0;
}

/** The screen up of the side view: the level's up in the plane, else +Y. */
export function sideUp(up: Vec3): Vec3 {
  const inPlane = { x: up.x, y: up.y, z: 0 };
  return v3.length(inPlane) > 0.3 ? v3.normalize(inPlane) : { x: 0, y: 1, z: 0 };
}

/** The side camera looking at `target` in the plane z = `target.z` from +z. */
export function sideCamera(target: Vec3, up: Vec3): CameraView {
  const position = { x: target.x, y: target.y, z: target.z + SIDE_DISTANCE };
  return { position, orientation: q.lookRotation({ x: 0, y: 0, z: -1 }, sideUp(up)), fovY: SIDE_FOV, near: SIDE_DISTANCE - CUT_MARGIN, far: SIDE_DISTANCE + 420 };
}

function nlerp(a: Quat, b: Quat, t: number): Quat {
  const d = a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w;
  const s = d < 0 ? -1 : 1;
  return q.normalize({ x: a.x + (b.x * s - a.x) * t, y: a.y + (b.y * s - a.y) * t, z: a.z + (b.z * s - a.z) * t, w: a.w + (b.w * s - a.w) * t });
}

/** The camera `a` (0..1) of the way from the game's camera to the side camera; its near plane cuts the rock in front of the plane. */
export function blendCamera(game: CameraView, side: CameraView, a: number, planeZ: number): CameraView {
  const e = a * a * (3 - 2 * a);
  const position = v3.lerp(game.position, side.position, e);
  const orientation = nlerp(game.orientation, side.orientation, e);
  const forward = q.forward(orientation);
  const toPlane = Math.abs(forward.z) > 0.2 ? (planeZ - position.z) / forward.z : 0;
  return {
    position,
    orientation,
    fovY: game.fovY + (side.fovY - game.fovY) * e,
    near: Math.max(game.near, toPlane - CUT_MARGIN),
    far: Math.max(game.far, side.far),
  };
}

class WrongGenre implements EventVisual {
  private readonly env: EventEnv;
  private follow: Vec3 | null = null;
  private planeZ: number | null = null;
  private started = false;
  private ended = false;
  private music: { stop(f?: number): void } | null = null;

  constructor(env: EventEnv) {
    this.env = env;
  }

  update(i: EventInput, clock: EventClock, out: EventOutput): boolean {
    const sound = i.quiet ? null : this.env.sound;
    const a = sideAmount(clock.t, clock.over);
    const full = quarterCell(i.screenHeight);
    const g = clock.over === null ? glitch(clock.t) : 0;
    if (a > 0 || g > 0) out.pixel = pixelLookAt(Math.max(a, g), full);
    if (clock.t >= 0 && clock.over === null && !this.started) {
      this.started = true;
      sound?.play("genre.start");
      this.music = sound?.loop("genre.chiptune", { gain: 0.9 }) ?? null;
    }
    if (clock.over !== null && !this.ended) {
      this.ended = true;
      this.music?.stop(0.1);
      this.music = null;
      sound?.play("genre.oneup");
    }
    // The title cards.
    if (clock.over === null && clock.t > 0.15 && clock.t < 2.4) {
      out.hud.retro = { title: "WORLD 1-1", sub: "WRONG GENRE", alpha: smooth(0.15, 0.3, clock.t) * (1 - smooth(2.0, 2.4, clock.t)) };
    } else if (clock.over !== null && clock.over < 1.8) {
      out.hud.retro = { title: "1UP", sub: "GENRE RESTORED", alpha: 1 - smooth(1.3, 1.8, clock.over) };
    }
    // Alive until the 1UP card is gone.
    if (a <= 0) return clock.over === null || clock.over < 1.8;
    const own = i.own;
    if (own) {
      if (this.planeZ === null || clock.over === null) this.planeZ = own.position.z;
      const z0 = this.planeZ;
      const target = this.followTarget(own, i.dt);
      const side = sideCamera({ x: target.x, y: target.y, z: z0 }, i.up);
      out.view = blendCamera(out.view ?? i.camera, side, a, z0);
      out.thirdPerson = a > 0.05;
      out.cutaway = true;
      // Ships read like sprites: bigger, lit in their colours, rolled to show their tops.
      out.shipBoost = { scale: 1 + 0.9 * a, emissive: 0.32 * a, roll: 0.65 * a };
      // Everything else is drawn in the own plane.
      out.ships = i.ships.map((s) => ({ ...s, position: { ...s.position, z: s.position.z + (z0 - s.position.z) * a } }));
      out.shots = i.shots.map((p) => ({ ...p, position: { ...p.position, z: p.position.z + (z0 - p.position.z) * a } }));
    }
    return true;
  }

  /** A platformer camera: the ship plus a little lead along its motion, smoothed. */
  private followTarget(own: OwnShip, dt: number): Vec3 {
    const lead = { x: own.velocity.x * 0.25, y: own.velocity.y * 0.15, z: 0 };
    const l = v3.length(lead);
    const want = v3.add(own.position, l > 4 ? v3.scale(lead, 4 / l) : lead);
    if (!this.follow) this.follow = want;
    const k = 1 - Math.exp(-dt * 6);
    this.follow = v3.lerp(this.follow, want, k);
    return this.follow;
  }

  dispose(): void {
    this.music?.stop(0.1);
    this.music = null;
  }
}

export const wrongGenre = (env: EventEnv): EventVisual => new WrongGenre(env);
