// The three announcer voice packs; a match uses one, picked from its seed.

import type { VoicePack } from "./lines.ts";
import { OVEREXCITED } from "./overexcited.ts";
import { PASSIVE_AGGRESSIVE } from "./passiveAggressive.ts";
import { TIRED } from "./tired.ts";

export const PACKS: readonly VoicePack[] = [TIRED, OVEREXCITED, PASSIVE_AGGRESSIVE];

/** The pack of a match: `seed` is any 32-bit match seed (the share code's hash today). */
export function packForSeed(seed: number): VoicePack {
  // Mix so neighbouring seeds spread over the packs.
  let h = (seed ^ 0x9e3779b9) >>> 0;
  h = Math.imul(h ^ (h >>> 16), 0x85ebca6b) >>> 0;
  h = Math.imul(h ^ (h >>> 13), 0xc2b2ae35) >>> 0;
  return PACKS[((h ^ (h >>> 16)) >>> 0) % PACKS.length] as VoicePack;
}

export function packById(id: string): VoicePack | undefined {
  return PACKS.find((p) => p.id === id);
}
