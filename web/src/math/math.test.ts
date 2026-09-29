import assert from "node:assert/strict";
import { test } from "node:test";
import * as mat4 from "./mat4.ts";
import * as q from "./quat.ts";
import type { Vec3 } from "./vec3.ts";

const close = (a: number, b: number, eps = 1e-5): void => assert.ok(Math.abs(a - b) < eps, `${a} != ${b}`);
const closeV = (a: Vec3, b: Vec3, eps = 1e-5): void => {
  close(a.x, b.x, eps);
  close(a.y, b.y, eps);
  close(a.z, b.z, eps);
};

test("identity axes follow the contract (forward -Z, right +X, up +Y)", () => {
  const i = q.identity();
  closeV(q.forward(i), { x: 0, y: 0, z: -1 });
  closeV(q.right(i), { x: 1, y: 0, z: 0 });
  closeV(q.up(i), { x: 0, y: 1, z: 0 });
});

test("mul(a, b) rotates by b first, then a", () => {
  const a = q.fromAxisAngle({ x: 0, y: 1, z: 0 }, Math.PI / 2);
  const b = q.fromAxisAngle({ x: 1, y: 0, z: 0 }, Math.PI / 2);
  const v = { x: 0, y: 0, z: -1 };
  closeV(q.rotate(q.mul(a, b), v), q.rotate(a, q.rotate(b, v)));
});

test("lookRotation produces the requested forward and a level right axis", () => {
  const fwd = { x: 0.6, y: 0.3, z: -0.74 };
  const len = Math.hypot(fwd.x, fwd.y, fwd.z);
  const rot = q.lookRotation(fwd, { x: 0, y: 1, z: 0 });
  closeV(q.forward(rot), { x: fwd.x / len, y: fwd.y / len, z: fwd.z / len });
  close(q.right(rot).y, 0);
  assert.ok(q.up(rot).y > 0);
  // Also exercise the non-positive-trace branches.
  for (const f of [
    { x: 0, y: 0, z: 1 },
    { x: 1, y: 0, z: 0 },
    { x: -1, y: 0.2, z: 0 },
  ]) {
    const n = Math.hypot(f.x, f.y, f.z);
    closeV(q.forward(q.lookRotation(f, { x: 0, y: 1, z: 0 })), { x: f.x / n, y: f.y / n, z: f.z / n });
  }
});

test("view matrix maps a point ahead of the camera to -Z in view space", () => {
  const pos = { x: 10, y: 5, z: 3 };
  const rot = q.lookRotation({ x: 1, y: 0, z: 0 }, { x: 0, y: 1, z: 0 });
  const view = mat4.viewFromPose(pos, rot);
  const [x, y, z, w] = mat4.transformPoint(view, { x: 14, y: 5, z: 3 });
  close(x, 0);
  close(y, 0);
  close(z, -4);
  close(w, 1);
  // A point to the camera's right lands on +X.
  const [rx] = mat4.transformPoint(view, { x: 10, y: 5, z: 5 });
  close(rx, 2);
});

test("perspective maps near to depth 0 and far to depth 1 (WebGPU range)", () => {
  const p = mat4.perspective(Math.PI / 2, 1, 0.1, 100);
  const near = mat4.transformPoint(p, { x: 0, y: 0, z: -0.1 });
  const far = mat4.transformPoint(p, { x: 0, y: 0, z: -100 });
  close(near[2] / near[3], 0);
  close(far[2] / far[3], 1, 1e-4);
});

test("multiply composes transforms in order", () => {
  const view = mat4.viewFromPose({ x: 1, y: 2, z: 3 }, q.fromAxisAngle({ x: 0, y: 1, z: 0 }, 0.3));
  const proj = mat4.perspective(1, 1.5, 0.1, 50);
  const vp = mat4.multiply(proj, view);
  const pt = { x: 0.5, y: 1.5, z: -4 };
  const a = mat4.transformPoint(vp, pt);
  const [vx, vy, vz] = mat4.transformPoint(view, pt);
  const b = mat4.transformPoint(proj, { x: vx, y: vy, z: vz });
  a.forEach((v, i) => close(v, b[i] ?? NaN, 1e-4));
});
