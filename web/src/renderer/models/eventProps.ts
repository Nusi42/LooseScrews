// Props of the comedy events of M4 C1 (Event visuals I), built with the VQ4
// modelling kit in toy materials (toyPalette.ts):
// - the rubber chicken every shot turns into during Rubber chickens (beak
//   forward along -Z, legs trailing, about 1 u long at scale 1), and the
//   giant egg the Tectonic Charge becomes;
// - Moving day's cardboard box (1 u cube, body only, top at y = 0.5), its
//   two lid flaps (hinged at the origin, lying along +Z) and the tape strip
//   (along +Z, length 1: instance `stretch` lays it);
// - Big head day's balloon head (unit sphere, translucent, the side's
//   colour), its face (eyes, brows, a grin and cheeks on the -Z side) and
//   the googly pupil;
// - Gravity coffee break's coffee mug (about 1 u tall, centred, handle at +X).

import { Model, type V3 } from "./kit.ts";
import { toyPart } from "./toyPalette.ts";

const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const add = (a: V3, b: V3): V3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const mul = (a: V3, k: number): V3 => [a[0] * k, a[1] * k, a[2] * k];
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const norm = (a: V3): V3 => {
  const l = Math.hypot(a[0], a[1], a[2]) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
};

/**
 * A tube along a polyline: one ring of `n` points per path point
 * (radius per point, 0 closes it to a tip), frames carried along the path
 * so the tube does not twist.
 */
export function tube(m: Model, path: readonly V3[], radii: readonly number[], n: number, part: number, group: number, caps = true): Model {
  const rings: V3[][] = [];
  let ref: V3 = [0, 1, 0];
  for (let i = 0; i < path.length; i++) {
    const p = path[i]!;
    const prev = path[Math.max(0, i - 1)]!;
    const next = path[Math.min(path.length - 1, i + 1)]!;
    const t = norm(sub(next, prev));
    if (Math.abs(t[0] * ref[0] + t[1] * ref[1] + t[2] * ref[2]) > 0.95) ref = Math.abs(t[0]) < 0.9 ? [1, 0, 0] : [0, 0, 1];
    const u = norm(cross(t, ref));
    const w = cross(u, t);
    ref = w;
    const r = radii[i] ?? radii[radii.length - 1] ?? 0.05;
    const ring: V3[] = [];
    for (let k = 0; k < n; k++) {
      const a = (k / n) * Math.PI * 2;
      ring.push(r <= 0 ? p : add(p, add(mul(u, Math.cos(a) * r), mul(w, Math.sin(a) * r))));
    }
    rings.push(ring);
  }
  return m.loft(rings, { part, group, capStart: caps && (radii[0] ?? 1) > 0, capEnd: caps && (radii[radii.length - 1] ?? 1) > 0 });
}

/** Quadratic Bezier sampled at `n + 1` points. */
function bezier(a: V3, c: V3, b: V3, n: number): V3[] {
  const out: V3[] = [];
  for (let i = 0; i <= n; i++) {
    const t = i / n;
    const s = 1 - t;
    out.push([s * s * a[0] + 2 * s * t * c[0] + t * t * b[0], s * s * a[1] + 2 * s * t * c[1] + t * t * b[1], s * s * a[2] + 2 * s * t * c[2] + t * t * b[2]]);
  }
  return out;
}

/**
 * The rubber chicken: a plucked yellow body, a long neck stretched forward
 * into a screaming head (beak wide open, red comb and wattle, googly eyes),
 * stubby wings and long orange legs trailing behind with three toes each.
 */
