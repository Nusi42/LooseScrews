// Smooth blend weights for terrain chunk meshes (VQ3b). Material, paint and
// surface states arrive as one id per vertex (mesh.ts MeshData). Linearly
// interpolated one-hot weights put every border on the 1 u vertex grid: a
// staircase however the shader sharpens it. Here the per-vertex weights are
// blurred over the mesh (BLUR_PASSES rounds of averaging each vertex with
// its edge neighbours), which turns the staircase into a ramp a few
// vertices wide; the terrain shader then sharpens that ramp around 0.5 with
// rock noise into an organic edge.
//
// Chunk seams: vertices within PIN_MARGIN of a chunk face (world multiples
// of CHUNK_CELLS) are shared by the two chunk meshes, which blur with
// different neighbourhoods. They keep their own one-hot weights (they still
// feed their neighbours' blur), so both chunks agree on them and the drawn
// weights stay continuous across the seam.
//
// Materials: when the mesh carries display weights (`materialWeights`,
// ls_worldgen MaterialRegion::blend_weights) they are used as they are.
// They are a smooth function of the position, continuous across chunk
// seams, so material borders need no blur and no pinning: the border is
// the 0.5 line of a ramp several vertices wide, a smooth curve however the
// triangles lie (D5: the blurred one-hot ids still followed the 1 u vertex
// grid in places and kept their staircase at chunk seams). Meshes without
// them (tests, bare sample grids) blur the ids as before.
//
// Only the neighbourhood of borders is touched (vertices within
// BLUR_PASSES edges of a triangle whose corners differ), so uniform rock
// costs one pass over the triangles. Paint team and effect colour are
// dilated onto unpainted vertices next to paint, so the colour of a blurred
// paint edge is always defined.
//
// GPU layout (two u32 per vertex, both `unorm8x4`, shaders/terrain.ts):
//   extra0: x = baked AO (255 open), y = weak rock, z = hard rock, w = paint coverage
//   extra1: x = hot, y = charged, z = glowing, w = scorched

import { expandTeams, MATERIAL_HARD, MATERIAL_ROCK, MATERIAL_WEAK, NO_TEAM, SURFACE_CHARGED, SURFACE_GLOW, SURFACE_HOT, SURFACE_SCORCH, type MeshData } from "./mesh.ts";

/** Averaging rounds over the mesh edges (ramp half-width in vertices). */
export const BLUR_PASSES = 2;
/** Chunk edge in cells (ls_worldgen::CHUNK_CELLS); chunk faces lie on world multiples of it. */
export const CHUNK_CELLS = 32;
/** Vertices this close (u) to a chunk face keep their own weights (may be shared with the neighbour chunk). */
export const PIN_MARGIN = 1.5;

/** True when a vertex at this coordinate may be shared with a neighbouring chunk. */
export function nearChunkFace(c: number): boolean {
  const r = c - Math.round(c / CHUNK_CELLS) * CHUNK_CELLS;
  return Math.abs(r) < PIN_MARGIN;
}
/** Blended channels per vertex: weak, hard, paint, hot, charged, glow, scorch. */
const CHANNELS = 7;

export interface TerrainVertexData {
  /** Packed team attribute (mesh.ts expandTeams) after paint dilation. */
  team: Uint32Array;
  /** Two u32 per vertex (see the file header). */
  extra: Uint32Array;
  /** Vertices whose weights were blurred (border neighbourhood). */
  blurred: number;
}

const unorm = (x: number): number => Math.max(0, Math.min(255, Math.round(x * 255)));

