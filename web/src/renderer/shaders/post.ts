// Post-processing shaders (VQ2): bloom down/up chain and the final
// composite (exposure, colour grade, bloom, vignette, AgX tonemap, sRGB
// encode, dither, optional film grain). Driven by renderer/post.ts.

import { PIXEL_WGSL } from "../pixelPost.ts";

/** Fullscreen triangle; uv (0,0) is the top-left texel. */
const FULLSCREEN_VS = /* wgsl */ `
struct PostVsOut {
  @builtin(position) clip: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> PostVsOut {
  let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
  var out: PostVsOut;
  out.clip = vec4f(p, 0.0, 1.0);
  out.uv = vec2f(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
  return out;
}
`;

/** Floats in the bloom pass uniform (texel size of the source, mode, threshold). */
export const BLOOM_UNIFORM_FLOATS = 4;

/**
 * Bloom downsample: 13-tap filter (Jimenez, "Next generation post
 * processing in Call of Duty: Advanced Warfare"). The first pass (mode 1)
 * weights its five boxes by Karis' 1/(1+luma) average, so single bright
 * pixels do not flicker, and keeps only light above a soft-knee threshold
 * (emissives, flashes, hot rock), so lit rock stays crisp; it also clamps
 * NaN/inf from the scene.
 */
export const BLOOM_DOWN_WGSL = /* wgsl */ `
${FULLSCREEN_VS}
struct Bloom { texel: vec2f, mode: f32, threshold: f32 };
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> p: Bloom;

fn tap(uv: vec2f, dx: f32, dy: f32) -> vec3f {
  let c = textureSampleLevel(src, samp, uv + vec2f(dx, dy) * p.texel, 0.0).rgb;
  // Clamp: no NaN/inf, and no single texel brighter than a strong flash.
  return min(select(c, vec3f(0.0), c != c), vec3f(64.0));
}

fn karis(c: vec3f) -> f32 {
  return 1.0 / (1.0 + dot(c, vec3f(0.2126, 0.7152, 0.0722)));
}

@fragment
fn fs_main(in: PostVsOut) -> @location(0) vec4f {
  let uv = in.uv;
  let a = tap(uv, -2.0, -2.0); let b = tap(uv, 0.0, -2.0); let c = tap(uv, 2.0, -2.0);
  let d = tap(uv, -2.0, 0.0);  let e = tap(uv, 0.0, 0.0);  let f = tap(uv, 2.0, 0.0);
  let g = tap(uv, -2.0, 2.0);  let h = tap(uv, 0.0, 2.0);  let i = tap(uv, 2.0, 2.0);
  let j = tap(uv, -1.0, -1.0); let k = tap(uv, 1.0, -1.0);
  let l = tap(uv, -1.0, 1.0);  let m = tap(uv, 1.0, 1.0);
  if (p.mode > 0.5) {
    let g0 = (a + b + d + e) * 0.25;
    let g1 = (b + c + e + f) * 0.25;
    let g2 = (d + e + g + h) * 0.25;
    let g3 = (e + f + h + i) * 0.25;
    let g4 = (j + k + l + m) * 0.25;
    let w0 = karis(g0) * 0.125; let w1 = karis(g1) * 0.125;
    let w2 = karis(g2) * 0.125; let w3 = karis(g3) * 0.125; let w4 = karis(g4) * 0.5;
    let sum = (g0 * w0 + g1 * w1 + g2 * w2 + g3 * w3 + g4 * w4) / max(w0 + w1 + w2 + w3 + w4, 1e-4);
    // Soft knee (knee = threshold / 2) around the threshold.
    let t = p.threshold;
    let knee = t * 0.5;
    let br = max(sum.r, max(sum.g, sum.b));
    var rq = clamp(br - t + knee, 0.0, 2.0 * knee);
    rq = rq * rq / max(4.0 * knee, 1e-4);
    let w = max(rq, br - t) / max(br, 1e-4);
    return vec4f(sum * max(w, 0.0), 1.0);
  }
  let o = e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
  return vec4f(o, 1.0);
}
`;