export function rubberChickenModel(): Model {
  const m = new Model();
  const yellow = toyPart("rubberYellow");
  const red = toyPart("rubberRed");
  const orange = toyPart("rubberOrange");
  const black = toyPart("glossBlack");
  const white = toyPart("glossWhite");
  const smooth = m.smooth(60);
  // Body: a fat teardrop, the round end forward.
  const body: Array<[number, number]> = [];
  for (let i = 0; i <= 12; i++) {
    const a = (i / 12) * Math.PI;
    const z = 0.2 - Math.cos(a) * 0.3;
    const r = Math.sin(a) * (0.16 + 0.03 * Math.sin(a * 0.5)) * (1 - 0.35 * Math.max(0, (z - 0.25) / 0.25));
    body.push([Math.max(r, 0.0005), z]);
  }
  m.lathe(body, 14, { part: yellow, group: smooth, sx: 1, sy: 1.08, center: [0, 0, 0], capStart: true, capEnd: true });
  // Tail tuft.
  tube(m, bezier([0, 0.06, 0.42], [0, 0.12, 0.5], [0, 0.2, 0.55], 4), [0.06, 0.05, 0.035, 0.02, 0], 8, yellow, smooth);
  // Neck: from the chest forward and up into the head.
  const neck = bezier([0, 0.03, -0.02], [0, 0.04, -0.3], [0, 0.15, -0.44], 7);
  tube(m, neck, [0.085, 0.075, 0.068, 0.062, 0.058, 0.056, 0.056, 0.06], 10, yellow, smooth, false);
  // Head.
  const head: V3 = [0, 0.17, -0.49];
  m.sphere(head, [0.085, 0.095, 0.1], 2, yellow, smooth);
  // Beak, open wide: upper and lower halves.
  tube(m, [[0, 0.19, -0.55], [0, 0.2, -0.62], [0, 0.205, -0.7]], [0.04, 0.026, 0], 8, orange, m.smooth(70));
  tube(m, [[0, 0.135, -0.55], [0, 0.11, -0.61], [0, 0.085, -0.66]], [0.033, 0.022, 0], 8, orange, m.smooth(70));
  // The throat between them.
  m.sphere([0, 0.16, -0.565], [0.03, 0.03, 0.02], 1, red, smooth);
  // Comb: three red lobes over the crown; wattle under the beak.
  m.sphere([0, 0.265, -0.53], [0.022, 0.04, 0.032], 1, red, smooth);
  m.sphere([0, 0.28, -0.48], [0.024, 0.048, 0.034], 1, red, smooth);
  m.sphere([0, 0.265, -0.43], [0.022, 0.04, 0.03], 1, red, smooth);
  m.sphere([0, 0.08, -0.56], [0.022, 0.04, 0.022], 1, red, smooth);
  // Googly eyes on both sides.
  for (const s of [-1, 1]) {
    m.sphere([s * 0.07, 0.2, -0.52], 0.034, 1, white, smooth);
    m.sphere([s * 0.098, 0.205, -0.535], 0.016, 1, black, smooth);
  }
  // Stubby wings, flattened and swept back.
  for (const s of [-1, 1]) {
    tube(m, bezier([s * 0.14, 0.06, 0.05], [s * 0.22, 0.05, 0.16], [s * 0.2, 0.0, 0.3], 4), [0.05, 0.06, 0.05, 0.035, 0], 8, yellow, smooth);
  }
  // Long legs trailing back and down, three toes each.
  for (const s of [-1, 1]) {
    const hip: V3 = [s * 0.06, -0.12, 0.26];
    const knee: V3 = [s * 0.075, -0.24, 0.42];
    const ankle: V3 = [s * 0.08, -0.3, 0.62];
    tube(m, [hip, knee, ankle], [0.032, 0.022, 0.02], 7, orange, m.smooth(70), true);
    for (const toe of [-0.5, 0, 0.5]) {
      const tip: V3 = [ankle[0] + s * 0.02 + toe * 0.05, ankle[1] - 0.05, ankle[2] + 0.08];
      tube(m, [ankle, add(ankle, mul(sub(tip, ankle), 0.5)), tip], [0.016, 0.013, 0], 6, orange, m.smooth(70));
    }
  }
  return m;
}

/** The giant egg the Tectonic Charge becomes: pointed end forward (-Z), speckled. */
export function giantEggModel(): Model {
  const m = new Model();
  const egg = toyPart("egg");
  const speckle = toyPart("speckle");
  const smooth = m.smooth(70);
  const profile: Array<[number, number]> = [];
  const n = 16;
  const pts: Array<[number, number]> = [];
  for (let i = 0; i <= n; i++) {
    const a = (i / n) * Math.PI;
    // z from the pointed end (-0.6) to the round end (+0.45).
    const z = -Math.cos(a) * 0.52 - 0.04;
    const t = (z + 0.56) / 1.04;
    const r = Math.sin(a) * 0.38 * (0.78 + 0.22 * t);
    pts.push([Math.max(r, 0.0005), z]);
  }
  profile.push(...pts);
  m.lathe(profile, 18, { part: egg, group: smooth, capStart: true, capEnd: true });
  // Speckles: small flattened spots just proud of the shell.
  let seed = 7;
  const rnd = (): number => {
    seed = (seed * 16807) % 2147483647;
    return seed / 2147483647;
  };
  for (let i = 0; i < 22; i++) {
    const a = rnd() * Math.PI * 2;
    const z = -0.42 + rnd() * 0.8;
    const t = (z + 0.56) / 1.04;
    const s = Math.sqrt(Math.max(0, 1 - ((z + 0.04) / 0.52) ** 2));
    const r = s * 0.38 * (0.78 + 0.22 * t) - 0.004;
    const c: V3 = [Math.cos(a) * r, Math.sin(a) * r, z];
    const k = 0.012 + rnd() * 0.02;
    m.sphere(c, [k, k, k], 0, speckle, smooth);
  }
  return m;
}

