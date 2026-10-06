// Materials of M4 C3's spectacle props (rubber ducks, hats, the cat paw, the
// disco ball, light trails; models/spectacleProps.ts). WGSL included by
// models/shading.ts into MODEL_SHADING_WGSL: `shade_model` hands every part
// in SPECTACLE_PART_MIN..SPECTACLE_PART_MAX to `shade_spectacle`.
//
// Colour literals are display (sRGB) values, linearised at build time like
// the VQ4 materials. Parts marked "instance colour" take the instance's
// colour (a duck's yellow, a hat's felt, the cat's fur).

import { srgbToLinear } from "../shaders/frame.ts";

/** Glossy rubber in the instance colour (duck bodies, rubber trim). */
export const PART_RUBBER = 40;
/** Glossy orange rubber (duck beaks, propeller blades). */
export const PART_ORANGE = 41;
/** Glossy black (eyes, hat bands, sunglasses, a top hat's silk). */
export const PART_INK = 42;
/** Matte white cloth (chef's toque, party stripes, pom-poms). */
export const PART_WHITE = 43;
/** Fur in the instance colour with tabby stripes (the cat paw and its arm). */
export const PART_FUR = 44;
/** Pink toe beans (the paw's pads). */
export const PART_BEAN = 45;
/** Ivory (claws, horns). */
export const PART_IVORY = 46;
/** Mirror tiles (the disco ball). */
export const PART_MIRROR = 47;
/** Polished gold (crown, pom-pom, rivets). */
export const PART_GOLD = 48;
/** Felt in the instance colour (hats). */
export const PART_FELT = 49;
/** Self-lit white (eye catchlights). */
export const PART_SHINE = 50;
/** Ruby gems, softly glowing. */
export const PART_GEM = 51;

export const SPECTACLE_PART_MIN = PART_RUBBER;
export const SPECTACLE_PART_MAX = PART_GEM;

const lin = (r: number, g: number, b: number): string =>
  `vec3f(${[r, g, b].map((c) => srgbToLinear(c).toFixed(5)).join(", ")})`;

/**
 * `shade_spectacle(part, n, world_pos, lp, ln, color)`: needs `light_spec`,
 * `model_spec`, `model_noise`, `model_hash`, the frame's environment cube
 * (`env_cube`, `env_samp`, `sky_dir`) and `emissive_glow`.
 */
