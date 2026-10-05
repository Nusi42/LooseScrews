import assert from "node:assert/strict";
import { test } from "node:test";
import { MATERIAL_HARD, MATERIAL_WEAK, NO_TEAM, PAINT_PLAIN, SURFACE_HOT, type MeshData } from "../renderer/mesh.ts";
import { applyDecor, decorSpots, pickBorder } from "./decor.ts";
import { rayMesh, type Probe } from "./raycast.ts";
import { sceneById } from "./scenes.ts";

/** A wall in the plane z = 10 from x, y = -5..5, vertices every 1 u; weak rock for x < 0. */
function wall(): MeshData {
  const positions: number[] = [];
  const team: number[] = [];
  const materials: number[] = [];
  for (let j = 0; j <= 10; j++) {
    for (let i = 0; i <= 10; i++) {
      positions.push(i - 5, j - 5, 10);
      team.push(NO_TEAM);
      materials.push(i - 5 < 0 ? MATERIAL_WEAK : MATERIAL_HARD);
    }
  }
  const indices: number[] = [];
  for (let j = 0; j < 10; j++) {
    for (let i = 0; i < 10; i++) {
      const a = j * 11 + i;
      indices.push(a, a + 1, a + 12, a, a + 12, a + 11);
    }
  }
  return {
    positions: new Float32Array(positions),
    normals: new Float32Array(positions.length),
    indices: new Uint32Array(indices),
    team: new Uint8Array(team),
    materials: new Uint8Array(materials),
  };
}

test("decor scenes exist and the decor marks only vertices inside its spheres", () => {
  assert.equal(sceneById("paint-edge")?.decor, "paint");
  assert.equal(sceneById("surface-states")?.decor, "states");
  assert.equal(sceneById("material-border")?.pose, "border");
  const m = wall();
  const target = { x: 0, y: 0, z: 10 };
  const normal = { x: 0, y: 0, z: -1 };
  assert.deepEqual(decorSpots("none", target, normal), []);
  assert.equal(applyDecor(m, []), m);
  const paint = decorSpots("paint", target, normal);
  assert.equal(paint.length, 3);
  const painted = applyDecor(m, paint);
  assert.notEqual(painted, m);
  assert.equal(m.team.every((t) => t === NO_TEAM), true, "input unchanged");
  const teams = new Set(painted.team);
  assert.ok(teams.has(0) && teams.has(1) && teams.has(NO_TEAM));
  // Plain team-0 paint where only the first spot reaches.
  const first = paint[0]!;
  assert.equal(first.color, PAINT_PLAIN);
  // Spots lie in the wall plane around the target.
  for (const s of paint) assert.ok(Math.abs(s.center.z - 10) < 1e-6);
  const states = applyDecor(m, decorSpots("states", target, normal));
  assert.ok(states.surface && states.surface.some((s) => (s & SURFACE_HOT) !== 0));
  assert.ok(states.team.every((t) => t === NO_TEAM), "states do not paint");
});

test("pickBorder finds the probe that looks at a material border", () => {
  const meshes = [rayMesh(wall())];
  const origin = { x: 0, y: 0, z: 0 };
  const hit = (x: number): Probe => {
    const d = Math.hypot(x, 10);
    return { dir: { x: x / d, y: 0, z: 10 / d }, hit: { t: d, normal: { x: 0, y: 0, z: -1 } } };
  };
  // Far from the border (x = 4.5: only hard rock within 2.5 u) vs on it (x = 0).
  const best = pickBorder([hit(4.5), hit(0), hit(-4.5)], meshes, origin);
  assert.equal(best?.dir.x, 0);
  assert.equal(pickBorder([hit(4.5)], meshes, origin), null);
});
