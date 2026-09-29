// Rejoining after a match restart: the server closes every connection when
// it replaces a finished match (ls_game::MATCH_RESTART_REASON) and rebuilds
// the room on the first new join, which takes a moment. The client keeps the
// player's settings and reconnects with a short backoff.

import type { ControlSettings } from "./controls.ts";

/** What survives a rejoin. */
export interface PlayerPrefs {
  name: string;
  horizonAssist: boolean;
  debugOverlay: boolean;
  controls: ControlSettings;
  /** Ship type (ls-sim ShipType index) to ask for when joining. */
  shipType?: number;
}

export interface RejoinOptions {
  /** Connection attempts before giving up. */
  attempts?: number;
  /** Delay before attempt `n` (0-based), ms. */
  delayMs?: (attempt: number) => number;
  sleep?: (ms: number) => Promise<void>;
  /** Called before each attempt (status text). */
  onAttempt?: (attempt: number, lastError: unknown) => void;
  /** Stops retrying (e.g. the page is being torn down). */
  cancelled?: () => boolean;
}

export const REJOIN_ATTEMPTS = 12;

/** 0.5 s before the first attempt (like the bots), then 1, 2, 3, 4, 4, … s. */
export function rejoinDelay(attempt: number): number {
  return attempt === 0 ? 500 : Math.min(4000, 1000 * attempt);
}

export class RejoinCancelled extends Error {
  override name = "RejoinCancelled";
}

const defaultSleep = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

/** Calls `connect` until it succeeds; rejects with the last error after `attempts` failures. */
export async function rejoin<T>(connect: () => Promise<T>, opts: RejoinOptions = {}): Promise<T> {
  const attempts = Math.max(1, opts.attempts ?? REJOIN_ATTEMPTS);
  const delay = opts.delayMs ?? rejoinDelay;
  const sleep = opts.sleep ?? defaultSleep;
  let lastError: unknown = null;
  for (let attempt = 0; attempt < attempts; attempt++) {
    await sleep(delay(attempt));
    if (opts.cancelled?.()) throw new RejoinCancelled("rejoin cancelled");
    opts.onAttempt?.(attempt, lastError);
    try {
      return await connect();
    } catch (err) {
      lastError = err;
    }
  }
  throw lastError instanceof Error ? lastError : new Error(String(lastError));
}
