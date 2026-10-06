// Everything is cake (M4 C1, milestone-4-plan.md §3.2): the terrain shader's
// cake branch. Inside a sphere around the wave's origin (TerrainParams
// `cake`: origin, radius; `cake_up`: the level's up, amount) the rock turns
// into a layer cake: golden sponge with pores, cream and jam layers along
// the up, pink frosting on rock facing up that drips down the walls, and
// rainbow sprinkles on the frosting. The wave's front is a bright band of
// frosting foam. Arithmetic only (no texture samples or derivatives), so it
// sits in non-uniform control flow. Driven by renderer/cake.ts.

/** Display colours (sRGB) of the cake's materials. */
export const CAKE_COLORS = {
  sponge: [0.93, 0.68, 0.33],
  crumb: [0.78, 0.5, 0.2],
  cream: [0.99, 0.95, 0.86],
  jam: [0.78, 0.07, 0.17],
  frosting: [1.0, 0.62, 0.78],
  frostingLight: [1.0, 0.86, 0.92],
} as const;

/** Sprinkle colours (display sRGB). */
export const SPRINKLES: readonly (readonly [number, number, number])[] = [
  [1.0, 0.25, 0.3],
  [1.0, 0.85, 0.15],
  [0.25, 0.65, 1.0],
  [0.35, 0.9, 0.4],
  [1.0, 1.0, 1.0],
  [0.75, 0.4, 1.0],
];

/** Width (u) of the bright band at the wave's front. */
export const CAKE_FRONT_U = 2.2;
/** Height (u) of one sponge-cream-jam repeat along the up. */
export const CAKE_LAYER_U = 2.6;

const v = (c: readonly number[]): string => `vec3f(${c.map((x) => x.toFixed(4)).join(", ")})`;

/**
 * WGSL: `cake_amount(p)` (0 outside the wave, 1 inside; the front blends),
 * `cake_front(p)` and `cake_look(p, n, noise) -> CakeLook`. Needs the
 * TerrainParams `tp` with `cake` and `cake_up`, `srgb_to_linear`.
 */
