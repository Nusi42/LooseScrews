// `?event=<id>` (milestone-4-plan.md §3.1 "Practice and tests"): in
// practice, the comedy event with that data/events id is forced a few
// seconds after the start (ls-wasm `Game.practiceEvent`), so an event can be
// reviewed or smoke-tested without waiting for the director. Ids are the
// ls-sim `EventKind` order (audio/announcer/lines.ts EVENT_IDS).

import { EVENT_IDS } from "../../audio/announcer/lines.ts";

/** The `EventKind` index `?event=` asks for, or null (absent or unknown). */
export function forcedEvent(loc: { search: string } | undefined): number | null {
  if (!loc) return null;
  const id = new URLSearchParams(loc.search).get("event");
  if (!id) return null;
  const k = (EVENT_IDS as readonly string[]).indexOf(id);
  return k >= 0 ? k : null;
}
