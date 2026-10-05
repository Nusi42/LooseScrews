import assert from "node:assert/strict";
import { test } from "node:test";
import {
  clampFov,
  DEFAULT_CAMERA,
  DEFAULT_FOV_DEG,
  FOV_KICK_DEG,
  FovKick,
  fovYFor,
  horizontalFromVertical,
  verticalFromHorizontal,
} from "./camera.ts";

const DEG = Math.PI / 180;
const near = (a: number, b: number, eps = 0.05): void => assert.ok(Math.abs(a - b) < eps, `${a} vs ${b}`);
/** Horizontal FOV in degrees the camera shows for setting `h` at `aspect`. */
const shownH = (h: number, aspect: number, kick = 0): number => horizontalFromVertical(fovYFor(h, aspect, kick), aspect) / DEG;

test("16:9 shows the setting horizontally", () => {
  near(shownH(100, 16 / 9), 100);
  near(shownH(80, 16 / 9), 80);
  near(shownH(120, 16 / 9), 120);
  // 100° horizontal at 16:9 is 67.7° vertical.
  near(fovYFor(100, 16 / 9) / DEG, 67.67);
  near(DEFAULT_CAMERA.fovY / DEG, 67.67);
});

test("4:3 and 16:10 keep the horizontal FOV (the vertical grows)", () => {
  near(shownH(100, 4 / 3), 100);
  near(fovYFor(100, 4 / 3) / DEG, 83.6);
  near(shownH(100, 16 / 10), 100);
  assert.ok(fovYFor(100, 4 / 3) > fovYFor(100, 16 / 9));
});

test("21:9 is Hor+ with the 16:9 vertical, below the 115° clamp", () => {
  const aspect = 21 / 9;
  near(fovYFor(100, aspect), fovYFor(100, 16 / 9), 1e-9);
  const h = shownH(100, aspect);
  assert.ok(h > 100 && h <= 115, `${h}`);
  near(h, 114.8);
});

test("very wide screens clamp at 115° (or at a wider setting)", () => {
  near(shownH(100, 32 / 9), 115);
  near(shownH(90, 32 / 9), 115);
  // A setting above the clamp is kept as the horizontal FOV.
  near(shownH(120, 21 / 9), 120);
  near(shownH(120, 32 / 9), 120);
  // 21:9 at 110 would be 122° Hor+: clamped.
  near(shownH(110, 21 / 9), 115);
});

test("the boost kick adds its degrees on top of the setting", () => {
  near(shownH(100, 16 / 9, FOV_KICK_DEG), 104);
  near(shownH(100, 4 / 3, FOV_KICK_DEG), 104);
  near(shownH(100, 32 / 9, FOV_KICK_DEG), 119);
});

test("setting values are clamped to 80-120 and junk falls back to the default", () => {
  assert.equal(clampFov(60), 80);
  assert.equal(clampFov(150), 120);
  assert.equal(clampFov(99.6), 100);
  assert.equal(clampFov("95"), 95);
  assert.equal(clampFov("x"), DEFAULT_FOV_DEG);
  assert.equal(clampFov(undefined), DEFAULT_FOV_DEG);
  // A broken aspect falls back to 16:9.
  near(fovYFor(100, 0), fovYFor(100, 16 / 9), 1e-9);
  near(fovYFor(100, Number.NaN), fovYFor(100, 16 / 9), 1e-9);
});

test("vertical and horizontal conversions are inverse", () => {
  for (const a of [4 / 3, 16 / 9, 21 / 9]) {
    near(horizontalFromVertical(verticalFromHorizontal(1.7, a), a), 1.7, 1e-9);
  }
});

test("the kick eases in over 250 ms and back out", () => {
  const k = new FovKick();
  assert.equal(k.update(0.1, false), 0);
  const a = k.update(0.0625, true);
  const b = k.update(0.0625, true);
  assert.ok(a > 0 && a < b && b < FOV_KICK_DEG, `${a} ${b}`);
  near(k.update(0.125, true), FOV_KICK_DEG, 1e-9);
  near(k.update(1, true), FOV_KICK_DEG, 1e-9);
  // Half way out after 125 ms (smoothstep is symmetric).
  near(k.update(0.125, false), FOV_KICK_DEG / 2, 1e-9);
  assert.equal(k.update(0.2, false), 0);
  k.update(0.25, true);
  k.reset();
  assert.equal(k.degrees, 0);
});
