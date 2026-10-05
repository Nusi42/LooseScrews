// Wrecks: the actual hull of a ship type broken into its 3-4 pieces
// (ships.ts), each piece turned and thrown a little away from the centre,
// lights and glass dead, the paint burnt (PART_SCORCH: the object shader
// draws smooth soot patches with flickering embers over the burnt paint).

import {
  chain,
  hash01,
  Model,
  PART_CANOPY,
  PART_ENGINE,
  PART_GLOW,
  PART_HULL,
  PART_LAMP,
  PART_LIVERY,
  PART_NAV_PORT,
  PART_NAV_STARBOARD,
  PART_NAV_TAIL,
  PART_SCORCH,
  PART_TRIM,
  rotate,
  scale,
  translate,
  type V3,
} from "./kit.ts";
import { hullOf } from "./ships.ts";

/** How far (u, at instance scale 1) the pieces drift apart. */
export const WRECK_SPREAD = 0.13;

const DEAD_PARTS = new Set([PART_ENGINE, PART_NAV_PORT, PART_NAV_STARBOARD, PART_NAV_TAIL, PART_LAMP, PART_GLOW, PART_CANOPY]);

function centre(m: Model): V3 {
  const b = m.bounds();
  return [(b.min[0] + b.max[0]) / 2, (b.min[1] + b.max[1]) / 2, (b.min[2] + b.max[2]) / 2];
}

/** The wreck of `shipType`: its hull pieces scattered and charred. */
export function wreckModel(shipType: number): Model {
  const hull = hullOf(shipType);
  const out = new Model();
  hull.pieces.forEach((piece, i) => {
    const c = centre(piece);
    const len = Math.hypot(c[0], c[1], c[2]);
    const seed = shipType * 17 + i * 5;
    const dir: V3 = len > 1e-3 ? [c[0] / len, c[1] / len, c[2] / len] : [hash01(seed) - 0.5, 0.5, hash01(seed + 1) - 0.5];
    const turn = chain(
      translate([-c[0], -c[1], -c[2]]),
      rotate("x", (hash01(seed + 2) - 0.5) * 1.4),
      rotate("y", (hash01(seed + 3) - 0.5) * 1.8),
      rotate("z", (hash01(seed + 4) - 0.5) * 1.6),
      translate([c[0] + dir[0] * WRECK_SPREAD, c[1] + dir[1] * WRECK_SPREAD, c[2] + dir[2] * WRECK_SPREAD]),
    );
    out.add(piece, turn);
  });
  out.tris.forEach((t) => {
    if (DEAD_PARTS.has(t.part)) t.part = PART_TRIM;
    else if (t.part === PART_HULL || t.part === PART_LIVERY) t.part = PART_SCORCH;
  });
  // Keep the wreck about as big as the ship was.
  return out.transformed(scale(0.92 * (0.5 / Math.max(0.5, out.maxRadius()))));
}
