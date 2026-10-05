// Dynamic point lights (VQ6): the light list, the per-frame light info and
// the clustered-forward grid. A compute pass ("lights", renderer/lights.ts)
// tests every light against every froxel of a CLUSTER_X x CLUSTER_Y x
// CLUSTER_Z grid (screen tiles x exponential depth slices) and writes one
// 128-bit mask per froxel; the scene shaders (lighting.ts) look up their
// fragment's froxel and loop over the set bits only.
//
// Froxel coordinates: tiles split NDC x and y evenly; slice s covers view
// depth [near * ratio^(s / Z), near * ratio^((s + 1) / Z)] with ratio =
// far / near, slice 0 reaching down to the camera and the last slice out to
// infinity. The CPU mirror in renderer/lights.ts (`clusterOf`,
// `froxelBounds`) is unit tested against the same maths.

/** Screen tiles across and down, depth slices. */
export const CLUSTER_X = 16;
export const CLUSTER_Y = 9;
export const CLUSTER_Z = 24;
export const CLUSTER_COUNT = CLUSTER_X * CLUSTER_Y * CLUSTER_Z;
/** Hard cap of the light list (the per-froxel mask has one bit per light). */
export const MAX_LIGHTS = 128;
/** u32 words per froxel mask. */
export const MASK_WORDS = MAX_LIGHTS / 32;
/** Floats per light in the shading buffer: world pos + range, linear colour x intensity. */
export const LIGHT_FLOATS = 8;
/** Floats per light in the froxel builder's buffer: view-space pos + range. */
export const LIGHT_VIEW_FLOATS = 4;
/** Floats of the LightInfo uniform. */
export const LIGHT_INFO_FLOATS = 16;
/** Depth slicing range (u): slice 0 ends at CLUSTER_NEAR x ratio^(1/Z), the last starts at CLUSTER_FAR / ratio^(1/Z). */
export const CLUSTER_NEAR = 0.5;
export const CLUSTER_FAR = 240;
/** Compute workgroup size (froxels per workgroup). */
export const CLUSTER_WORKGROUP = 64;

/** Structs shared by the compute pass and the scene shaders. */
export const LIGHT_STRUCTS_WGSL = /* wgsl */ `
const CLUSTER_X = ${CLUSTER_X}u;
const CLUSTER_Y = ${CLUSTER_Y}u;
const CLUSTER_Z = ${CLUSTER_Z}u;
const CLUSTER_COUNT = ${CLUSTER_COUNT}u;

// Shading data per light (the froxel builder reads view-space copies:
// x right, y up, depth forward (> 0 in front), w = range).
struct PointLight {
  pos: vec4f,   // xyz world position, w = range (u; no light beyond)
  color: vec4f, // linear rgb x intensity, w unused
};

struct LightInfo {
  params: vec4f, // x = light count, y = tan(fovX / 2), z = tan(fovY / 2), w = headlight spot on (1) / point (0)
  slices: vec4f, // x = CLUSTER_NEAR, y = CLUSTER_FAR, z = CLUSTER_Z / ln(far / near), w = ln(near)
  spot: vec4f,   // xyz camera forward (world), w = cos of the spot's outer half-angle
  spot2: vec4f,  // x = cos of the inner half-angle, y = light outside the cone (0..1), z = distance falloff k, w = cone gain
};

// Depth slice of a view depth (CPU mirror: renderer/lights.ts sliceOf).
fn cluster_slice(depth: f32, info: LightInfo) -> u32 {
  let s = floor((log(max(depth, 1e-4)) - info.slices.w) * info.slices.z);
  return u32(clamp(s, 0.0, f32(CLUSTER_Z - 1u)));
}
`;

/** The froxel builder: one invocation per froxel, every light tested against its view-space box. */
export const CLUSTER_COMPUTE_WGSL = /* wgsl */ `
${LIGHT_STRUCTS_WGSL}

@group(0) @binding(0) var<uniform> info: LightInfo;
@group(0) @binding(1) var<storage, read> views: array<vec4f, ${MAX_LIGHTS}>;
@group(0) @binding(2) var<storage, read_write> masks: array<vec4<u32>, ${CLUSTER_COUNT}>;

fn slice_depth(s: u32) -> f32 {
  return exp(info.slices.w + f32(s) / info.slices.z);
}

@compute @workgroup_size(${CLUSTER_WORKGROUP})
fn cs_main(@builtin(global_invocation_id) gid: vec3u) {
  let c = gid.x;
  if (c >= CLUSTER_COUNT) { return; }
  let tx = c % CLUSTER_X;
  let ty = (c / CLUSTER_X) % CLUSTER_Y;
  let s = c / (CLUSTER_X * CLUSTER_Y);
  // View-space box of the froxel: NDC tile edges scaled by the depth range.
  var d0 = slice_depth(s);
  var d1 = slice_depth(s + 1u);
  if (s == 0u) { d0 = 0.0; }
  if (s == CLUSTER_Z - 1u) { d1 = 1e6; }
  let x0 = (f32(tx) / f32(CLUSTER_X) * 2.0 - 1.0) * info.params.y;
  let x1 = (f32(tx + 1u) / f32(CLUSTER_X) * 2.0 - 1.0) * info.params.y;
  let y0 = (f32(ty) / f32(CLUSTER_Y) * 2.0 - 1.0) * info.params.z;
  let y1 = (f32(ty + 1u) / f32(CLUSTER_Y) * 2.0 - 1.0) * info.params.z;
  let bmin = vec3f(min(x0 * d0, x0 * d1), min(y0 * d0, y0 * d1), d0);
  let bmax = vec3f(max(x1 * d0, x1 * d1), max(y1 * d0, y1 * d1), d1);
  var mask = vec4<u32>(0u);
  let count = u32(info.params.x);
  for (var i = 0u; i < count; i++) {
    let l = views[i];
    let q = clamp(l.xyz, bmin, bmax) - l.xyz;
    if (dot(q, q) <= l.w * l.w) {
      mask[i >> 5u] |= 1u << (i & 31u);
    }
  }
  masks[c] = mask;
}
`;
