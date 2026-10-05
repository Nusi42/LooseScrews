import assert from "node:assert/strict";
import { test } from "node:test";
import type { Vec3 } from "../math/vec3.ts";
import { QUALITY_PRESETS } from "./quality.ts";
import { LIGHTING_WGSL } from "./shaders/lighting.ts";
import { SHADOW_CASTER_WGSL, SHELL_CASTER_MARGIN, SUN_SHADOW_UNIFORM_FLOATS, SUN_SHADOW_WGSL } from "./shaders/shadow.ts";
import { TERRAIN_WGSL } from "./shaders/terrain.ts";
import { INSTANCED_WGSL } from "./shaders/objects.ts";
import { VOLUMETRIC_UNIFORM_FLOATS, volumetricWgsl } from "./shaders/volumetric.ts";
import {
  addDirty,
  boxContains,
  boxUnion,
  MAX_DIRTY_RECTS,
  rectsOverlap,
  shadowClip,
  shadowFrame,
  SHIP_SHADOW_RANGE,
  SHIP_SHADOW_SIZE,
  shipShadowFrame,
  snapBox,
  sunAxes,
  texelRect,
  type Rect,
} from "./sunShadow.ts";
import { marchSize, VOLUMETRIC_LOOKS, volumetricLook } from "./volumetric.ts";

const LEVEL = { min: { x: 0, y: 0, z: 0 }, max: { x: 256, y: 128, z: 256 } };
const SUNS: Vec3[] = [
  { x: 0.35, y: 0.85, z: 0.4 },
  { x: -0.6, y: 0.47, z: 0.2 },
  { x: 0.1, y: 0.99, z: -0.05 },
  { x: 0, y: 1, z: 0 },
];

const norm = (v: Vec3): Vec3 => {
  const l = Math.hypot(v.x, v.y, v.z);
  return { x: v.x / l, y: v.y / l, z: v.z / l };
};
const along = (p: Vec3, d: Vec3, t: number): Vec3 => ({ x: p.x + d.x * t, y: p.y + d.y * t, z: p.z + d.z * t });

test("sun axes are orthonormal and right-handed (r x u = s, so sun-facing triangles are front faces)", () => {
  for (const sun of SUNS) {
    const { r, u, s } = sunAxes(sun);
    const dot = (a: Vec3, b: Vec3): number => a.x * b.x + a.y * b.y + a.z * b.z;
    for (const v of [r, u, s]) assert.ok(Math.abs(dot(v, v) - 1) < 1e-9);
    assert.ok(Math.abs(dot(r, u)) < 1e-9 && Math.abs(dot(r, s)) < 1e-9 && Math.abs(dot(u, s)) < 1e-9);
    const c = { x: r.y * u.z - r.z * u.y, y: r.z * u.x - r.x * u.z, z: r.x * u.y - r.y * u.x };
    assert.ok(Math.abs(dot(c, s) - 1) < 1e-9);
  }
});

test("the level's corners land inside the map, depth 0 toward the sun", () => {
  for (const sun of SUNS) {
    const f = shadowFrame(sun, LEVEL, 4096);
    for (let i = 0; i < 8; i++) {
      const c = { x: i & 1 ? 256 : 0, y: i & 2 ? 128 : 0, z: i & 4 ? 256 : 0 };
      const [x, y, z] = shadowClip(f.clip, c);
      assert.ok(x > -1 && x < 1 && y > -1 && y < 1, `xy ${x} ${y}`);
      assert.ok(z > 0 && z < 1, `z ${z}`);
    }
    const s = norm(sun);
    const p = { x: 128, y: 64, z: 128 };
    const a = shadowClip(f.clip, p);
    const b = shadowClip(f.clip, along(p, s, 10));
    // Moving toward the sun keeps the texel and lowers the depth by 10 u of the range.
    assert.ok(Math.abs(a[0] - b[0]) < 1e-5 && Math.abs(a[1] - b[1]) < 1e-5);
    assert.ok(Math.abs(a[2] - b[2] - 10 / f.depthRange) < 1e-5);
    // About 0.09 u per texel over the default level at 4096.
    assert.ok(f.texel[0] < 0.11 && f.texel[1] < 0.11, `texel ${f.texel}`);
  }
});

test("a point under rock compares deeper than the rock's far side; the rock's top does not", () => {
  // Rock slab between y = 60 and y = 70: the map stores its back face as seen
  // from the sun (the bottom, y = 60) at every texel the slab covers.
  for (const sun of SUNS) {
    const f = shadowFrame(sun, LEVEL, 4096);
    const s = norm(sun);
    const floor = { x: 128, y: 20, z: 128 };
    // Where the ray from the floor toward the sun meets the slab's bottom.
    const t = (60 - floor.y) / s.y;
    const bottom = along(floor, s, t);
    const stored = shadowClip(f.clip, bottom)[2];
    assert.ok(shadowClip(f.clip, floor)[2] > stored, "floor under the slab is shadowed");
    const top = along(floor, s, (70 - floor.y) / s.y);
    assert.ok(shadowClip(f.clip, top)[2] < stored, "the slab's sunlit top lies in front of its own far side");
  }
});

