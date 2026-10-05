import assert from "node:assert/strict";
import { test } from "node:test";
import * as mat4 from "../math/mat4.ts";
import * as q from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import { Frustum } from "./frustum.ts";
import {
  clusterOf,
  falloff,
  froxelBounds,
  froxelsOf,
  HEADLIGHT_SPOT,
  hotSpotAt,
  hotSpots,
  lightInfo,
  lightRange,
  MAX_RANGE,
  packLights,
  selectLights,
  sliceOf,
  sphereInFrustum,
  type PointLight,
} from "./lights.ts";
import { SURFACE_CHARGED, SURFACE_HOT, type MeshData } from "./mesh.ts";
import { QUALITY_PRESETS } from "./quality.ts";
import { CLUSTER_COUNT, CLUSTER_FAR, CLUSTER_NEAR, CLUSTER_X, CLUSTER_Y, CLUSTER_Z, CLUSTER_COMPUTE_WGSL, LIGHT_FLOATS, MAX_LIGHTS } from "./shaders/lights.ts";
import { LIGHTING_WGSL } from "./shaders/lighting.ts";
import { srgbToLinear } from "./shaders/frame.ts";

const at = (x: number, y: number, z: number): Vec3 => ({ x, y, z });
const light = (x: number, y: number, z: number, intensity = 5, extra: Partial<PointLight> = {}): PointLight => ({
  position: at(x, y, z),
  color: [1, 0.5, 0.25],
  intensity,
  ...extra,
});

function camera(pos = at(0, 0, 0), fovY = Math.PI / 2, aspect = 16 / 9): { view: Float32Array; frustum: Frustum; tanX: number; tanY: number } {
  const view = mat4.viewFromPose(pos, q.identity());
  const proj = mat4.perspective(fovY, aspect, 0.05, 600);
  const frustum = new Frustum();
  frustum.setFromViewProj(mat4.multiply(proj, view));
  const tanY = Math.tan(fovY / 2);
  return { view, frustum, tanX: tanY * aspect, tanY };
}

test("falloff: inverse square with a 1 u core, smoothly zero at the range", () => {
  assert.equal(falloff(10, 8, 0), 10);
  let prev = Infinity;
  for (let d = 0; d <= 8; d += 0.25) {
    const v = falloff(10, 8, d);
    assert.ok(v <= prev, `monotonic at ${d}`);
    prev = v;
  }
  assert.equal(falloff(10, 8, 8), 0);
  assert.equal(falloff(10, 8, 9), 0);
  assert.ok(falloff(10, 8, 7.9) < 0.01, "fades out before the cut");
  // Default range: where the light drops to about 2 % of the sun, clamped.
  assert.ok(lightRange(1) >= 2 && lightRange(1) < 10);
  assert.equal(lightRange(1e6), MAX_RANGE);
  assert.ok(lightRange(50) > lightRange(5));
});

test("selectLights keeps in-view lights, best first, up to the cap", () => {
  const cam = camera();
  const lights = [
    light(0, 0, -5, 2),
    light(0, 0, -5, 20), // brighter at the same place
    light(0, 0, 50, 50, { range: 10 }), // behind the camera, out of reach
    light(0, 0, -60, 2), // far ahead
    light(0, 0, -5, 0), // dark
  ];
  const chosen = selectLights(lights, at(0, 0, 0), 128, cam.frustum);
  assert.equal(chosen.length, 3);
  assert.equal(chosen[0]!.intensity, 20);
  assert.equal(chosen[2]!.position.z, -60, "the far light scores lowest");
  assert.deepEqual(selectLights(lights, at(0, 0, 0), 1, cam.frustum).map((l) => l.intensity), [20]);
  // A light behind the camera whose range reaches into the view is kept.
  assert.equal(selectLights([light(0, 0, 3, 20, { range: 8 })], at(0, 0, 0), 8, cam.frustum).length, 1);
  // Never more than MAX_LIGHTS, whatever the caller asks for.
  const many = Array.from({ length: 300 }, (_, i) => light(i % 7, 0, -3 - i * 0.1));
  assert.equal(selectLights(many, at(0, 0, 0), 1000).length, MAX_LIGHTS);
  // Objectives (priority) win over equal lights.
  const prio = selectLights([light(0, 0, -5, 5), light(1, 0, -5, 5, { priority: 3 })], at(0, 0, 0), 1);
  assert.equal(prio[0]!.priority, 3);
});

