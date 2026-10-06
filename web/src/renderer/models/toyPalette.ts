// Toy materials (M4 C1): the comedy events' props are toys (rubber chickens,
// balloon heads, cardboard boxes, a coffee mug), many colours on one mesh.
// A model vertex whose part is TOY_PART_BASE + i takes palette entry i:
// a display colour and a gloss (0 matte cardboard .. 1 polished plastic).
// Entry 0 takes the instance colour (balloons in a side's colour). The
// object shaders shade these parts with `shade_toy` (TOY_WGSL, included by
// models/shading.ts): soft rubber and plastic highlights, a sheen at the
// rim, cardboard fibres, and a small self-light floor so toys read as toys
// in a dark cave. Other event packages (C2, C3) may append entries.

import { srgbToLinear } from "../shaders/frame.ts";

/** First toy part (kit.ts parts end at PART_LAMP 11; PART_PLAIN is 255). */
export const TOY_PART_BASE = 32;

export interface ToyMaterial {
  name: string;
  /** Display (sRGB) colour; ignored for `instance`. */
  color: readonly [number, number, number];
  /** 0 matte .. 1 polished. */
  gloss: number;
}

/** The palette (index = part - TOY_PART_BASE). Append only. */
export const TOY_MATERIALS: readonly ToyMaterial[] = [
  { name: "instance", color: [1, 1, 1], gloss: 0.75 },
  { name: "rubberYellow", color: [1.0, 0.8, 0.1], gloss: 0.5 },
  { name: "rubberRed", color: [0.93, 0.13, 0.1], gloss: 0.55 },
  { name: "rubberOrange", color: [1.0, 0.5, 0.06], gloss: 0.5 },
  { name: "glossBlack", color: [0.025, 0.025, 0.03], gloss: 0.95 },
  { name: "glossWhite", color: [0.96, 0.95, 0.92], gloss: 0.85 },
  { name: "cardboard", color: [0.68, 0.48, 0.27], gloss: 0.04 },
  { name: "cardboardPrint", color: [0.22, 0.13, 0.07], gloss: 0.04 },
  { name: "tape", color: [0.86, 0.68, 0.38], gloss: 0.8 },
  { name: "coffee", color: [0.2, 0.09, 0.03], gloss: 0.95 },
  { name: "egg", color: [0.98, 0.91, 0.77], gloss: 0.35 },
  { name: "pink", color: [1.0, 0.5, 0.6], gloss: 0.45 },
  { name: "stampRed", color: [0.78, 0.1, 0.08], gloss: 0.08 },
  { name: "mugBlue", color: [0.12, 0.5, 0.95], gloss: 0.85 },
  { name: "speckle", color: [0.55, 0.36, 0.2], gloss: 0.3 },
  { name: "feather", color: [0.95, 0.93, 0.87], gloss: 0.15 },
];

/** Part of palette entry `name`. */
export function toyPart(name: string): number {
  const i = TOY_MATERIALS.findIndex((m) => m.name === name);
  if (i < 0) throw new Error(`unknown toy material "${name}"`);
  return TOY_PART_BASE + i;
}

/** Palette entry of a part, or null for other parts. */
export function toyMaterialOf(part: number): ToyMaterial | null {
  return part >= TOY_PART_BASE ? (TOY_MATERIALS[part - TOY_PART_BASE] ?? null) : null;
}

const lin = (c: readonly [number, number, number]): string => `vec3f(${c.map((x) => srgbToLinear(x).toFixed(5)).join(", ")})`;

/**
 * WGSL: `toy_material(i)` (linear colour, gloss) and `shade_toy`. Needs
 * `light_spec`, `model_spec`, `model_noise` and `frame` (models/shading.ts).
 */
export const TOY_WGSL = /* wgsl */ `
const TOY_PART_BASE = ${TOY_PART_BASE}u;
const TOY_COUNT = ${TOY_MATERIALS.length}u;

fn toy_material(i: u32, instance: vec3f) -> vec4f {
  switch i {
${TOY_MATERIALS.map((m, i) => (i === 0 ? `    case 0u: { return vec4f(instance, ${m.gloss.toFixed(3)}); }` : `    case ${i}u: { return vec4f(${lin(m.color)}, ${m.gloss.toFixed(3)}); }`)).join("\n")}
    default: { return vec4f(instance, 0.5); }
  }
}

// A toy surface: lit diffuse with a little wrap (rubber and plastic scatter),
// a gloss-dependent highlight from the sun, headlight and dynamic lights, a
// soft rim sheen and a self-light floor. Cardboard (gloss < 0.1) gets fibres.
fn shade_toy(i: u32, n: vec3f, world_pos: vec3f, lp: vec3f, color: vec4f) -> vec3f {
  let m = toy_material(i, color.rgb);
  var base = m.rgb;
  let gloss = m.a;
  let v = normalize(frame.camera_pos.xyz - world_pos);
  if (gloss < 0.1) {
    let fib = model_noise(lp * vec3f(60.0, 9.0, 60.0)) * 0.6 + model_noise(lp * 23.0) * 0.4;
    base *= 0.86 + 0.28 * fib;
  }
  let power = 6.0 + gloss * gloss * 260.0;
  var dspec = vec3f(0.0);
  let lit = light_spec(n, world_pos, v, power, &dspec);
  let fres = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 3.0);
  let spec = model_spec(n, v, power) * gloss * (0.25 + 1.6 * gloss) + dspec * gloss * 1.2;
  let sheen = fres * (0.1 + 0.25 * gloss) * (base * 0.6 + vec3f(0.12, 0.14, 0.18));
  return base * (lit + vec3f(0.05)) + vec3f(spec) * vec3f(1.0, 0.98, 0.95) + sheen;
}
`;
