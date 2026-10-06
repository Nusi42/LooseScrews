// WGSL for the instanced objects' materials (VQ4). Included by the
// instanced and translucent object shaders after the Frame struct and the
// shared `light()` / `apply_fog()` helpers. `shade_model` picks the
// material by the vertex's part (kit.ts PART_*; PART_PLAIN and unknown parts
// keep the pre-VQ4 look: instance colour, lit, mixed toward glowing by the
// instance's emissive amount).
//
// Colour literals are display (sRGB) values, linearised here (VQ2 colour
// convention, shaders/frame.ts). Emissive strengths are constants below,
// in the HDR scene's units (above 1 blooms; VQ6 retuned them for the HDR
// pipeline: idle engines and steady nav lights bloom softly, boost and the
// nav flash bloom hard).

import {
  PART_CANOPY,
  PART_ENAMEL,
  PART_ENGINE,
  PART_FLESH,
  PART_GLOW,
  PART_HULL,
  PART_LAMP,
  PART_LIVERY,
  PART_METAL,
  PART_NAV_PORT,
  PART_NAV_STARBOARD,
  PART_NAV_TAIL,
  PART_SCORCH,
  PART_STONE,
  PART_TRIM,
} from "./kit.ts";
import { srgbToLinear } from "../shaders/frame.ts";
import { TOY_WGSL } from "./toyPalette.ts";

/** Engine glow at rest and at full boost (multiplies the engine colour). */
export const ENGINE_IDLE = 2.6;
export const ENGINE_BOOST = 7.5;
/** Nav light: steady level and flash peak. */
export const NAV_STEADY = 1.4;
export const NAV_FLASH = 7.0;
/** Nav light blink period (s) and flash length (s). */
export const NAV_PERIOD = 1.2;
export const NAV_FLASH_LEN = 0.14;

/** Nav light brightness (NAV_STEADY..NAV_STEADY+NAV_FLASH) at time `t` (s) with per-ship `phase` (s); `strobe` double-flashes. */
export function navBlink(t: number, phase: number, strobe = false): number {
  const local = (((t + phase) % NAV_PERIOD) + NAV_PERIOD) % NAV_PERIOD;
  const on = local < NAV_FLASH_LEN || (strobe && local > 2 * NAV_FLASH_LEN && local < 3 * NAV_FLASH_LEN);
  return NAV_STEADY + (on ? NAV_FLASH : 0);
}

const f = (x: number): string => (Number.isInteger(x) ? `${x}.0` : String(x));
/** WGSL literal of a display (sRGB) colour, linearised. */
const lin = (r: number, g: number, b: number): string =>
  `vec3f(${[r, g, b].map((c) => srgbToLinear(c).toFixed(5)).join(", ")})`;

