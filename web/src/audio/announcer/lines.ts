// Announcer line sets (M4 A6, milestone-4-plan.md §3.7): what the announcer
// says, per voice pack (tired.ts, overexcited.ts, passiveAggressive.ts).
// Client-only text, not hashed, not in data/. Triggers:
//   event.<event id>   the 3 s telegraph of a comedy event ({target} = the
//                      player it aims at, where it has one)
//   firstBlood         the match's first kill ({killer}, {victim})
//   multiKill2/3/4     a pilot's 2nd, 3rd, 4th-or-more kill within a few s ({killer})
//   killcam            the Tired announcer event narrating a kill ({killer}, {victim})
//   caveIn             a tunnel caves in
//   matchStart, matchEnd ({winner} may be empty), beltUp (Belt Up start lights)
// Event packages add lines for their events here when they change the
// wording; a missing trigger falls back to `generic`.

import type { VoiceParams } from "../voice.ts";

/** The 18 comedy events (ls-sim `EventKind` order, data/events/<id>.ron). */
export const EVENT_IDS = [
  "bored_level",
  "tax_drone",
  "gravity_coffee_break",
  "rubber_chickens",
  "cat_paw",
  "tired_announcer",
  "wrong_genre",
  "gossip_wall",
  "pizza_delivery",
  "everything_is_cake",
  "duck_parade",
  "hat_rain",
  "customer_survey",
  "disco_inferno",
  "big_head_day",
  "moving_day",
  "mimic_pickups",
  "level_hiccups",
] as const;

export type EventId = (typeof EVENT_IDS)[number];

export const BASE_TRIGGERS = ["generic", "matchStart", "matchEnd", "firstBlood", "multiKill2", "multiKill3", "multiKill4", "killcam", "caveIn", "beltUp"] as const;

export type Trigger = (typeof BASE_TRIGGERS)[number] | `event.${EventId}`;

/** Every trigger a complete pack covers. */
export const ALL_TRIGGERS: readonly Trigger[] = [...BASE_TRIGGERS, ...EVENT_IDS.map((e) => `event.${e}` as const)];

export interface VoicePack {
  id: "tired" | "overexcited" | "passiveAggressive";
  /** The announcer's name on the caption. */
  name: string;
  /** Job title under the name. */
  title: string;
  /** Caption accent colour (CSS). */
  accent: string;
  voice: VoiceParams;
  lines: Readonly<Partial<Record<Trigger, readonly string[]>>>;
}

export type LineVars = Readonly<Record<string, string | number | undefined>>;

/** Replaces `{name}` placeholders; a missing value reads as "someone" ({winner}: "nobody"). */
export function fillLine(line: string, vars: LineVars): string {
  return line.replace(/\{(\w+)\}/g, (_, k: string) => {
    const v = vars[k];
    if (v === undefined || v === "") return k === "winner" ? "nobody" : "someone";
    return String(v);
  });
}

/** The lines of `trigger` in `pack`, or its generic lines. */
export function linesFor(pack: VoicePack, trigger: Trigger): readonly string[] {
  return pack.lines[trigger] ?? pack.lines.generic ?? ["…"];
}

/**
 * Picks lines without repeating the last pick of each trigger (a line
 * cycles through its alternatives in a shuffled order).
 */
export class LinePicker {
  private readonly last = new Map<string, number>();
  private readonly rnd: () => number;
  constructor(rnd: () => number = Math.random) {
    this.rnd = rnd;
  }
  pick(pack: VoicePack, trigger: Trigger): string {
    const ls = linesFor(pack, trigger);
    if (ls.length <= 1) return ls[0] ?? "…";
    const key = `${pack.id}/${trigger}`;
    const prev = this.last.get(key) ?? -1;
    let i: number;
    if (prev < 0) i = Math.floor(this.rnd() * ls.length);
    else {
      i = Math.floor(this.rnd() * (ls.length - 1));
      if (i >= prev) i++;
    }
    i = Math.min(ls.length - 1, i);
    this.last.set(key, i);
    return ls[i] ?? "…";
  }
}
