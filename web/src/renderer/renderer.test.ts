import assert from "node:assert/strict";
import { test } from "node:test";
import * as mat4 from "../math/mat4.ts";
import * as q from "../math/quat.ts";
import { Frustum, type Aabb } from "./frustum.ts";
import { computeAabb, expandTeams, MeshBuilder, MeshError, NO_TEAM, validateMesh, type MeshData } from "./mesh.ts";
import { appendBox, appendOrientedBox, projectileMesh, shipMesh } from "./primitives.ts";
import { INSTANCE_FLOATS, packInstances } from "./renderer.ts";

function frustumLookingDownNegZ(): Frustum {
  const view = mat4.viewFromPose({ x: 0, y: 0, z: 0 }, q.identity());
  const proj = mat4.perspective(Math.PI / 2, 1, 0.1, 100);
  const f = new Frustum();
  f.setFromViewProj(mat4.multiply(proj, view));
  return f;
}

const box = (x: number, y: number, z: number, h = 1): Aabb => ({
  min: { x: x - h, y: y - h, z: z - h },
  max: { x: x + h, y: y + h, z: z + h },
});

test("frustum keeps boxes in view and culls boxes behind, beside and beyond", () => {
  const f = frustumLookingDownNegZ();
  assert.equal(f.intersectsAabb(box(0, 0, -10)), true);
  assert.equal(f.intersectsAabb(box(0, 0, 10)), false, "behind");
  assert.equal(f.intersectsAabb(box(-50, 0, -10)), false, "left");
  assert.equal(f.intersectsAabb(box(0, 50, -10)), false, "above");
  assert.equal(f.intersectsAabb(box(0, 0, -200)), false, "beyond far");
  // Straddling the left plane is kept (conservative test).
  assert.equal(f.intersectsAabb(box(-10.5, 0, -10)), true);
  // A box containing the camera is always visible.
  assert.equal(f.intersectsAabb(box(0, 0, 0, 5)), true);
});

function sampleMesh(): MeshData {
  const b = new MeshBuilder();
  appendBox(b, 0, 0, 0, 1, 2, 3, (n) => (n[1] > 0.5 ? 1 : NO_TEAM));
  return b.build();
}

test("mesh layout: 24 vertices / 36 indices per box, one team byte per vertex", () => {
  const m = sampleMesh();
  assert.equal(m.positions.length, 24 * 3);
  assert.equal(m.normals.length, 24 * 3);
  assert.equal(m.team.length, 24);
  assert.equal(m.indices.length, 36);
  assert.ok(m.team instanceof Uint8Array && m.indices instanceof Uint32Array);
  assert.equal([...m.team].filter((t) => t === 1).length, 4, "only the top face is painted");
  assert.equal([...m.team].filter((t) => t === NO_TEAM).length, 20, "the rest is unpainted");
  validateMesh(m);
  assert.deepEqual(computeAabb(m.positions), { min: { x: -1, y: -2, z: -3 }, max: { x: 1, y: 2, z: 3 } });
  assert.deepEqual([...expandTeams(new Uint8Array([0, 1, 255]))], [0, 1, 255]);
});

test("validateMesh rejects inconsistent meshes", () => {
  const m = sampleMesh();
  assert.throws(() => validateMesh({ ...m, team: new Uint8Array(3) }), MeshError);
  assert.throws(() => validateMesh({ ...m, normals: new Float32Array(3) }), MeshError);
  assert.throws(() => validateMesh({ ...m, indices: new Uint32Array([0, 1, 999]) }), MeshError);
  assert.equal(computeAabb(new Float32Array(0)), null);
});

/** Every triangle of a convex mesh around `centre` faces away from it (CCW outward). */
function assertOutwardWinding(m: MeshData, centre = { x: 0, y: 0, z: 0 }): void {
  const p = (i: number): { x: number; y: number; z: number } => ({
    x: m.positions[i * 3]!,
    y: m.positions[i * 3 + 1]!,
    z: m.positions[i * 3 + 2]!,
  });
  for (let t = 0; t < m.indices.length; t += 3) {
    const ia = m.indices[t]!;
    const a = p(ia), b = p(m.indices[t + 1]!), c = p(m.indices[t + 2]!);
    const ux = b.x - a.x, uy = b.y - a.y, uz = b.z - a.z;
    const vx = c.x - a.x, vy = c.y - a.y, vz = c.z - a.z;
    const n = { x: uy * vz - uz * vy, y: uz * vx - ux * vz, z: ux * vy - uy * vx };
    const g = {
      x: (a.x + b.x + c.x) / 3 - centre.x,
      y: (a.y + b.y + c.y) / 3 - centre.y,
      z: (a.z + b.z + c.z) / 3 - centre.z,
    };
    assert.ok(n.x * g.x + n.y * g.y + n.z * g.z > 0, `triangle ${t / 3} is wound inward`);
    const sn = { x: m.normals[ia * 3]!, y: m.normals[ia * 3 + 1]!, z: m.normals[ia * 3 + 2]! };
    assert.ok(sn.x * n.x + sn.y * n.y + sn.z * n.z > 0, `normal of triangle ${t / 3} disagrees`);
  }
}

test("boxes, oriented boxes and primitives are wound counter-clockwise outward", () => {
  assertOutwardWinding(sampleMesh());
  const b = new MeshBuilder();
  const rot = q.normalize({ x: 0.3, y: -0.5, z: 0.2, w: 0.8 });
  const r = q.right(rot), u = q.up(rot), f = q.rotate(rot, { x: 0, y: 0, z: 1 });
  // Deliberately left-handed axis order: the winding must still come out right.
  appendOrientedBox(b, [5, 6, 7], [f.x * 2, f.y * 2, f.z * 2], [u.x, u.y, u.z], [r.x * 3, r.y * 3, r.z * 3]);
  assertOutwardWinding(b.build(), { x: 5, y: 6, z: 7 });
  assertOutwardWinding(projectileMesh());
  assertOutwardWinding(shipMesh(), { x: 0, y: 0.02, z: 0.2 });
});

test("ship mesh points down -Z and is about one ship length long", () => {
  const aabb = computeAabb(shipMesh().positions)!;
  assert.ok(aabb.min.z < -0.5 && aabb.max.z > 0.4);
  const length = aabb.max.z - aabb.min.z;
  assert.ok(length > 0.9 && length < 1.1, `length ${length}`);
});

test("packInstances writes offset+scale, rotation, colour+emissive", () => {
  const data = packInstances([
    { position: { x: 1, y: 2, z: 3 }, rotation: { x: 0, y: 0, z: 0, w: 1 }, scale: 2, color: [0.5, 0.25, 1], emissive: 1 },
    { position: { x: 4, y: 5, z: 6 }, rotation: { x: 0, y: 1, z: 0, w: 0 }, scale: 1, color: [0, 0, 0], emissive: 0 },
  ]);
  assert.equal(data.length, 2 * INSTANCE_FLOATS);
  assert.deepEqual([...data.slice(0, 12)], [1, 2, 3, 2, 0, 0, 0, 1, 0.5, 0.25, 1, 1]);
  assert.deepEqual([...data.slice(12, 16)], [4, 5, 6, 1]);
  const out = new Float32Array(64);
  assert.equal(packInstances([], out), out, "reuses a large enough output buffer");
});
