// Weapon scenes for the visual gallery (M4 B7, the Paperweight): a thrown
// Paperweight tumbling toward its spot and a landed one, upright and
// glowing, the gravity well it has become hauling debris and a ship in.
// The positions are fixed relative to the camera (`wall` places the landed
// stone on the rock ahead), so the frame is the same every run.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { PAPERWEIGHT_LANDED } from "../game/scene.ts";
import type { EntityState, ProjectileState, ShipState, VisualDef } from "../game/views.ts";

interface Cam {
  position: Vec3;
  orientation: Quat;
}

/** What the scene shows. */
export interface PaperweightMoment {
  ships: ShipState[];
  shots: ProjectileState[];
  entities: EntityState[];
}

/**
 * The Paperweight scene at time `t`: `wallAhead` is the distance (u) to the
 * rock ahead; with `onWall` the landed stone sits on that rock (it stuck
 * there), else it floats mid-air (it ran out of flight). Debris pieces
 * spiral into the well; a Tinker beside it is pulled in, nose sideways.
 * `T` is the scene's time: the next stone was thrown 0.4 s before it.
 */
export function paperweightAt(cam: Cam, visuals: readonly VisualDef[], t: number, wallAhead: number, onWall: boolean, T = t): PaperweightMoment {
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const at = (fw: number, rt: number, upw: number): Vec3 => v3.add(cam.position, v3.add(v3.scale(f, fw), v3.add(v3.scale(r, rt), v3.scale(u, upw))));
  const paperweight = visuals.findIndex((v) => v.shape === "paperweight");
  const debris = visuals.findIndex((v) => v.shape === "debris");
  const out: PaperweightMoment = { ships: [], shots: [], entities: [] };
  if (paperweight < 0) return out;
  const depth = onWall ? Math.max(3.5, Math.min(wallAhead - 0.45, 9)) : 5;
  const centre = at(depth, 0.5, -0.15);
  // The landed stone (custom bit A: landed, a well now).
  out.shots.push({ kind: 27, owner: 1, position: centre, velocity: { x: 0, y: 0, z: 0 }, visual: paperweight, state: 1 | PAPERWEIGHT_LANDED });
  // The next one on its way, tumbling across the view from the left.
  const from = at(depth - 1.2, -2.2, 0.9);
  const dir = v3.normalize(v3.sub(at(depth + 1, 1.5, 0.3), from));
  if (t > T - 0.4) {
    out.shots.push({ kind: 27, owner: 0, position: v3.addScaled(from, dir, 25 * (t - T)), velocity: v3.scale(dir, 25), visual: paperweight, state: 0 });
  }
  // Debris spiralling in: closer pieces further round the spiral.
  if (debris >= 0) {
    for (let i = 0; i < 7; i++) {
      const a = i * 2.1 + t * 0.9;
      const rad = 0.8 + 0.3 * i;
      const p = v3.add(centre, v3.add(v3.scale(r, Math.cos(a) * rad), v3.add(v3.scale(u, Math.sin(a) * rad * 0.6), v3.scale(f, Math.sin(a * 0.7) * rad * 0.5))));
      const tangent = v3.normalize(v3.add(v3.scale(r, -Math.sin(a)), v3.scale(u, Math.cos(a) * 0.6)));
      const inward = v3.normalize(v3.sub(centre, p));
      out.entities.push({
        id: 300 + i,
        kind: "debris",
        visual: debris,
        owner: 255,
        team: 255,
        position: p,
        velocity: v3.add(v3.scale(tangent, 3), v3.scale(inward, 1.5)),
        rotation: q.fromAxisAngle(v3.normalize({ x: 0.3 + i, y: 1, z: 0.5 }), i * 1.3),
        state: [0, 0, 0, 0],
        radius: 0.12 + 0.04 * (i % 4),
      });
    }
  }
  // A Tinker 2.6 u to the right of the stone, being hauled in sideways.
  const ship = v3.add(centre, v3.add(v3.scale(r, 2.6), v3.add(v3.scale(u, -0.7), v3.scale(f, -0.8))));
  out.ships.push({
    id: 1,
    team: 1,
    alive: true,
    boosting: true,
    position: ship,
    orientation: q.lookRotation(v3.normalize(v3.add(v3.scale(f, -0.7), v3.scale(u, 0.15))), u),
    shipType: 1,
    radius: 0.5,
    gadgetActive: false,
  });
  return out;
}