test("texel rectangles: cover a chunk's projection, clamp to the map, null when outside", () => {
  const f = shadowFrame(SUNS[0]!, LEVEL, 4096);
  const chunk = { min: { x: 96, y: 32, z: 96 }, max: { x: 128, y: 64, z: 128 } };
  const r = texelRect(f.clip, chunk, 4096)!;
  assert.ok(r);
  for (let i = 0; i < 8; i++) {
    const c = { x: i & 1 ? 128 : 96, y: i & 2 ? 64 : 32, z: i & 4 ? 128 : 96 };
    const [x, y] = shadowClip(f.clip, c);
    const px = (x * 0.5 + 0.5) * 4096, py = (0.5 - y * 0.5) * 4096;
    assert.ok(px >= r[0] && px <= r[2] && py >= r[1] && py <= r[3]);
  }
  // A chunk's rectangle is a small part of the map (partial updates pay off).
  assert.ok((r[2] - r[0]) * (r[3] - r[1]) < 0.05 * 4096 * 4096);
  const whole = texelRect(f.clip, LEVEL, 4096)!;
  assert.deepEqual(whole.slice(0, 2), [0, 0]);
  assert.ok(whole[2] === 4096 && whole[3] === 4096);
  assert.equal(texelRect(f.clip, { min: { x: 5000, y: 0, z: 5000 }, max: { x: 5032, y: 32, z: 5032 } }, 4096), null);
});

test("dirty rectangles merge when they overlap and collapse past the cap", () => {
  let list: Rect[] = [];
  list = addDirty(list, [0, 0, 10, 10]);
  list = addDirty(list, [5, 5, 20, 20]);
  assert.deepEqual(list, [[0, 0, 20, 20]]);
  list = addDirty(list, [100, 100, 110, 110]);
  assert.equal(list.length, 2);
  // A rectangle bridging both merges all three.
  list = addDirty(list, [15, 15, 105, 105]);
  assert.deepEqual(list, [[0, 0, 110, 110]]);
  let many: Rect[] = [];
  for (let i = 0; i <= MAX_DIRTY_RECTS; i++) many = addDirty(many, [i * 100, 0, i * 100 + 10, 10]);
  assert.equal(many.length, 1);
  assert.deepEqual(many[0], [0, 0, MAX_DIRTY_RECTS * 100 + 10, 10]);
  assert.ok(rectsOverlap([0, 0, 2, 2], [1, 1, 3, 3]) && !rectsOverlap([0, 0, 2, 2], [2, 0, 4, 2]));
});

test("the region snaps to the chunk grid and grows by union", () => {
  const a = snapBox({ min: { x: 3, y: -1, z: 33 }, max: { x: 30, y: 31.5, z: 64 } });
  assert.deepEqual(a, { min: { x: 0, y: -32, z: 32 }, max: { x: 32, y: 32, z: 64 } });
  const b = boxUnion(a, { min: { x: 64, y: 0, z: 0 }, max: { x: 96, y: 32, z: 32 } });
  assert.ok(boxContains(b, a) && !boxContains(a, b));
});

/** Floats of a WGSL struct made of vec4f / mat4x4f fields. */
function structFloats(wgsl: string, name: string): number {
  const m = new RegExp(`struct ${name} \\{([^}]*)\\}`).exec(wgsl);
  assert.ok(m, `struct ${name}`);
  let n = 0;
  for (const line of (m[1] ?? "").split("\n")) {
    const field = /:\s*([^,]+),/.exec(line.replace(/\/\/.*$/, ""));
    if (!field) continue;
    const type = (field[1] ?? "").trim();
    if (type === "vec4f") n += 4;
    else if (type === "mat4x4f") n += 16;
    else assert.fail(`unexpected field type ${type} in ${name}`);
  }
  return n;
}

