import assert from "node:assert/strict";
import { test } from "node:test";
import * as mat4 from "../math/mat4.ts";
import * as q from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import {
  atmosphereFor,
  decodeShareCode,
  LEVEL_WALL_THICKNESS,
  seededRandom,
  shaftFade,
  stripRoomWalls,
  sunScreen,
  worldGrade,
  worldWeights,
} from "./atmosphere.ts";
import { MeshBuilder, NO_TEAM, type MeshData } from "./mesh.ts";
import { appendBox } from "./primitives.ts";

// ----- share codes -----------------------------------------------------------------------------

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/**
 * Test-only encoder (ls-worldgen sharecode.rs layout; version 4 with the
 * archetype field, versions 1-3 the old layout), checked against Rust-made
 * codes below.
 */
function encode(version: number, seed: bigint, sliders: number[], archetype = 0): string {
  const legacy = version < 4;
  let bits = BigInt(version);
  if (!legacy) bits = (bits << 3n) | BigInt(archetype);
  bits = (bits << 64n) | seed;
  if (legacy) {
    for (const v of sliders) bits = (bits << 7n) | BigInt(v);
  } else {
    bits = (bits << 47n) | sliders.reduce((n, v) => n * 101n + BigInt(v), 0n);
  }
  const packed = bits << (legacy ? 7n : 6n);
  let crc = 0;
  for (let i = 15; i >= 0; i--) {
    crc ^= Number((packed >> BigInt(i * 8)) & 0xffn);
    for (let k = 0; k < 8; k++) crc = crc & 0x80 ? ((crc << 1) ^ 0x07) & 0xff : (crc << 1) & 0xff;
  }
  bits = (bits << 8n) | BigInt(crc);
  if (legacy) bits <<= 1n;
  let body = "";
  for (let i = 25; i >= 0; i--) body += CROCKFORD[Number((bits >> BigInt(i * 5)) & 31n)];
  let out = "LS";
  for (let i = 0; i < body.length; i++) out += (i >= 2 && (i - 2) % 4 === 0 ? "-" : "") + body[i];
  return out;
}

test("share codes decode like ls-worldgen (codes made by the Rust encoder)", () => {
  // crates/ls-game/src/setup.rs: generator version 1, Enclosure 75, Density 20.
  const a = decodeShareCode("LS04-0000-00VT-PVXV-WPA6-8001-8F6R");
  assert.ok(a);
  assert.equal(a.version, 1);
  assert.equal(a.sliders.enclosure, 75);
  assert.equal(a.sliders.density, 20);
  // The other sliders at ls-worldgen's defaults (verticality 50, absurdity 20, instability 30).
  assert.deepEqual([a.sliders.verticality, a.sliders.gravityChaos, a.sliders.palette, a.sliders.absurdity, a.sliders.instability], [50, 0, 0, 20, 30]);
  assert.equal(a.archetype, 0);
  // The M3 smoke and e2e world: version 3, seed 1, open sky.
  const v3 = decodeShareCode("LS0C-0000-0000-0000-8006-8001-8F9M");
  assert.deepEqual(v3 && [v3.version, v3.seed, v3.sliders.enclosure, v3.sliders.density, v3.archetype], [3, 1n, 0, 0, 0]);
  // Generator version 4 (archetype field, sliders packed base-101): the
  // smoke and e2e world, same seed and sliders.
  const b = decodeShareCode("LS0G-0000-0000-0000-1001-6RFX-250V");
  assert.deepEqual(b && [b.version, b.seed, b.sliders.enclosure, b.sliders.density, b.archetype], [4, 1n, 0, 0, 0]);
  assert.deepEqual(b?.sliders, v3?.sliders);
  // The bots' soak world: caverns.
  assert.equal(decodeShareCode("LS0G-2TNV-VD0V-ZFQY-JC58-T2XM-57QY")?.sliders.enclosure, 50);
  // Asteroid belts (crates/ls-worldgen/tests/fixtures/golden_hashes.txt).
  const belt = decodeShareCode("LS0G-G000-0016-MYKT-EH2T-94GC-EEK7");
  assert.deepEqual(belt && [belt.archetype, belt.sliders.enclosure, belt.sliders.density, belt.sliders.verticality, belt.sliders.gravityChaos], [1, 70, 80, 70, 60]);
  // Lenient like the Rust decoder: case, dashes, whitespace, O/I/L look-alikes.
  assert.deepEqual(decodeShareCode(" ls0g 0000-oooo-0000-1oo1-6rfx-25ov "), b);
});

