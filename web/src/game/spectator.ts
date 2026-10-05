// Spectator camera (M3): eliminated players and late joiners in elimination
// modes have no ship; they watch with a free-fly camera (WASD, Space/C, Q/E
// roll, mouse look, Shift for speed, passing through rock) or follow a
// living ship from behind. Left click follows the next ship, right click
// goes back to free flight. Pure state (no DOM), so Node tests drive it.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { InputState } from "../input/input.ts";
import type { ShipState } from "./views.ts";

/** Free-fly speed (u/s) and the factor while boost (Shift) is held. */
export const SPECTATOR_SPEED = 22;
export const SPECTATOR_BOOST = 3;
/** Roll rate (rad/s) on Q/E. */
export const SPECTATOR_ROLL = 1.6;
/** Chase camera offset behind and above a followed ship (u). */
export const FOLLOW_BACK = 5.5;
export const FOLLOW_UP = 1.6;
/** How fast the chase camera catches up (1/s). */
export const FOLLOW_STIFFNESS = 8;

export interface SpectatorView {
  position: Vec3;
  orientation: Quat;
}

export type SpectatorMode = { kind: "free" } | { kind: "follow"; id: number };

/** Level box [minX, minY, minZ, maxX, maxY, maxZ] the free camera stays in (with a margin). */
export type Bounds = ArrayLike<number>;

export class SpectatorCamera {
  position: Vec3;
  orientation: Quat;
  mode: SpectatorMode = { kind: "free" };
  private prevPrimary = false;
  private prevSecondary = false;

  constructor(start: SpectatorView) {
    this.position = { ...start.position };
    this.orientation = q.normalize(start.orientation);
  }

  /** The followed ship id, or null in free flight. */
  get following(): number | null {
    return this.mode.kind === "follow" ? this.mode.id : null;
  }

  /**
   * Advances by `dt` seconds. `ships` are the living ships to follow;
   * `lookScale` turns CSS pixels into radians (the control settings).
   */
  update(dt: number, input: InputState, ships: readonly ShipState[], lookScale: number, bounds?: Bounds, invertY = false): SpectatorView {
    const living = ships.filter((s) => s.alive).sort((a, b) => a.id - b.id);
    // Click edges: left = next ship, right = free flight.
    if (input.firePrimary && !this.prevPrimary) this.followNext(living);
    if (input.fireSecondary && !this.prevSecondary) this.mode = { kind: "free" };
    this.prevPrimary = input.firePrimary;
    this.prevSecondary = input.fireSecondary;

    if (this.mode.kind === "follow") {
      const id = this.mode.id;
      const target = living.find((s) => s.id === id);
      if (!target) {
        // The followed ship died or left: keep the view and fly free.
        this.mode = { kind: "free" };
      } else {
        const fwd = q.forward(target.orientation);
        const up = q.up(target.orientation);
        const want = {
          x: target.position.x - fwd.x * FOLLOW_BACK + up.x * FOLLOW_UP,
          y: target.position.y - fwd.y * FOLLOW_BACK + up.y * FOLLOW_UP,
          z: target.position.z - fwd.z * FOLLOW_BACK + up.z * FOLLOW_UP,
        };
        const k = 1 - Math.exp(-FOLLOW_STIFFNESS * Math.max(0, dt));
        this.position = {
          x: this.position.x + (want.x - this.position.x) * k,
          y: this.position.y + (want.y - this.position.y) * k,
          z: this.position.z + (want.z - this.position.z) * k,
        };
        this.orientation = target.orientation;
        return this.view;
      }
    }

    // Free flight: 6DOF like a ship, without inertia or collision.
    const yaw = -input.lookX * lookScale;
    const pitch = -input.lookY * lookScale * (invertY ? -1 : 1);
    const roll = -input.roll * SPECTATOR_ROLL * dt;
    let o = this.orientation;
    if (yaw !== 0) o = q.mul(o, q.fromAxisAngle({ x: 0, y: 1, z: 0 }, yaw));
    if (pitch !== 0) o = q.mul(o, q.fromAxisAngle({ x: 1, y: 0, z: 0 }, pitch));
    if (roll !== 0) o = q.mul(o, q.fromAxisAngle({ x: 0, y: 0, z: 1 }, roll));
    this.orientation = q.normalize(o);
    const speed = SPECTATOR_SPEED * (input.boost ? SPECTATOR_BOOST : 1) * Math.max(0, dt);
    const f = q.forward(this.orientation);
    const r = q.right(this.orientation);
    const u = q.up(this.orientation);
    const mf = clampUnit(input.thrustForward) * speed;
    const mr = clampUnit(input.thrustRight) * speed;
    const mu = clampUnit(input.thrustUp) * speed;
    let p = {
      x: this.position.x + f.x * mf + r.x * mr + u.x * mu,
      y: this.position.y + f.y * mf + r.y * mr + u.y * mu,
      z: this.position.z + f.z * mf + r.z * mr + u.z * mu,
    };
    if (bounds && bounds.length >= 6) p = clampToBounds(p, bounds, 20);
    this.position = p;
    return this.view;
  }

  /** Follows the living ship after the current one (by id), or the first. */
  followNext(living: readonly ShipState[]): void {
    if (living.length === 0) {
      this.mode = { kind: "free" };
      return;
    }
    const cur = this.following;
    const next = cur === null ? living[0] : (living.find((s) => s.id > cur) ?? living[0]);
    if (next) this.mode = { kind: "follow", id: next.id };
  }

  get view(): SpectatorView {
    return { position: this.position, orientation: this.orientation };
  }
}

function clampUnit(v: number): number {
  return Number.isFinite(v) ? Math.max(-1, Math.min(1, v)) : 0;
}

/** Clamps `p` into the box grown by `margin` on every side. */
export function clampToBounds(p: Vec3, b: Bounds, margin: number): Vec3 {
  const at = (i: number): number => b[i] ?? 0;
  const c = (v: number, lo: number, hi: number): number => Math.max(lo - margin, Math.min(hi + margin, v));
  return { x: c(p.x, at(0), at(3)), y: c(p.y, at(1), at(4)), z: c(p.z, at(2), at(5)) };
}
