import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import { atmosphereFor, worldGrade, worldWeights } from "./atmosphere.ts";
import { FRAME_DEPTH_RANGE, FRAME_FAR, FRAME_NEAR } from "./hullFrame.ts";
import {
  HATCH_SPACING_PX,
  INK,
  INK_PALETTE,
  inkActive,
  inkForPalette,
  isMonochromePalette,
  lineScale,
  MONOCHROME_PALETTE,
  needsLinePass,
  NO_INK,
  packInkUniforms,
  PAPER,
  paletteSaturation,
} from "./ink.ts";
import { DEFAULT_POST, packPostUniforms } from "./post.ts";
import { srgbToLinear } from "./shaders/frame.ts";
import { HATCH_DIRECTIONS, HATCH_THRESHOLDS, INK_FRAME_DEPTH, INK_UNIFORM_FLOATS, inkWgsl } from "./shaders/ink.ts";
import { COMPOSITE_WGSL, KEEP_ALPHA, POST_UNIFORM_FLOATS } from "./shaders/post.ts";
import { TERRAIN_WGSL } from "./shaders/terrain.ts";

const close = (a: number, b: number, eps = 1e-6) => assert.ok(Math.abs(a - b) <= eps, `${a} vs ${b}`);

test("Palette steps (milestone-4-plan.md §3.3): desaturate to 50 %, then ink, then monochrome from 80", () => {
  // 0-49: no ink, saturation 1 -> 0.5.
  for (let p = 0; p < INK_PALETTE; p++) {
    assert.deepEqual(inkForPalette(p), NO_INK, `palette ${p}`);
    assert.equal(inkActive(inkForPalette(p)), false);
  }
  assert.equal(paletteSaturation(0), 1);
  close(paletteSaturation(49), 0.5);
  close(paletteSaturation(40), 1 - (0.5 * 40) / 49);
  // 50-79: outlines, toon and grain fade in over colour; no hatching, no monochrome.
  for (let p = INK_PALETTE; p < MONOCHROME_PALETTE; p++) {
    const ink = inkForPalette(p);
    assert.ok(ink.outline > 0 && ink.toon > 0 && ink.grain > 0, `palette ${p}`);
    assert.equal(ink.hatch, 0);
    assert.equal(ink.mono, 0);
    assert.equal(isMonochromePalette(p), false);
    assert.ok(needsLinePass(ink));
  }
  // 80-100: monochrome ink with hatching, more contrast toward 100.
  for (let p = MONOCHROME_PALETTE; p <= 100; p++) {
    const ink = inkForPalette(p);
    assert.equal(ink.mono, 1, `palette ${p}`);
    assert.equal(ink.outline, 1);
    assert.ok(ink.hatch >= 0.75 && ink.hatch <= 1);
    assert.ok(isMonochromePalette(p));
  }
  assert.equal(inkForPalette(100).contrast, 1);
  assert.equal(inkForPalette(80).contrast, 0);
  // Out of range values clamp.
  assert.deepEqual(inkForPalette(-5), NO_INK);
  assert.deepEqual(inkForPalette(130), inkForPalette(100));
  assert.equal(paletteSaturation(-3), 1);
});

test("the ink fades in monotonically and the saturation never rises with the Palette", () => {
  let prev = inkForPalette(0);
  let prevSat = paletteSaturation(0);
  for (let p = 1; p <= 100; p++) {
    const ink = inkForPalette(p);
    for (const k of ["outline", "toon", "grain", "hatch", "mono", "contrast"] as const) assert.ok(ink[k] >= prev[k], `${k} at ${p}`);
    const sat = paletteSaturation(p);
    assert.ok(sat <= prevSat + 1e-12, `saturation at ${p}`);
    prev = ink;
    prevSat = sat;
  }
  // Continuous where ink starts: 49 -> 50 is one small step of saturation.
  assert.ok(paletteSaturation(49) - paletteSaturation(50) < 0.01);
});

test("the world's grade and atmosphere follow the Palette (worldGrade recalibrated)", () => {
  const caves = worldGrade(worldWeights(50), 0);
  for (const p of [0, 40, 49, 50, 65, 79, 85, 100]) {
    close(worldGrade(worldWeights(50), p).saturation, caves.saturation * paletteSaturation(p), 1e-12);
  }
  const code = "LS04-0000-00VT-PVXV-WPA6-8001-8F6R"; // Palette 0
  assert.deepEqual(atmosphereFor(code).ink, NO_INK);
  assert.deepEqual(atmosphereFor(code, undefined, { palette: 85 }).ink, inkForPalette(85));
  close(atmosphereFor(code, undefined, { palette: 85 }).grade.saturation, atmosphereFor(code).grade.saturation * paletteSaturation(85), 1e-12);
});