test("share codes with a typo, a wrong length or no prefix do not decode", () => {
  assert.equal(decodeShareCode("LS0C-0000-0000-0000-8006-8001-8F9N"), null, "CRC");
  assert.equal(decodeShareCode("LS0G-0000-0000-0000-1001-6RFX-250W"), null, "CRC v4");
  assert.equal(decodeShareCode("LS0C-0000-0000-0000-8006-8001-8F9"), null, "length");
  assert.equal(decodeShareCode("XX0C-0000-0000-0000-8006-8001-8F9M"), null, "prefix");
  assert.equal(decodeShareCode("LS0C-0000-0000-0000-8006-8001-8FU!"), null, "symbol");
  assert.equal(decodeShareCode(""), null);
});

test("the test encoder round-trips through the decoder", () => {
  const rnd = seededRandom(5);
  for (let i = 0; i < 50; i++) {
    const seed = (BigInt(Math.floor(rnd() * 2 ** 32)) << 32n) | BigInt(Math.floor(rnd() * 2 ** 32));
    const sliders = Array.from({ length: 7 }, () => Math.floor(rnd() * 101));
    for (const [version, archetype] of [[3, 0], [4, 0], [4, 1]] as const) {
      const w = decodeShareCode(encode(version, seed, sliders, archetype));
      assert.ok(w);
      assert.equal(w.seed, seed);
      assert.equal(w.version, version);
      assert.equal(w.archetype, archetype);
      assert.deepEqual(Object.values(w.sliders), sliders);
    }
  }
  assert.equal(encode(3, 1n, [0, 0, 50, 0, 0, 20, 30]), "LS0C-0000-0000-0000-8006-8001-8F9M");
  assert.equal(encode(4, 1n, [0, 0, 50, 0, 0, 20, 30]), "LS0G-0000-0000-0000-1001-6RFX-250V");
  assert.equal(encode(4, 42n, [50, 40, 50, 0, 0, 20, 30]), "LS0G-0000-0000-0001-AC58-T2XM-57ND");
});

test("asteroid belts are open space with a sky", () => {
  const belt = atmosphereFor(encode(4, 9n, [100, 50, 50, 0, 0, 20, 30], 1), [0, 0, 0, 320, 96, 320]);
  assert.equal(belt.open, 1);
  assert.ok(belt.planet && belt.stars > 0);
  const mines = atmosphereFor(encode(4, 9n, [100, 50, 50, 0, 0, 20, 30], 0), [0, 0, 0, 256, 128, 256]);
  assert.equal(mines.open, 0);
});

// ----- world types, sky and grades -------------------------------------------------------------

test("world weights follow the generator's Enclosure blend", () => {
  assert.deepEqual(worldWeights(0), { sky: 1, cave: 0, mine: 0 });
  assert.deepEqual(worldWeights(50), { sky: 0, cave: 1, mine: 0 });
  assert.deepEqual(worldWeights(100), { sky: 0, cave: 0, mine: 1 });
  const w = worldWeights(25);
  assert.ok(Math.abs(w.sky - 0.5) < 1e-9 && Math.abs(w.cave - 0.5) < 1e-9 && w.mine === 0);
  const m = worldWeights(80);
  assert.ok(Math.abs(m.mine - 0.6) < 1e-9 && Math.abs(m.cave - 0.4) < 1e-9);
});

const BOUNDS = [0, 0, 0, 256, 128, 256];
const angle = (a: Vec3, b: Vec3): number => Math.acos(Math.min(1, Math.max(-1, a.x * b.x + a.y * b.y + a.z * b.z)));

test("an open-sky world gets a sky, a planet, the room box and no height fog", () => {
  const a = atmosphereFor(encode(3, 1n, [0, 0, 50, 0, 0, 20, 30]), BOUNDS);
  assert.equal(a.open, 1);
  assert.ok(a.planet && a.stars > 0 && a.nebula.strength > 0 && a.shafts > 0);
  const t = LEVEL_WALL_THICKNESS;
  assert.deepEqual(a.room, { min: { x: t, y: t, z: t }, max: { x: 256 - t, y: 128 - t, z: 256 - t } });
  assert.equal(a.heightFog.density, 0);
  // Same code, same sky.
  assert.deepEqual(atmosphereFor(encode(3, 1n, [0, 0, 50, 0, 0, 20, 30]), BOUNDS), a);
});