test("sphereInFrustum culls spheres fully outside one plane", () => {
  const { frustum } = camera();
  assert.equal(sphereInFrustum(frustum.planes, at(0, 0, -10), 1), true);
  assert.equal(sphereInFrustum(frustum.planes, at(0, 0, 10), 1), false);
  assert.equal(sphereInFrustum(frustum.planes, at(0, 0, 10), 11), true);
  assert.equal(sphereInFrustum(frustum.planes, at(-100, 0, -10), 5), false);
});

test("packLights writes world position, range, linear colour x intensity and view-space position", () => {
  const cam = camera(at(1, 2, 3));
  const { data, views } = packLights([light(1, 2, -7, 4, { range: 9 })], cam.view);
  assert.deepEqual([...data.subarray(0, 4)], [1, 2, -7, 9]);
  assert.ok(Math.abs(data[4]! - srgbToLinear(1) * 4) < 1e-6);
  assert.ok(Math.abs(data[5]! - srgbToLinear(0.5) * 4) < 1e-6);
  // 10 u straight ahead of the camera: view x 0, y 0, depth 10.
  assert.ok(Math.abs(views[0]!) < 1e-5 && Math.abs(views[1]!) < 1e-5 && Math.abs(views[2]! - 10) < 1e-5);
  assert.equal(views[3], 9);
  assert.equal(data.length, MAX_LIGHTS * LIGHT_FLOATS);
});

test("froxels: every point lies in the box of its own froxel", () => {
  const { tanX, tanY } = camera();
  const pts: Vec3[] = [];
  for (let i = 0; i < 400; i++) {
    const depth = 0.1 + ((i * 7919) % 1000) * 0.4;
    const nx = (((i * 104729) % 200) / 100 - 1) * 0.999;
    const ny = (((i * 1299709) % 200) / 100 - 1) * 0.999;
    pts.push(at(nx * depth * tanX, ny * depth * tanY, depth));
  }
  for (const p of pts) {
    const c = clusterOf(p, tanX, tanY);
    assert.ok(c >= 0 && c < CLUSTER_COUNT);
    const b = froxelBounds(c, tanX, tanY);
    const eps = 1e-4 * (1 + p.z);
    assert.ok(p.x >= b.min.x - eps && p.x <= b.max.x + eps, `x of ${JSON.stringify(p)}`);
    assert.ok(p.y >= b.min.y - eps && p.y <= b.max.y + eps, `y of ${JSON.stringify(p)}`);
    assert.ok(p.z >= b.min.z - eps && p.z <= b.max.z + eps, `depth of ${JSON.stringify(p)}`);
  }
  // Slices are exponential: the first ends past CLUSTER_NEAR, the last reaches out.
  assert.equal(sliceOf(0.01), 0);
  assert.equal(sliceOf(CLUSTER_FAR * 10), CLUSTER_Z - 1);
  assert.equal(sliceOf(CLUSTER_NEAR * 1.0001), 0);
  assert.ok(sliceOf(10) > sliceOf(5) && sliceOf(100) > sliceOf(10));
});

test("froxel assignment is conservative: every lit point's froxel holds the light", () => {
  const { tanX, tanY } = camera();
  const center = at(1.5, -0.7, 8);
  const range = 4;
  const set = new Set(froxelsOf(center, range, tanX, tanY));
  assert.ok(set.size > 1 && set.size < CLUSTER_COUNT / 4, `${set.size} froxels`);
  for (let i = 0; i < 500; i++) {
    // Points inside the sphere (and in front of the camera).
    const u = (i * 0.618034) % 1, v = (i * 0.414214) % 1, w = (i * 0.732051) % 1;
    const theta = u * Math.PI * 2, phi = Math.acos(2 * v - 1), r = range * Math.cbrt(w);
    const p = at(center.x + r * Math.sin(phi) * Math.cos(theta), center.y + r * Math.sin(phi) * Math.sin(theta), center.z + r * Math.cos(phi));
    if (Math.abs(p.x) > p.z * tanX || Math.abs(p.y) > p.z * tanY) continue;
    assert.ok(set.has(clusterOf(p, tanX, tanY)), `point ${JSON.stringify(p)} misses the light`);
  }
  // A far froxel does not get it.
  assert.equal(set.has(clusterOf(at(0, 0, 100), tanX, tanY)), false);
});

