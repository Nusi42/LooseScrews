// Level-shape scenes of generator version 4 (M4 A1, docs/milestone-4-plan.md
// §3.3 and §3.8): the flat and tall level boxes of the Verticality slider,
// gravity wells (Gravity Chaos) in caverns and in open sky, and the asteroid
// belt of Belt Up. Pure data (imported by scenes.ts) plus the camera poses
// that need the level's well and belt sites (levelPose), which gallery.ts
// takes from ls-wasm (`levelWells`, `levelBelt`).

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { GalleryScene } from "./scenes.ts";

/** Archetype index of an asteroid belt (ls-math `Archetype::Belt`). */
export const BELT = 1;

export const LEVEL_SCENES: readonly GalleryScene[] = [
  {
    id: "shape-flat",
    title: "Flat level (Verticality 5: 320 x 64 x 320 u) from its centre: one wide layer",
    covers: "level shapes",
    world: { seed: 11, enclosure: 22, density: 15, verticality: 5 },
    pose: "centre",
    content: "none",
    time: 1.0,
  },
  {
    id: "shape-tall",
    title: "Tall level (Verticality 95: 192 x 224 x 192 u) from its floor: islands in tiers",
    covers: "level shapes",
    world: { seed: 11, enclosure: 22, density: 15, verticality: 95 },
    pose: "rise",
    content: "none",
    time: 1.0,
  },
  {
    id: "wells-cave",
    title: "Gravity well in the caverns: a rock core in its hollow (Gravity Chaos 100)",
    covers: "gravity wells",
    world: { seed: 7, enclosure: 50, density: 50, gravityChaos: 100 },
    pose: "well",
    content: "none",
    time: 1.0,
  },
  {
    id: "wells-sky",
    title: "Gravity well in open sky: a floating stone (Gravity Chaos 100)",
    covers: "gravity wells",
    world: { seed: 7, enclosure: 10, density: 60, gravityChaos: 100 },
    pose: "well",
    content: "none",
    time: 1.0,
  },
  {
    id: "belt-overview",
    title: "Asteroid belt (Belt Up): the ring from above its outer edge",
    covers: "belt",
    world: { seed: 21, enclosure: 50, density: 60, verticality: 50, gravityChaos: 40, archetype: BELT },
    pose: "belt",
    content: "none",
    time: 1.0,
  },
  {
    id: "belt-slot",
    title: "Asteroid belt: a start slot looking along the racing line",
    covers: "belt",
    world: { seed: 21, enclosure: 50, density: 60, verticality: 50, gravityChaos: 40, archetype: BELT },
    pose: "slot",
    content: "none",
    time: 1.0,
  },
];

/** Floats per well of ls-wasm `levelWells`. */
const WELL_FLOATS = 8;

const UP: Vec3 = { x: 0, y: 1, z: 0 };

function look(dir: Vec3): Quat {
  return q.lookRotation(v3.normalize(dir), Math.abs(dir.y) > 0.95 * v3.length(dir) ? { x: 1, y: 0, z: 0 } : UP);
}

/**
 * Camera pose of a level scene from the level's wells (`levelWells`) and
 * belt (`levelBelt`); null when the scene needs a site the level has not
 * (then the gallery falls back to a vista):
 * - `well`: inside the first well's hollow, 6.5 u from the core, looking
 *   at it from slightly above;
 * - `belt`: outside the ring, above its outer edge, looking across the
 *   whole ring and the exit;
 * - `slot`: just behind and above start slot 0, looking prograde;
 * - `centre`: inside the level box at mid height, looking across it
 *   (`bounds`: ls-wasm `Game.bounds()`);
 * - `rise`: near the floor, looking up the box.
 */
export function levelPose(
  pose: string,
  wells: ArrayLike<number> | null,
  belt: ArrayLike<number> | null,
  bounds: ArrayLike<number> | null = null,
): { position: Vec3; orientation: Quat } | null {
  if ((pose === "centre" || pose === "rise") && bounds && bounds.length >= 6) {
    const min = { x: bounds[0] ?? 0, y: bounds[1] ?? 0, z: bounds[2] ?? 0 };
    const max = { x: bounds[3] ?? 0, y: bounds[4] ?? 0, z: bounds[5] ?? 0 };
    const c = v3.scale(v3.add(min, max), 0.5);
    if (pose === "centre") {
      // Level with the middle of the box, looking along the diagonal.
      const position = { x: min.x + 0.2 * (max.x - min.x), y: c.y, z: min.z + 0.2 * (max.z - min.z) };
      return { position, orientation: look({ x: 1, y: -0.04, z: 0.8 }) };
    }
    // Near the floor at the centre, looking up the tall box.
    const position = { x: c.x - 0.3 * (max.x - min.x), y: min.y + 8, z: c.z - 0.3 * (max.z - min.z) };
    return { position, orientation: look({ x: 0.55, y: 1.1, z: 0.55 }) };
  }
  if (pose === "well") {
    if (!wells || wells.length < WELL_FLOATS) return null;
    const core = { x: wells[0] ?? 0, y: wells[1] ?? 0, z: wells[2] ?? 0 };
    const hollow = wells[4] ?? 8;
    const from = v3.normalize({ x: 0.75, y: 0.35, z: 0.55 });
    const position = v3.addScaled(core, from, Math.min(6.5, hollow - 1.2));
    return { position, orientation: look(v3.sub(core, position)) };
  }
  if (!belt || belt.length < 10) return null;
  const center = { x: belt[0] ?? 0, y: belt[1] ?? 0, z: belt[2] ?? 0 };
  const outer = belt[4] ?? 145;
  if (pose === "belt") {
    // From the south-west, above the outer edge, looking down across the
    // ring to the exit at its centre.
    const dir = v3.normalize({ x: -0.62, y: 0, z: 0.78 });
    const position = v3.add(v3.addScaled(center, dir, outer * 0.95), { x: 0, y: 95, z: 0 });
    const target = v3.addScaled(center, dir, -0.15 * outer);
    return { position, orientation: look(v3.sub(target, position)) };
  }
  if (pose === "slot") {
    const slot = { x: belt[10] ?? 0, y: belt[11] ?? 0, z: belt[12] ?? 0 };
    // Prograde: counter-clockwise seen from above (azimuth about +Y).
    const r = v3.sub(slot, center);
    const ahead = v3.normalize({ x: r.z, y: 0, z: -r.x });
    const position = v3.add(v3.addScaled(slot, ahead, -4), { x: 0, y: 1.5, z: 0 });
    // Look along the clear corridor: its point about 25 u ahead.
    const line: Vec3[] = [];
    for (let i = 34; i + 2 < belt.length; i += 3) line.push({ x: belt[i] ?? 0, y: belt[i + 1] ?? 0, z: belt[i + 2] ?? 0 });
    const target =
      line
        .filter((p) => v3.dot(v3.sub(p, slot), ahead) > 0)
        .sort((a, b) => Math.abs(v3.length(v3.sub(a, slot)) - 25) - Math.abs(v3.length(v3.sub(b, slot)) - 25))[0] ?? v3.addScaled(slot, ahead, 25);
    return { position, orientation: look(v3.sub(v3.add(target, { x: 0, y: 0.5, z: 0 }), position)) };
  }
  return null;
}