/** Moving day's box body: a 1 u cardboard cube (top at y = +0.5) with "this way up" arrows and a red stamp. */
export function cardboardBoxModel(): Model {
  const m = new Model();
  const board = toyPart("cardboard");
  const print = toyPart("cardboardPrint");
  const stamp = toyPart("stampRed");
  const tape = toyPart("tape");
  m.box([0, 0, 0], [0.5, 0.5, 0.5], { part: board, bevel: 0.025 });
  // Up arrows on two opposite sides (+X, -X): a stem and a head, proud of the face.
  for (const s of [-1, 1]) {
    for (const off of [-0.13, 0.13]) {
      const x = s * 0.505;
      m.box([x, 0.02, off], [0.006, 0.12, 0.022], { part: print });
      // Arrow head: a flat triangle prism.
      const top = 0.2;
      const a: V3 = [x + s * 0.006, top, off];
      const b: V3 = [x + s * 0.006, top - 0.09, off - 0.07];
      const c: V3 = [x + s * 0.006, top - 0.09, off + 0.07];
      const a2: V3 = [x - s * 0.004, top, off];
      const b2: V3 = [x - s * 0.004, top - 0.09, off - 0.07];
      const c2: V3 = [x - s * 0.004, top - 0.09, off + 0.07];
      m.triangle(a, b, c, print).triangle(a2, c2, b2, print);
      m.quad(b, b2, c2, c, print).quad(a, a2, b2, b, print).quad(c, c2, a2, a, print);
    }
    // Two printed lines under the arrows ("THIS SIDE UP").
    m.box([s * 0.505, -0.2, 0], [0.006, 0.018, 0.24], { part: print });
    m.box([s * 0.505, -0.26, 0], [0.006, 0.014, 0.17], { part: print });
  }
  // A red stamp frame on the front (-Z) and the tape running down both ends.
  const z = -0.506;
  m.box([0, 0.24, z], [0.3, 0.018, 0.006], { part: stamp });
  m.box([0, -0.02, z], [0.3, 0.018, 0.006], { part: stamp });
  m.box([-0.285, 0.11, z], [0.018, 0.13, 0.006], { part: stamp });
  m.box([0.285, 0.11, z], [0.018, 0.13, 0.006], { part: stamp });
  m.box([0, 0.11, z], [0.2, 0.05, 0.006], { part: stamp });
  for (const s of [-1, 1]) m.box([0, 0.36, s * 0.506], [0.08, 0.14, 0.005], { part: tape });
  return m;
}

/** One lid flap: hinged at the origin along X, lying along +Z (0.5 deep, 1 wide). */
export function boxFlapModel(): Model {
  const m = new Model();
  m.box([0, 0.012, 0.25], [0.5, 0.012, 0.25], { part: toyPart("cardboard"), bevel: 0.006 });
  return m;
}

/** The tape strip: 0.16 wide, along +Z from 0 to 1 (instance `stretch` lays it). */
export function tapeStripModel(): Model {
  const m = new Model();
  m.box([0, 0.004, 0.5], [0.08, 0.004, 0.5], { part: toyPart("tape") });
  return m;
}

/** Balloon head: a unit sphere in the instance colour (drawn translucent). */
export function balloonModel(): Model {
  const m = new Model();
  m.sphere([0, 0, 0], [1, 1.06, 1], 2, toyPart("instance"), m.smooth(80));
  // The knot at the bottom.
  tube(m, [[0, -1.03, 0], [0, -1.1, 0], [0, -1.16, 0]], [0.07, 0.1, 0.05], 10, toyPart("instance"), m.smooth(80));
  return m;
}

/** On a unit sphere: the point at yaw `a` (from -Z toward +X) and pitch `b`, pushed out by `out`. */
function onSphere(a: number, b: number, out = 1): V3 {
  return [Math.sin(a) * Math.cos(b) * out, Math.sin(b) * out, -Math.cos(a) * Math.cos(b) * out];
}