test("caverns and mines have no sky but a warm height fog; mixed worlds open partly", () => {
  const caves = atmosphereFor(encode(3, 9n, [50, 40, 50, 0, 0, 20, 30]), BOUNDS);
  assert.equal(caves.open, 0);
  assert.equal(caves.planet, null);
  assert.equal(caves.stars, 0);
  assert.equal(caves.shafts, 0);
  assert.ok(caves.heightFog.density > 0);
  assert.equal(caves.heightFog.baseY, LEVEL_WALL_THICKNESS);
  const mines = atmosphereFor(encode(3, 9n, [100, 40, 50, 0, 0, 20, 30]), BOUNDS);
  assert.equal(mines.open, 0);
  // Sodium haze: much more red than blue.
  assert.ok(mines.horizon[0] > 2.5 * mines.horizon[2]);
  const mixed = atmosphereFor(encode(3, 9n, [30, 40, 50, 0, 0, 20, 30]), BOUNDS);
  assert.ok(mixed.open > 0.3 && mixed.open < 0.5);
  // An unknown code still gets a (caverns) atmosphere.
  assert.equal(atmosphereFor("not a code").open, 0);
});

test("seeds pick different skies; the sun stays high and away from the planet", () => {
  const suns = new Set<string>();
  const palettes = new Set<string>();
  for (let i = 0; i < 200; i++) {
    const a = atmosphereFor(encode(3, BigInt(i * 7919 + 1), [0, 40, 50, 0, 0, 20, 30]), BOUNDS);
    const s = a.sunDirection;
    assert.ok(Math.abs(Math.hypot(s.x, s.y, s.z) - 1) < 1e-9);
    const elevation = (Math.asin(s.y) * 180) / Math.PI;
    assert.ok(elevation >= 28 && elevation <= 58.001, `sun elevation ${elevation}`);
    assert.ok(a.planet);
    const deg = (x: number): number => (x * 180) / Math.PI;
    assert.ok(deg(a.planet.radius) >= 8 && deg(a.planet.radius) <= 15);
    assert.ok(deg(angle(s, a.planet.direction)) - deg(a.planet.radius) > 45, "planet clear of the sun");
    suns.add(s.x.toFixed(3));
    palettes.add(a.nebula.a.join());
  }
  assert.ok(suns.size > 150);
  assert.ok(palettes.size >= 5, `${palettes.size} nebula palettes`);
});

test("grades: caves warm, mines sodium, sky cold; the Palette slider desaturates", () => {
  const sky = worldGrade(worldWeights(0), 0);
  const caves = worldGrade(worldWeights(50), 0);
  const mines = worldGrade(worldWeights(100), 0);
  assert.ok(caves.slope[0] > caves.slope[2], "caves warm");
  assert.ok(mines.slope[0] / mines.slope[2] > caves.slope[0] / caves.slope[2], "mines warmer than caves");
  assert.ok(sky.slope[2] > sky.slope[0], "sky cold");
  const ink = worldGrade(worldWeights(50), 100);
  assert.ok(ink.saturation <= 0.11 * caves.saturation + 1e-9);
  assert.equal(worldGrade(worldWeights(50), 50).saturation, caves.saturation * 0.55);
  // The atmosphere carries the world's grade.
  assert.deepEqual(atmosphereFor(encode(3, 2n, [100, 40, 50, 0, 70, 20, 30]), BOUNDS).grade, worldGrade(worldWeights(100), 70));
});

// ----- hidden walls ----------------------------------------------------------------------------