export const SPECTACLE_SHADING_WGSL = /* wgsl */ `
const SPECTACLE_PART_MIN = ${SPECTACLE_PART_MIN}u;
const SPECTACLE_PART_MAX = ${SPECTACLE_PART_MAX}u;

// Environment reflection (the world's sky cube, dimmed by the level's light).
fn spectacle_env(r: vec3f, rough_mip: f32) -> vec3f {
  return textureSampleLevel(env_cube, env_samp, sky_dir(r), rough_mip).rgb * frame.rock_color.w;
}

// A glossy dielectric: diffuse plus a sharp highlight, a broad sheen and a fresnel rim.
fn spectacle_gloss(base: vec3f, n: vec3f, world_pos: vec3f, v: vec3f, power: f32, shine: f32) -> vec3f {
  var dspec = vec3f(0.0);
  let lit = light_spec(n, world_pos, v, power, &dspec);
  let fres = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 5.0);
  let refl = spectacle_env(reflect(-v, n), 4.0);
  // Soft wrap: rubber and plastic let a little light around the terminator.
  let wrap = base * 0.06 * frame.rock_color.w;
  return base * lit + wrap + vec3f(model_spec(n, v, power) * shine) + dspec * shine * 1.2
    + refl * (0.04 + 0.5 * fres) * shine + vec3f(0.3, 0.32, 0.36) * fres * 0.25 * base;
}

fn shade_spectacle(part: u32, n: vec3f, world_pos: vec3f, lp: vec3f, ln: vec3f, color: vec4f) -> vec3f {
  let v = normalize(frame.camera_pos.xyz - world_pos);
  var out = vec3f(0.0);
  if (part == ${PART_RUBBER}u) {
    out = spectacle_gloss(color.rgb, n, world_pos, v, 70.0, 0.9);
  } else if (part == ${PART_ORANGE}u) {
    out = spectacle_gloss(${lin(1.0, 0.42, 0.04)}, n, world_pos, v, 60.0, 0.85);
  } else if (part == ${PART_INK}u) {
    out = spectacle_gloss(${lin(0.025, 0.025, 0.03)}, n, world_pos, v, 110.0, 1.3);
  } else if (part == ${PART_WHITE}u) {
    // Cloth: soft weave, a faint sheen.
    var dspec = vec3f(0.0);
    let lit = light_spec(n, world_pos, v, 12.0, &dspec);
    let weave = 0.92 + 0.08 * model_noise(lp * 90.0);
    let base = ${lin(0.93, 0.92, 0.88)} * weave;
    out = base * lit + dspec * 0.15 + base * 0.05 * frame.rock_color.w;
  } else if (part == ${PART_FUR}u) {
    // Fur: strands along the limb (local Z), tabby stripes, a soft sheen at grazing angles.
    var dspec = vec3f(0.0);
    let lit = light_spec(n, world_pos, v, 8.0, &dspec);
    let strands = model_noise(vec3f(lp.x * 70.0, lp.y * 70.0, lp.z * 9.0));
    let tufts = model_noise(lp * 6.0);
    let stripe = smoothstep(0.35, 0.65, sin(lp.z * 13.0 + tufts * 3.5 + lp.y * 2.0) * 0.5 + 0.5);
    let base = color.rgb * mix(0.62, 1.0, stripe) * (0.72 + 0.4 * strands);
    let fres = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 2.0);
    // Light catching the fur's edge (a cheap velvet term).
    let sheen = color.rgb * fres * 0.55 * (0.35 + 0.65 * lit);
    out = base * lit + sheen + dspec * 0.12;
  } else if (part == ${PART_BEAN}u) {
    out = spectacle_gloss(${lin(1.0, 0.56, 0.64)}, n, world_pos, v, 35.0, 0.55);
  } else if (part == ${PART_IVORY}u) {
    out = spectacle_gloss(${lin(0.96, 0.93, 0.84)}, n, world_pos, v, 90.0, 1.0);
  } else if (part == ${PART_MIRROR}u) {
    // Mirror tiles: a lat-long grid; each tile tilted a little at random, so
    // tiles flash one by one as the ball turns. Grout between tiles.
    let sp = normalize(lp);
    let lat = asin(clamp(sp.y, -1.0, 1.0));
    let lon = atan2(sp.z, sp.x);
    // The tile grid of the model (spectacleProps.ts DISCO_ROWS, discoColumns): rows by latitude, columns per row's centre.
    let rows = 16.0;
    let row = floor((lat / 3.14159265 + 0.5) * rows);
    let lat_c = ((row + 0.5) / rows - 0.5) * 3.14159265;
    let cols = max(4.0, floor(32.0 * cos(lat_c) + 0.5));
    let cell = vec2f(row, floor((lon / 6.2831853 + 0.5) * cols));
    let jitter = (vec3f(model_hash(vec3f(cell, 1.0)), model_hash(vec3f(cell, 2.0)), model_hash(vec3f(cell, 3.0))) - 0.5) * 0.3;
    let tn = normalize(n + jitter);
    let r = reflect(-v, tn);
    var dspec = vec3f(0.0);
    let lit = light_spec(tn, world_pos, v, 160.0, &dspec);
    let tile_tone = 0.75 + 0.35 * model_hash(vec3f(cell, 4.0));
    let refl = spectacle_env(r, 1.0) * 0.9 + vec3f(0.03) * frame.rock_color.w;
    // Glitter: tiles catch a spot for a moment as the ball turns (random per tile and time step), in the spots' colours.
    let t = frame.camera_pos.w;
    let step_t = floor(t * 7.0 + model_hash(vec3f(cell, 5.0)) * 13.0);
    let flash = pow(model_hash(vec3f(cell, step_t)), 18.0);
    let hue = model_hash(vec3f(cell, step_t + 0.5)) * 6.2831853;
    let flash_col = mix(vec3f(1.0), 0.5 + 0.5 * cos(vec3f(hue, hue + 2.1, hue + 4.2)), 0.6);
    out = refl * tile_tone + vec3f(model_spec(tn, v, 600.0) * 5.0) + dspec * 4.0 + lit * 0.06 + flash_col * flash * 9.0;
  } else if (part == ${PART_GOLD}u) {
    var dspec = vec3f(0.0);
    let lit = light_spec(n, world_pos, v, 80.0, &dspec);
    let base = ${lin(1.0, 0.76, 0.3)};
    let refl = spectacle_env(reflect(-v, n), 2.0);
    out = base * (lit * 0.45 + refl * 0.8 + vec3f(model_spec(n, v, 80.0) * 1.6) + dspec * 1.6) + base * 0.04 * frame.rock_color.w;
  } else if (part == ${PART_FELT}u) {
    var dspec = vec3f(0.0);
    let lit = light_spec(n, world_pos, v, 10.0, &dspec);
    let fuzz = 0.88 + 0.14 * model_noise(lp * 60.0);
    let fres = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 3.0);
    let base = color.rgb * fuzz;
    out = base * lit + base * fres * 0.35 * (0.3 + 0.7 * lit) + dspec * 0.08 + base * 0.04 * frame.rock_color.w;
  } else if (part == ${PART_SHINE}u) {
    out = vec3f(2.2, 2.2, 2.1);
  } else {
    // PART_GEM: deep red with a glow from inside and a glint.
    var dspec = vec3f(0.0);
    let lit = light_spec(n, world_pos, v, 140.0, &dspec);
    out = ${lin(0.85, 0.05, 0.12)} * (lit * 0.6 + 0.9) + vec3f(model_spec(n, v, 140.0) * 2.0) + dspec * 2.0;
  }
  // Instance emissive (ghost copies of the paw, fading props) washes toward the colour.
  return mix(out, emissive_glow(color.rgb, color.a), color.a);
}
`;
