import assert from "node:assert/strict";
import { test } from "node:test";
import { MATERIAL_HARD, MATERIAL_ROCK, MATERIAL_WEAK, NO_TEAM, PAINT_RED, SURFACE_HOT, type MeshData } from "./mesh.ts";
import { QUALITY_PRESETS } from "./quality.ts";
import { hashSeed, MAX_STRATA_TILT, mipLevelCount, ROCK_GEN_WGSL, ROCK_MIP_WGSL, terrainParams, TERRAIN_PARAM_FLOATS } from "./rockTextures.ts";
import { FRAME_WGSL, ROOM_CLIP, ROOM_FADE, TEAM_SLOTS } from "./shaders/frame.ts";
import { MATERIAL_EDGE_SHARPNESS, TERRAIN_WGSL } from "./shaders/terrain.ts";
import { BLUR_PASSES, nearChunkFace, PIN_MARGIN, terrainVertexData } from "./terrainBlend.ts";

/**
 * A flat grid mesh in the y = 16 plane (mid-chunk), `n` x `n` quads of 1 u starting at
 * (x0, z0), with per-vertex material / team / surface from callbacks.
 */
function grid(
  n: number,
  x0: number,
  z0: number,
  at: (x: number, z: number) => { material?: number; team?: number; color?: number; surface?: number; ao?: number },
): MeshData {
  const positions: number[] = [];
  const normals: number[] = [];
  const team: number[] = [];
  const materials: number[] = [];
  const colors: number[] = [];
  const surface: number[] = [];
  const ao: number[] = [];
  for (let j = 0; j <= n; j++) {
    for (let i = 0; i <= n; i++) {
      const x = x0 + i, z = z0 + j;
      positions.push(x, 16, z);
      normals.push(0, 1, 0);
      const v = at(x, z);
      team.push(v.team ?? NO_TEAM);
      materials.push(v.material ?? MATERIAL_ROCK);
      colors.push(v.color ?? 0);
      surface.push(v.surface ?? 0);
      ao.push(v.ao ?? 255);
    }
  }
  const indices: number[] = [];
  const id = (i: number, j: number): number => j * (n + 1) + i;
  for (let j = 0; j < n; j++) {
    for (let i = 0; i < n; i++) indices.push(id(i, j), id(i, j + 1), id(i + 1, j + 1), id(i, j), id(i + 1, j + 1), id(i + 1, j));
  }
  return {
    positions: new Float32Array(positions),
    normals: new Float32Array(normals),
    indices: new Uint32Array(indices),
    team: new Uint8Array(team),
    materials: new Uint8Array(materials),
    paintColors: new Uint8Array(colors),
    surface: new Uint8Array(surface),
    ao: new Uint8Array(ao),
  };
}

/** Unpacked weights of vertex i: [ao, weak, hard, paint, hot, charged, glow, scorch] in 0..255. */
function weights(extra: Uint32Array, i: number): number[] {
  const a = extra[i * 2] ?? 0, b = extra[i * 2 + 1] ?? 0;
  return [a & 255, (a >>> 8) & 255, (a >>> 16) & 255, a >>> 24, b & 255, (b >>> 8) & 255, (b >>> 16) & 255, b >>> 24];
}

const vertexAt = (n: number, i: number, j: number): number => j * (n + 1) + i;

// ----- blend weights ----------------------------------------------------------------------

test("uniform terrain keeps one-hot weights, AO and the plain team packing", () => {
  const m = grid(8, 4, 4, (x) => ({ material: MATERIAL_HARD, ao: 100 + x }));
  const d = terrainVertexData(m);
  assert.equal(d.blurred, 0);
  for (let i = 0; i < m.team.length; i++) {
    const w = weights(d.extra, i);
    assert.equal(w[0], m.ao?.[i]);
    assert.deepEqual(w.slice(1), [0, 255, 0, 0, 0, 0, 0]);
  }
  // Missing AO and materials: open, ordinary rock.
  const bare = terrainVertexData({ positions: m.positions, normals: m.normals, indices: m.indices, team: m.team });
  assert.deepEqual(weights(bare.extra, 0), [255, 0, 0, 0, 0, 0, 0, 0]);
});