/** The balloon's face (opaque, on the -Z side of the unit sphere): eye whites, brows, a wide grin with a tongue, cheeks. */
export function balloonFaceModel(): Model {
  const m = new Model();
  const white = toyPart("glossWhite");
  const black = toyPart("glossBlack");
  const pink = toyPart("pink");
  const red = toyPart("rubberRed");
  const smooth = m.smooth(70);
  for (const s of [-1, 1]) {
    const c = onSphere(s * 0.34, 0.24, 0.96);
    m.sphere(c, [0.2, 0.25, 0.12], 2, white, smooth);
    // Brows: arched tubes above the eyes, raised in surprise.
    const pts: V3[] = [];
    for (let i = 0; i <= 6; i++) {
      const t = i / 6;
      pts.push(onSphere(s * (0.16 + 0.34 * t), 0.56 + 0.06 * Math.sin(t * Math.PI), 1.02));
    }
    tube(m, pts, [0.03, 0.04, 0.045, 0.045, 0.04, 0.035, 0.025], 6, black, smooth);
    // Cheeks.
    m.sphere(onSphere(s * 0.58, -0.12, 0.985), [0.13, 0.08, 0.04], 1, pink, smooth);
  }
  // The grin: a thick dark arc from cheek to cheek.
  const grin: V3[] = [];
  for (let i = 0; i <= 14; i++) {
    const t = i / 14;
    const a = -0.5 + t;
    grin.push(onSphere(a, -0.2 - 0.17 * Math.sin(t * Math.PI), 1.0));
  }
  tube(m, grin, grin.map((_, i) => 0.03 + 0.05 * Math.sin((i / 14) * Math.PI)), 8, black, smooth);
  // The tongue in the middle of the grin.
  m.sphere(onSphere(0.05, -0.36, 1.0), [0.11, 0.05, 0.05], 1, red, smooth);
  return m;
}

/** A googly pupil (unit: the eye white is 0.2 across; the pupil 0.09). */
export function pupilModel(): Model {
  const m = new Model();
  m.sphere([0, 0, 0], [0.09, 0.09, 0.05], 1, toyPart("glossBlack"), m.smooth(80));
  return m;
}

/** The coffee mug (Gravity coffee break): white ceramic, a blue band, handle at +X, full of coffee. */
export function coffeeMugModel(): Model {
  const m = new Model();
  const white = toyPart("glossWhite");
  const blue = toyPart("mugBlue");
  const coffee = toyPart("coffee");
  const smooth = m.smooth(50);
  const ro = 0.36, ri = 0.31, h = 0.5;
  // One closed profile: the base, the outer wall (bottom up), the rim, the
  // inner wall down to the coffee and the coffee's surface to the axis.
  const profile: Array<[number, number]> = [
    [0.0005, -h],
    [ro - 0.04, -h],
    [ro, -h + 0.04],
    [ro, -0.12],
    [ro + 0.003, -0.1],
    [ro + 0.003, 0.08],
    [ro, 0.1],
    [ro, h - 0.02],
    [(ro + ri) / 2, h],
    [ri, h - 0.02],
    [ri, h - 0.12],
    [0.0005, h - 0.12],
  ];
  // Segment 4 is the blue band, segment 10 the coffee.
  m.lathe(profile, 24, { part: (seg) => (seg === 4 ? blue : seg === 10 ? coffee : white), group: smooth, axis: "y" });
  // Handle: a C from near the top to near the bottom on +X.
  const handle: V3[] = [];
  for (let i = 0; i <= 10; i++) {
    const a = -Math.PI / 2 + (i / 10) * Math.PI;
    handle.push([ro - 0.02 + Math.cos(a) * 0.2, Math.sin(-a) * 0.25 + 0.02, 0]);
  }
  tube(m, handle, handle.map(() => 0.05), 10, white, smooth);
  // A red heart on the side facing away from the handle.
  const heart = toyPart("rubberRed");
  m.sphere([-ro + 0.004, 0.29, -0.058], [0.009, 0.068, 0.068], 1, heart, smooth);
  m.sphere([-ro + 0.004, 0.29, 0.058], [0.009, 0.068, 0.068], 1, heart, smooth);
  m.sphere([-ro + 0.004, 0.235, 0], [0.009, 0.075, 0.085], 1, heart, smooth);
  m.sphere([-ro + 0.004, 0.18, 0], [0.009, 0.04, 0.035], 1, heart, smooth);
  return m;
}
