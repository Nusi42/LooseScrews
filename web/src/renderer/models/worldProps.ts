// World props of M4 B5, built with the VQ4 modelling kit: the pigment drop
// (a glossy teardrop of paint, tip up, about 1 u tall at instance scale 1).
// A pigment pickup is three drops in the primaries circling a glowing core
// (game/worldFx.ts places them); a splash is one big drop in the side's
// colour. The drop takes the instance colour (PART_PLAIN); its keep mask
// (Instance.keepColor) keeps it in colour in monochrome ink levels.

import { Model, PART_PLAIN } from "./kit.ts";

/** Teardrop of paint about the Y axis: a round belly, a pointed tip at +0.5. */
export function pigmentDropModel(): Model {
  const m = new Model();
  const profile: Array<[number, number]> = [];
  // Belly: a sphere of radius 0.3 centred at y = -0.18, from the bottom to a
  // little above its equator, then a straight-ish taper into the tip.
  const n = 10;
  const top = 0.6;
  for (let i = 0; i <= n; i++) {
    const a = -Math.PI / 2 + (i / n) * (Math.PI / 2 + top);
    const y = -0.18 + Math.sin(a) * 0.3;
    let r = Math.cos(a) * 0.3;
    if (y > -0.18) {
      const k = (y + 0.18) / 0.3;
      r *= 1 - 0.35 * k * k;
    }
    profile.push([Math.max(r, 0.0005), y]);
  }
  profile.push([0.15, 0.14], [0.085, 0.3], [0.035, 0.42], [0.0005, 0.5]);
  m.lathe(profile, 16, { part: PART_PLAIN, axis: "y", group: m.smooth(70), capStart: true, capEnd: true });
  return m;
}