test("a material border becomes a symmetric ramp several vertices wide, crossing 0.5 at the border", () => {
  // Weak rock for x < 10, hard rock from x = 10 on (far from chunk faces).
  const n = 16;
  const m = grid(n, 2, 8, (x) => ({ material: x < 10 ? MATERIAL_WEAK : MATERIAL_HARD }));
  const d = terrainVertexData(m);
  assert.ok(d.blurred > 0);
  const row = 8;
  const weak = (x: number): number => weights(d.extra, vertexAt(n, x - 2, row))[1] ?? 0;
  const hard = (x: number): number => weights(d.extra, vertexAt(n, x - 2, row))[2] ?? 0;
  // Far from the border: untouched.
  assert.equal(weak(4), 255);
  assert.equal(hard(4), 0);
  assert.equal(weak(16), 0);
  // At the border both are partial, and the ramp spans more than one cell.
  assert.ok((weak(9) ?? 0) < 255 && (weak(9) ?? 0) > 127, `weak(9) ${weak(9)}`);
  assert.ok((weak(10) ?? 0) > 0 && (weak(10) ?? 0) < 128, `weak(10) ${weak(10)}`);
  assert.ok((weak(8) ?? 0) < 255, "ramp reaches two vertices in");
  assert.ok((weak(11) ?? 0) > 0, "ramp reaches two vertices out");
  // Weights stay a partition (up to rounding).
  for (let x = 2; x <= 18; x++) assert.ok(Math.abs(weak(x) + hard(x) - 255) <= 2, `x ${x}`);
  // The ramp is monotonic across the border.
  for (let x = 3; x <= 18; x++) assert.ok(weak(x) <= weak(x - 1), `x ${x}`);
});

test("display weights from the mesher are used as they are, with no blur or pinning (D5)", () => {
  // Hard rock from x = 10 on, but the mesher's smooth weights ramp across it,
  // also over a chunk face (x = 32: pinned vertices would keep one-hot ids).
  const n = 40;
  const m = grid(n, 2, 8, (x) => ({ material: x < 10 ? MATERIAL_WEAK : MATERIAL_HARD, ao: 77, surface: x === 5 ? SURFACE_HOT : 0 }));
  const count = m.team.length;
  const mw = new Uint8Array(count * 2);
  for (let v = 0; v < count; v++) {
    const x = m.positions[v * 3] ?? 0;
    mw[v * 2] = Math.round(255 * Math.min(1, Math.max(0, 0.5 - (x - 10) / 8)));
    mw[v * 2 + 1] = Math.round(255 * Math.min(1, Math.max(0, 0.5 + (x - 33) / 8)));
  }
  const d = terrainVertexData({ ...m, materialWeights: mw });
  for (let v = 0; v < count; v++) {
    const w = weights(d.extra, v);
    assert.equal(w[0], 77, "AO kept");
    assert.equal(w[1], mw[v * 2], `weak at vertex ${v}`);
    assert.equal(w[2], mw[v * 2 + 1], `hard at vertex ${v}`);
  }
  // The surface state still blurs around x = 5.
  assert.ok(d.blurred > 0);
  assert.ok((weights(d.extra, vertexAt(n, 4, 8))[4] ?? 0) > 0);
  // Material ids alone no longer mark borders: without states nothing blurs.
  const plain = grid(n, 2, 8, (x) => ({ material: x < 10 ? MATERIAL_WEAK : MATERIAL_HARD }));
  assert.equal(terrainVertexData({ ...plain, materialWeights: mw }).blurred, 0);
});

