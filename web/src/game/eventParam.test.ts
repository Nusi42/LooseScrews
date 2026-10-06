import assert from "node:assert/strict";
import { test } from "node:test";

import { applyEventParam, DEFAULT_EVENT_DELAY_SECS, parseEventParam, type EventDebug, type EventGame } from "./eventParam.ts";

const IDS = ["bored_level", "tax_drone", "duck_parade", "hat_rain"];

test("?event= parses ids, delays, lists and all", () => {
  assert.deepEqual(parseEventParam("", IDS), { events: [], unknown: [] });
  assert.deepEqual(parseEventParam("?event=duck_parade", IDS).events, [{ id: "duck_parade", delaySecs: DEFAULT_EVENT_DELAY_SECS }]);
  assert.deepEqual(parseEventParam("?event=duck_parade@10,hat_rain@0.5", IDS).events, [
    { id: "duck_parade", delaySecs: 10 },
    { id: "hat_rain", delaySecs: 0.5 },
  ]);
  const all = parseEventParam("?practice&event=all@3", IDS).events;
  assert.deepEqual(
    all.map((e) => e.id),
    IDS,
  );
  assert.ok(all.every((e) => e.delaySecs === 3));
  const bad = parseEventParam("?event=ducks,hat_rain@-4", IDS);
  assert.deepEqual(bad.unknown, ["ducks"]);
  assert.deepEqual(bad.events, [{ id: "hat_rain", delaySecs: DEFAULT_EVENT_DELAY_SECS }]);
});

test("applyEventParam schedules in practice and installs the debug handle", () => {
  const calls: [string, number][] = [];
  const warps: number[] = [];
  const game: EventGame = {
    practiceEvent: (id, d) => {
      calls.push([id, d]);
      return true;
    },
    practiceWarp: (s) => {
      warps.push(s);
      return Math.round(s * 60);
    },
    modeState: () => JSON.stringify({ mood: { moodId: "giddy", eventId: "duck_parade", phaseId: "active" } }),
  };
  const target: { __lsEvents?: EventDebug } = {};
  const got = applyEventParam(game, "?event=duck_parade@4", IDS, target);
  assert.deepEqual(got, [{ id: "duck_parade", delaySecs: 4 }]);
  assert.deepEqual(calls, [["duck_parade", 4]]);
  const dbg = target.__lsEvents!;
  assert.deepEqual(dbg.ids, IDS);
  assert.equal(dbg.warp(2), 120);
  assert.deepEqual(warps, [2]);
  assert.equal(dbg.mood().eventId, "duck_parade");
  assert.equal(dbg.mood().mood, "giddy");
  assert.ok(dbg.schedule("hat_rain"));
  assert.deepEqual(calls[1], ["hat_rain", 0]);
});

test("online games (no practiceEvent) schedule nothing", () => {
  const target: { __lsEvents?: EventDebug } = {};
  assert.deepEqual(applyEventParam({}, "?event=duck_parade", IDS, target), []);
  assert.equal(target.__lsEvents, undefined);
  // An online core refuses (practiceEvent returns false).
  assert.deepEqual(applyEventParam({ practiceEvent: () => false }, "?event=duck_parade", IDS, target), []);
});
