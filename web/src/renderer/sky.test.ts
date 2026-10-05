import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import { atmosphereFor } from "./atmosphere.ts";
import { FRAME_UNIFORM_FLOATS, FRAME_WGSL } from "./shaders/frame.ts";
import {
  ENV_BAKE_WGSL,
  ENV_UNIFORM_FLOATS,
  PLANET_BAKE_WGSL,
  PLANET_UNIFORM_FLOATS,
  SHAFT_BLUR_WGSL,
  SHAFT_UNIFORM_FLOATS,
  SKY_UNIFORM_FLOATS,
  SKY_WGSL,
} from "./shaders/sky.ts";
import { ENV_MIPS, ENV_SIZE, gridStrength, GRID_RANGE, packEnvUniforms, packSkyUniforms, planetCone } from "./sky.ts";

/** Floats of a WGSL struct made of vec4f / mat4x4f / array<vec4f, N> fields. */
function structFloats(wgsl: string, name: string): number {
  const m = new RegExp(`struct ${name} \\{([^}]*)\\}`).exec(wgsl);
  assert.ok(m, `struct ${name}`);
  let n = 0;
  for (const line of (m[1] ?? "").split("\n")) {
    const field = /:\s*(array<[^>]*>|[^,]+),/.exec(line.replace(/\/\/.*$/, ""));
    if (!field) continue;
    const type = (field[1] ?? "").trim();
    if (type === "vec4f") n += 4;
    else if (type === "mat4x4f") n += 16;
    else if (/^array<vec4f, \d+>$/.test(type)) n += 4 * Number(/\d+/.exec(type)?.[0]);
    else assert.fail(`unexpected field type ${type} in ${name}`);
  }
  return n;
}

test("uniform sizes match the WGSL structs", () => {
  assert.equal(structFloats(SKY_WGSL, "Sky"), SKY_UNIFORM_FLOATS);
  assert.equal(structFloats(ENV_BAKE_WGSL, "Env"), ENV_UNIFORM_FLOATS);
  assert.equal(structFloats(PLANET_BAKE_WGSL, "PlanetBake"), PLANET_UNIFORM_FLOATS);
  assert.equal(structFloats(SHAFT_BLUR_WGSL, "Shaft"), SHAFT_UNIFORM_FLOATS);
  assert.equal(structFloats(FRAME_WGSL, "Frame"), FRAME_UNIFORM_FLOATS);
});

test("the environment cube has a full mip chain", () => {
  assert.equal(ENV_SIZE >> (ENV_MIPS - 1), 1);
});

const SKY = atmosphereFor("LS0C-0000-0000-0000-8006-8001-8F9M", [0, 0, 0, 256, 128, 256]);

test("packSkyUniforms: camera basis scaled by the field of view, pixel size, sun, planet, room, band", () => {
  const cam = { position: { x: 100, y: 60, z: 120 }, orientation: q.identity(), fovY: Math.PI / 2 };
  const d = packSkyUniforms(SKY, cam, 1600, 900, 3.5);
  assert.equal(d.length, SKY_UNIFORM_FLOATS);
  const near = (a: number | undefined, b: number): void => assert.ok(Math.abs((a ?? NaN) - b) < 1e-5, `${a} vs ${b}`);
  // right x tanX (tan 45° x 16/9), up x tanY, forward -Z.
  near(d[0], 16 / 9);
  near(d[5], 1);
  near(d[10], -1);
  near(d[3], 2 / 900); // angular pixel size
  near(d[7], 3.5);
  near(d[16], SKY.sunDirection.x);
  near(d[19], SKY.sunSize);
  near(d[27], SKY.planet?.radius ?? 0);
  near(d[43], Math.cos(planetCone(SKY.planet ?? { radius: 0, rings: 0 })));
  near(d[48], 2);
  near(d[54], 254);
  near(d[56], 1600);
  near(d[60], SKY.nebula.band.x);
  // Far from every face of the room: no boundary grid.
  near(d[15], 0);
  const atWall = packSkyUniforms(SKY, { ...cam, position: { x: 5, y: 60, z: 120 } }, 1600, 900, 0);
  near(atWall[15], 1);
});

test("packEnvUniforms: haze, nebula, band, sun", () => {
  const e = packEnvUniforms(SKY);
  assert.equal(e.length, ENV_UNIFORM_FLOATS);
  assert.equal(e[3], SKY.nebula.strength);
  assert.equal(e[11], ENV_SIZE);
  assert.ok(Math.abs((e[32] ?? 0) - SKY.sunDirection.x) < 1e-6);
  // Linear colours: the display zenith colour gets darker.
  assert.ok((e[0] ?? 1) < SKY.zenith[0]);
});

test("boundary grid and planet cone", () => {
  const room = { min: { x: 0, y: 0, z: 0 }, max: { x: 100, y: 100, z: 100 } };
  assert.equal(gridStrength(room, { x: 50, y: 50, z: 50 }), 0);
  assert.equal(gridStrength(room, { x: 50, y: GRID_RANGE - 1, z: 50 }), 1);
  assert.equal(gridStrength(null, { x: 1, y: 1, z: 1 }), 0);
  assert.ok(Math.abs(planetCone({ radius: 0.2, rings: 0 }) - 0.26) < 1e-9);
  const ringed = planetCone({ radius: 0.2, rings: 2.2 });
  assert.ok(ringed > Math.asin(Math.sin(0.2) * 2.2) && ringed < Math.PI);
  assert.equal(planetCone({ radius: 0.3, rings: 4 }), Math.PI);
});
