// Terrain chunk shader (VQ3b): triplanar procedural rock (rockTextures.ts)
// with detail normals at two scales, albedo variation and per-world strata;
// per-material looks (weak: sandy and crumbly, rock: neutral, hard: dark
// blue-grey with glossy mineral veins); GGX specular from the sun and the
// headlight; VQ3a's baked vertex AO; team paint with the colour-blind paint
// patterns and the surface states (hot, charged, glowing, scorched) as
// emissive HDR light. Output is linear HDR radiance (post.ts tonemaps).
//
// Smooth blends: the vertex shader turns the packed `team` attribute into
// per-vertex weights (material one-hot, paint coverage, premultiplied team
// tint, paint effect one-hot, surface state bits) that interpolate across
// the triangle; the fragment shader sharpens them around 0.5 with rock noise,
// so material, paint and state borders are organic curves instead of the
// per-triangle switches of the old `@interpolate(flat)` attribute.
//
// `DETAIL` (pipeline override, RenderQuality.terrainDetail) false = the
// "low" variant: one rock scale, no grain or vein samples (3 texture
// samples instead of 9).

import { FRAME_WGSL, PAINT_COLOR_SHIFT, TEAM_SLOTS } from "./frame.ts";
import { LIGHTING_WGSL } from "./lighting.ts";

/** World units per texture repeat of the rock layer (detail normals, cracks). */
export const ROCK_SCALE_U = 9;
/** World units per repeat of the macro (albedo blotch) sample of the rock layer. */
export const MACRO_SCALE_U = 31;
/** World units per repeat of the grain layer (fine normals, pits, veins). */
export const GRAIN_SCALE_U = 2.7;

