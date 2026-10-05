// Side colours and names: teams 0-3 come from a fixed palette (the paint
// colours' hull shades, plus lime and magenta for 3- and 4-team matches);
// in free-for-all every ship is its own side (side id 0-253, the ship's
// `team` field) and gets a colour from a larger palette of distinct hues,
// so opponents read apart in the scene, the name tags, the scoreboard and
// the results.

import type { Rgb } from "../renderer/renderer.ts";

/** Hull colour per team (teams 2-4 modes). */
export const TEAM_PALETTE: readonly Rgb[] = [
  [0.95, 0.5, 0.2], // orange
  [0.25, 0.6, 0.95], // blue
  [0.55, 0.9, 0.25], // lime
  [0.9, 0.35, 0.85], // magenta
];

/** Team names, matching TEAM_PALETTE. */
export const TEAM_NAMES: readonly string[] = ["Orange", "Blue", "Lime", "Magenta"];

/** Hull colour for unknown sides (no team, out of range). */
export const NEUTRAL_SIDE: Rgb = [0.8, 0.8, 0.8];

/**
 * Free-for-all hues, ordered so neighbouring side ids differ strongly
 * (side ids are handed out in join order).
 */
export const FFA_PALETTE: readonly Rgb[] = [
  [1.0, 0.55, 0.15], // orange
  [0.25, 0.65, 1.0], // sky
  [0.55, 0.95, 0.25], // lime
  [0.95, 0.35, 0.85], // magenta
  [1.0, 0.88, 0.2], // yellow
  [0.2, 0.9, 0.8], // teal
  [1.0, 0.3, 0.3], // red
  [0.6, 0.45, 1.0], // violet
  [0.45, 1.0, 0.6], // mint
  [1.0, 0.62, 0.75], // pink
  [0.3, 0.42, 1.0], // cobalt
  [0.85, 0.62, 0.35], // amber
];

/** Colour of a free-for-all side: the palette, lighter / deeper on each further lap. */
export function ffaColor(side: number): Rgb {
  if (!Number.isInteger(side) || side < 0 || side > 253) return NEUTRAL_SIDE;
  const n = FFA_PALETTE.length;
  const base = FFA_PALETTE[side % n] ?? NEUTRAL_SIDE;
  const lap = Math.floor(side / n) % 3;
  const [r, g, b] = base;
  if (lap === 1) return [r + (1 - r) * 0.45, g + (1 - g) * 0.45, b + (1 - b) * 0.45];
  if (lap === 2) return [r * 0.62, g * 0.62, b * 0.62];
  return base;
}

/** Hull colour of `side` in a match with `teams` teams (0 = free-for-all). */
export function sideColor(side: number, teams: number): Rgb {
  if (teams === 0) return ffaColor(side);
  return TEAM_PALETTE[side] ?? NEUTRAL_SIDE;
}

/** Team name ("Orange", ...; "Team 5" beyond the palette). */
export function teamName(team: number): string {
  return TEAM_NAMES[team] ?? `Team ${team + 1}`;
}

/** CSS colour of an Rgb (0..1 channels). */
export function cssColor(c: Rgb, alpha = 1): string {
  const ch = c.map((v) => Math.round(Math.max(0, Math.min(1, v)) * 255));
  return alpha >= 1 ? `rgb(${ch[0]}, ${ch[1]}, ${ch[2]})` : `rgba(${ch[0]}, ${ch[1]}, ${ch[2]}, ${alpha})`;
}

/** CSS colour of `side` (see sideColor). */
export function sideCss(side: number, teams: number, alpha = 1): string {
  return cssColor(sideColor(side, teams), alpha);
}