export const CAKE_WGSL = /* wgsl */ `
struct CakeLook {
  albedo: vec3f,
  rough: f32,
  emissive: vec3f,
  f0: f32,
};

fn cake_hash(p: vec3f) -> f32 {
  let q = fract(p * vec3f(0.1031, 0.1030, 0.0973));
  let r = q + dot(q, q.yzx + 33.33);
  return fract((r.x + r.y) * r.z);
}

fn cake_noise(p: vec3f) -> f32 {
  let i = floor(p);
  let f = p - i;
  let u = f * f * (3.0 - 2.0 * f);
  let a = mix(mix(cake_hash(i), cake_hash(i + vec3f(1.0, 0.0, 0.0)), u.x), mix(cake_hash(i + vec3f(0.0, 1.0, 0.0)), cake_hash(i + vec3f(1.0, 1.0, 0.0)), u.x), u.y);
  let b = mix(mix(cake_hash(i + vec3f(0.0, 0.0, 1.0)), cake_hash(i + vec3f(1.0, 0.0, 1.0)), u.x), mix(cake_hash(i + vec3f(0.0, 1.0, 1.0)), cake_hash(i + vec3f(1.0, 1.0, 1.0)), u.x), u.y);
  return mix(a, b, u.z);
}

// Signed distance (u) from the wave's front: positive inside the cake.
fn cake_depth(p: vec3f) -> f32 {
  let wobble = (cake_noise(p * 0.35) - 0.5) * 3.0;
  return tp.cake.w - length(p - tp.cake.xyz) + wobble;
}

fn cake_amount(p: vec3f) -> f32 {
  if (tp.cake_up.w <= 0.0) { return 0.0; }
  return smoothstep(-0.6, 0.6, cake_depth(p)) * tp.cake_up.w;
}

fn cake_look(p: vec3f, n: vec3f, noise: f32) -> CakeLook {
  var out: CakeLook;
  let up = tp.cake_up.xyz;
  let h = dot(p, up);
  let facing = dot(n, up);
  // Horizontal coordinates (the drips run along walls in one of them).
  let side = normalize(select(cross(up, vec3f(1.0, 0.0, 0.0)), cross(up, vec3f(0.0, 0.0, 1.0)), abs(up.x) > 0.9));
  let side2 = cross(up, side);
  let hz = vec2f(dot(p, side), dot(p, side2));
  // Layers: sponge, a cream line, a jam line, wavy with the rock.
  let lh = h / ${CAKE_LAYER_U} + (cake_noise(p * 0.8) - 0.5) * 0.12 + noise * 0.05;
  let f = fract(lh);
  let cream = smoothstep(0.02, 0.05, f) * (1.0 - smoothstep(0.12, 0.15, f));
  let jam = smoothstep(0.15, 0.17, f) * (1.0 - smoothstep(0.21, 0.23, f));
  // Sponge pores: little dark pits.
  let pore = smoothstep(0.62, 0.8, cake_noise(p * 7.0)) * 0.5 + smoothstep(0.7, 0.85, cake_noise(p * 15.0 + vec3f(3.1))) * 0.35;
  let sponge = mix(srgb_to_linear(${v(CAKE_COLORS.sponge)}), srgb_to_linear(${v(CAKE_COLORS.crumb)}), pore + noise * 0.3);
  var alb = mix(sponge, srgb_to_linear(${v(CAKE_COLORS.cream)}), cream);
  alb = mix(alb, srgb_to_linear(${v(CAKE_COLORS.jam)}), jam);
  var rough = mix(0.92, 0.45, cream + jam);
  // Frosting on faces turned up, dripping down the walls: the frosting's edge
  // (in how much a face turns up) dips in columns along the wall, each drip
  // with its own length and a round end.
  let along = select(hz.x, hz.y, abs(dot(n, side)) > abs(dot(n, side2)));
  let col = floor(along / 1.3);
  let u = (fract(along / 1.3) - 0.5) / (0.16 + 0.14 * cake_hash(vec3f(col, 3.0, 1.0)));
  let drip_len = select(0.0, 0.15 + 0.45 * cake_hash(vec3f(col, 7.0, 2.0)), cake_hash(vec3f(col, 1.0, 9.0)) < 0.45);
  let drip = drip_len * sqrt(max(0.0, 1.0 - u * u));
  let edge = 0.4 + (noise - 0.0) * 0.12 + (cake_noise(p * 1.3) - 0.5) * 0.12 - drip;
  let frost = smoothstep(edge - 0.035, edge + 0.035, facing);
  let gloss_tone = smoothstep(0.6, 0.95, cake_noise(p * 2.3));
  let frost_col = mix(srgb_to_linear(${v(CAKE_COLORS.frosting)}), srgb_to_linear(${v(CAKE_COLORS.frostingLight)}), gloss_tone * 0.6);
  // Sprinkles: a short capsule in a random place and direction per 0.6 u cell, in one of six colours.
  let sp = p * 1.66;
  let cell = floor(sp);
  let local = sp - cell - vec3f(cake_hash(cell), cake_hash(cell + vec3f(5.1)), cake_hash(cell + vec3f(9.7))) * 0.5 - vec3f(0.25);
  let dir = normalize(vec3f(cake_hash(cell + vec3f(1.3)), cake_hash(cell + vec3f(2.9)), cake_hash(cell + vec3f(4.4))) - 0.5);
  let t_along = clamp(dot(local, dir), -0.2, 0.2);
  let dist = length(local - dir * t_along);
  let sprinkle = (1.0 - smoothstep(0.08, 0.11, dist)) * step(0.4, cake_hash(cell + vec3f(7.7))) * frost;
  let k = u32(cake_hash(cell + vec3f(3.3)) * 5.99);
  var sc = srgb_to_linear(${v(SPRINKLES[0]!)});
  if (k == 1u) { sc = srgb_to_linear(${v(SPRINKLES[1]!)}); }
  else if (k == 2u) { sc = srgb_to_linear(${v(SPRINKLES[2]!)}); }
  else if (k == 3u) { sc = srgb_to_linear(${v(SPRINKLES[3]!)}); }
  else if (k == 4u) { sc = srgb_to_linear(${v(SPRINKLES[4]!)}); }
  else if (k == 5u) { sc = srgb_to_linear(${v(SPRINKLES[5]!)}); }
  alb = mix(alb, frost_col, frost);
  alb = mix(alb, sc, sprinkle);
  rough = mix(rough, 0.28, frost);
  out.albedo = alb;
  out.rough = rough;
  out.f0 = mix(0.03, 0.05, frost);
  // The wave front: a bright band of frosting foam with sparkles.
  let d = cake_depth(p);
  let band = (1.0 - smoothstep(0.0, ${CAKE_FRONT_U}, abs(d - ${CAKE_FRONT_U * 0.5}))) * tp.cake_up.w;
  // Round twinkles in the band (a random point per 0.25 u cell, blinking).
  let gp = p * 4.0;
  let gc = floor(gp);
  let gd = length(gp - gc - vec3f(cake_hash(gc + vec3f(1.7)), cake_hash(gc + vec3f(8.3)), cake_hash(gc + vec3f(4.1))));
  let twinkle = (1.0 - smoothstep(0.06, 0.16, gd)) * step(0.82, cake_hash(gc + vec3f(floor(frame.camera_pos.w * 8.0)))) * band;
  out.emissive = srgb_to_linear(${v(CAKE_COLORS.frostingLight)}) * (band * 0.45 + twinkle * 4.0) + sc * sprinkle * 0.12;
  return out;
}
`;
