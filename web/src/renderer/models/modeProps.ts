// Mode objective props (M3 C1), built with the VQ4 modelling kit: the
// Heist core (a glowing crystal in a metal cage) and the Hot Reactor (a
// glowing core in crossed containment rings with vents). About 1 u across at
// instance scale 1. PART_GLOW takes the instance colour (an HDR display
// value above 1 glows hot), metal and trim are the shared materials.

import { Model, PART_GLOW, PART_METAL, PART_TRIM, rotate, type V3 } from "./kit.ts";

/** A torus around the Y axis: ring radius `big`, tube radius `small`. */
function torus(m: Model, big: number, small: number, segments: number, part: number, sides = 8): void {
  const profile: Array<[number, number]> = [];
  for (let i = 0; i <= sides; i++) {
    const a = (i / sides) * Math.PI * 2;
    profile.push([big + Math.cos(a) * small, Math.sin(a) * small]);
  }
  m.lathe(profile, segments, { part, axis: "y", group: m.smooth(60) });
}

/** Heist core: an elongated faceted crystal held by three tilted metal hoops. */
export function coreModel(): Model {
  const m = new Model();
  const n = 6;
  const ring = (y: number, r: number, twist = 0): V3[] =>
    Array.from({ length: n }, (_, i) => {
      const a = ((i + twist) / n) * Math.PI * 2;
      return [Math.cos(a) * r, y, Math.sin(a) * r] as V3;
    });
  // Bipyramid crystal with a short prism waist.
  m.loft([ring(-0.5, 0.001), ring(-0.14, 0.2), ring(0.14, 0.2, 0.5), ring(0.5, 0.001, 0.5)], { part: PART_GLOW, group: m.flat() });
  // Three hoops, tilted like a gyroscope cage.
  for (let k = 0; k < 3; k++) {
    const hoop = new Model();
    torus(hoop, 0.36, 0.022, 28, PART_METAL);
    m.add(hoop, (p) => rotate("y", (k * Math.PI) / 3)(rotate("x", Math.PI / 2 - 0.35)(p)));
  }
  // Clamps where the hoops hold the crystal tips.
  m.sphere([0, 0.42, 0], 0.045, 1, PART_TRIM, m.smooth(60));
  m.sphere([0, -0.42, 0], 0.045, 1, PART_TRIM, m.smooth(60));
  return m;
}

/** Hot Reactor: a glowing sphere in two crossed containment rings with six vents. */
export function reactorModel(): Model {
  const m = new Model();
  m.sphere([0, 0, 0], 0.27, 2, PART_GLOW, m.smooth(70));
  // Containment rings.
  const a = new Model();
  torus(a, 0.34, 0.045, 32, PART_METAL);
  m.add(a, rotate("x", 0.4));
  const b = new Model();
  torus(b, 0.34, 0.045, 32, PART_METAL);
  m.add(b, (p) => rotate("z", Math.PI / 2 + 0.4)(p));
  // Equator belt.
  m.lathe([[0.275, -0.035], [0.3, -0.02], [0.3, 0.02], [0.275, 0.035]], 24, { part: PART_TRIM, axis: "y", group: m.smooth(45) });
  // Six vents: dark collars with glowing mouths.
  const dirs: V3[] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];
  for (const d of dirs) {
    const vent = new Model();
    vent.lathe([[0.075, 0.24], [0.08, 0.33], [0.06, 0.35]], 10, { part: PART_TRIM, group: vent.smooth(50) });
    vent.lathe([[0.06, 0.345], [0.05, 0.36], [0.0, 0.365]], 10, { part: PART_GLOW, group: vent.smooth(70), capStart: true });
    const xf = (p: V3): V3 => {
      if (d[2] === 1) return p;
      if (d[2] === -1) return [p[0], -p[1], -p[2]];
      if (d[0] === 1) return [p[2], p[1], -p[0]];
      if (d[0] === -1) return [-p[2], p[1], p[0]];
      if (d[1] === 1) return [p[0], p[2], -p[1]];
      return [p[0], -p[2], p[1]];
    };
    m.add(vent, xf);
  }
  return m;
}
