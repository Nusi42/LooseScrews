// Decodes ls-wasm's overlay arrays (layouts in wasm/api.ts) into plain
// objects for the debug overlay, the scoreboard and the end-of-match screen.

import { DEBUG_STATS_LEN, MATCH_INFO_LEN, SCORE_STRIDE } from "../wasm/api.ts";

/** Seconds the server shows the result before it restarts (ls-server `--intermission` default). */
export const DEFAULT_INTERMISSION_SECS = 10;

/** Team names, matching the paint colours (ls-wasm `team_name`). */
export const TEAM_NAMES: readonly string[] = ["Orange", "Blue"];

export function teamName(team: number): string {
  return TEAM_NAMES[team] ?? `Team ${team + 1}`;
}

export interface DebugStats {
  /** Smoothed round trip in ms, null before the first pong (and offline). */
  pingMs: number | null;
  accepted: number;
  corrected: number;
  snapped: number;
  lastError: number;
  shotsPredicted: number;
  shotsConfirmed: number;
  shotsDropped: number;
  shotsUnpredicted: number;
  synced: boolean;
}

export interface ScoreRow {
  id: number;
  team: number;
  alive: boolean;
  kills: number;
  deaths: number;
  isYou: boolean;
}

export interface MatchInfo {
  over: boolean;
  /** Winning team, null for a draw (or while running). */
  winner: number | null;
  teamCells: [number, number];
  surfaceCells: number;
  secsSinceEnd: number;
}

function at(a: ArrayLike<number>, i: number): number {
  const v = a[i];
  return v !== undefined && Number.isFinite(v) ? v : 0;
}

export function parseDebugStats(a: ArrayLike<number>): DebugStats {
  const ping = a.length >= DEBUG_STATS_LEN ? at(a, 0) : -1;
  return {
    pingMs: ping >= 0 ? ping : null,
    accepted: at(a, 1),
    corrected: at(a, 2),
    snapped: at(a, 3),
    lastError: at(a, 4),
    shotsPredicted: at(a, 5),
    shotsConfirmed: at(a, 6),
    shotsDropped: at(a, 7),
    shotsUnpredicted: at(a, 8),
    synced: at(a, 9) !== 0,
  };
}

export function parseScoreboard(a: ArrayLike<number>): ScoreRow[] {
  const rows: ScoreRow[] = [];
  for (let i = 0; i + SCORE_STRIDE <= a.length; i += SCORE_STRIDE) {
    rows.push({
      id: at(a, i),
      team: at(a, i + 1),
      alive: at(a, i + 2) !== 0,
      kills: at(a, i + 3),
      deaths: at(a, i + 4),
      isYou: at(a, i + 5) !== 0,
    });
  }
  return rows;
}

export function parseMatchInfo(a: ArrayLike<number>): MatchInfo {
  if (a.length < MATCH_INFO_LEN) {
    return { over: false, winner: null, teamCells: [0, 0], surfaceCells: 0, secsSinceEnd: 0 };
  }
  const winner = at(a, 1);
  return {
    over: at(a, 0) !== 0,
    winner: winner >= 0 ? winner : null,
    teamCells: [at(a, 2), at(a, 3)],
    surfaceCells: at(a, 4),
    secsSinceEnd: Math.max(0, at(a, 5)),
  };
}

/** Display name of a scoreboard row (names are not on the wire yet; matches the kill feed). */
export function playerName(row: Pick<ScoreRow, "id" | "isYou">): string {
  return row.isYou ? "You" : `Player ${row.id + 1}`;
}

/** Team share of the paintable surface in percent (0 when unknown). */
export function paintPercent(info: MatchInfo, team: number): number {
  if (!(info.surfaceCells > 0)) return 0;
  return (100 * (info.teamCells[team] ?? 0)) / info.surfaceCells;
}

/** Seconds until the server restarts the match (rounded up, never negative). */
export function intermissionLeft(info: MatchInfo, intermissionSecs = DEFAULT_INTERMISSION_SECS): number {
  return Math.max(0, Math.ceil(intermissionSecs - info.secsSinceEnd));
}

/** Rows grouped per team (0, 1, then any others), keeping ls-wasm's order inside a team. */
export function rowsByTeam(rows: readonly ScoreRow[]): Array<{ team: number; rows: ScoreRow[] }> {
  const teams = new Map<number, ScoreRow[]>();
  for (const t of [0, 1]) teams.set(t, []);
  for (const r of rows) {
    const list = teams.get(r.team) ?? [];
    list.push(r);
    teams.set(r.team, list);
  }
  return [...teams.entries()].sort((a, b) => a[0] - b[0]).map(([team, list]) => ({ team, rows: list }));
}