export const MODEL_SHADING_WGSL = /* wgsl */ `
const PART_HULL = ${PART_HULL}u;
const PART_LIVERY = ${PART_LIVERY}u;
const PART_TRIM = ${PART_TRIM}u;
const PART_METAL = ${PART_METAL}u;
const PART_CANOPY = ${PART_CANOPY}u;
const PART_ENGINE = ${PART_ENGINE}u;
const PART_NAV_PORT = ${PART_NAV_PORT}u;
const PART_NAV_STARBOARD = ${PART_NAV_STARBOARD}u;
const PART_NAV_TAIL = ${PART_NAV_TAIL}u;
const PART_GLOW = ${PART_GLOW}u;
const PART_SCORCH = ${PART_SCORCH}u;
const PART_LAMP = ${PART_LAMP}u;
const PART_STONE = ${PART_STONE}u;
const PART_ENAMEL = ${PART_ENAMEL}u;
const PART_FLESH = ${PART_FLESH}u;

// Small-argument hash (model-space coordinates stay below a few units).
fn model_hash(p: vec3f) -> f32 {
  let q = fract(p * vec3f(0.1031, 0.1030, 0.0973));
  let r = q + dot(q, q.yzx + 33.33);
  return fract((r.x + r.y) * r.z);
}

// Panel seams: a staggered grid of plates projected along the dominant
// model-space axis. Returns 0 on a seam, 1 on a plate.
fn panel_mask(lp: vec3f, ln: vec3f) -> f32 {
  let a = abs(ln);
  var uv = lp.xz;
  if (a.x >= a.y && a.x >= a.z) { uv = lp.zy; } else if (a.z >= a.y) { uv = lp.xy; }
  let cell = vec2f(0.21, 0.12);
  let row = floor(uv.y / cell.y);
  let u = uv.x / cell.x + 0.5 * (row - 2.0 * floor(row * 0.5));
  let g = vec2f(fract(u), fract(uv.y / cell.y));
  let edge = min(min(g.x, 1.0 - g.x) * cell.x, min(g.y, 1.0 - g.y) * cell.y);
  return smoothstep(0.0008, 0.0026, edge);
}

// Plate-to-plate shade variation (weathering).
fn panel_tone(lp: vec3f) -> f32 {
  return 0.93 + 0.12 * model_hash(floor(lp * vec3f(4.8, 8.3, 4.8)));
}

// Smooth 3D value noise in 0..1 (model space).
fn model_noise(p: vec3f) -> f32 {
  let i = floor(p);
  let f = p - i;
  let u = f * f * (3.0 - 2.0 * f);
  let a = mix(mix(model_hash(i), model_hash(i + vec3f(1.0, 0.0, 0.0)), u.x), mix(model_hash(i + vec3f(0.0, 1.0, 0.0)), model_hash(i + vec3f(1.0, 1.0, 0.0)), u.x), u.y);
  let b = mix(mix(model_hash(i + vec3f(0.0, 0.0, 1.0)), model_hash(i + vec3f(1.0, 0.0, 1.0)), u.x), mix(model_hash(i + vec3f(0.0, 1.0, 1.0)), model_hash(i + vec3f(1.0, 1.0, 1.0)), u.x), u.y);
  return mix(a, b, u.z);
}

// Blinn-Phong specular from the sun and the camera headlight.
fn model_spec(n: vec3f, v: vec3f, power: f32) -> f32 {
  let h = normalize(frame.light_dir.xyz + v);
  // sun_vis_last: the sun's visibility from light_spec() just before (VQ9, lighting.ts).
  let sun = pow(max(dot(n, h), 0.0), power) * step(0.0, dot(n, frame.light_dir.xyz)) * sun_vis_last;
  let head = pow(max(dot(n, v), 0.0), power * 2.0) * frame.light_dir.w * 0.6;
  return sun + head;
}

fn nav_blink(t: f32, phase: f32, strobe: bool) -> f32 {
  let local = fract((t + phase) / ${f(NAV_PERIOD)}) * ${f(NAV_PERIOD)};
  var on = local < ${f(NAV_FLASH_LEN)};
  if (strobe) { on = on || (local > ${f(2 * NAV_FLASH_LEN)} && local < ${f(3 * NAV_FLASH_LEN)}); }
  return ${f(NAV_STEADY)} + select(0.0, ${f(NAV_FLASH)}, on);
}

${TOY_WGSL}

// Material of a model vertex. color: instance rgb + emissive amount (a);
// glow: engine strength (0 idle .. 1 full boost); blink: nav light phase (s).
fn shade_model(part: u32, n: vec3f, world_pos: vec3f, lp: vec3f, ln: vec3f, color: vec4f, glow: f32, blink: f32) -> vec3f {
  let v = normalize(frame.camera_pos.xyz - world_pos);
  let t = frame.camera_pos.w;
  // Toy parts (M4 C1, toyPalette.ts): the comedy events' props.
  if (part >= TOY_PART_BASE && part < TOY_PART_BASE + TOY_COUNT) {
    return mix(shade_toy(part - TOY_PART_BASE, n, world_pos, lp, color), emissive_glow(color.rgb, color.a), color.a);
  }
  // PART_PLAIN and unknown parts: the pre-VQ4 instance look (lit once, below).
  if (part > PART_LAMP && (part < PART_STONE || part > PART_FLESH)) { return shade(color.rgb, color.a, n, world_pos); }
  // Dynamic lights (VQ6) also leave a highlight; each material scales it.
  var dspec = vec3f(0.0);
  var lit = vec3f(0.0);
  // Self-lit parts (engines, nav lights, lamps) skip the light loop.
  if (part < PART_ENGINE || part == PART_SCORCH || part >= PART_STONE) {
    lit = light_spec(n, world_pos, v, 40.0, &dspec);
  }
  let fres = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 4.0);
  // Cool rim light lifts silhouettes off dark rock.
  let rim = ${lin(0.35, 0.45, 0.6)} * fres * 0.5;
  var out = vec3f(0.0);
  if (part == PART_HULL) {
    let base = mix(${lin(0.47, 0.49, 0.53)}, color.rgb, 0.2) * panel_tone(lp) * mix(0.62, 1.0, panel_mask(lp, ln));
    out = base * lit + vec3f(model_spec(n, v, 40.0) * 0.35) + dspec * 0.5 + rim * base;
  } else if (part == PART_LIVERY) {
    let base = color.rgb * mix(0.7, 1.0, panel_mask(lp, ln));
    out = base * lit + vec3f(model_spec(n, v, 48.0) * 0.4) + dspec * 0.5 + base * 0.08 + rim * base;
  } else if (part == PART_TRIM) {
    let base = ${lin(0.085, 0.09, 0.105)};
    out = base * lit + vec3f(model_spec(n, v, 24.0) * 0.2) + dspec * 0.25 + rim * 0.3;
  } else if (part == PART_METAL) {
    let base = ${lin(0.6, 0.62, 0.66)};
    out = base * lit * 0.8 + vec3f(model_spec(n, v, 70.0) * 0.9) + dspec * 1.1 + rim * base;
  } else if (part == PART_CANOPY) {
    // Dark tinted glass: strong highlight, sky reflection at grazing angles, a faint lit cockpit.
    let refl = mix(frame.fog_color.rgb * 2.0 + ${lin(0.05, 0.08, 0.12)}, ${lin(0.5, 0.6, 0.7)}, fres);
    out = ${lin(0.02, 0.035, 0.05)} * lit + refl * (0.15 + 0.6 * fres) + vec3f(model_spec(n, v, 120.0) * 1.4) + dspec * 1.2 + ${lin(0.04, 0.12, 0.16)};
  } else if (part == PART_ENGINE) {
    // Hot white centre (facing the viewer) fading to a coloured rim: orange idle, blue-white boost.
    let g = clamp(glow, 0.0, 1.0);
    let rim_col = mix(${lin(1.0, 0.38, 0.1)}, ${lin(0.3, 0.6, 1.0)}, g);
    let core = pow(clamp(dot(n, v), 0.0, 1.0), 2.0);
    let hot = mix(rim_col, mix(${lin(1.0, 0.8, 0.5)}, ${lin(0.85, 0.95, 1.0)}, g), core);
    let flicker = 0.92 + 0.08 * sin(t * 37.0 + blink * 11.0);
    out = hot * mix(${f(ENGINE_IDLE)}, ${f(ENGINE_BOOST)}, g) * (0.55 + 0.45 * core) * flicker;
  } else if (part == PART_NAV_PORT) {
    out = ${lin(1.0, 0.12, 0.08)} * nav_blink(t, blink, false);
  } else if (part == PART_NAV_STARBOARD) {
    out = ${lin(0.12, 1.0, 0.3)} * nav_blink(t, blink, false);
  } else if (part == PART_NAV_TAIL) {
    out = ${lin(1.0, 1.0, 0.95)} * nav_blink(t, blink + 0.6, true);
  } else if (part == PART_LAMP) {
    out = ${lin(1.0, 0.95, 0.8)} * 4.5;
  } else if (part == PART_GLOW) {
    out = color.rgb * (1.1 + 0.6 * color.a) + vec3f(0.15);
  } else if (part == PART_SCORCH) {
    // Burnt paint: soot patches (smooth noise, no per-triangle switches) with glowing embers at their hearts.
    let paint = mix(${lin(0.3, 0.31, 0.33)}, color.rgb, 0.25) * mix(0.6, 1.0, panel_mask(lp, ln));
    let burn = smoothstep(0.42, 0.68, model_noise(lp * 7.0) * 0.7 + model_noise(lp * 19.0) * 0.3);
    let soot = ${lin(0.035, 0.03, 0.028)};
    let base = mix(paint, soot, burn);
    let heat = smoothstep(0.74, 0.86, model_noise(lp * 31.0 + vec3f(7.0))) * burn * burn;
    let ember = heat * (0.65 + 0.35 * sin(t * 2.5 + model_noise(lp * 5.0) * 12.0));
    out = base * lit + vec3f(model_spec(n, v, 20.0) * 0.12 * (1.0 - burn)) + ${lin(1.0, 0.32, 0.06)} * ember * 0.8;
  } else if (part == PART_STONE) {
    // Carved stone (M4 C2): the instance colour, weathered by two octaves of grain and darker in the creases.
    let grain = model_noise(lp * 7.0) * 0.55 + model_noise(lp * 23.0) * 0.3 + model_noise(lp * 61.0) * 0.15;
    let pits = smoothstep(0.74, 0.88, model_noise(lp * 19.0 + vec3f(3.0)));
    let base = color.rgb * (0.62 + 0.6 * grain) * (1.0 - 0.22 * pits);
    out = base * lit + vec3f(model_spec(n, v, 12.0) * 0.05) + dspec * 0.15 + rim * base * 0.6;
  } else if (part == PART_ENAMEL) {
    // Glossy white (teeth, eyeballs): a bright base, a sharp highlight and a faint warm bounce.
    let base = ${lin(0.93, 0.92, 0.88)};
    out = base * lit + vec3f(model_spec(n, v, 90.0) * 1.1) + dspec * 1.0 + base * 0.06 + rim * 0.5;
  } else if (part == PART_FLESH) {
    // Wet pink (lips, tongues): soft wrapped light, a glint and a little glow from inside.
    let base = ${lin(0.86, 0.36, 0.44)};
    out = base * (lit * 0.85 + 0.12) + vec3f(model_spec(n, v, 60.0) * 0.8) + dspec * 0.7 + rim * base;
  } else {
    // PART_PLAIN: the pre-VQ4 instance look.
    return shade(color.rgb, color.a, n, world_pos);
  }
  // Instance emissive (cloak shimmer, ghosts, returning decoys) washes the material toward its colour.
  return mix(out, emissive_glow(color.rgb, color.a), color.a);
}
`;
