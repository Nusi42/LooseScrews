import assert from "node:assert/strict";
import { test } from "node:test";
import { bloomMipSizes, DEFAULT_POST, GRADE_PRESETS, NEUTRAL_GRADE, packPostUniforms } from "./post.ts";
import { AUTO_SECONDS, levelForGpuTime, QUALITY_PRESETS, QualityAuto, qualityFromSearch } from "./quality.ts";
import { DEFAULT_SETTINGS } from "./renderer.ts";
import { linearToSrgb, srgbToLinear } from "./shaders/frame.ts";
import { emissiveGain, EMISSIVE_PEAK, INSTANCED_WGSL, TRANSLUCENT_WGSL } from "./shaders/objects.ts";
import { BLOOM_DOWN_WGSL, BLOOM_UP_WGSL, COMPOSITE_WGSL, POST_UNIFORM_FLOATS } from "./shaders/post.ts";
import { TERRAIN_WGSL } from "./shaders/terrain.ts";

const close = (a: number, b: number, eps = 1e-6) => assert.ok(Math.abs(a - b) <= eps, `${a} vs ${b}`);

test("sRGB transfer: known points, monotonic and an exact round trip", () => {
  close(srgbToLinear(0), 0);
  close(srgbToLinear(1), 1);
  close(srgbToLinear(0.5), 0.21404, 1e-5);
  close(linearToSrgb(0.18), 0.46135, 1e-5);
  let prev = -1;
  for (let i = 0; i <= 100; i++) {
    const c = i / 100;
    const l = srgbToLinear(c);
    assert.ok(l > prev);
    prev = l;
    close(linearToSrgb(l), c, 1e-9);
  }
});

test("emissive gain: lit at 0, the HDR peak at 1, monotonic", () => {
  assert.equal(emissiveGain(0), 1);
  assert.equal(emissiveGain(1), EMISSIVE_PEAK);
  assert.ok(EMISSIVE_PEAK >= 4 && EMISSIVE_PEAK <= 8, "bolt cores at 4-8 (plan §5)");
  assert.ok(emissiveGain(0.35) < emissiveGain(0.7));
  // The shader uses the same constant.
  assert.match(INSTANCED_WGSL, new RegExp(`1\\.0 \\+ ${EMISSIVE_PEAK - 1}\\.0 \\* a \\* a`));
});

test("bloom mips halve from half resolution and stop before 2 texels", () => {
  assert.deepEqual(bloomMipSizes(1920, 1080, 6), [
    [960, 540],
    [480, 270],
    [240, 135],
    [120, 68],
    [60, 34],
    [30, 17],
  ]);
  assert.deepEqual(bloomMipSizes(1920, 1080, 4).length, 4);
  assert.deepEqual(bloomMipSizes(8, 8, 6), [
    [4, 4],
    [2, 2],
  ]);
  assert.deepEqual(bloomMipSizes(1920, 1080, 0), []);
});

test("post uniforms follow struct Post: exposure, bloom, grade, look, vignette, grain", () => {
  const grade = { ...GRADE_PRESETS.lastColor, monoTint: [1, 0.42, 0.1] as const, monoKeep: 1 };
  const u = packPostUniforms({ ...DEFAULT_POST, grade }, 12.5, 6);
  assert.equal(u.length, POST_UNIFORM_FLOATS);
  assert.equal(u[3], Math.fround(DEFAULT_POST.exposure));
  assert.equal(u[7], Math.fround(DEFAULT_POST.bloomStrength));
  assert.equal(u[11], 1, "saturation");
  assert.equal(u[12], Math.fround(DEFAULT_POST.agxPower));
  assert.equal(u[14], 1, "monochrome");
  assert.equal(u[15], Math.fround(DEFAULT_POST.vignette));
  assert.equal(u[16], 0, "film grain is off by default");
  assert.equal(u[17], 12.5);
  assert.equal(u[18], 6);
  assert.equal(u[19], 1, "bloom on");
  assert.equal(u[23], 1, "mono keep");
  close(u[20] ?? 0, 1, 1e-6);
  close(u[21] ?? 0, srgbToLinear(0.42), 1e-6);
  // Without bloom mips the bloom strength is zeroed and the flag cleared.
  const off = packPostUniforms(DEFAULT_POST, 0, 0);
  assert.equal(off[7], 0);
  assert.equal(off[19], 0);
  for (const field of ["slope_exposure", "offset_bloom", "power_sat", "look", "misc", "mono_tint"]) {
    assert.ok(COMPOSITE_WGSL.includes(`${field}: vec4f`), field);
  }
});

