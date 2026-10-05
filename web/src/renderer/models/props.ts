// Props built with the modelling kit: rocket, harpoon, mine, supply crate,
// upgrade and gadget pickups. About 1 u across at instance scale 1, facing
// -Z like the primitives they replace. PART_PLAIN keeps the visual's colour
// (data-driven per weapon or pickup), PART_GLOW glows in it, the rest are
// shared materials (metal, dark trim, engine).

import type { MeshData } from "../mesh.ts";
import { Model, PART_ENGINE, PART_GLOW, PART_METAL, PART_PLAIN, PART_TRIM, rotate, type V3 } from "./kit.ts";

/** Rocket: smooth body in the weapon colour, metal nose and fins, a glowing motor. */
export function rocketModel(): Model {
  const m = new Model();
  m.lathe(
    [
      [0, -0.5],
      [0.04, -0.46],
      [0.075, -0.36],
      [0.09, -0.24],
      [0.09, 0.26],
      [0.075, 0.34],
    ],
    14,
    { part: (seg) => (seg <= 1 ? PART_METAL : seg === 3 ? PART_PLAIN : seg === 2 ? PART_TRIM : PART_PLAIN), group: m.smooth(45) },
  );
  // Motor bell and its glowing core.
  m.lathe([[0.075, 0.34], [0.08, 0.4], [0.06, 0.4], [0.055, 0.36]], 14, { part: PART_METAL, group: m.smooth(45) });
  m.lathe([[0.056, 0.33], [0.056, 0.37], [0.02, 0.39]], 10, { part: PART_ENGINE, group: m.smooth(70), capStart: true, capEnd: true });
  // Four swept fins.
  for (let k = 0; k < 4; k++) {
    const fin = new Model().loft(
      [
        [[0.06, -0.008, 0.12], [0.06, 0.008, 0.12], [0.06, 0.008, 0.36], [0.06, -0.008, 0.36]],
        [[0.2, -0.004, 0.3], [0.2, 0.004, 0.3], [0.2, 0.004, 0.4], [0.2, -0.004, 0.4]],
      ],
      { part: PART_METAL, capStart: true, capEnd: true },
    );
    m.add(fin, rotate("z", (k * Math.PI) / 2 + Math.PI / 4));
  }
  // Guidance band.
  m.lathe([[0.092, -0.1], [0.092, -0.05]], 14, { part: PART_GLOW, group: m.smooth(45) });
  return m;
}

/** Harpoon: metal shaft, barbed head, cable eye at the back. */
export function harpoonModel(): Model {
  const m = new Model();
  m.lathe([[0, -0.5], [0.05, -0.3], [0.03, -0.28], [0.028, 0.42], [0.04, 0.45], [0.04, 0.5], [0, 0.5]], 10, {
    part: (seg) => (seg <= 1 ? PART_METAL : seg >= 4 ? PART_TRIM : PART_PLAIN),
    group: m.smooth(40),
  });
  for (let k = 0; k < 3; k++) {
    const barb = new Model().loft(
      [
        [[0.02, -0.006, -0.32], [0.02, 0.006, -0.32], [0.02, 0.006, -0.22], [0.02, -0.006, -0.22]],
        [[0.11, -0.003, -0.2], [0.11, 0.003, -0.2], [0.11, 0.003, -0.18], [0.11, -0.003, -0.18]],
      ],
      { part: PART_METAL, capStart: true, capEnd: true },
    );
    m.add(barb, rotate("z", (k * 2 * Math.PI) / 3));
  }
  return m;
}