/** Bloom upsample: 3x3 tent over the smaller mip, added onto the larger one (additive blend). */
export const BLOOM_UP_WGSL = /* wgsl */ `
${FULLSCREEN_VS}
struct Bloom { texel: vec2f, mode: f32, threshold: f32 };
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> p: Bloom;

fn tap(uv: vec2f, dx: f32, dy: f32) -> vec3f {
  return textureSampleLevel(src, samp, uv + vec2f(dx, dy) * p.texel, 0.0).rgb;
}

@fragment
fn fs_main(in: PostVsOut) -> @location(0) vec4f {
  let uv = in.uv;
  var s = tap(uv, 0.0, 0.0) * 4.0;
  s += (tap(uv, -1.0, 0.0) + tap(uv, 1.0, 0.0) + tap(uv, 0.0, -1.0) + tap(uv, 0.0, 1.0)) * 2.0;
  s += tap(uv, -1.0, -1.0) + tap(uv, 1.0, -1.0) + tap(uv, -1.0, 1.0) + tap(uv, 1.0, 1.0);
  return vec4f(s / 16.0, 1.0);
}
`;

/**
 * Scene alpha above 1 marks pixels whose colour the monochrome grade keeps
 * (Last Color Standing ships, M3 C1): alpha = 1 + keep x KEEP_ALPHA. Small,
 * so blends that read the scene alpha (the sky under the scene) stay close
 * to an alpha of 1; rgba16float resolves it in 64 steps.
 */
export const KEEP_ALPHA = 0.0625;

/** Floats in the composite uniform (struct Post below). */
export const POST_UNIFORM_FLOATS = 40;

/**
 * Final composite into the swapchain: exposure, colour grade (ASC CDL slope /
 * offset / power, saturation, monochrome) in scene-linear, bloom, vignette,
 * AgX (Troy Sobotka's AgX, Benjamin Wrensch's polynomial fit) with a look
 * (power, saturation), sRGB encoding, a ±0.5 LSB dither against banding in
 * dark gradients and optional film grain. With the Palette slider's ink look
 * (M4 A7, renderer/ink.ts) after the tonemap: toon bands in display
 * lightness, paper grain, the monochrome ink on paper (tone lifted toward the
 * paper, distant rock fading into a pale mist, a brushed wash in the sky) and
 * the ink pass's outlines and hatching. Kept pixels (KEEP_ALPHA) skip the
 * monochrome and the hatching, not the outlines.
 */