/** A closed box room from -h to +h: six wall boxes 1 u thick, inner faces on the room box. */
function roomWithRock(h: number): MeshData {
  const b = new MeshBuilder();
  const t = (): number => NO_TEAM;
  appendBox(b, -h - 0.5, 0, 0, 0.5, h + 1, h + 1, t);
  appendBox(b, h + 0.5, 0, 0, 0.5, h + 1, h + 1, t);
  appendBox(b, 0, -h - 0.5, 0, h + 1, 0.5, h + 1, t);
  appendBox(b, 0, h + 0.5, 0, h + 1, 0.5, h + 1, t);
  appendBox(b, 0, 0, -h - 0.5, h + 1, h + 1, 0.5, t);
  appendBox(b, 0, 0, h + 0.5, h + 1, h + 1, 0.5, t);
  appendBox(b, 4, 0, 0, 1, 1, 1, t); // a rock in the room
  return b.build();
}

test("stripRoomWalls removes exactly the faces on the room box that face into the room", () => {
  const h = 10;
  const room = { min: { x: -h, y: -h, z: -h }, max: { x: h, y: h, z: h } };
  const mesh = roomWithRock(h);
  const stripped = stripRoomWalls(mesh, room);
  assert.equal(mesh.indices.length - stripped.indices.length, 6 * 2 * 3, "six inner faces, two triangles each");
  // The rock is intact: its 12 triangles are all still there.
  const p = stripped.positions;
  const tris = stripped.indices.length / 3;
  let rockTris = 0;
  for (let i = 0; i < tris; i++) {
    const v = (stripped.indices[i * 3] ?? 0) * 3;
    if (Math.abs((p[v] ?? 0) - 4) <= 1 + 1e-6 && Math.abs(p[v + 1] ?? 0) <= 1 + 1e-6) rockTris++;
  }
  assert.equal(rockTris, 12);
  // Positions and attributes are shared; a mesh without walls comes back as is.
  assert.equal(stripped.positions, mesh.positions);
  const far = { min: { x: -50, y: -50, z: -50 }, max: { x: 50, y: 50, z: 50 } };
  assert.equal(stripRoomWalls(mesh, far), mesh);
});

test("stripRoomWalls also removes the rounded edges of the box but keeps rock meeting a wall", () => {
  const room = { min: { x: 0, y: 0, z: 0 }, max: { x: 10, y: 10, z: 10 } };
  const s = Math.SQRT1_2;
  const mesh: MeshData = {
    // Edge strip between the x = 0 and y = 0 faces (normals lean between both),
    // then a rock triangle with one vertex on the wall.
    positions: new Float32Array([0.1, 0.3, 2, 0.3, 0.1, 2, 0.2, 0.2, 4, 0, 5, 5, 3, 5, 5, 0, 6, 6]),
    normals: new Float32Array([0.9, 0.44, 0, 0.44, 0.9, 0, s, s, 0, 0.95, 0.3, 0, 0, 1, 0, 0.95, 0.3, 0]),
    indices: new Uint32Array([0, 1, 2, 3, 4, 5]),
    team: new Uint8Array(6).fill(NO_TEAM),
  };
  const out = stripRoomWalls(mesh, room);
  assert.deepEqual([...out.indices], [3, 4, 5]);
});

// ----- light shafts ----------------------------------------------------------------------------

test("sunScreen projects the sun direction; shaftFade fades it out off screen", () => {
  const vp = (orientation: q.Quat): mat4.Mat4 =>
    mat4.multiply(mat4.perspective(Math.PI / 2, 16 / 9, 0.1, 600), mat4.viewFromPose({ x: 50, y: 20, z: -7 }, orientation));
  const ahead = sunScreen(vp(q.identity()), { x: 0, y: 0, z: -1 });
  assert.ok(ahead && Math.abs(ahead.x) < 1e-6 && Math.abs(ahead.y) < 1e-6);
  assert.equal(sunScreen(vp(q.identity()), { x: 0, y: 0, z: 1 }), null, "behind the camera");
  // 45° up with a 90° vertical field of view: the top edge of the screen.
  const up = sunScreen(vp(q.identity()), { x: 0, y: Math.SQRT1_2, z: -Math.SQRT1_2 });
  assert.ok(up && Math.abs(up.y - 1) < 1e-5 && Math.abs(up.x) < 1e-6);
  assert.equal(shaftFade(ahead), 1);
  assert.equal(shaftFade(up), 1);
  assert.equal(shaftFade({ x: 2, y: 0 }), 0);
  const mid = shaftFade({ x: 1.4, y: 0 });
  assert.ok(mid > 0 && mid < 1);
  assert.equal(shaftFade(null), 0);
});
