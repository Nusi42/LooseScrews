// Per-world atmosphere (docs/visual-quality-plan.md VQ7): everything the sky,
// the fog and the colour grade need, derived from the world's share code.
// Pure data and maths (Node unit tests: atmosphere.test.ts); sky.ts turns an
// Atmosphere into GPU state.
//
// - The Enclosure slider picks the world type: 0 open sky, 50 caverns, 100
//   mine shafts, blended linearly like the generator's openness fields
//   (ls-worldgen generator.rs `w_sky`, `w_cave`, `w_mine`). Open-sky weight
//   above 0 shows the sky: the sealing walls of the level box are hidden
//   (stripRoomWalls) and the sky pass draws stars, nebula, sun and planet
//   behind the terrain, with a boundary grid near the camera.
// - The seed picks the sky (nebula palette, sun, planet, galactic band), so a
//   share code always shows the same sky.
// - The Palette slider (game design doc: "Full color" .. "black-and-white ink
//   style, via desaturated") lowers the grade's saturation.
//
// Colours are display (sRGB) values like every other authored colour.

import type { Vec3 } from "../math/vec3.ts";
import type { MeshData } from "./mesh.ts";
import { GRADE_PRESETS, type ColorGrade } from "./post.ts";

type Rgb = readonly [number, number, number];

/** Sliders of a share code (ls-worldgen `Sliders`, share-code order). */
export interface WorldSliders {
  enclosure: number;
  density: number;
  verticality: number;
  gravityChaos: number;
  palette: number;
  absurdity: number;
  instability: number;
}

export interface WorldCode {
  version: number;
  seed: bigint;
  sliders: WorldSliders;
}

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const CODE_SYMBOLS = 26;
const TOTAL_BITS = 130n;
const DATA_BITS = 121n;

function crc8(bytes: readonly number[]): number {
  let crc = 0;
  for (const byte of bytes) {
    crc ^= byte;
    for (let i = 0; i < 8; i++) crc = crc & 0x80 ? ((crc << 1) ^ 0x07) & 0xff : (crc << 1) & 0xff;
  }
  return crc;
}

/**
 * Decodes a share code (ls-worldgen sharecode.rs: 8 bit version, 64 bit
 * seed, 7 x 7 bit sliders, CRC-8, one padding bit, as 26 Crockford base32
 * symbols after `LS`). Null when it is not a valid code.
 */
export function decodeShareCode(text: string): WorldCode | null {
  const cleaned = text.toUpperCase().replace(/[\s-]+/g, "");
  if (!cleaned.startsWith("LS")) return null;
  const body = cleaned.slice(2).replace(/O/g, "0").replace(/[IL]/g, "1");
  if (body.length !== CODE_SYMBOLS) return null;
  let bits = 0n;
  for (const ch of body) {
    const v = CROCKFORD.indexOf(ch);
    if (v < 0) return null;
    bits = (bits << 5n) | BigInt(v);
  }
  const read = (offset: bigint, n: bigint): bigint => (bits >> (TOTAL_BITS - offset - n)) & ((1n << n) - 1n);
  if (read(TOTAL_BITS - 1n, 1n) !== 0n) return null;
  // The CRC covers the 121 data bits packed MSB first into 16 bytes.
  const packed = read(0n, DATA_BITS) << (128n - DATA_BITS);
  const bytes: number[] = [];
  for (let i = 15; i >= 0; i--) bytes.push(Number((packed >> BigInt(i * 8)) & 0xffn));
  if (crc8(bytes) !== Number(read(DATA_BITS, 8n))) return null;
  const s = (i: number): number => Number(read(72n + BigInt(i * 7), 7n));
  const sliders: WorldSliders = {
    enclosure: s(0),
    density: s(1),
    verticality: s(2),
    gravityChaos: s(3),
    palette: s(4),
    absurdity: s(5),
    instability: s(6),
  };
  if (Object.values(sliders).some((v) => v > 100)) return null;
  return { version: Number(read(0n, 8n)), seed: read(8n, 64n), sliders };
}