test("grade presets are complete and neutral is the identity", () => {
  assert.deepEqual(NEUTRAL_GRADE.slope, [1, 1, 1]);
  assert.deepEqual(NEUTRAL_GRADE.offset, [0, 0, 0]);
  assert.deepEqual(NEUTRAL_GRADE.power, [1, 1, 1]);
  assert.equal(NEUTRAL_GRADE.saturation, 1);
  assert.equal(NEUTRAL_GRADE.monochrome, 0);
  for (const g of Object.values(GRADE_PRESETS)) {
    assert.equal(g.slope.length, 3);
    assert.equal(g.offset.length, 3);
    assert.equal(g.power.length, 3);
  }
  assert.equal(DEFAULT_SETTINGS.post.grade, NEUTRAL_GRADE);
});

test("the shaders are split and each scene module carries the frame block and lighting", () => {
  for (const src of [TERRAIN_WGSL, INSTANCED_WGSL, TRANSLUCENT_WGSL]) {
    assert.match(src, /struct Frame/);
    assert.match(src, /fn light\(/);
    assert.match(src, /fn srgb_to_linear\(/);
  }
  // HDR: nothing in the scene shaders clamps radiance to 1.
  for (const src of [TERRAIN_WGSL, INSTANCED_WGSL]) assert.doesNotMatch(src, /saturate|clamp\(color/);
  for (const src of [BLOOM_DOWN_WGSL, BLOOM_UP_WGSL, COMPOSITE_WGSL]) assert.match(src, /fn vs_main/);
  assert.match(COMPOSITE_WGSL, /fn agx\(/);
  assert.match(COMPOSITE_WGSL, /fn linear_to_srgb\(/);
});

test("quality presets: MSAA 4x and 6 bloom mips at high, no MSAA or bloom at low", () => {
  assert.deepEqual(QUALITY_PRESETS.high, { level: "high", msaa: 4, bloomMips: 6, shafts: true });
  assert.equal(QUALITY_PRESETS.medium.msaa, 4);
  assert.equal(QUALITY_PRESETS.medium.bloomMips, 4);
  assert.deepEqual(QUALITY_PRESETS.low, { level: "low", msaa: 1, bloomMips: 0, shafts: false });
  assert.equal(qualityFromSearch("?quality=low"), "low");
  assert.equal(qualityFromSearch("?quality=ultra"), null);
  assert.equal(qualityFromSearch(""), null);
  assert.equal(levelForGpuTime(5), "high");
  assert.equal(levelForGpuTime(8), "high");
  assert.equal(levelForGpuTime(11), "medium");
  assert.equal(levelForGpuTime(20), "low");
});

test("the automatic quality pick waits for the first seconds, picks once and only steps down", () => {
  const slow = new QualityAuto();
  let t = 0;
  let picked = null;
  for (; t < AUTO_SECONDS * 1000 - 20; t += 16) assert.equal(slow.sample(11, t, "high"), null);
  for (; picked === null && t < AUTO_SECONDS * 1000 + 1000; t += 16) picked = slow.sample(11, t, "high");
  assert.equal(picked, "medium");
  assert.ok(slow.decided);
  assert.equal(slow.sample(30, t + 16, "medium"), null, "one shot");

  const fast = new QualityAuto();
  picked = null;
  for (t = 0; picked === null && t < 10_000; t += 16) picked = fast.sample(2, t, "medium");
  assert.equal(picked, "medium", "never steps up");

  const untimed = new QualityAuto();
  for (t = 0; t < 10_000; t += 16) assert.equal(untimed.sample(null, t, "high"), null);
  assert.equal(untimed.decided, false);

  const forced = new QualityAuto("low");
  assert.ok(forced.decided);
  assert.equal(forced.sample(30, 99_999, "low"), null);
});
