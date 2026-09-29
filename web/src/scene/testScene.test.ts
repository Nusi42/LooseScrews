import assert from "node:assert/strict";
import { test } from "node:test";
import { computeAabb, validateMesh } from "../renderer/mesh.ts";
import { CHUNK_SIZE, generateTestScene } from "./testScene.ts";

test("test scene: hundreds of boxes in valid, correctly keyed chunk meshes", () => {
  const scene = generateTestScene();
  assert.ok(scene.boxCount >= 300, `only ${scene.boxCount} boxes`);
  assert.ok(scene.chunks.size > 10);
  for (const [key, mesh] of scene.chunks) {
    validateMesh(mesh);
    const box = computeAabb(mesh.positions)!;
    const [cx, cy, cz] = key.split(",").map(Number) as [number, number, number];
    // Geometry is assigned by box centre, so it may overhang its chunk a little, not a lot.
    const slack = 20;
    assert.ok(box.min.x >= cx * CHUNK_SIZE - slack && box.max.x <= (cx + 1) * CHUNK_SIZE + slack, key);
    assert.ok(box.min.y >= cy * CHUNK_SIZE - slack && box.max.y <= (cy + 1) * CHUNK_SIZE + slack, key);
    assert.ok(box.min.z >= cz * CHUNK_SIZE - slack && box.max.z <= (cz + 1) * CHUNK_SIZE + slack, key);
  }
  assert.ok(scene.paintShare[0]! > 0 && scene.paintShare[1]! > 0);
  assert.ok(scene.paintShare[0]! + scene.paintShare[1]! < 1);
});

test("spawn point is inside the tunnel with clearance", () => {
  const scene = generateTestScene();
  const s = scene.spawnPosition;
  let nearest = Infinity;
  for (const mesh of scene.chunks.values()) {
    for (let i = 0; i < mesh.positions.length; i += 3) {
      const d = Math.hypot(mesh.positions[i]! - s.x, mesh.positions[i + 1]! - s.y, mesh.positions[i + 2]! - s.z);
      nearest = Math.min(nearest, d);
    }
  }
  assert.ok(nearest > 5, `nearest vertex ${nearest}`);
});

test("same seed gives the same scene", () => {
  const a = generateTestScene({ seed: 42 });
  const b = generateTestScene({ seed: 42 });
  assert.deepEqual([...a.chunks.keys()], [...b.chunks.keys()]);
  for (const [k, m] of a.chunks) assert.deepEqual(m.positions, b.chunks.get(k)!.positions);
});
