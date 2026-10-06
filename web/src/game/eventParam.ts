/**
 * `?event=<id>` (M4 plan §3.1): forces comedy events in offline practice,
 * for looking at an event without waiting for the director, for the
 * gallery's live strips and the browser smoke's moods phase.
 *
 * - `?event=duck_parade` forces Duck parade 2 s after practice starts.
 * - `?event=duck_parade@10` 10 s after the start.
 * - `?event=hat_rain,cat_paw@20` several (each starts at its time, or as
 *   soon as the running one ended).
 * - `?event=all` every event in turn, 2 s apart (each waits for the last).
 *
 * The page also gets `window.__lsEvents` while practice runs (scripts and
 * the smoke): `ids`, `schedule(id, delaySecs)`, `warp(secs)` (runs the
 * simulation ahead at once) and `mood()` (the parsed mood state). Online
 * the server's director decides (hosts force events with ls-server's
 * testing flag `--event-schedule`), so nothing is scheduled there.
 */

import { parseModeState, type MoodState } from "./modeState.ts";

/** Seconds after the start of practice an event forced without `@<secs>` is announced. */
export const DEFAULT_EVENT_DELAY_SECS = 2;

export interface ForcedEvent {
  id: string;
  /** Seconds from the start of practice. */
  delaySecs: number;
}

/** The practice game calls this module needs (ls-wasm `Game`; optional in older cores). */
export interface EventGame {
  practiceEvent?(id: string, delaySecs: number): boolean;
  practiceWarp?(secs: number): number;
  modeState?(): string;
}

/** The page's debug handle (`window.__lsEvents`). */
export interface EventDebug {
  readonly ids: readonly string[];
  /** Forces `id` `delaySecs` from now; false for an unknown id or online. */
  schedule(id: string, delaySecs?: number): boolean;
  /** Runs `secs` of simulation at once (the own ship idles); returns the ticks run. */
  warp(secs: number): number;
  mood(): MoodState;
}

/**
 * Parses `?event=` against the known event ids (`eventIds()`); unknown ids
 * are dropped (`unknown` lists them). An empty list without the parameter.
 */
export function parseEventParam(search: string, known: readonly string[]): { events: ForcedEvent[]; unknown: string[] } {
  const raw = new URLSearchParams(search).get("event");
  const events: ForcedEvent[] = [];
  const unknown: string[] = [];
  if (raw === null) return { events, unknown };
  const items = raw
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s !== "");
  for (const item of items) {
    const [id = "", at] = item.split("@");
    const secs = at === undefined ? Number.NaN : Number(at);
    const delaySecs = Number.isFinite(secs) && secs >= 0 ? secs : DEFAULT_EVENT_DELAY_SECS;
    if (id === "all") {
      for (const k of known) events.push({ id: k, delaySecs });
    } else if (known.includes(id)) {
      events.push({ id, delaySecs });
    } else {
      unknown.push(id);
    }
  }
  return { events, unknown };
}

/**
 * Schedules the `?event=` events in a practice game and installs
 * `window.__lsEvents`. Returns the events scheduled (none online or with a
 * core without `practiceEvent`).
 */
export function applyEventParam(
  game: EventGame,
  search: string,
  known: readonly string[],
  target: { __lsEvents?: EventDebug } | null = typeof window === "undefined" ? null : (window as unknown as { __lsEvents?: EventDebug }),
): ForcedEvent[] {
  if (!game.practiceEvent) return [];
  const debug: EventDebug = {
    ids: known,
    schedule: (id, delaySecs = 0) => game.practiceEvent?.(id, delaySecs) ?? false,
    warp: (secs) => game.practiceWarp?.(secs) ?? 0,
    mood: () => parseModeState(game.modeState?.()).mood,
  };
  if (target) target.__lsEvents = debug;
  const { events, unknown } = parseEventParam(search, known);
  if (unknown.length) console.warn(`?event=: unknown event ${unknown.join(", ")} (known: ${known.join(", ")})`);
  return events.filter((e) => debug.schedule(e.id, e.delaySecs));
}