export const COMPOSITE_WGSL = /* wgsl */ `
${FULLSCREEN_VS}
struct Post {
  slope_exposure: vec4f,  // rgb CDL slope, w = exposure (linear multiplier)
  offset_bloom: vec4f,    // rgb CDL offset, w = bloom strength
  power_sat: vec4f,       // rgb CDL power, w = grade saturation
  look: vec4f,            // x = AgX look power, y = AgX look saturation, z = monochrome 0..1, w = vignette
  misc: vec4f,            // x = film grain, y = time (s), z = bloom mips, w = 1 when bloom is on
  mono_tint: vec4f,       // rgb colour kept by the monochrome grade, w = how much of it is kept
  ink_look: vec4f,        // ink (M4 A7): x toon, y paper grain, z monochrome ink, w contrast
  ink_paper: vec4f,       // rgb paper (linear), w = 1 when the ink look is on
  ink_color: vec4f,       // rgb ink (linear), w = 1 when the ink lines are valid this frame
  pixel: vec4f,           // Wrong genre's 8-bit look (M4 C1, pixelPost.ts): x cell (px), y palette, z dither, w = 1 when on
};
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var bloom: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> post: Post;
@group(0) @binding(4) var ink_lines: texture_2d<f32>;

const LUMA = vec3f(0.2126, 0.7152, 0.0722);
${PIXEL_WGSL}

fn agx_contrast(x: vec3f) -> vec3f {
  let x2 = x * x;
  let x4 = x2 * x2;
  return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn agx(c: vec3f) -> vec3f {
  let inset = mat3x3f(
    0.842479062253094, 0.0423282422610123, 0.0423756549057051,
    0.0784335999999992, 0.878468636469772, 0.0784336,
    0.0792237451477643, 0.0791661274605434, 0.879142973793104);
  let min_ev = -12.47393;
  let max_ev = 4.026069;
  var v = inset * max(c, vec3f(0.0));
  v = clamp(log2(max(v, vec3f(1e-10))), vec3f(min_ev), vec3f(max_ev));
  v = (v - min_ev) / (max_ev - min_ev);
  return agx_contrast(v);
}

fn agx_look(v: vec3f) -> vec3f {
  let luma = dot(v, LUMA);
  let p = pow(max(v, vec3f(0.0)), vec3f(post.look.x));
  return luma + post.look.y * (p - luma);
}

fn agx_eotf(v: vec3f) -> vec3f {
  let outset = mat3x3f(
    1.19687900512017, -0.0528968517574562, -0.0529716355144438,
    -0.0980208811401368, 1.15190312990417, -0.0980434501171241,
    -0.0990297440797205, -0.0989611768448433, 1.15107367264116);
  // Back to linear display light (the fit's display encoding is a 2.2 power).
  return pow(max(outset * v, vec3f(0.0)), vec3f(2.2));
}

fn linear_to_srgb(c: vec3f) -> vec3f {
  let lo = c * 12.92;
  let hi = 1.055 * pow(max(c, vec3f(0.0)), vec3f(1.0 / 2.4)) - 0.055;
  return select(hi, lo, c <= vec3f(0.0031308));
}

fn hash12(p: vec2f) -> f32 {
  var p3 = fract(vec3f(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

fn vnoise(p: vec2f) -> f32 {
  let i = floor(p);
  let f = p - i;
  let u = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash12(i), hash12(i + vec2f(1.0, 0.0)), u.x), mix(hash12(i + vec2f(0.0, 1.0)), hash12(i + vec2f(1.0, 1.0)), u.x), u.y);
}

// Paper: fine tooth, a softer mottle and faint horizontal fibres (screen
// space: the paper is the screen). About -0.5..0.5.
fn paper_grain(px: vec2f) -> f32 {
  let tooth = vnoise(px * 0.55);
  let mottle = vnoise(px * 0.09 + vec2f(17.0, 3.0));
  let fibre = vnoise(vec2f(px.x * 0.03, px.y * 0.4) + vec2f(5.0, 41.0));
  return 0.45 * tooth + 0.35 * mottle + 0.2 * fibre - 0.5;
}

// Soft steps: x in [0, 1] held on n plateaus, each step blended over 2 soft.
fn soft_bands(x: f32, n: f32, soft: f32) -> f32 {
  let y = x * n;
  let i = floor(y);
  return (i + smoothstep(0.5 - soft, 0.5 + soft, y - i)) / n;
}

// The ink look on the tonemapped colour (display-linear).
fn ink(c: vec3f, px: vec2f, keep: f32) -> vec3f {
  let lines = select(vec4f(0.0), textureLoad(ink_lines, vec2i(px), 0), post.ink_color.w > 0.5);
  let sky = lines.a;
  let l = max(dot(c, LUMA), 0.0);
  let pl = pow(l, 1.0 / 2.2);
  // Toon: soft plateaus of lightness, softer in the sky.
  let toon = post.ink_look.x * (1.0 - 0.7 * sky);
  let pb = mix(pl, soft_bands(pl, 4.0, 0.14), toon);
  var out = c * select(pow(pb / max(pl, 1e-4), 2.2), 0.0, pl < 1e-4);
  let grain = paper_grain(px) * post.ink_look.y;
  let mono = post.ink_look.z * (1.0 - keep);
  if (mono > 0.0) {
    // Ink on paper: the tone lifted toward the paper (lit rock is paper,
    // shade is ink and hatching), more contrast toward Palette 100.
    let k = post.ink_look.w;
    var m = smoothstep(mix(0.04, 0.08, k), mix(0.74, 0.58, k), pb);
    // Distant rock fades into a pale mist like an ink-wash landscape, the
    // sky is the same pale wash with soft blotches, bright things paper.
    m = mix(m, 0.74, lines.b * (1.0 - sky) * 0.85);
    let wash = vnoise(px * vec2f(0.0022, 0.0045)) * 0.65 + vnoise(px * vec2f(0.007, 0.016) + vec2f(31.0, 7.0)) * 0.35;
    let sky_m = 0.64 + 0.2 * smoothstep(0.3, 0.75, wash) + 0.35 * smoothstep(0.12, 0.7, pb);
    m = mix(m, min(sky_m, 1.0), sky);
    let paper = post.ink_paper.rgb * (1.0 + 0.22 * grain);
    out = mix(out, mix(post.ink_color.rgb, paper, m), mono);
  } else {
    out *= 1.0 + 0.1 * grain;
  }
  // Lines only ever darken (ink over a darker sky must not show as a lighter line).
  let inked = clamp(lines.r + lines.g * (1.0 - keep), 0.0, 1.0);
  return mix(out, min(out, post.ink_color.rgb), inked);
}

fn grade(c: vec3f, keep_px: f32) -> vec3f {
  var g = pow(max(c * post.slope_exposure.rgb + post.offset_bloom.rgb, vec3f(0.0)), post.power_sat.rgb);
  let luma = dot(g, LUMA);
  // Kept colour (paint, pigment: KEEP_ALPHA) is not desaturated by the Palette slider either.
  let sat = mix(post.power_sat.w, max(post.power_sat.w, 1.0), keep_px);
  g = max(vec3f(luma) + sat * (g - vec3f(luma)), vec3f(0.0));
  // Monochrome (Last Color Standing): grey, except saturated colours whose
  // hue matches mono_tint (compared on the chroma, the colour minus its grey).
  let mono = post.look.z * (1.0 - keep_px);
  if (mono > 0.0) {
    let l2 = dot(g, LUMA);
    let chroma = g - vec3f(l2);
    let tint = post.mono_tint.rgb;
    let tint_chroma = tint - vec3f(dot(tint, LUMA));
    let same_hue = smoothstep(0.8, 0.97, dot(normalize(chroma + 1e-6), normalize(tint_chroma + 1e-6)));
    let saturated = smoothstep(0.05, 0.2, length(chroma) / max(l2, 1e-3));
    let keep = post.mono_tint.w * same_hue * saturated;
    g = mix(g, mix(vec3f(l2), g, keep), mono);
  }
  return g;
}

@fragment
fn fs_main(in: PostVsOut) -> @location(0) vec4f {
  let size = vec2f(textureDimensions(scene));
  // Wrong genre (M4 C1): every pixel of a cell takes the cell centre's colour.
  let on_px = post.pixel.w > 0.5;
  let fc = select(in.clip.xy, min(pixel_center(in.clip.xy), size - vec2f(0.5)), on_px);
  let uv = select(in.uv, fc / size, on_px);
  let px = textureLoad(scene, vec2i(fc), 0);
  var hdr = px.rgb;
  // Colour kept through the monochrome grade (objects.ts marks it in the alpha).
  let keep_px = clamp((px.a - 1.0) / ${KEEP_ALPHA}, 0.0, 1.0);
  hdr = select(hdr, vec3f(0.0), hdr != hdr);
  if (post.misc.w > 0.5) {
    // The upsampled chain holds the sum of every mip (light above the
    // threshold only); average it and add it.
    let b = textureSampleLevel(bloom, samp, uv, 0.0).rgb / max(post.misc.z, 1.0);
    hdr += b * post.offset_bloom.w;
  }
  hdr *= post.slope_exposure.w;
  hdr = grade(hdr, keep_px);
  // Vignette: darken toward the corners (aspect-corrected radius).
  let q = (uv - 0.5) * vec2f(size.x / size.y, 1.0);
  hdr *= 1.0 - post.look.w * smoothstep(0.35, 1.1, dot(q, q) * 1.6);
  var ldr = agx_eotf(agx_look(agx(hdr)));
  if (post.ink_paper.w > 0.5) {
    ldr = ink(ldr, fc, keep_px);
  }
  if (post.misc.x > 0.0) {
    let n = hash12(fc + fract(post.misc.y * 7.13) * 431.0) - 0.5;
    ldr *= 1.0 + post.misc.x * n;
  }
  var out = linear_to_srgb(clamp(ldr, vec3f(0.0), vec3f(1.0)));
  if (on_px) {
    out = pixel_palette(out, in.clip.xy);
  } else {
    // Triangular-ish dither of half an 8-bit step.
    out += (hash12(in.clip.xy) - 0.5) / 255.0;
  }
  return vec4f(out, 1.0);
}
`;
