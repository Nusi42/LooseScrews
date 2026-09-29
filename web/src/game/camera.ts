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

export const DEFAULT_CAMERA: CameraParams = {
  fovY: (75 * Math.PI) / 180,
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

  /** Follows `pose` if present (and alive); otherwise keeps the previous view. */
  update(pose: LocalPose | null): CameraView {
    if (pose && pose.alive) {
      this.view = { position: pose.position, orientation: pose.orientation, ...this.params };
    }
    return this.view;
  }

  get current(): CameraView {
    return this.view;
  }
}
