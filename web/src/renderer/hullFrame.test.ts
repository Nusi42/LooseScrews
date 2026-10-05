import assert from "node:assert/strict";
import { test } from "node:test";
import { KEEP_ALPHA } from "./shaders/post.ts";
import * as q from "../math/quat.ts";
import { FRAME_EYES, frameModel, hullFit } from "./hullFrameModels.ts";
import { FRAME_DEPTH_RANGE, FRAME_FAR, FRAME_NEAR, HULL_FRAME_UNIFORM_FLOATS, HULL_FRAME_WGSL, packHullFrameUniforms } from "./hullFrame.ts";
import { PART_LIVERY } from "./models/kit.ts";
import { hullOf } from "./models/ships.ts";

const TAN_H = Math.tan((50 * Math.PI) / 180);
const TAN_V = TAN_H / (16 / 9);

/** Screen position (-1..1) of a view-space point at 100° horizontal, 16:9; null behind the eye. */
function project(p: readonly number[]): [number, number] | null {
  const [x, y, z] = p as [number, number, number];
  if (z > -1e-3) return null;
  return [x / -z / TAN_H, y / -z / TAN_V];
}

for (const [type, name] of ["Skitter", "Tinker", "Hauler", "Tug"].entries()) {
  test(`${name}: the cockpit frame keeps the middle of the view clear`, () => {
    const m = frameModel(type);
    assert.ok(m.triangleCount > 200 && m.triangleCount < 4000, `${m.triangleCount} triangles`);
    let onScreen = 0;
    let lowestNose = Infinity;
    for (const t of m.tris) {
      for (const p of [t.a, t.b, t.c]) {
        const s = project(p);
        if (!s || Math.abs(s[0]) > 1 || Math.abs(s[1]) > 1) continue;
        onScreen++;
        // Nothing in the middle of the view: the centre 35 % of the width
        // between the lower 30 % and a top bow along the upper edge.
        assert.ok(!(Math.abs(s[0]) < 0.35 && s[1] > -0.4 && s[1] < 0.8), `${name}: vertex ${p} at screen ${s}`);
        // ...and the nose stays at least 15° below the crosshair.
        if (Math.abs(s[0]) < 0.35 && s[1] < 0) lowestNose = Math.min(lowestNose, Math.atan2(-p[1]!, -p[2]!));
      }
    }
    assert.ok(onScreen > 50, `${name}: only ${onScreen} vertices in view`);
    assert.ok(lowestNose >= (15 * Math.PI) / 180, `${name}: nose ${(lowestNose * 180) / Math.PI}° below`);
  });

  test(`${name}: the frame shows the hull's real length and the team livery`, () => {
    const m = frameModel(type);
    const fit = hullFit(type);
    const eye = FRAME_EYES[type]!;
    // The nose tip lies where the drawn hull's tip is, seen from the eye.
    const tip = hullOf(type).model.bounds().min[2];
    assert.ok(Math.abs(m.bounds().min[2] - (tip - eye[2] * fit)) < 0.02, `${m.bounds().min[2]} vs ${tip}`);
    assert.ok(fit > 0.6 && fit < 1.4, `fit ${fit}`);
    assert.ok(m.tris.some((t) => t.part === PART_LIVERY));
  });
}

test("unknown ship types get the Tinker's frame", () => {
  assert.equal(frameModel(9).triangleCount, frameModel(1).triangleCount);
});

test("hull frame uniforms: projection, camera, colour and scale by radius", () => {
  const u = packHullFrameUniforms(
    { position: { x: 1, y: 2, z: 3 }, orientation: q.identity(), fovY: 1.2 },
    16 / 9,
    { shipType: 2, radius: 0.65, color: [0.1, 0.2, 0.3], glow: 1 },
  );
  assert.equal(u.length, HULL_FRAME_UNIFORM_FLOATS);
  assert.ok(Math.abs((u[5] ?? 0) - 1 / Math.tan(0.6)) < 1e-6);
  // near/far of the frame's own projection: m[14] = n f / (n - f).
  assert.ok(Math.abs((u[14] ?? 0) - (FRAME_NEAR * FRAME_FAR) / (FRAME_NEAR - FRAME_FAR)) < 1e-6);
  assert.deepEqual([...u.subarray(16, 20)], [0, 0, 0, 1]);
  assert.deepEqual([...u.subarray(20, 23)], [1, 2, 3]);
  assert.ok(Math.abs((u[28] ?? 0) - 1.3) < 1e-6);
  assert.equal(u[29], 1);
  assert.ok(HULL_FRAME_WGSL.includes(`clip.z * ${FRAME_DEPTH_RANGE}`));
  assert.equal(u[27], 0, "no colour kept outside Last Color Standing");
});

test("Last Color Standing: the own cockpit frame writes the keep mask like the other hulls (D5)", () => {
  const view = { position: { x: 0, y: 0, z: 0 }, orientation: q.identity(), fovY: 1.2 };
  const state = { shipType: 1, radius: 0.5, color: [1, 0.5, 0] as const };
  assert.equal(packHullFrameUniforms(view, 1, { ...state, keepColor: 1 })[27], 1);
  assert.equal(packHullFrameUniforms(view, 1, { ...state, keepColor: 3 })[27], 1, "clamped");
  assert.ok(HULL_FRAME_WGSL.includes(`1.0 + clamp(hull.color.w, 0.0, 1.0) * ${KEEP_ALPHA}`));
});
