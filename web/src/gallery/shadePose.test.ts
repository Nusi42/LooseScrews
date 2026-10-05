import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import { MeshBuilder } from "../renderer/mesh.ts";
import { probeAround, rayMesh, sphereDirections } from "./raycast.ts";
import { pickShadePose, sunlit } from "./shadePose.ts";

/** A horizontal quad at height y over [x0, x1] x [z0, z1], facing `up` (+1) or down (-1). */
function quad(b: MeshBuilder, y: number, x0: number, x1: number, z0: number, z1: number, up: 1 | -1): void {
  const a = b.vertex(x0, y, z0, 0, up, 0);
  const c = b.vertex(x1, y, z0, 0, up, 0);
  const d = b.vertex(x1, y, z1, 0, up, 0);
  const e = b.vertex(x0, y, z1, 0, up, 0);
  b.triangle(a, d, c);
  b.triangle(a, e, d);
}

test("shade pose: a camera under the roof looks at the sunlit floor beyond it", () => {
  // Floor at y = 0 everywhere; a roof at y = 20 over x < 10 only; the sun straight up.
  const b = new MeshBuilder();
  quad(b, 0, -60, 60, -60, 60, 1);
  quad(b, 20, -60, 10, -60, 60, -1);
  const meshes = [rayMesh(b.build())];
  const sun = { x: 0, y: 1, z: 0 };
  const origin = { x: -20, y: 8, z: 0 };
  assert.equal(sunlit(meshes, origin, sun), false);
  assert.equal(sunlit(meshes, { x: 30, y: 1, z: 0 }, sun), true);
  const probes = probeAround(meshes, origin, sphereDirections(64), 160);
  const pose = pickShadePose(meshes, probes, origin, sun, null);
  assert.ok(pose);
  assert.equal(sunlit(meshes, pose.position, sun), false, "the camera stands in the shade");
  const f = q.forward(pose.orientation);
  assert.ok(f.x > 0.3, `looks out toward the lit floor (forward ${f.x.toFixed(2)}, ${f.y.toFixed(2)}, ${f.z.toFixed(2)})`);
});

test("shade pose: none when nothing is in the shade", () => {
  const b = new MeshBuilder();
  quad(b, 0, -60, 60, -60, 60, 1);
  const meshes = [rayMesh(b.build())];
  const origin = { x: 0, y: 8, z: 0 };
  const probes = probeAround(meshes, origin, sphereDirections(32), 160);
  assert.equal(pickShadePose(meshes, probes, origin, { x: 0, y: 1, z: 0 }, null), null);
});
