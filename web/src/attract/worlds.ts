// Names a generated world for captions ("Caverns open to the sky · LS..").
// The world type follows the Enclosure slider like the generator's blend
// (renderer/atmosphere.ts: below 50 the sky shows, 50 caverns, 100 mine shafts).

import { decodeShareCode } from "../renderer/atmosphere.ts";

/** The world type of an Enclosure value (0..100). */
export function worldKind(enclosure: number): string {
  if (enclosure < 15) return "Open space";
  if (enclosure < 30) return "Sky islands";
  if (enclosure < 50) return "Caverns open to the sky";
  if (enclosure < 70) return "Caverns";
  if (enclosure < 90) return "Caverns and mine shafts";
  return "Mine shafts";
}

/** Caption for the world behind the start screen: kind and share code. */
export function describeWorld(code: string): string {
  const w = decodeShareCode(code);
  return w ? `${worldKind(w.sliders.enclosure)} · ${code}` : code;
}
