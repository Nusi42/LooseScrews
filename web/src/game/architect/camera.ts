// The architect's god camera (M4 C4, milestone-4-plan.md §3.5): no ship,
// two views.
// - Free flight: 6DOF like the spectator camera (WASD, Space/C, Q/E roll,
//   mouse look, Shift fast), faster, through rock, kept near the level box.
// - Overview: a map view over a cut through the level. The camera looks
//   down at a target point on the cut plane from OVERVIEW_DISTANCE away
//   (mouse turns and tilts it, WASD pans along the ground, Space/C raise and
//   lower the cut, Q/E turn); the renderer clips the rock above the cut
//   (`Renderer.cutaway`), so the tunnel floors read like a floor plan.
// V switches; the switch keeps the view continuous (overview looks down at
// the point under the free camera, free flight starts where the overview
// camera was). Pure state (no DOM), so Node tests drive it.

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { InputState } from "../../input/input.ts";

/** Free-flight speed (u/s) and the factor with Shift. */
export const ARCHITECT_SPEED = 30;
export const ARCHITECT_BOOST = 3;
/** Roll (free) and turn (overview) rate on Q/E (rad/s). */
export const ARCHITECT_ROLL = 1.6;
/** Overview: distance from the target, pan speed (u/s), cut speed (u/s). */
export const OVERVIEW_DISTANCE = 120;
export const OVERVIEW_PAN = 45;
export const OVERVIEW_CUT_SPEED = 18;
/** Overview pitch the map view starts at (rad below the horizon: steep, so it reads as a map). */
export const OVERVIEW_PITCH = 1.2;
/** Overview pitch range below the horizon (rad). */
export const OVERVIEW_PITCH_MIN = 0.55;
export const OVERVIEW_PITCH_MAX = 1.45;
/** Free flight stays this close to the level box (u). */
export const FREE_MARGIN = 12;
/** The cut sits this far above the target (the floor under the camera stays, the ceiling goes). */
export const CUT_ABOVE = 6;
/** The map view shows a slab this deep under the cut (one storey of the level). */
export const SLAB_DEPTH = 34;

export type ArchitectCameraMode = "free" | "overview";

export interface ArchitectPose {
  position: Vec3;
  orientation: Quat;
}

/** Clip box for the renderer's cutaway (rock outside is not drawn). */
export interface Cutaway {
  min: Vec3;
  max: Vec3;
  /** Fade width (u) at the cut. */
  fade: number;
}

const UP: Vec3 = { x: 0, y: 1, z: 0 };

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

function unit(v: number): number {
  return Number.isFinite(v) ? clamp(v, -1, 1) : 0;
}

/** Level box [minX, minY, minZ, maxX, maxY, maxZ]. */
export type Bounds = ArrayLike<number>;

const at = (b: Bounds, i: number): number => b[i] ?? 0;

export class ArchitectCamera {
  mode: ArchitectCameraMode = "free";
  position: Vec3;
  orientation: Quat;
  /** Overview: the point looked at (its y is the cut's floor), yaw about +Y and pitch below the horizon. */
  target: Vec3;
  yaw = 0;
  pitch = 1.0;
  distance = OVERVIEW_DISTANCE;

  constructor(start: ArchitectPose) {
    this.position = { ...start.position };
    this.orientation = q.normalize(start.orientation);
    this.target = { ...start.position };
  }

  /** Switches between free flight and the overview, keeping the view continuous. */
  toggle(bounds?: Bounds): void {
    if (this.mode === "free") {
      const f = q.forward(this.orientation);
      // Look down at the point under the camera, from behind along its heading.
      this.yaw = Math.atan2(-f.x, -f.z);
      this.pitch = OVERVIEW_PITCH;
      this.target = { ...this.position };
      if (bounds && bounds.length >= 6) this.target = clampTarget(this.target, bounds);
      this.mode = "overview";
      this.overviewPose();
    } else {
      // Fly on from the overview camera, looking where it looked.
      this.mode = "free";
    }
  }

  /**
   * The map view over `target`, heading toward the level's middle (the
   * architect starts here: over the runners' start, looking down the level).
   */
  lookDownAt(target: Vec3, bounds: Bounds): void {
    const cx = (at(bounds, 0) + at(bounds, 3)) / 2;
    const cz = (at(bounds, 2) + at(bounds, 5)) / 2;
    const dx = cx - target.x;
    const dz = cz - target.z;
    this.yaw = Math.hypot(dx, dz) > 1e-3 ? Math.atan2(-dx, -dz) : 0;
    this.pitch = OVERVIEW_PITCH;
    this.target = bounds.length >= 6 ? clampTarget(target, bounds) : { ...target };
    this.mode = "overview";
    this.overviewPose();
  }

