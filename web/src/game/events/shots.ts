// Follows projectiles from frame to frame (M4 C1). The client's projectile
// list has no ids, so a shot is matched to last frame's by owner, weapon and
// its predicted position; a shot without a match is new (a squeak on fire),
// a shot from last frame without a match has hit something (feathers, cake
// crumbs at its last position).

import type { Vec3 } from "../../math/vec3.ts";
import type { ProjectileState } from "../views.ts";

export interface TrackedShot {
  id: number;
  owner: number;
  kind: number;
  visual: number;
  position: Vec3;
  velocity: Vec3;
  state: number;
  /** Time (s) it was first seen. */
  born: number;
}

export interface ShotFrame {
  /** Id of every shot of this frame (same order). */
  ids: number[];
  /** Shots seen for the first time. */
  born: TrackedShot[];
  /** Last frame's shots that are gone (at their last position). */
  gone: TrackedShot[];
}

export class ShotTracker {
  private prev: TrackedShot[] = [];
  private nextId = 1;

  /** Matches `shots` against the last frame (`dt` seconds ago). */
  update(shots: readonly ProjectileState[], dt: number, time: number): ShotFrame {
    const used = new Uint8Array(this.prev.length);
    const ids: number[] = [];
    const born: TrackedShot[] = [];
    const next: TrackedShot[] = [];
    for (const s of shots) {
      let best = -1;
      let bd = Infinity;
      const speed = Math.hypot(s.velocity.x, s.velocity.y, s.velocity.z);
      const tol = 1.5 + speed * dt * 1.5;
      for (let i = 0; i < this.prev.length; i++) {
        if (used[i]) continue;
        const p = this.prev[i]!;
        if (p.owner !== s.owner || p.kind !== s.kind) continue;
        const px = p.position.x + p.velocity.x * dt - s.position.x;
        const py = p.position.y + p.velocity.y * dt - s.position.y;
        const pz = p.position.z + p.velocity.z * dt - s.position.z;
        const d = Math.hypot(px, py, pz);
        if (d < tol && d < bd) {
          bd = d;
          best = i;
        }
      }
      let t: TrackedShot;
      if (best >= 0) {
        used[best] = 1;
        const p = this.prev[best]!;
        t = { ...p, position: s.position, velocity: s.velocity, state: s.state, visual: s.visual };
      } else {
        t = { id: this.nextId++, owner: s.owner, kind: s.kind, visual: s.visual, position: s.position, velocity: s.velocity, state: s.state, born: time };
        born.push(t);
      }
      ids.push(t.id);
      next.push(t);
    }
    const gone = this.prev.filter((_, i) => !used[i]);
    this.prev = next;
    return { ids, born, gone };
  }

  reset(): void {
    this.prev = [];
  }
}