test("ink uniforms follow struct Ink: camera basis, footprint, line scale, strengths", () => {
  const camera = { position: { x: 1, y: 2, z: 3 }, orientation: q.identity(), fovY: Math.PI / 3, near: 0.05, far: 600 };
  const look = inkForPalette(90);
  const u = packInkUniforms(camera, 16 / 9, { width: 1920, height: 1080 }, look, 1.4, 0.012);
  assert.equal(u.length, INK_UNIFORM_FLOATS);
  assert.deepEqual([u[0], u[1], u[2]], [1, 2, 3]);
  close(u[3] ?? 0, 0.05);
  close(u[7] ?? 0, 600);
  const tanY = Math.tan(Math.PI / 6);
  // Forward is -z for the identity orientation; right and up are scaled by the half-angle tangents.
  close(u[6] ?? 0, -1);
  close(u[8] ?? 0, tanY * (16 / 9), 1e-6);
  close(u[13] ?? 0, tanY, 1e-6);
  close(u[11] ?? 0, (2 * tanY) / 1080, 1e-9);
  assert.equal(u[15], 1, "line scale 1 at 1080p");
  close(u[16] ?? 0, look.outline);
  close(u[17] ?? 0, look.hatch);
  close(u[18] ?? 0, 1.4);
  close(u[19] ?? 0, 0.012);
  assert.deepEqual([u[20], u[21], u[22]], [1920, 1080, HATCH_SPACING_PX]);
  close(u[23] ?? 0, tanY, 1e-6);
  assert.equal(lineScale(720), 1);
  assert.equal(lineScale(2160), 2);
  for (const field of ["cam", "fwd", "right", "up", "look", "size"]) assert.ok(inkWgsl(true).includes(`${field}: vec4f`), field);
});

test("the ink shader comes in a multisampled and a single-sampled variant and decodes the cockpit frame's depth", () => {
  assert.match(inkWgsl(true), /texture_depth_multisampled_2d/);
  assert.match(inkWgsl(false), /texture_depth_2d/);
  assert.doesNotMatch(inkWgsl(false), /multisampled/);
  assert.equal(INK_FRAME_DEPTH.range, FRAME_DEPTH_RANGE);
  assert.equal(INK_FRAME_DEPTH.near, FRAME_NEAR);
  assert.equal(INK_FRAME_DEPTH.far, FRAME_FAR);
  // Hatch families are unit length, not axis aligned (mine shafts are), and not parallel to each other.
  for (const d of HATCH_DIRECTIONS) {
    close(Math.hypot(...d), 1, 1e-9);
    assert.ok(Math.max(...d.map(Math.abs)) < 0.9, "not axis aligned");
  }
  for (let i = 0; i < HATCH_DIRECTIONS.length; i++) {
    for (let j = i + 1; j < HATCH_DIRECTIONS.length; j++) {
      const a = HATCH_DIRECTIONS[i]!;
      const b = HATCH_DIRECTIONS[j]!;
      assert.ok(Math.abs(a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) < 0.5, `${i} ${j}`);
    }
  }
  // Darker tones add layers.
  assert.ok(HATCH_THRESHOLDS[0] > HATCH_THRESHOLDS[1] && HATCH_THRESHOLDS[1] > HATCH_THRESHOLDS[2]);
});

test("post uniforms carry the ink look; the composite keeps paint and pigment out of the monochrome", () => {
  const ink = inkForPalette(100);
  const u = packPostUniforms({ ...DEFAULT_POST, ink }, 0, 6, undefined, true);
  assert.equal(u.length, POST_UNIFORM_FLOATS);
  assert.deepEqual([u[24], u[25], u[26], u[27]], [ink.toon, ink.grain, ink.mono, ink.contrast]);
  close(u[28] ?? 0, srgbToLinear(PAPER[0]));
  assert.equal(u[31], 1, "ink look on");
  close(u[32] ?? 0, srgbToLinear(INK[0]));
  assert.equal(u[35], 1, "lines valid");
  const off = packPostUniforms(DEFAULT_POST, 0, 6);
  assert.equal(off[31], 0, "no ink by default");
  assert.equal(off[35], 0, "no lines by default");
  for (const field of ["ink_look", "ink_paper", "ink_color"]) assert.ok(COMPOSITE_WGSL.includes(`${field}: vec4f`), field);
  // Kept pixels skip the monochrome and the hatching (not the outlines), and the Palette's desaturation.
  assert.match(COMPOSITE_WGSL, /let mono = post\.ink_look\.z \* \(1\.0 - keep\)/);
  assert.match(COMPOSITE_WGSL, /lines\.r \+ lines\.g \* \(1\.0 - keep\)/);
  assert.match(COMPOSITE_WGSL, /mix\(post\.power_sat\.w, max\(post\.power_sat\.w, 1\.0\), keep_px\)/);
  // Painted rock writes the keep mask.
  assert.match(TERRAIN_WGSL, new RegExp(`keep \\* ${KEEP_ALPHA}`));
  assert.match(TERRAIN_WGSL, /keep = paint_w;/);
});