test("blurring stays within BLUR_PASSES edges of the border", () => {
  const n = 20;
  const m = grid(n, 2, 2, (x) => ({ material: x < 12 ? MATERIAL_WEAK : MATERIAL_ROCK }));
  const d = terrainVertexData(m);
  const row = 10;
  for (let x = 2; x <= 22; x++) {
    const w = weights(d.extra, vertexAt(n, x - 2, row))[1] ?? 0;
    // Ring 0 is x = 11, 12; the blur reaches BLUR_PASSES further.
    if (x < 11 - BLUR_PASSES) assert.equal(w, 255, `x ${x}`);
    if (x > 12 + BLUR_PASSES) assert.equal(w, 0, `x ${x}`);
  }
});

test("vertices near a chunk face keep their own weights, so both chunks agree on them", () => {
  assert.ok(nearChunkFace(32 - PIN_MARGIN / 2));
  assert.ok(nearChunkFace(64 + 0.4));
  assert.ok(nearChunkFace(-0.3));
  assert.ok(!nearChunkFace(16));
  // A border crossing the chunk face at x = 32: the two chunks see different
  // neighbourhoods, but the shared vertices (x in [31, 33]) are identical.
  const at = (_x: number, z: number) => ({ material: z < 10 ? MATERIAL_WEAK : MATERIAL_HARD });
  const left = grid(6, 27, 4, at); // x 27..33
  const right = grid(6, 31, 4, at); // x 31..37
  const dl = terrainVertexData(left);
  const dr = terrainVertexData(right);
  for (let j = 0; j <= 6; j++) {
    for (const x of [31, 32, 33]) {
      const wl = weights(dl.extra, vertexAt(6, x - 27, j));
      const wr = weights(dr.extra, vertexAt(6, x - 31, j));
      assert.deepEqual(wl, wr, `x ${x} z ${4 + j}`);
    }
  }
});

test("paint coverage blurs and the team and colour are dilated next to paint", () => {
  const n = 16;
  // Team 1 red paint in a disc of radius 4 around (10, 10).
  const m = grid(n, 2, 2, (x, z) => (Math.hypot(x - 10, z - 10) <= 4 ? { team: 1, color: PAINT_RED } : {}));
  const d = terrainVertexData(m);
  const centre = vertexAt(n, 8, 8);
  assert.equal(weights(d.extra, centre)[3], 255);
  // Just outside the disc: some coverage, and the vertex carries team 1 and red.
  const out = vertexAt(n, 8 + 5 - 0, 8); // x = 15
  const w = weights(d.extra, out)[3] ?? 0;
  assert.ok(w > 0 && w < 128, `coverage ${w}`);
  assert.equal((d.team[out] ?? 0) & 0xff, 1);
  assert.equal((d.team[out] ?? 0) >>> 24, PAINT_RED);
  // Far away: unpainted.
  assert.equal((d.team[vertexAt(n, 0, 0)] ?? 0) & 0xff, NO_TEAM);
  assert.equal(weights(d.extra, vertexAt(n, 0, 0))[3], 0);
});

test("surface states blur like materials", () => {
  const n = 12;
  const m = grid(n, 4, 4, (x) => (x < 10 ? { surface: SURFACE_HOT } : {}));
  const d = terrainVertexData(m);
  const hot = (x: number): number => weights(d.extra, vertexAt(n, x - 4, 6))[4] ?? 0;
  assert.equal(hot(5), 255);
  assert.equal(hot(15), 0);
  assert.ok(hot(9) > 127 && hot(10) < 128 && hot(10) > 0);
});

test("blend weights stay cheap: a 64x64 grid with borders in well under the chunk budget", () => {
  const n = 64;
  const m = grid(n, 2, 2, (x, z) => ({
    material: (x * 7 + z * 3) % 23 < 9 ? MATERIAL_WEAK : MATERIAL_HARD,
    team: Math.hypot(x - 30, z - 30) < 12 ? 0 : NO_TEAM,
  }));
  terrainVertexData(m); // warm up
  const t0 = performance.now();
  const runs = 5;
  for (let i = 0; i < runs; i++) terrainVertexData(m);
  const ms = (performance.now() - t0) / runs;
  // 8 192 triangles, most of them near a border; real chunks average ~15 000
  // triangles with far fewer borders. The play budget is 4 ms per frame.
  assert.ok(ms < 8, `${ms.toFixed(2)} ms`);
});

