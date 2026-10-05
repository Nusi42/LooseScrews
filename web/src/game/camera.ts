// First-person camera attached to the local ship's rendered pose (prediction
// online, local simulation in practice). Before the first pose arrives, or
// while dead, it holds the last known view or looks across the level.

import * as q from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { CameraView } from "../renderer/renderer.ts";
import type { LocalPose } from "./views.ts";

export interface CameraParams {
  fovY: number;
  near: number;
  far: number;
}

// ----- field of view (visual-quality-plan.md VQ1) -----------------------------
//
// The player sets a horizontal FOV, defined at the 16:9 reference aspect.
// Narrower screens (4:3, 16:10) keep that horizontal FOV (the vertical grows);
// wider screens keep the reference vertical FOV and see more at the sides
// (Hor+), but never more than FOV_MAX_WIDE_DEG horizontally (or the setting,
// if it is wider), so 21:9 and 32:9 do not stretch the periphery.

const DEG = Math.PI / 180;
/** Default horizontal FOV (degrees at 16:9). */
export const DEFAULT_FOV_DEG = 100;
/** Settings slider range (degrees). */
export const FOV_MIN_DEG = 80;
export const FOV_MAX_DEG = 120;
/** Reference aspect at which the setting is the horizontal FOV. */
export const FOV_REFERENCE_ASPECT = 16 / 9;
/** Hor+ on wide screens stops at this horizontal FOV (degrees). */
export const FOV_MAX_WIDE_DEG = 115;
/** Boost FOV kick: extra horizontal degrees, eased in and out over FOV_KICK_SECS. */
export const FOV_KICK_DEG = 4;
export const FOV_KICK_SECS = 0.25;

/** Clamps a stored or typed FOV setting into the slider range (default for junk). */
export function clampFov(deg: unknown): number {
  const v = typeof deg === "number" ? deg : Number(deg);
  if (!Number.isFinite(v)) return DEFAULT_FOV_DEG;
  return Math.min(FOV_MAX_DEG, Math.max(FOV_MIN_DEG, Math.round(v)));
}

/** Vertical FOV (rad) for a horizontal FOV (rad) at `aspect` (width / height). */
export function verticalFromHorizontal(h: number, aspect: number): number {
  return 2 * Math.atan(Math.tan(h / 2) / aspect);
}

/** Horizontal FOV (rad) for a vertical FOV (rad) at `aspect`. */
export function horizontalFromVertical(v: number, aspect: number): number {
  return 2 * Math.atan(Math.tan(v / 2) * aspect);
}

/**
 * Vertical FOV (rad) the camera uses for the horizontal setting `hDeg`
 * (degrees at 16:9) on a screen of `aspect`, plus `kickDeg` (boost) on top.
 */
export function fovYFor(hDeg: number, aspect: number, kickDeg = 0): number {
  const a = Number.isFinite(aspect) && aspect > 0 ? aspect : FOV_REFERENCE_ASPECT;
  const h = (clampFov(hDeg) + kickDeg) * DEG;
  if (a <= FOV_REFERENCE_ASPECT) return verticalFromHorizontal(h, a);
  // Hor+: the reference vertical FOV, clamped so the horizontal stays in bounds.
  const v = verticalFromHorizontal(h, FOV_REFERENCE_ASPECT);
  const maxH = (Math.max(FOV_MAX_WIDE_DEG, clampFov(hDeg)) + kickDeg) * DEG;
  return horizontalFromVertical(v, a) > maxH ? verticalFromHorizontal(maxH, a) : v;
}

/** Smooth 0..1 boost FOV kick: eases toward 1 while boosting and back to 0 after, FOV_KICK_SECS each way. */
export class FovKick {
  private t = 0;

  /** Advances by `dt` seconds and returns the extra horizontal FOV in degrees. */
  update(dt: number, boosting: boolean): number {
    const step = Math.max(0, dt) / FOV_KICK_SECS;
    this.t = boosting ? Math.min(1, this.t + step) : Math.max(0, this.t - step);
    return this.degrees;
  }

  get degrees(): number {
    const t = this.t;
    return FOV_KICK_DEG * t * t * (3 - 2 * t);
  }

  reset(): void {
    this.t = 0;
  }
}

export const DEFAULT_CAMERA: CameraParams = {
  fovY: fovYFor(DEFAULT_FOV_DEG, FOV_REFERENCE_ASPECT),
  near: 0.05,
  far: 600,
};

/** Level box as returned by ls-wasm `Game.bounds()`: [minX, minY, minZ, maxX, maxY, maxZ]. */
export function boundsCenter(b: ArrayLike<number>): Vec3 {
  const at = (i: number): number => b[i] ?? 0;
  return { x: (at(0) + at(3)) / 2, y: (at(1) + at(4)) / 2, z: (at(2) + at(5)) / 2 };
}

export class ShipCamera {
  params: CameraParams;
  private view: CameraView;

  constructor(fallback: Vec3, params: CameraParams = DEFAULT_CAMERA) {
    this.params = { ...params };
    this.view = { position: { ...fallback }, orientation: q.identity(), ...this.params };
  }

  /**
   * Follows `pose` if present (and alive); otherwise keeps the previous view.
   * `ride` replaces the pose (the Eyeball camera while guiding); `shake`
   * offsets the returned view along its right and up axes (u) without
   * moving the held view.
   */
  update(
    pose: LocalPose | null,
    ride: { position: Vec3; orientation: q.Quat } | null = null,
    shake: { x: number; y: number } = { x: 0, y: 0 },
  ): CameraView {
    if (ride) {
      this.view = { position: ride.position, orientation: ride.orientation, ...this.params };
    } else if (pose && pose.alive) {
      this.view = { position: pose.position, orientation: pose.orientation, ...this.params };
    }
    if (shake.x === 0 && shake.y === 0) return this.view;
    const r = q.right(this.view.orientation);
    const u = q.up(this.view.orientation);
    const p = this.view.position;
    return {
      ...this.view,
      position: {
        x: p.x + r.x * shake.x + u.x * shake.y,
        y: p.y + r.y * shake.x + u.y * shake.y,
        z: p.z + r.z * shake.x + u.z * shake.y,
      },
    };
  }

  get current(): CameraView {
    return this.view;
  }
}