  /** The cut through the level (overview only); null in free flight. */
  cutaway(bounds: Bounds): Cutaway | null {
    if (this.mode !== "overview" || bounds.length < 6) return null;
    const cut = Math.min(at(bounds, 4) + 4, this.target.y + CUT_ABOVE);
    // One storey: the layers far below would show through the tunnels as a tangle.
    return { min: { x: -1e5, y: cut - SLAB_DEPTH, z: -1e5 }, max: { x: 1e5, y: cut, z: 1e5 }, fade: 1.5 };
  }

  /** Advances by `dt` seconds with the sampled input; `lookScale` turns CSS pixels into radians. */
  update(dt: number, input: InputState, lookScale: number, bounds?: Bounds, invertY = false): ArchitectPose {
    const t = Math.max(0, dt);
    const fast = input.boost ? ARCHITECT_BOOST : 1;
    if (this.mode === "overview") {
      this.yaw += -input.lookX * lookScale - input.roll * ARCHITECT_ROLL * t;
      this.pitch = clamp(this.pitch + input.lookY * lookScale * (invertY ? -1 : 1), OVERVIEW_PITCH_MIN, OVERVIEW_PITCH_MAX);
      const s = Math.sin(this.yaw);
      const c = Math.cos(this.yaw);
      // Forward on the ground is where the camera looks (yaw 0 looks along -Z).
      const fwd = { x: -s, z: -c };
      const right = { x: c, z: -s };
      const pan = OVERVIEW_PAN * fast * t;
      const mf = unit(input.thrustForward) * pan;
      const mr = unit(input.thrustRight) * pan;
      this.target = {
        x: this.target.x + fwd.x * mf + right.x * mr,
        y: this.target.y + unit(input.thrustUp) * OVERVIEW_CUT_SPEED * fast * t,
        z: this.target.z + fwd.z * mf + right.z * mr,
      };
      if (bounds && bounds.length >= 6) this.target = clampTarget(this.target, bounds);
      this.overviewPose();
      return this.view;
    }
    const yaw = -input.lookX * lookScale;
    const pitch = -input.lookY * lookScale * (invertY ? -1 : 1);
    const roll = -input.roll * ARCHITECT_ROLL * t;
    let o = this.orientation;
    if (yaw !== 0) o = q.mul(o, q.fromAxisAngle(UP, yaw));
    if (pitch !== 0) o = q.mul(o, q.fromAxisAngle({ x: 1, y: 0, z: 0 }, pitch));
    if (roll !== 0) o = q.mul(o, q.fromAxisAngle({ x: 0, y: 0, z: 1 }, roll));
    this.orientation = q.normalize(o);
    const speed = ARCHITECT_SPEED * fast * t;
    const f = q.forward(this.orientation);
    const r = q.right(this.orientation);
    const u = q.up(this.orientation);
    const mf = unit(input.thrustForward) * speed;
    const mr = unit(input.thrustRight) * speed;
    const mu = unit(input.thrustUp) * speed;
    let p = {
      x: this.position.x + f.x * mf + r.x * mr + u.x * mu,
      y: this.position.y + f.y * mf + r.y * mr + u.y * mu,
      z: this.position.z + f.z * mf + r.z * mr + u.z * mu,
    };
    if (bounds && bounds.length >= 6) {
      const m = FREE_MARGIN;
      p = { x: clamp(p.x, at(bounds, 0) - m, at(bounds, 3) + m), y: clamp(p.y, at(bounds, 1) - m, at(bounds, 4) + m), z: clamp(p.z, at(bounds, 2) - m, at(bounds, 5) + m) };
    }
    this.position = p;
    return this.view;
  }

  /** Places the overview camera from the target, yaw, pitch and distance. */
  private overviewPose(): void {
    const cp = Math.cos(this.pitch);
    const sp = Math.sin(this.pitch);
    const s = Math.sin(this.yaw);
    const c = Math.cos(this.yaw);
    // Looking direction: down by `pitch`, heading by `yaw` (yaw 0 = -Z).
    const look = { x: -s * cp, y: -sp, z: -c * cp };
    this.position = { x: this.target.x - look.x * this.distance, y: this.target.y - look.y * this.distance, z: this.target.z - look.z * this.distance };
    this.orientation = q.lookRotation(look, { x: -s, y: 0, z: -c });
  }

  get view(): ArchitectPose {
    return { position: this.position, orientation: this.orientation };
  }
}

/** Keeps the overview target inside the level box (its floor at least 2 u above the bottom). */
export function clampTarget(p: Vec3, b: Bounds): Vec3 {
  return {
    x: clamp(p.x, at(b, 0), at(b, 3)),
    y: clamp(p.y, at(b, 1) + 2, at(b, 4)),
    z: clamp(p.z, at(b, 2), at(b, 5)),
  };
}
