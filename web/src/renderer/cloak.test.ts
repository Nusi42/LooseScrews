// Cloak refraction (VQP): uniforms, shader interface and scene routing.

import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import { buildScene } from "../game/scene.ts";
import type { ShipState } from "../game/views.ts";
import { packCloakUniforms } from "./cloak.ts";
import { CLOAK_UNIFORM_FLOATS, cloakWgsl } from "./shaders/cloak.ts";
import { FRAME_UNIFORM_FLOATS, FRAME_WGSL, ROOM_FADE } from "./shaders/frame.ts";

test("cloak uniforms: camera right and up, viewport size and its inverse", () => {
  const u = packCloakUniforms(q.identity(), 1920, 1080);
  assert.equal(u.length, CLOAK_UNIFORM_FLOATS);
  assert.deepEqual([...u.subarray(0, 3)], [1, 0, 0]);
  assert.deepEqual([...u.subarray(4, 7)], [0, 1, 0]);
  assert.deepEqual([u[8], u[9]], [1920, 1080]);
  assert.ok(Math.abs((u[10] ?? 0) - 1 / 1920) < 1e-9);
});

test("cloak shader reads the scene copy and the scene depth of either kind", () => {
  for (const ms of [false, true]) {
    const w = cloakWgsl(ms);
    assert.ok(w.includes("fn vs_main") && w.includes("fn fs_main"));
    assert.ok(w.includes(ms ? "texture_depth_multisampled_2d" : "texture_depth_2d"));
    assert.ok(w.includes("discard"), "dropped behind the scene");
    assert.ok(w.includes("@group(1) @binding(1) var scene_copy"));
  }
});

test("cloaked ships go to the refraction pass when asked, else the old translucent hull", () => {
  const s: ShipState = { id: 5, team: 1, alive: true, boosting: false, position: { x: 0, y: 0, z: -4 }, orientation: q.identity(), shipType: 1, radius: 0.5, gadgetActive: true };
  const cloak = (): string => "cloak";
  const lens = buildScene([s], [], [], [], () => undefined, 0, { gadgetOf: cloak, refractCloak: true });
  const refracted = Object.values(lens.refract ?? {}).flat();
  assert.equal(refracted.length, 1);
  assert.equal(refracted[0]?.opacity, 1);
  assert.equal(Object.values(lens.translucent).flat().length, 0);
  assert.equal(Object.values(lens.opaque).flat().length, 0);
  const flat = buildScene([s], [], [], [], () => undefined, 0, { gadgetOf: cloak });
  assert.equal(Object.values(flat.refract ?? {}).flat().length, 0);
  assert.equal(Object.values(flat.translucent).flat().length, 1);
});

test("the frame uniform carries the room box for the fade at hidden walls", () => {
  assert.ok(FRAME_WGSL.includes("room_min: vec4f") && FRAME_WGSL.includes("room_max: vec4f"));
  assert.ok(FRAME_WGSL.includes("fn room_fade"));
  assert.equal(FRAME_UNIFORM_FLOATS, 60);
  assert.ok(ROOM_FADE > 0 && ROOM_FADE < 10);
});