export const TERRAIN_WGSL = /* wgsl */ `
${FRAME_WGSL}
${LIGHTING_WGSL}

struct TerrainParams {
  strata: vec4f,  // xyz unit strata normal, w bands per u
  offset: vec4f,  // texture offsets per world
  look: vec4f,    // x strata contrast, y vein amount
};
@group(1) @binding(0) var rock_tex: texture_2d_array<f32>;
@group(1) @binding(1) var rock_samp: sampler;
@group(1) @binding(2) var<uniform> tp: TerrainParams;

override DETAIL: bool = true;

const PI: f32 = 3.14159265;

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) world_pos: vec3f,
  @location(1) normal: vec3f,
  // paint team one-hot (TEAM_SLOTS = 4) of painted (or paint-dilated) vertices
  @location(2) team: vec4f,
  // paint effect one-hot (plain, red, blue, yellow) of those vertices
  @location(3) effect: vec4f,
  // x weak rock, y hard rock, z paint coverage (blurred weights, terrainBlend.ts), w baked AO
  @location(4) mat: vec4f,
  // hot, charged, glowing, scorched (blurred weights)
  @location(5) surf: vec4f,
};

@vertex
fn vs_main(
  @location(0) pos: vec3f,
  @location(1) normal: vec3f,
  @location(2) team_bits: u32,
  @location(3) extra0: vec4f,
  @location(4) extra1: vec4f,
) -> VsOut {
  var out: VsOut;
  out.clip = frame.view_proj * vec4f(pos, 1.0);
  out.world_pos = pos;
  out.normal = normal;
  let team = team_bits & 0xffu;
  out.team = vec4f(vec4<bool>(team == 0u, team == 1u, team == 2u, team == 3u));
  let ok = select(0.0, 1.0, team < ${TEAM_SLOTS}u);
  let pc = (team_bits >> ${PAINT_COLOR_SHIFT}u) & 0xffu;
  out.effect = vec4f(vec4<bool>(pc == 0u || pc > 3u, pc == 1u, pc == 2u, pc == 3u)) * ok;
  out.mat = vec4f(extra0.yzw, extra0.x);
  out.surf = extra1;
  return out;
}

// One triplanar sample of a texture layer: height gradient in world units
// (surface gradient), height and the alpha channel, blended by w.
struct Tri {
  grad: vec3f,
  h: f32,
  a: f32,
};

fn tri(p: vec3f, w: vec3f, repeat_u: f32, layer: i32, off: vec2f) -> Tri {
  let q = p / repeat_u;
  let sx = textureSample(rock_tex, rock_samp, q.zy + off, layer);
  let sy = textureSample(rock_tex, rock_samp, q.xz + off.yx + vec2f(0.5, 0.25), layer);
  let sz = textureSample(rock_tex, rock_samp, q.xy + off * 1.7 + vec2f(0.25, 0.5), layer);
  let gx = sx.xy * 2.0 - 1.0;
  let gy = sy.xy * 2.0 - 1.0;
  let gz = sz.xy * 2.0 - 1.0;
  var t: Tri;
  t.grad = (vec3f(0.0, gx.y, gx.x) * w.x + vec3f(gy.x, 0.0, gy.y) * w.y + vec3f(gz.x, gz.y, 0.0) * w.z) / repeat_u;
  t.h = dot(vec3f(sx.z, sy.z, sz.z), w);
  t.a = dot(vec3f(sx.w, sy.w, sz.w), w);
  return t;
}

// Bumps the normal by a world-space height gradient (surface gradient
// framework: only the part tangent to the surface tilts the normal).
fn perturb(n: vec3f, g: vec3f) -> vec3f {
  return normalize(n - (g - n * dot(g, n)));
}

// Interpolated 0..1 blend weight sharpened into an organic border: the
// 0.5 iso-line moves with the noise and the edge stays a few cm soft.
fn sharpen(w: f32, noise: f32) -> f32 {
  return smoothstep(0.42, 0.58, w + noise);
}

// Paint effect colours (PAINT_* 1..3): red bouncy, blue slippery, yellow sticky (display values).
const PAINT_RED: vec3f = vec3f(0.95, 0.18, 0.15);
const PAINT_BLUE: vec3f = vec3f(0.2, 0.45, 1.0);
const PAINT_YELLOW: vec3f = vec3f(1.0, 0.85, 0.15);

// Dominant one of interpolated one-hot weights: a high power, normalised.
fn pick(w: vec4f) -> vec4f {
  let q = pow(max(w, vec4f(0.0)), vec4f(10.0));
  return q / max(q.x + q.y + q.z + q.w, 1e-6);
}

// The three paint patterns at surface coordinates uv (red stripes, blue
// waves, yellow dots), anti-aliased: distance to each band's centre, so
// fwidth stays small at the wrap. Stripes and waves cover [0.62, 1.0) of
// their period, dots a 0.24 radius per cell.
// \`fw\` is fwidth(uv), taken by the caller in uniform control flow (this
// runs only on painted pixels).
fn paint_marks(uv: vec2f, fw: vec2f) -> vec3f {
  let stripe_raw = (uv.x + uv.y) * 0.9;
  let wave_raw = uv.y * 0.8 + 0.3 * sin(uv.x * 2.2);
  let stripe_d = abs(fract(stripe_raw + 0.69) - 0.5);
  let wave_d = abs(fract(wave_raw + 0.69) - 0.5);
  let dot_d = length(fract(uv * 0.9) - vec2f(0.5));
  return vec3f(
    1.0 - aa_step(0.19, stripe_d, (fw.x + fw.y) * 0.9),
    1.0 - aa_step(0.19, wave_d, fw.y * 0.8 + fw.x * 0.66),
    1.0 - aa_step(0.24, dot_d, (fw.x + fw.y) * 0.9),
  );
}

// Anti-aliased step: 1 where x > edge, softened over one pixel.
fn aa_step(edge: f32, x: f32, fw: f32) -> f32 {
  return smoothstep(edge - fw, edge + fw, x);
}

// Integer hash (charged sparks): exact on large coordinates, unlike sin().
fn hash_cell(c: vec3i, t: i32) -> f32 {
  var v = vec3u(c + vec3i(t * 3, t * 7, t * 13)) * vec3u(1664525u) + vec3u(1013904223u);
  v.x += v.y * v.z;
  v.y += v.z * v.x;
  v.z += v.x * v.y;
  v = v ^ (v >> vec3u(16u));
  v.x += v.y * v.z;
  return f32(v.x) / 4294967295.0;
}

// GGX / Smith (Schlick-GGX) specular towards light l, times n.l.
fn ggx(n: vec3f, v: vec3f, l: vec3f, rough: f32, f0: f32) -> f32 {
  let nl = max(dot(n, l), 0.0);
  let nv = max(dot(n, v), 1e-3);
  let h = normalize(v + l + vec3f(0.0, 1e-5, 0.0));
  let nh = max(dot(n, h), 0.0);
  let vh = clamp(dot(v, h), 0.0, 1.0);
  let a = rough * rough;
  let a2 = a * a;
  let dd = nh * nh * (a2 - 1.0) + 1.0;
  let d = a2 / (PI * dd * dd);
  let k = (rough + 1.0) * (rough + 1.0) / 8.0;
  let g = (nv / (nv * (1.0 - k) + k)) * (nl / (nl * (1.0 - k) + k));
  let f = f0 + (1.0 - f0) * pow(1.0 - vh, 5.0);
  return d * g * f / (4.0 * nv * max(nl, 1e-3)) * nl;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let p = in.world_pos;
  let n0 = normalize(in.normal);
  let t = frame.camera_pos.w;
  // Triplanar weights from the smooth normal, but a projection whose plane
  // is nearly edge-on to the actual triangle (vertex normals rounded over
  // a sharp crease) is faded out, so it never smears into streaks.
  let nf = abs(normalize(cross(dpdx(in.world_pos), dpdy(in.world_pos))));
  var w = pow(abs(n0), vec3f(4.0)) * smoothstep(vec3f(0.12), vec3f(0.4), nf) + vec3f(1e-5);
  w = w / (w.x + w.y + w.z);
  let off = tp.offset.xy;

  // Rock relief (always), macro blotch, grain and veins (DETAIL only).
  let rock = tri(p, w, ${ROCK_SCALE_U}.0, 0, off);
  var macro_h = 0.5;
  var grain = Tri(vec3f(0.0), 0.5, 0.0);
  if (DETAIL) {
    macro_h = tri(p, w, ${MACRO_SCALE_U}.0, 0, off.yx + vec2f(0.37, 0.11)).h;
    grain = tri(p, w, ${GRAIN_SCALE_U}, 1, off * 2.3);
  }
  let noise = (rock.h - 0.5) * 0.55 + (grain.h - 0.5) * 0.35 + (macro_h - 0.5) * 0.5;

  // Material weights, sharpened with noise (borders follow the rock).
  let weak = sharpen(in.mat.x, noise);
  let hard = sharpen(in.mat.y, -noise * 0.8);
  let mats = vec3f(weak, max(0.0, 1.0 - weak - hard), hard);
  let mw = mats / max(mats.x + mats.y + mats.z, 1e-4);

  // Strata: bands along the per-world tilted axis, warped by the relief.
  let s = dot(p, tp.strata.xyz) * tp.strata.w + (rock.h - 0.5) * 0.25 + (macro_h - 0.5) * 0.8;
  let band = textureSample(rock_tex, rock_samp, vec2f(s * 0.5, 0.37 + tp.offset.z), 0).b;
  let fine_band = textureSample(rock_tex, rock_samp, vec2f(s * 3.1, 0.71 + tp.offset.w), 0).b;
  let strata = mix(vec3f(1.12, 1.0, 0.86), vec3f(0.78, 0.82, 0.92), smoothstep(0.3, 0.7, band));
  let strata_tint = mix(vec3f(1.0), strata * (0.9 + 0.25 * fine_band), tp.look.x);

  // Per-material albedo, roughness, F0 and bump strength.
  let base = frame.rock_color.rgb;
  let blotch = 0.7 + 0.6 * macro_h;
  let cavity = 0.7 + 0.6 * rock.h;
  let rock_alb = base * blotch * strata_tint * (0.86 + 0.28 * grain.h) * cavity;
  let weak_alb = base * vec3f(1.38, 1.1, 0.74) * (0.62 + 0.7 * grain.h) * (0.8 + 0.4 * macro_h) * mix(vec3f(1.0), strata_tint, 0.6);
  let vein = smoothstep(0.55, 0.8, rock.a) * tp.look.y;
  let hard_alb = mix(base * vec3f(0.42, 0.5, 0.68) * (0.7 + 0.6 * macro_h) * (0.85 + 0.3 * grain.a) * cavity, vec3f(0.62, 0.7, 0.8), vein);
  var albedo = rock_alb * mw.y + weak_alb * mw.x + hard_alb * mw.z;
  var rough = 0.78 * mw.y + 0.95 * mw.x + mix(0.42, 0.18, vein) * mw.z;
  var f0 = 0.04 * (mw.x + mw.y) + mix(0.05, 0.55, vein) * mw.z;
  var bump = vec2f(1.0, 0.7) * mw.y + vec2f(1.15, 1.5) * mw.x + vec2f(0.8, 0.35) * mw.z;

  // Paint and surface states only where a vertex of the triangle has them
  // (uniform rock skips this ALU).
  let fwp = fwidth(p);
  var emissive = vec3f(0.0);
  let ok = in.team.x + in.team.y + in.team.z + in.team.w;
  if (ok > 0.0) {
    // Paint: coverage sharpened into a splattered edge. Team and effect come
    // as interpolated one-hot weights; a high power picks the dominant one
    // (noise moves the boundary), so two paints meet at a crisp organic edge
    // instead of a mixed hue. Effect patterns (stripes, waves, dots) are
    // drawn per triplanar plane and blended like the rock.
    let paint_w = sharpen(in.mat.z, noise * 0.7 + (grain.h - 0.5) * 0.2) * smoothstep(0.0, 0.05, ok);
    let jitter = vec4f(noise, -noise, noise * 0.7, -noise * 0.7) * 0.35;
    let tw = pick(in.team / max(ok, 1e-4) + jitter);
    let tint = frame.team_colors[0].rgb * tw.x + frame.team_colors[1].rgb * tw.y + frame.team_colors[2].rgb * tw.z + frame.team_colors[3].rgb * tw.w;
    let fw = pick(in.effect / max(ok, 1e-4) + jitter.yxwz);
    let fx_sum = fw.y + fw.z + fw.w;
    let fx_w = fx_sum * paint_w;
    let fx_col = srgb_to_linear(PAINT_RED) * fw.y + srgb_to_linear(PAINT_BLUE) * fw.z + srgb_to_linear(PAINT_YELLOW) * fw.w;
    // Patterns project along the dominant axis of the smooth normal: a
    // crisp switch along a smooth curve (blending two crossing patterns
    // reads as blurry plaid).
    let an = abs(n0);
    let wp = select(select(vec3f(0.0, 0.0, 1.0), vec3f(0.0, 1.0, 0.0), an.y >= an.z), vec3f(1.0, 0.0, 0.0), an.x >= an.y && an.x >= an.z);
    let marks = paint_marks(p.yz, fwp.yz) * wp.x + paint_marks(p.xz, fwp.xz) * wp.y + paint_marks(p.xy, fwp.xy) * wp.z;
    let mark = dot(marks, fw.yzw) / max(fx_sum, 1e-4);
    let plain_alb = mix(albedo, tint, 0.85);
    let fx_alb = mix(fx_col / max(fx_sum, 1e-4), tint * 0.05, mark);
    // Where two teams or effects meet, a thin darker line (overlapping paint
    // edges) instead of a mixed hue.
    let mixed = 1.0 - min(max(max(tw.x, tw.y), max(tw.z, tw.w)), max(max(fw.x, fw.y), max(fw.z, fw.w)));
    let seam = 1.0 - 0.85 * smoothstep(0.12, 0.4, mixed);
    let paint_alb = mix(plain_alb, fx_alb, fx_sum) * seam;
    albedo = mix(albedo, paint_alb, paint_w);
    rough = mix(rough, 0.38, paint_w);
    f0 = mix(f0, 0.05, paint_w);
    bump = mix(bump, vec2f(0.75, 0.15), paint_w);
    emissive = tint * 0.3 * (paint_w - fx_w) + fx_col / max(fx_sum, 1e-4) * 0.4 * (1.0 - mark) * fx_w;
  }
  if (dot(in.surf, vec4f(1.0)) > 0.0) {
    // Surface states (HDR, these bloom). Hot: dark crust with glowing cracks
    // (yellow-white in the cracks, deep red on the crust), pulsing. Charged:
    // a faint blue glow, flickering arcs along the cracks and small round
    // sparks. Glowing: soft green light. Scorched: charred and matte.
    let hot = sharpen(in.surf.x, noise * 0.7);
    let charged = sharpen(in.surf.y, noise * 0.7);
    let glow = sharpen(in.surf.z, noise * 0.7);
    let scorch = sharpen(in.surf.w, noise * 0.9);
    let pulse = 0.75 + 0.25 * sin(t * 3.0 + macro_h * 12.0);
    // Glowing seams: the rock's own cracks plus a cooling-crust network (an
    // iso-line of the relief), wobbling with the fine grain.
    let seam_h = rock.h + (grain.h - 0.5) * 0.12;
    let crack = max(1.0 - smoothstep(0.2, 0.36, seam_h), 1.0 - smoothstep(0.0, 0.035, abs(seam_h - 0.5)));
    let lava = mix(vec3f(0.8, 0.1, 0.02) * 0.18, vec3f(1.0, 0.5, 0.12) * 5.0, crack);
    emissive += lava * pulse * hot;
    let cell = p * 6.0;
    let local = fract(cell) - vec3f(0.5);
    let spark = step(0.9, hash_cell(vec3i(floor(cell)), i32(floor(t * 18.0)))) * (1.0 - smoothstep(0.08, 0.3, length(local)));
    let arc_flicker = step(0.45, hash_cell(vec3i(floor(p * 0.7)), i32(floor(t * 12.0))));
    let arc = (1.0 - smoothstep(0.02, 0.14, abs(rock.h - 0.3 - 0.1 * sin(t * 7.0 + macro_h * 20.0)))) * arc_flicker;
    emissive += (vec3f(0.01, 0.04, 0.12) + vec3f(0.35, 0.7, 1.0) * (spark * 7.0 + arc * 1.6)) * charged;
    emissive += vec3f(0.3, 1.0, 0.5) * (0.9 + 0.5 * grain.h) * glow;
    albedo = mix(albedo, albedo * (0.08 + 0.16 * grain.h * rock.h), scorch);
    rough = mix(rough, 0.95, scorch);
    albedo = mix(albedo, albedo * 0.25 * (0.6 + 0.8 * rock.h), hot);
  }

  // Detail normal, occlusion, light.
  // Slopes decode to 24 (rock) and 60 (grain) height units per texture
  // repeat; relief about 0.3 u and 0.04 u per height unit.
  let n = perturb(n0, rock.grad * (24.0 * 0.3) * bump.x + grain.grad * (60.0 * 0.04) * bump.y);
  let ao = in.mat.w;
  let occl = mix(1.0, smoothstep(0.08, 0.95, ao), 0.85) * mix(1.0, smoothstep(0.05, 0.6, rock.h), 0.3);
  let to_cam = frame.camera_pos.xyz - p;
  let d = length(to_cam);
  let v = to_cam / max(d, 1e-4);
  let sun_l = frame.light_dir.xyz;
  let sun_rad = vec3f(1.0, 0.93, 0.8) * 0.9;
  let head_rad = vec3f(1.0, 0.97, 0.92) * frame.light_dir.w / (1.0 + 0.004 * d * d);
  let spec = ggx(n, v, sun_l, rough, f0) * sun_rad + ggx(n, v, v, rough, f0) * head_rad;
  let color = albedo * light(n, p) * occl + spec * occl + emissive;
  return vec4f(apply_fog(color, p), 1.0);
}
`;
