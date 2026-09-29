// 6DOF camera flown with an InputState.
//
// This is a client-only *preview* flight model so the renderer can be tried
// before networking exists. It mirrors the design doc's speed model (thrust
// capped per axis in ship space: cruise 12 u/s, boost 16.5 u/s per axis) but it
// is not authoritative: in wave 3 the local ship pose comes from ls-sim
// prediction (WASM) and this class only supplies the look/roll feel, or is
// replaced by a camera that follows the predicted ship.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import { addScaled, vec3, type Vec3 } from "../math/vec3.ts";
import type { CameraView } from "../renderer/renderer.ts";
import type { InputState } from "./input.ts";

export interface FlightParams {
  /** Per-axis speed cap without boost (u/s). */
  cruiseSpeed: number;
  /** Per-axis speed cap while boosting (u/s). */
  boostSpeed: number;
  /** How fast each local velocity axis approaches its target (u/s^2). */
  acceleration: number;
  /** Radians of rotation per CSS pixel of mouse movement. */
  lookSensitivity: number;
  invertY: boolean;
  /** Roll rate for Q/E (rad/s). */
  rollRate: number;
  /** Horizon assist correction rate (fraction of the bank angle removed per second). */
  horizonAssistRate: number;
  fovY: number;
  near: number;
  far: number;
}

export const DEFAULT_FLIGHT: FlightParams = {
  cruiseSpeed: 12,
  boostSpeed: 16.5,
  acceleration: 36,
  lookSensitivity: 0.0022,
  invertY: false,
  rollRate: 2.2,
  horizonAssistRate: 1.5,
  fovY: (75 * Math.PI) / 180,
  near: 0.05,
  far: 400,
};

const X_AXIS: Vec3 = { x: 1, y: 0, z: 0 };
const Y_AXIS: Vec3 = { x: 0, y: 1, z: 0 };
const Z_AXIS: Vec3 = { x: 0, y: 0, z: 1 };

export class FlightCamera {
  position: Vec3;
  orientation: Quat = q.identity();
  /** World-space velocity (u/s). */
  velocity: Vec3 = vec3();
  /** Optional roll levelling (on by default for new players, per the design doc). */
  horizonAssist = true;
  params: FlightParams;

  constructor(position: Vec3 = vec3(), params: FlightParams = DEFAULT_FLIGHT) {
    this.position = { ...position };
    this.params = { ...params };
  }

  /**
   * Advances the camera by `dt` seconds. `boosting` is passed separately
   * because whether boost is available depends on the boost meter.
   */
  step(input: InputState, dt: number, boosting: boolean): void {
    if (input.toggleHorizonAssist) this.horizonAssist = !this.horizonAssist;
    this.rotate(input, dt);
    this.thrust(input, dt, boosting);
    this.position = addScaled(this.position, this.velocity, dt);
  }

  /** Current speed in u/s. */
  get speed(): number {
    return Math.hypot(this.velocity.x, this.velocity.y, this.velocity.z);
  }

  view(): CameraView {
    const p = this.params;
    return { position: this.position, orientation: this.orientation, fovY: p.fovY, near: p.near, far: p.far };
  }

  /** Signed bank angle in radians (positive = right wing up = banked left). */
  bankAngle(): number {
    const r = q.right(this.orientation);
    const u = q.up(this.orientation);
    return Math.atan2(r.y, u.y);
  }

  private rotate(input: InputState, dt: number): void {
    const p = this.params;
    const pitch = -input.lookY * p.lookSensitivity * (p.invertY ? -1 : 1);
    const yaw = -input.lookX * p.lookSensitivity;
    // Roll right (+1) = negative rotation about local +Z (i.e. clockwise seen by the pilot).
    let roll = -input.roll * p.rollRate * dt;

    if (this.horizonAssist && input.roll === 0) {
      // Level the wings gradually; fade out when looking straight up/down,
      // where "level" is ill-defined.
      const fwd = q.forward(this.orientation);
      const weight = 1 - Math.min(1, Math.abs(fwd.y));
      const bank = this.bankAngle();
      const correction = bank * Math.min(1, p.horizonAssistRate * dt) * weight;
      roll -= correction;
    }

    // Local-frame rotations (post-multiply) give true 6DOF: no gimbal, no fixed up.
    let o = this.orientation;
    if (yaw !== 0) o = q.mul(o, q.fromAxisAngle(Y_AXIS, yaw));
    if (pitch !== 0) o = q.mul(o, q.fromAxisAngle(X_AXIS, pitch));
    if (roll !== 0) o = q.mul(o, q.fromAxisAngle(Z_AXIS, roll));
    this.orientation = q.normalize(o);
  }

  private thrust(input: InputState, dt: number, boosting: boolean): void {
    const p = this.params;
    const cap = boosting ? p.boostSpeed : p.cruiseSpeed;
    // Work in ship space so the cap applies per local axis (Descent-style trichording).
    const inv = q.conjugate(this.orientation);
    const local = q.rotate(inv, this.velocity);
    const maxDv = p.acceleration * dt;
    const approach = (v: number, target: number): number => {
      const d = target - v;
      return Math.abs(d) <= maxDv ? target : v + Math.sign(d) * maxDv;
    };
    const next: Vec3 = {
      x: approach(local.x, clampUnit(input.thrustRight) * cap),
      y: approach(local.y, clampUnit(input.thrustUp) * cap),
      z: approach(local.z, -clampUnit(input.thrustForward) * cap), // forward is -Z
    };
    this.velocity = q.rotate(this.orientation, next);
  }
}

function clampUnit(v: number): number {
  return Math.max(-1, Math.min(1, v));
}