test("uniform sizes match the WGSL structs; scene shaders take the sun's visibility", () => {
  assert.equal(structFloats(SUN_SHADOW_WGSL, "SunShadow"), SUN_SHADOW_UNIFORM_FLOATS);
  assert.equal(structFloats(SHADOW_CASTER_WGSL, "SunShadow"), SUN_SHADOW_UNIFORM_FLOATS);
  assert.equal(structFloats(volumetricWgsl(true), "Volumetric"), VOLUMETRIC_UNIFORM_FLOATS);
  for (const b of [8, 9, 10, 11]) assert.match(LIGHTING_WGSL, new RegExp(`@group\\(0\\) @binding\\(${b}\\)`));
  assert.match(LIGHTING_WGSL, /sun_vis_last = sun_visibility\(world_pos, n\)/);
  // Terrain's GGX sun highlight and the models' sun highlight are shadowed too.
  assert.match(TERRAIN_WGSL, /sun_rad \* sun_vis_last/);
  assert.match(INSTANCED_WGSL, /\* sun_vis_last;/);
  assert.match(volumetricWgsl(false), /texture_depth_2d;/);
  assert.match(volumetricWgsl(true), /texture_depth_multisampled_2d;/);
});

test("volumetric look: no dust in open space, dusty caves; off at low quality", () => {
  assert.equal(VOLUMETRIC_LOOKS.sky.scatter, 0);
  const cave = volumetricLook({ sky: 0, cave: 1, mine: 0 });
  assert.deepEqual(cave, VOLUMETRIC_LOOKS.cave);
  // Half-open worlds (Enclosure 36: cave weight 0.72) keep clear air; nearly closed ones fill up.
  assert.equal(volumetricLook({ sky: 0.28, cave: 0.72, mine: 0 }).scatter, 0);
  const nearly = volumetricLook({ sky: 0.06, cave: 0.94, mine: 0 }).scatter;
  assert.ok(nearly > 0.8 * VOLUMETRIC_LOOKS.cave.scatter && nearly < VOLUMETRIC_LOOKS.cave.scatter);
  assert.equal(QUALITY_PRESETS.low.volumetric.divisor, 0);
  assert.ok(QUALITY_PRESETS.high.volumetric.steps >= QUALITY_PRESETS.medium.volumetric.steps);
  assert.deepEqual(marchSize(1920, 1080, 2), { width: 960, height: 540 });
  assert.deepEqual(marchSize(1919, 1081, 3), { width: 640, height: 361 });
});

test("ship shadows (D5): the ship map spans the range around the camera, snapped to texels", () => {
  for (const sun of SUNS) {
    const s = norm(sun);
    const cam = { x: 101.3, y: 47.9, z: 88.2 };
    const f = shipShadowFrame(s, cam);
    assert.ok(Math.abs(f.texel[0] - (2 * SHIP_SHADOW_RANGE) / SHIP_SHADOW_SIZE) < 1e-9);
    const [x, y, z] = shadowClip(f.clip, cam);
    // The camera sits within a texel of the centre, halfway down the depth range.
    assert.ok(Math.abs(x) <= 1 / SHIP_SHADOW_SIZE + 1e-6 && Math.abs(y) <= 1 / SHIP_SHADOW_SIZE + 1e-6, `${x} ${y}`);
    assert.ok(Math.abs(z - 0.5) < 1e-6);
    // Toward the sun is nearer (smaller depth); a point RANGE across the sun lies at the map's edge.
    assert.ok(shadowClip(f.clip, along(cam, s, 10))[2] < z);
    const { r } = sunAxes(s);
    assert.ok(Math.abs(shadowClip(f.clip, along(cam, r, SHIP_SHADOW_RANGE))[0] - 1) < 2 / SHIP_SHADOW_SIZE + 1e-6);
    // The map moves in whole texels only (no crawling shadow edges).
    const moved = shipShadowFrame(s, along(cam, r, f.texel[0] * 3.4));
    const shift = ((moved.clip[12] ?? 0) - (f.clip[12] ?? 0)) * (SHIP_SHADOW_SIZE / 2);
    assert.ok(Math.abs(shift - Math.round(shift)) < 1e-3, `${shift}`);
  }
});

test("ship shadows multiply the rock's; the Squeeze shell casts none (D5)", () => {
  // Receivers: both maps, the ship map only around the camera.
  assert.ok(SUN_SHADOW_WGSL.includes("return terrain_visibility(p, n) * ship_visibility(p, n);"));
  assert.ok(SUN_SHADOW_WGSL.includes("var ship_shadow_map: texture_depth_2d;"));
  // Casters: hull instances into the ship map; rock beyond the shell sphere is dropped.
  assert.ok(SHADOW_CASTER_WGSL.includes("fn vs_ship("));
  assert.ok(SHADOW_CASTER_WGSL.includes("return sun_shadow.ship_clip * vec4f(world, 1.0);"));
  assert.ok(SHADOW_CASTER_WGSL.includes(`distance(in.world, s.xyz) > s.w - ${SHELL_CASTER_MARGIN}) { discard; }`));
  assert.ok(SHELL_CASTER_MARGIN > 0 && SHELL_CASTER_MARGIN < 2);
});