test("lightInfo: count, frustum tangents, slicing and the headlight spot", () => {
  const info = lightInfo(5, { view: new Float32Array(16), tanX: 1.6, tanY: 0.9, forward: at(0, 0, -1) });
  assert.equal(info[0], 5);
  assert.ok(Math.abs(info[1]! - 1.6) < 1e-6 && Math.abs(info[2]! - 0.9) < 1e-6);
  assert.equal(info[3], 1, "spot headlight on");
  assert.ok(Math.abs(info[6]! - CLUSTER_Z / Math.log(CLUSTER_FAR / CLUSTER_NEAR)) < 1e-4);
  assert.deepEqual([...info.subarray(8, 11)], [0, 0, -1]);
  assert.ok(info[11]! < info[12]!, "outer cone is wider than the inner");
  assert.ok(Math.abs(info[11]! - Math.cos((HEADLIGHT_SPOT.outer * Math.PI) / 180)) < 1e-6);
});

test("quality presets cap the lights: 128 high, 32 medium, a few at low", () => {
  assert.equal(QUALITY_PRESETS.high.maxLights, MAX_LIGHTS);
  assert.equal(QUALITY_PRESETS.medium.maxLights, 32);
  assert.ok(QUALITY_PRESETS.low.maxLights > 0 && QUALITY_PRESETS.low.maxLights < 32);
});

function patch(cells: Array<{ x: number; y: number; z: number; n: number; bits: number }>): MeshData {
  const pos: number[] = [], nrm: number[] = [], surface: number[] = [];
  for (const c of cells) {
    for (let i = 0; i < c.n; i++) {
      pos.push(c.x + (i % 3) * 0.3, c.y, c.z + Math.floor(i / 3) * 0.3);
      nrm.push(0, 1, 0);
      surface.push(c.bits);
    }
  }
  return {
    positions: new Float32Array(pos),
    normals: new Float32Array(nrm),
    indices: new Uint32Array(0),
    team: new Uint8Array(surface.length).fill(255),
    surface: new Uint8Array(surface),
  };
}

test("hot spots: one light per glowing patch, off the rock, brighter for bigger patches", () => {
  const mesh = patch([
    { x: 1, y: 1, z: 1, n: 12, bits: SURFACE_HOT },
    { x: 20, y: 1, z: 1, n: 4, bits: SURFACE_HOT },
    { x: 40, y: 1, z: 1, n: 9, bits: SURFACE_CHARGED },
    { x: 60, y: 1, z: 1, n: 2, bits: SURFACE_HOT }, // too small
    { x: 80, y: 1, z: 1, n: 30, bits: 8 }, // scorch does not glow
  ]);
  const spots = hotSpots(mesh);
  assert.equal(spots.length, 3);
  const hot = spots.filter((s) => s.color[0] === 1 && s.color[1] < 0.6);
  assert.equal(hot.length, 2);
  assert.ok(hot[0]!.intensity > hot[1]!.intensity, "the bigger patch is brighter");
  assert.ok(hot[0]!.position.y > 1.5, "pushed off the rock along the normal");
  assert.equal(hotSpots({ ...mesh, surface: new Uint8Array(0) }).length, 0);
  // Hot rock breathes but never goes dark.
  const levels = [0, 0.3, 0.7, 1.1, 2.9].map((t) => hotSpotAt(hot[0]!, t).intensity);
  assert.ok(Math.min(...levels) > 0 && Math.max(...levels) <= hot[0]!.intensity + 1e-9);
  assert.ok(new Set(levels.map((l) => l.toFixed(4))).size > 1);
});

test("shaders: bindings 1-3 in the scene, the froxel grid constants in both passes", () => {
  for (const b of [1, 2, 3]) assert.match(LIGHTING_WGSL, new RegExp(`@group\\(0\\) @binding\\(${b}\\)`));
  assert.doesNotMatch(LIGHTING_WGSL, /@binding\(0\)/, "binding 0 is the Frame uniform (frame.ts)");
  assert.match(LIGHTING_WGSL, /fn light\(n: vec3f, world_pos: vec3f\) -> vec3f/);
  assert.match(LIGHTING_WGSL, /fn light_spec\(/);
  assert.match(CLUSTER_COMPUTE_WGSL, new RegExp(`const CLUSTER_X = ${CLUSTER_X}u`));
  assert.match(CLUSTER_COMPUTE_WGSL, new RegExp(`const CLUSTER_Y = ${CLUSTER_Y}u`));
  assert.match(CLUSTER_COMPUTE_WGSL, /@compute @workgroup_size\(\d+\)/);
});