/** Mine: a smooth core in the weapon colour, six metal spikes and a ring of indicator lights. */
export function mineModel(): Model {
  const m = new Model();
  m.sphere([0, 0, 0], 0.27, 2, PART_PLAIN, m.smooth(60));
  // Armour belt with lights.
  m.lathe([[0.27, -0.05], [0.29, -0.035], [0.29, 0.035], [0.27, 0.05]], 18, { part: PART_TRIM, group: m.smooth(40), axis: "y" });
  for (let k = 0; k < 6; k++) {
    const a = (k / 6) * Math.PI * 2;
    m.sphere([Math.cos(a) * 0.29, 0, Math.sin(a) * 0.29], 0.022, 0, PART_GLOW);
  }
  const dirs: V3[] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];
  for (const d of dirs) {
    const spike = new Model().lathe([[0.05, 0.22], [0.04, 0.3], [0.012, 0.48], [0, 0.5]], 8, { part: PART_METAL, group: 0, capStart: true });
    // Point the +Z spike along d.
    const xf = (p: V3): V3 => {
      if (d[2] === 1) return p;
      if (d[2] === -1) return [p[0], -p[1], -p[2]];
      if (d[0] !== 0) return [p[2] * d[0], p[1], -p[0] * d[0]];
      return [p[0], p[2] * d[1], -p[1] * d[1]];
    };
    m.add(spike, xf);
  }
  return m;
}

/** Supply crate: panels in the pickup colour inside a dark bevelled frame, glowing light strips. */
export function crateModel(): Model {
  const m = new Model();
  m.box([0, 0, 0], [0.33, 0.33, 0.33], { part: PART_PLAIN, bevel: 0.03 });
  // Frame edges (12 bars).
  const h = 0.35, w = 0.035;
  for (const a of [-1, 1]) {
    for (const b of [-1, 1]) {
      m.box([a * (h - w), b * (h - w), 0], [w, w, h], { part: PART_TRIM, bevel: 0.008 });
      m.box([a * (h - w), 0, b * (h - w)], [w, h - 2 * w, w], { part: PART_TRIM, bevel: 0.008 });
      m.box([0, a * (h - w), b * (h - w)], [h - 2 * w, w, w], { part: PART_TRIM, bevel: 0.008 });
    }
  }
  // Light strips on four faces and metal latches.
  for (let k = 0; k < 4; k++) {
    const strip = new Model().box([0, 0, -0.335], [0.2, 0.025, 0.012], { part: PART_GLOW });
    strip.box([0.24, 0.2, -0.335], [0.03, 0.04, 0.014], { part: PART_METAL, bevel: 0.006 });
    strip.box([-0.24, 0.2, -0.335], [0.03, 0.04, 0.014], { part: PART_METAL, bevel: 0.006 });
    m.add(strip, rotate("y", (k * Math.PI) / 2));
  }
  return m;
}

/** Upgrade pickup: a cut gem in the pickup colour held in a metal ring. */
export function upgradeModel(): Model {
  const m = new Model();
  const n = 8;
  const ring = (y: number, r: number): V3[] =>
    Array.from({ length: n }, (_, i) => {
      const a = ((i + 0.5) / n) * Math.PI * 2;
      return [Math.cos(a) * r, y, Math.sin(a) * r] as V3;
    });
  // Pavilion (bottom point), girdle, crown and table.
  m.loft([ring(-0.5, 0.001), ring(0, 0.3), ring(0.04, 0.3), ring(0.2, 0.17)], { part: PART_GLOW, capEnd: true, group: m.flat() });
  m.lathe([[0.3, -0.04], [0.33, -0.02], [0.33, 0.03], [0.3, 0.05], [0.29, 0.005]], 16, { part: PART_METAL, axis: "y", group: m.smooth(50) });
  return m;
}

/** Gadget pickup: a bevelled hex token with a glowing core and a metal rim, standing on edge (faces ±Z). */
export function gadgetModel(): Model {
  const m = new Model();
  const hex = (z: number, r: number): V3[] =>
    Array.from({ length: 6 }, (_, i) => {
      const a = (i / 6) * Math.PI * 2;
      return [Math.cos(a) * r, Math.sin(a) * r, z] as V3;
    });
  m.loft([hex(-0.07, 0.36), hex(-0.1, 0.42), hex(0.1, 0.42), hex(0.07, 0.36)], { part: PART_METAL, capStart: true, capEnd: true });
  m.loft([hex(-0.1, 0.2), hex(-0.11, 0.22), hex(0.11, 0.22), hex(0.1, 0.2)], { part: PART_PLAIN, capStart: true, capEnd: true });
  m.sphere([0, 0, 0], [0.13, 0.13, 0.13], 1, PART_GLOW, m.smooth(70));
  return m;
}

/** Mesh helpers for MESH_BUILDERS. */
export const buildProp = (f: () => Model) => (): MeshData => f().build();