/** World-type weights from Enclosure (0..100), as the generator blends its openness fields. */
export function worldWeights(enclosure: number): { sky: number; cave: number; mine: number } {
  const e = Math.min(Math.max(enclosure, 0), 100) / 100;
  const sky = Math.max(0, 1 - 2 * e);
  const mine = Math.max(0, 2 * e - 1);
  return { sky, cave: 1 - sky - mine, mine };
}

/** Deterministic 32-bit PRNG (mulberry32) returning [0, 1). */
export function seededRandom(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Nebula palettes: two cloud colours and the colour of the bright cores. */
export const NEBULA_PALETTES: ReadonlyArray<{ name: string; a: Rgb; b: Rgb; core: Rgb }> = [
  { name: "aurora", a: [0.1, 0.55, 0.7], b: [0.72, 0.2, 0.62], core: [0.85, 0.92, 1.0] },
  { name: "ember", a: [0.95, 0.42, 0.14], b: [0.42, 0.16, 0.58], core: [1.0, 0.86, 0.62] },
  { name: "abyss", a: [0.14, 0.3, 0.88], b: [0.22, 0.78, 0.9], core: [0.8, 0.95, 1.0] },
  { name: "verdant", a: [0.12, 0.62, 0.5], b: [0.85, 0.66, 0.22], core: [1.0, 0.95, 0.78] },
  { name: "crimson", a: [0.85, 0.16, 0.24], b: [0.3, 0.24, 0.88], core: [1.0, 0.75, 0.78] },
  { name: "dusk", a: [0.52, 0.28, 0.88], b: [0.95, 0.42, 0.5], core: [1.0, 0.86, 0.95] },
];

/** Planet looks: kind 0 banded gas giant, 1 ocean world with clouds, 2 dry cratered world. */
export const PLANET_LOOKS: ReadonlyArray<{ name: string; kind: 0 | 1 | 2; c0: Rgb; c1: Rgb; c2: Rgb; air: Rgb; rings: number }> = [
  { name: "amber giant", kind: 0, c0: [0.85, 0.7, 0.52], c1: [0.58, 0.38, 0.26], c2: [0.95, 0.9, 0.8], air: [0.95, 0.78, 0.55], rings: 0.8 },
  { name: "teal giant", kind: 0, c0: [0.45, 0.72, 0.78], c1: [0.2, 0.4, 0.58], c2: [0.85, 0.93, 0.95], air: [0.5, 0.82, 1.0], rings: 0.5 },
  { name: "ocean world", kind: 1, c0: [0.06, 0.18, 0.42], c1: [0.36, 0.42, 0.24], c2: [0.96, 0.96, 0.98], air: [0.38, 0.64, 1.0], rings: 0.1 },
  { name: "rust world", kind: 2, c0: [0.72, 0.36, 0.2], c1: [0.42, 0.2, 0.13], c2: [0.9, 0.72, 0.52], air: [1.0, 0.62, 0.42], rings: 0.25 },
  { name: "ice world", kind: 2, c0: [0.78, 0.86, 0.94], c1: [0.48, 0.6, 0.76], c2: [0.97, 0.99, 1.0], air: [0.62, 0.86, 1.0], rings: 0.35 },
];

export interface Planet {
  /** Unit direction from the viewer. */
  direction: Vec3;
  /** Angular radius (rad). */
  radius: number;
  kind: 0 | 1 | 2;
  c0: Rgb;
  c1: Rgb;
  c2: Rgb;
  /** Atmosphere rim colour. */
  air: Rgb;
  /** Unit spin axis (bands follow it; the rings lie in its equatorial plane). */
  axis: Vec3;
  /** Ring outer radius in planet radii (0 = no rings). */
  rings: number;
  /** Surface pattern seed (0..1000). */
  seed: number;
}

export interface Atmosphere {
  /** World-type weights from Enclosure. */
  weights: { sky: number; cave: number; mine: number };
  /** Sky visibility 0..1 (the open-sky weight): above 0 the sky pass draws and the level box walls are hidden. */
  open: number;
  /** Interior of the level box (inside the sealing walls), when known. */
  room: { min: Vec3; max: Vec3 } | null;
  /** Unit vector towards the sun (also the terrain's sun light). */
  sunDirection: Vec3;
  sunColor: Rgb;
  /** Angular radius of the sun disc (rad). */
  sunSize: number;
  /** Background gradient of the sky / haze (display colours). */
  zenith: Rgb;
  horizon: Rgb;
  nadir: Rgb;
  nebula: { a: Rgb; b: Rgb; core: Rgb; strength: number; band: Vec3; offset: Vec3 };
  /** Star brightness multiplier (0 = none). */
  stars: number;
  planet: Planet | null;
  /** Flat fog colour (the haze's average; dust clouds blend from it). */
  fogColor: Rgb;
  fogDensity: number;
  /** Height fog: density at `baseY`, falling off by `falloff` per u upwards. */
  heightFog: { density: number; falloff: number; baseY: number };
  grade: ColorGrade;
  /** Light shaft strength (screen-space, from the sun; 0 = off). */
  shafts: number;
}

/** Thickness of the level box's sealing walls (ls-worldgen `WALL_THICKNESS`). */
export const LEVEL_WALL_THICKNESS = 2;

const mixRgb = (a: Rgb, b: Rgb, t: number): Rgb => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
const blend3 = (w: { sky: number; cave: number; mine: number }, s: Rgb, c: Rgb, m: Rgb): Rgb => [
  w.sky * s[0] + w.cave * c[0] + w.mine * m[0],
  w.sky * s[1] + w.cave * c[1] + w.mine * m[1],
  w.sky * s[2] + w.cave * c[2] + w.mine * m[2],
];
const blend = (w: { sky: number; cave: number; mine: number }, s: number, c: number, m: number): number => w.sky * s + w.cave * c + w.mine * m;

function unit(x: number, y: number, z: number): Vec3 {
  const l = Math.hypot(x, y, z) || 1;
  return { x: x / l, y: y / l, z: z / l };
}

function fromAngles(azimuth: number, elevation: number): Vec3 {
  return unit(Math.cos(elevation) * Math.cos(azimuth), Math.sin(elevation), Math.cos(elevation) * Math.sin(azimuth));
}

function pick<T>(xs: readonly T[], r: number): T {
  return xs[Math.min(xs.length - 1, Math.floor(r * xs.length))] as T;
}

/** The colour grade of a world: the presets blended by world type, desaturated by the Palette slider. */
export function worldGrade(weights: { sky: number; cave: number; mine: number }, palette: number): ColorGrade {
  const { sky, caves, mines } = GRADE_PRESETS;
  const p = Math.min(Math.max(palette, 0), 100) / 100;
  return {
    slope: blend3(weights, sky.slope, caves.slope, mines.slope),
    offset: blend3(weights, sky.offset, caves.offset, mines.offset),
    power: blend3(weights, sky.power, caves.power, mines.power),
    saturation: blend(weights, sky.saturation, caves.saturation, mines.saturation) * (1 - 0.9 * p),
    monochrome: 0,
    monoTint: [1, 1, 1],
    monoKeep: 0,
  };
}

/**
 * The atmosphere of the world with share code `code` (any string; an
 * undecodable one gets the caverns look from a hash of the text) and level
 * box `bounds` ([minX, minY, minZ, maxX, maxY, maxZ], ls-wasm `Game.bounds()`).
 */
export function atmosphereFor(code: string, bounds?: ArrayLike<number>): Atmosphere {
  const world = decodeShareCode(code);
  let seed32: number;
  if (world) {
    seed32 = Number((world.seed ^ (world.seed >> 32n)) & 0xffffffffn);
  } else {
    seed32 = 0;
    for (let i = 0; i < code.length; i++) seed32 = (Math.imul(seed32, 31) + code.charCodeAt(i)) >>> 0;
  }
  const rnd = seededRandom(seed32 ^ 0x5b7c1a93);
  const weights = worldWeights(world?.sliders.enclosure ?? 50);
  const palette = world?.sliders.palette ?? 0;

  let room: Atmosphere["room"] = null;
  if (bounds && bounds.length >= 6) {
    const b = (i: number): number => bounds[i] ?? 0;
    const t = LEVEL_WALL_THICKNESS;
    room = { min: { x: b(0) + t, y: b(1) + t, z: b(2) + t }, max: { x: b(3) - t, y: b(4) - t, z: b(5) - t } };
  }

  // Sun high enough that the terrain stays lit from above, any azimuth.
  const sunAz = rnd() * Math.PI * 2;
  const sunEl = (28 + rnd() * 30) * (Math.PI / 180);
  const sunDirection = fromAngles(sunAz, sunEl);
  const sunColor = pick<Rgb>([[1.0, 0.95, 0.88], [1.0, 0.9, 0.78], [0.9, 0.95, 1.0]], rnd());
  const neb = pick(NEBULA_PALETTES, rnd());
  const band = fromAngles(rnd() * Math.PI * 2, (rnd() * 2 - 1) * 1.2);
  const offset = { x: rnd() * 100, y: rnd() * 100, z: rnd() * 100 };

  // Planet: away from the sun (lit as a crescent or a half disc), not too high.
  const look = pick(PLANET_LOOKS, rnd());
  const side = rnd() < 0.5 ? -1 : 1;
  const planetAz = sunAz + side * (75 + rnd() * 70) * (Math.PI / 180);
  const planetEl = (-12 + rnd() * 34) * (Math.PI / 180);
  const tilt = (rnd() * 2 - 1) * 0.45;
  const planet: Planet = {
    direction: fromAngles(planetAz, planetEl),
    radius: (8 + rnd() * 7) * (Math.PI / 180),
    kind: look.kind,
    c0: look.c0,
    c1: look.c1,
    c2: look.c2,
    air: look.air,
    axis: unit(Math.sin(tilt) * Math.cos(planetAz + 1.3), Math.cos(tilt), Math.sin(tilt) * Math.sin(planetAz + 1.3)),
    rings: rnd() < look.rings ? 2.0 + rnd() * 0.5 : 0,
    seed: Math.floor(rnd() * 1000),
  };

  // Haze colours per world type: open space is near black with a tint of
  // the nebula, caves a warm dark haze, mines a sodium-lit dusty orange.
  const spaceZenith = mixRgb([0.012, 0.013, 0.022], neb.a, 0.03);
  const spaceHorizon = mixRgb([0.018, 0.019, 0.028], neb.b, 0.04);
  const spaceNadir: Rgb = [0.008, 0.008, 0.012];
  const zenith = blend3(weights, spaceZenith, [0.085, 0.08, 0.078], [0.15, 0.1, 0.05]);
  const horizon = blend3(weights, spaceHorizon, [0.075, 0.062, 0.052], [0.17, 0.105, 0.045]);
  const nadir = blend3(weights, spaceNadir, [0.045, 0.036, 0.03], [0.09, 0.055, 0.025]);
  const fogColor = mixRgb(horizon, mixRgb(zenith, nadir, 0.5), 0.4);

  const fogDensity = blend(weights, 0.0085, 0.012, 0.014);
  const heightFog = {
    density: blend(weights, 0, 0.015, 0.012),
    falloff: blend(weights, 0.1, 1 / 14, 1 / 10),
    baseY: room?.min.y ?? 2,
  };

  return {
    weights,
    open: weights.sky,
    room,
    sunDirection,
    sunColor,
    sunSize: 1.3 * (Math.PI / 180),
    zenith,
    horizon,
    nadir,
    nebula: { a: neb.a, b: neb.b, core: neb.core, strength: weights.sky, band, offset },
    stars: weights.sky,
    planet: weights.sky > 0 ? planet : null,
    fogColor,
    fogDensity,
    heightFog,
    grade: worldGrade(weights, palette),
    shafts: weights.sky > 0 ? 0.6 + 0.4 * weights.sky : 0,
  };
}

/**
 * True when a vertex lies on a face of the room box (within `eps`) with its
 * normal facing into the room (also the rounded edges and corners, whose
 * normals lean between two or three faces).
 */
function onWall(px: number, py: number, pz: number, nx: number, ny: number, nz: number, room: { min: Vec3; max: Vec3 }, eps: number): boolean {
  const ALIGN = 0.3;
  return (
    (Math.abs(px - room.min.x) < eps && nx > ALIGN) ||
    (Math.abs(px - room.max.x) < eps && nx < -ALIGN) ||
    (Math.abs(py - room.min.y) < eps && ny > ALIGN) ||
    (Math.abs(py - room.max.y) < eps && ny < -ALIGN) ||
    (Math.abs(pz - room.min.z) < eps && nz > ALIGN) ||
    (Math.abs(pz - room.max.z) < eps && nz < -ALIGN)
  );
}

/**
 * Removes the triangles of the level box's sealing walls from a chunk mesh
 * (all three vertices on a room face, normals facing into the room; the
 * rounded edges and corners of the box too), so the sky shows where the
 * open level ends. Rock touching the wall stays. Returns `mesh` itself when
 * nothing is removed. Collision is unchanged: the sky pass draws a boundary
 * grid where the camera comes near the wall.
 */
export function stripRoomWalls(mesh: MeshData, room: { min: Vec3; max: Vec3 }, eps = 0.5): MeshData {
  const { positions: p, normals: n, indices } = mesh;
  const count = p.length / 3;
  let wall: Uint8Array | null = null;
  for (let v = 0; v < count; v++) {
    const o = v * 3;
    if (!onWall(p[o] ?? 0, p[o + 1] ?? 0, p[o + 2] ?? 0, n[o] ?? 0, n[o + 1] ?? 0, n[o + 2] ?? 0, room, eps)) continue;
    wall ??= new Uint8Array(count);
    wall[v] = 1;
  }
  if (!wall) return mesh;
  const kept = new Uint32Array(indices.length);
  let k = 0;
  for (let i = 0; i + 2 < indices.length; i += 3) {
    const a = indices[i] ?? 0;
    const b = indices[i + 1] ?? 0;
    const c = indices[i + 2] ?? 0;
    if (wall[a] && wall[b] && wall[c]) continue;
    kept[k++] = a;
    kept[k++] = b;
    kept[k++] = c;
  }
  if (k === indices.length) return mesh;
  return { ...mesh, indices: kept.slice(0, k) };
}

/**
 * Screen position of the sun for the light shafts: NDC x/y of the direction
 * `sun` through the view-projection `viewProj` (column-major), or null when
 * the sun is behind the camera.
 */
export function sunScreen(viewProj: ArrayLike<number>, sun: Vec3): { x: number; y: number } | null {
  const e = (i: number): number => viewProj[i] ?? 0;
  // A direction (w = 0): translation drops out, so the far plane does not matter.
  const x = e(0) * sun.x + e(4) * sun.y + e(8) * sun.z;
  const y = e(1) * sun.x + e(5) * sun.y + e(9) * sun.z;
  const w = e(3) * sun.x + e(7) * sun.y + e(11) * sun.z;
  if (w <= 1e-4) return null;
  return { x: x / w, y: y / w };
}

/** Light shaft visibility 1 (sun on screen) .. 0 (sun far off screen or behind). */
export function shaftFade(ndc: { x: number; y: number } | null): number {
  if (!ndc) return 0;
  const m = Math.max(Math.abs(ndc.x), Math.abs(ndc.y));
  const t = Math.min(Math.max((m - 1.0) / 0.8, 0), 1);
  return 1 - t * t * (3 - 2 * t);
}
