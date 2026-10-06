import assert from "node:assert/strict";
import { test } from "node:test";

import * as q from "../math/quat.ts";
import { FRAME_WGSL } from "./shaders/frame.ts";
import { SKY_UNIFORM_FLOATS, SKY_WGSL } from "./shaders/sky.ts";
import { packSkyUniforms } from "./sky.ts";
import { sliceRows, SPREAD_SLICES, SunSweep } from "./sunShadow.ts";
import { packTurn, turnAngle, turnFromUp } from "./turn.ts";

const close = (a: number, b: number, eps = 1e-6): boolean => Math.abs(a - b) <= eps;

test("the turn takes +Y to the level's up and is the identity while up is +Y", () => {
  const id = turnFromUp({ x: 0, y: 1, z: 0 });
  assert.ok(turnAngle(id, q.identity()) < 1e-6);
  for (const up of [{ x: 1, y: 0, z: 0 }, { x: 0, y: 0, z: -1 }, { x: 0.6, y: 0.8, z: 0 }, { x: -0.3, y: -0.2, z: 0.93 }]) {
    const t = turnFromUp(up);
    const len = Math.hypot(up.x, up.y, up.z);
    const r = q.rotate(t, { x: 0, y: 1, z: 0 });
    assert.ok(close(r.x, up.x / len) && close(r.y, up.y / len) && close(r.z, up.z / len), JSON.stringify(up));
  }
  // A 90 degree turn about a horizontal axis (Bored level) is a 90 degree rotation.
  assert.ok(close(turnAngle(turnFromUp({ x: 1, y: 0, z: 0 }), q.identity()), Math.PI / 2, 1e-6));
  // Straight down still gives a rotation.
  const down = q.rotate(turnFromUp({ x: 0, y: -1, z: 0 }), { x: 0, y: 1, z: 0 });
  assert.ok(close(down.y, -1));
});

test("packTurn keeps heights at the level centre and turns the sky the other way", () => {
  const out = new Float32Array(8);
  const centre = { x: 128, y: 64, z: 128 };
  packTurn(q.identity(), centre, out, 0);
  assert.deepEqual([...out].map((v) => v + 0), [0, 1, 0, 0, 0, 0, 0, 1]);
  const t = turnFromUp({ x: 1, y: 0, z: 0 });
  packTurn(t, centre, out, 0);
  // height(p) = dot(p, up) + w: the centre keeps its height.
  const h = centre.x * out[0]! + centre.y * out[1]! + centre.z * out[2]! + out[3]!;
  assert.ok(close(h, centre.y, 1e-3));
  // The sky quaternion is the inverse turn: the world's up maps back to the sky's +Y.
  const inv = { x: out[4]!, y: out[5]!, z: out[6]!, w: out[7]! };
  const skyUp = q.rotate(inv, { x: 1, y: 0, z: 0 });
  assert.ok(close(skyUp.y, 1, 1e-6));
});

test("the frame and sky uniforms carry the turn", () => {
  assert.ok(FRAME_WGSL.includes("sky_turn: vec4f") && FRAME_WGSL.includes("fn sky_dir"));
  assert.ok(FRAME_WGSL.includes("sky_dir(dir)"), "the fog's haze looks up the turned sky");
  assert.ok(SKY_WGSL.includes("turn: vec4f") && SKY_WGSL.includes("sky_frame(dw)"));
  const t = turnFromUp({ x: 0, y: 0, z: 1 });
  const atmo = {
    sunDirection: { x: 0, y: 1, z: 0 },
    sunSize: 0.01,
    sunColor: [1, 1, 1],
    stars: 1,
    planet: null,
    room: null,
    nebula: { band: { x: 0, y: 1, z: 0 } },
  } as unknown as Parameters<typeof packSkyUniforms>[0];
  const u = packSkyUniforms(atmo, { position: { x: 0, y: 0, z: 0 }, orientation: q.identity(), fovY: 1 }, 100, 100, 0, new Float32Array(SKY_UNIFORM_FLOATS), t);
  assert.deepEqual([u[64], u[65], u[66], u[67]].map((v) => Math.round(v! * 1e6) / 1e6), [-t.x, -t.y, -t.z, t.w].map((v) => Math.round(v * 1e6) / 1e6));
});

test("a turning sun redraws the shadow map in slices and keeps only the newest waiting direction", () => {
  const rows = Array.from({ length: SPREAD_SLICES }, (_, i) => sliceRows(i, SPREAD_SLICES, 4096));
  assert.equal(rows[0]![0], 0);
  assert.equal(rows.at(-1)![1], 4096);
  for (let i = 1; i < rows.length; i++) assert.equal(rows[i]![0], rows[i - 1]![1], "slices tile the map");

  const s = new SunSweep(4);
  assert.equal(s.step(), null, "idle: nothing to draw");
  const a = { x: 0, y: 1, z: 0 };
  const b = { x: 0.1, y: 1, z: 0 };
  const c = { x: 0.2, y: 1, z: 0 };
  s.request(a);
  s.request(b);
  s.request(c);
  assert.equal(s.latest, c);
  const steps = [s.step(), s.step(), s.step(), s.step()];
  assert.deepEqual(steps.map((x) => x?.slice), [0, 1, 2, 3]);
  assert.deepEqual(steps.map((x) => x?.last), [false, false, false, true]);
  assert.equal(s.adopt(), a, "the first direction is finished");
  assert.equal(s.pending, c, "b was superseded by c");
  assert.equal(s.step()?.slice, 0);
  s.cancel();
  assert.equal(s.latest, null);
});