/** Builds the terrain vertex attributes of `mesh` (see the file header). */
export function terrainVertexData(mesh: MeshData): TerrainVertexData {
  const n = mesh.team.length;
  const idx = mesh.indices;
  const tris = idx.length - (idx.length % 3);
  const mats = mesh.materials && mesh.materials.length === n ? mesh.materials : null;
  const surf = mesh.surface && mesh.surface.length === n ? mesh.surface : null;
  const ao = mesh.ao && mesh.ao.length === n ? mesh.ao : null;
  const raw = mesh.team;
  const mw = mesh.materialWeights && mesh.materialWeights.length === n * 2 ? mesh.materialWeights : null;

  // Blended state per vertex as bits: weak, hard, paint, hot, charged, glow, scorch.
  const bits = new Uint8Array(n);
  for (let i = 0; i < n; i++) {
    const m = mats && !mw ? (mats[i] ?? MATERIAL_ROCK) : MATERIAL_ROCK;
    const sv = surf ? (surf[i] ?? 0) : 0;
    bits[i] =
      (m === MATERIAL_WEAK ? 1 : 0) |
      (m === MATERIAL_HARD ? 2 : 0) |
      ((raw[i] ?? NO_TEAM) < NO_TEAM ? 4 : 0) |
      (sv & SURFACE_HOT ? 8 : 0) |
      (sv & SURFACE_CHARGED ? 16 : 0) |
      (sv & SURFACE_GLOW ? 32 : 0) |
      (sv & SURFACE_SCORCH ? 64 : 0);
  }
  const packed = (i: number, b: number): void => {
    extra[i * 2] = ((ao ? (ao[i] ?? 255) : 255) | (b & 1 ? 0xff00 : 0) | (b & 2 ? 0xff0000 : 0) | (b & 4 ? 0xff000000 : 0)) >>> 0;
    extra[i * 2 + 1] = ((b & 8 ? 0xff : 0) | (b & 16 ? 0xff00 : 0) | (b & 32 ? 0xff0000 : 0) | (b & 64 ? 0xff000000 : 0)) >>> 0;
  };
  const extra = new Uint32Array(n * 2);

  // Ring 0: corners of triangles whose corners differ.
  const ring = new Uint8Array(n); // 0 = untouched, r + 1 = r edges from ring 0
  let any = false;
  for (let t = 0; t < tris; t += 3) {
    const a = idx[t] ?? 0, b = idx[t + 1] ?? 0, c = idx[t + 2] ?? 0;
    if (bits[a] !== bits[b] || bits[b] !== bits[c]) {
      ring[a] = ring[b] = ring[c] = 1;
      any = true;
    }
  }
  if (!any) {
    for (let i = 0; i < n; i++) packed(i, bits[i] ?? 0);
    if (mw) applyMaterialWeights(extra, mw);
    return { team: expandTeams(raw, mesh.surface, mesh.materials, mesh.paintColors), extra, blurred: 0 };
  }
  // Grow the rings; the last pass also collects the triangles that touch them.
  const active: number[] = [];
  for (let r = 1; r <= BLUR_PASSES; r++) {
    const last = r === BLUR_PASSES;
    for (let t = 0; t < tris; t += 3) {
      const a = idx[t] ?? 0, b = idx[t + 1] ?? 0, c = idx[t + 2] ?? 0;
      const ra = ring[a] ?? 0, rb = ring[b] ?? 0, rc = ring[c] ?? 0;
      if (ra === r || rb === r || rc === r) {
        if (!ra) ring[a] = r + 1;
        if (!rb) ring[b] = r + 1;
        if (!rc) ring[c] = r + 1;
      }
      if (last && (ring[a] || ring[b] || ring[c])) active.push(t);
    }
  }

  // Dilate team and paint colour onto unpainted vertices next to paint,
  // as far as the blur can carry paint coverage.
  const team = raw.slice();
  const colors = mesh.paintColors && mesh.paintColors.length === n ? mesh.paintColors.slice() : undefined;
  for (let round = 0; round <= BLUR_PASSES; round++) {
    for (const t of active) {
      const a = idx[t] ?? 0, b = idx[t + 1] ?? 0, c = idx[t + 2] ?? 0;
      const src = (team[a] ?? NO_TEAM) < NO_TEAM ? a : (team[b] ?? NO_TEAM) < NO_TEAM ? b : (team[c] ?? NO_TEAM) < NO_TEAM ? c : -1;
      if (src < 0) continue;
      const st = team[src] ?? NO_TEAM, sc = colors ? (colors[src] ?? 0) : 0;
      if (ring[a] && (team[a] ?? NO_TEAM) >= NO_TEAM) {
        team[a] = st;
        if (colors) colors[a] = sc;
      }
      if (ring[b] && (team[b] ?? NO_TEAM) >= NO_TEAM) {
        team[b] = st;
        if (colors) colors[b] = sc;
      }
      if (ring[c] && (team[c] ?? NO_TEAM) >= NO_TEAM) {
        team[c] = st;
        if (colors) colors[c] = sc;
      }
    }
  }

  // Weights of every vertex the active triangles touch.
  const C = CHANNELS;
  const weights = new Float32Array(n * C);
  for (const t of active) {
    for (let k = 0; k < 3; k++) {
      const v = idx[t + k] ?? 0;
      const b = bits[v] ?? 0;
      for (let ch = 0; ch < C; ch++) weights[v * C + ch] = (b >> ch) & 1;
    }
  }
  const pos = mesh.positions;
  const pinned = new Uint8Array(n);
  for (let i = 0; i < n; i++) {
    if (ring[i] && (nearChunkFace(pos[i * 3] ?? 0) || nearChunkFace(pos[i * 3 + 1] ?? 0) || nearChunkFace(pos[i * 3 + 2] ?? 0))) {
      pinned[i] = 1;
    }
  }
  // Blur: each pass averages every touched vertex with its edge neighbours
  // (an edge counts once per triangle, so interior edges weigh twice); the
  // vertex's own weight equals its neighbours' total, which keeps the 0.5
  // crossing of a straight border in place.
  const sum = new Float32Array(n * C);
  const count = new Uint16Array(n);
  for (let pass = 0; pass < BLUR_PASSES; pass++) {
    for (const t of active) {
      const a = idx[t] ?? 0, b = idx[t + 1] ?? 0, c = idx[t + 2] ?? 0;
      const oa = a * C, ob = b * C, oc = c * C;
      for (let ch = 0; ch < C; ch++) {
        const wa = weights[oa + ch] ?? 0, wb = weights[ob + ch] ?? 0, wc = weights[oc + ch] ?? 0;
        sum[oa + ch] = (sum[oa + ch] ?? 0) + wb + wc;
        sum[ob + ch] = (sum[ob + ch] ?? 0) + wc + wa;
        sum[oc + ch] = (sum[oc + ch] ?? 0) + wa + wb;
      }
      count[a] = (count[a] ?? 0) + 2;
      count[b] = (count[b] ?? 0) + 2;
      count[c] = (count[c] ?? 0) + 2;
    }
    for (const t of active) {
      for (let k = 0; k < 3; k++) {
        const v = idx[t + k] ?? 0;
        const cnt = count[v] ?? 0;
        if (cnt === 0) continue; // already done through another triangle
        if (ring[v] && !pinned[v]) {
          const o = v * C;
          for (let ch = 0; ch < C; ch++) weights[o + ch] = 0.5 * (weights[o + ch] ?? 0) + (0.5 * (sum[o + ch] ?? 0)) / cnt;
        }
        const o = v * C;
        for (let ch = 0; ch < C; ch++) sum[o + ch] = 0;
        count[v] = 0;
      }
    }
  }

  let blurred = 0;
  for (let i = 0; i < n; i++) {
    if (!ring[i]) {
      packed(i, bits[i] ?? 0);
      continue;
    }
    blurred++;
    const o = i * C;
    extra[i * 2] = ((ao ? (ao[i] ?? 255) : 255) | (unorm(weights[o] ?? 0) << 8) | (unorm(weights[o + 1] ?? 0) << 16) | (unorm(weights[o + 2] ?? 0) << 24)) >>> 0;
    extra[i * 2 + 1] =
      (unorm(weights[o + 3] ?? 0) | (unorm(weights[o + 4] ?? 0) << 8) | (unorm(weights[o + 5] ?? 0) << 16) | (unorm(weights[o + 6] ?? 0) << 24)) >>> 0;
  }
  if (mw) applyMaterialWeights(extra, mw);
  return { team: expandTeams(team, mesh.surface, mesh.materials, colors), extra, blurred };
}

/** Writes the mesh's material display weights into extra0's y (weak) and z (hard) bytes. */
function applyMaterialWeights(extra: Uint32Array, mw: Uint8Array): void {
  const n = mw.length >> 1;
  for (let i = 0; i < n; i++) {
    extra[i * 2] = (((extra[i * 2] ?? 0) & 0xff0000ff) | ((mw[i * 2] ?? 0) << 8) | ((mw[i * 2 + 1] ?? 0) << 16)) >>> 0;
  }
}