// ----- rock textures and shader -------------------------------------------------------------

test("rock texture helpers: mip count, seeds and per-world strata", () => {
  assert.equal(mipLevelCount(512), 10);
  assert.equal(mipLevelCount(1), 1);
  assert.equal(hashSeed("LS12-3456"), hashSeed("LS12-3456"));
  assert.notEqual(hashSeed("LS12-3456"), hashSeed("LS12-3457"));
  const seen = new Set<string>();
  for (let seed = 0; seed < 50; seed++) {
    const p = terrainParams(seed * 7919);
    assert.equal(p.length, TERRAIN_PARAM_FLOATS);
    const [x, y, z, freq] = [p[0] ?? 0, p[1] ?? 0, p[2] ?? 0, p[3] ?? 0];
    assert.ok(Math.abs(Math.hypot(x, y, z) - 1) < 1e-5, "unit strata normal");
    assert.ok(Math.acos(y) <= MAX_STRATA_TILT + 1e-5, "tilt bounded");
    assert.ok(freq > 0.05 && freq < 0.1);
    seen.add(`${x.toFixed(3)},${z.toFixed(3)}`);
  }
  assert.ok(seen.size > 40, "worlds differ");
  assert.deepEqual(terrainParams(42), terrainParams(42));
});

test("the terrain shader blends instead of switching per triangle and has no sin-hash", () => {
  assert.doesNotMatch(TERRAIN_WGSL, /interpolate\(flat\)/);
  assert.doesNotMatch(TERRAIN_WGSL, /43758/);
  assert.doesNotMatch(ROCK_GEN_WGSL, /43758/);
  assert.match(TERRAIN_WGSL, /fn ggx\(/);
  assert.match(TERRAIN_WGSL, /override DETAIL/);
  assert.match(TERRAIN_WGSL, /texture_2d_array/);
  assert.match(ROCK_GEN_WGSL, /fn gen\(/);
  assert.match(ROCK_MIP_WGSL, /fn mip\(/);
  // The team one-hot is a vec4: four paint slots.
  assert.equal(TEAM_SLOTS, 4);
  // WGSL reserved words that once slipped in.
  for (const src of [TERRAIN_WGSL, ROCK_GEN_WGSL]) assert.doesNotMatch(src, /\blet (patch|filter|meta|mod|self|target)\b/);
});

test("material borders: one material wins by a steep soft-max; rock is cut at hidden walls (D5)", () => {
  // The mesher's display weights resolved per pixel: no grey seam between weak and hard rock.
  assert.ok(TERRAIN_WGSL.includes(`* ${MATERIAL_EDGE_SHARPNESS}.0);`));
  assert.ok(MATERIAL_EDGE_SHARPNESS >= 20);
  assert.ok(TERRAIN_WGSL.includes("let mw = mat_e / (mat_e.x + mat_e.y + mat_e.z);"));
  // Rock within ROOM_CLIP of a hidden wall is cut along a smooth line (no stripped-wall sawtooth).
  assert.ok(FRAME_WGSL.includes(`* step(${ROOM_CLIP}, d);`));
  assert.ok(ROOM_CLIP > 0.5 && ROOM_CLIP < ROOM_FADE);
});

test("quality: only low drops the terrain detail", () => {
  assert.equal(QUALITY_PRESETS.high.terrainDetail, true);
  assert.equal(QUALITY_PRESETS.medium.terrainDetail, true);
  assert.equal(QUALITY_PRESETS.low.terrainDetail, false);
});
