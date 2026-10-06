import assert from "node:assert/strict";
import { test } from "node:test";

import * as mat4 from "../math/mat4.ts";
import * as q from "../math/quat.ts";
import { SCENES, PASS_BUDGETS_MS } from "../gallery/scenes.ts";
import { MOOD_SCENES } from "../gallery/moodScenes.ts";
import { PASS_ORDER } from "./gpuTimer.ts";
import { WELLS_HEADER_FLOATS, WELLS_KIND, WELLS_MAX_VOLUMES, WELLS_UNIFORM_FLOATS, WELLS_VOLUME_FLOATS, wellsWgsl } from "./shaders/wells.ts";
import { packWorldVolumes, sphereRect, volumeReach, volumesRect, type WorldVolume } from "./wells.ts";

const wellVol = (x = 0): WorldVolume => ({ kind: "well", center: { x, y: 0, z: -20 }, core: 2, lens: 2.6, reach: 9, glow: 2.2, color: [0.66, 0.4, 1], intensity: 0.85, axis: { x: 0, y: 1, z: 0 }, phase: 0 });

test("world volumes pack into the uniform: kind, size, colour linear, at most WELLS_MAX_VOLUMES", () => {
  assert.equal(WELLS_UNIFORM_FLOATS, WELLS_HEADER_FLOATS + WELLS_MAX_VOLUMES * WELLS_VOLUME_FLOATS);
  const out = new Float32Array(WELLS_UNIFORM_FLOATS);
  const vols: WorldVolume[] = [
    wellVol(),
    { kind: "crack", center: { x: 1, y: 2, z: 3 }, radius: 7.5, progress: 0.4, color: [1, 0.5, 0.16], intensity: 1.6, up: { x: 0, y: 1, z: 0 }, seed: 3 },
    { kind: "rubble", center: { x: 4, y: 5, z: 6 }, radius: 7, fresh: 0.7, color: [0.8, 0.74, 0.64], up: { x: 1, y: 0, z: 0 }, seed: 9 },
  ];
  assert.equal(packWorldVolumes(vols, out), 3);
  const at = (i: number, k: number): number => out[WELLS_HEADER_FLOATS + i * WELLS_VOLUME_FLOATS + k]!;
  assert.equal(at(0, 8), WELLS_KIND.well);
  assert.equal(at(0, 3), volumeReach(vols[0]!));
  assert.ok(at(0, 4) < 0.66, "colours go in linear");
  assert.equal(at(1, 8), WELLS_KIND.crack);
  assert.ok(Math.abs(at(1, 9) - 0.4) < 1e-6);
  assert.equal(at(2, 8), WELLS_KIND.rubble);
  assert.deepEqual([at(2, 12), at(2, 13), at(2, 14)], [1, 0, 0], "the level's up");
  const many = Array.from({ length: 20 }, (_, i) => wellVol(i));
  assert.equal(packWorldVolumes(many, out), WELLS_MAX_VOLUMES);
});

test("the pass draws only the screen rectangle the volumes cover", () => {
  const w = 1920, h = 1080;
  const vp = mat4.multiply(mat4.perspective(1.2, w / h, 0.05, 600), mat4.viewFromPose({ x: 0, y: 0, z: 0 }, q.identity()));
  const eye = { x: 0, y: 0, z: 0 };
  const ahead = q.forward(q.identity());
  const far = sphereRect(vp, { x: ahead.x * 80, y: ahead.y * 80, z: ahead.z * 80 }, 3, eye, w, h);
  assert.ok(far && far[2] - far[0] < 200 && far[3] - far[1] < 200, "a distant well is a small rectangle");
  assert.deepEqual(sphereRect(vp, { x: 0, y: 0, z: 0 }, 5, eye, w, h), [0, 0, w, h], "inside the volume: everything");
  const behind = sphereRect(vp, { x: -ahead.x * 50, y: -ahead.y * 50, z: -ahead.z * 50 }, 3, eye, w, h);
  assert.ok(behind === null || (behind[2] - behind[0]) * (behind[3] - behind[1]) === w * h, "behind the camera: skipped (or conservatively all)");
  assert.equal(volumesRect([], vp, eye, w, h), null);
});

test("the shader has the three kinds, the pipeline switches and keeps the scene's alpha", () => {
  for (const ms of [false, true]) {
    const src = wellsWgsl(ms);
    assert.ok(src.includes("override HAS_WELLS") && src.includes("override HAS_SURFACE"));
    assert.ok(src.includes(`== ${WELLS_KIND.crack}u`) && src.includes(`== ${WELLS_KIND.rubble}u`) && src.includes(`== ${WELLS_KIND.well}u`));
    assert.ok(src.includes("return vec4f(col + add, base.a)"), "alpha (sky and keep masks) passes through");
    assert.ok(src.includes(ms ? "texture_depth_multisampled_2d" : "texture_depth_2d"));
  }
  assert.ok(PASS_ORDER.includes("wells"));
  assert.equal(PASS_BUDGETS_MS["wells"], 0.4);
});

test("the B5 gallery scenes are in the gallery", () => {
  const ids = SCENES.map((s) => s.id);
  for (const id of ["wells-close", "wells-debris", "well-lensing", "cave-in-warning", "rubble", "pigment-pickup", "pigmented-vs-grey", "mood-calm", "mood-bored", "mood-grumpy", "mood-giddy", "bored-turn-a", "bored-turn-b", "bored-turn-c"]) {
    assert.ok(ids.includes(id), id);
  }
  for (const s of MOOD_SCENES) {
    assert.equal(s.content, "world");
    assert.equal(s.worldFx, s.id);
  }
  assert.equal(new Set(ids).size, ids.length, "unique ids");
  const ink = MOOD_SCENES.find((s) => s.id === "pigmented-vs-grey");
  assert.equal(ink?.world.palette, 90);
});
