// Tests of tools/e2e.mjs's helpers: the quick playlist's rule sets, log parsing of the rotation.
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  QUICK_ENTRIES,
  QUICK_EVENT_SCHEDULE,
  botTotals,
  eventScheduleArg,
  eventsFromLog,
  matchLength,
  parseFields,
  parsePrivateRooms,
  quickRuleSet,
  roomLines,
  rotationFromLog,
  setRonField,
  welcomeTimes,
  writeQuickPlaylist,
} from "../e2e.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const ruleset = (id) => readFileSync(join(root, "data", "rulesets", `${id}.ron`), "utf8");

test("the quick playlist shortens every built-in mode rule set it names", () => {
  const modes = new Set();
  for (const [id, overrides] of QUICK_ENTRIES) {
    const text = quickRuleSet(ruleset(id), overrides, 45);
    modes.add(/mode:\s*(\w+)/.exec(text)[1]);
    assert.ok(matchLength(text) <= 110, `${id}: ${matchLength(text)} s`);
  }
  // Every PvP mode of the default rotation, and (M4) The Architect and Belt Up.
  for (const m of ["TeamBrawl", "TheSqueeze", "PaintTheVoid", "HotReactor", "Heist", "LastColorStanding", "TheArchitect", "BeltUp"]) {
    assert.ok(modes.has(m), m);
  }
  const squeeze = quickRuleSet(ruleset("squeeze"), QUICK_ENTRIES.find(([id]) => id === "squeeze")[1], 45);
  assert.match(squeeze, /match_secs: 105\.0/);
  assert.match(squeeze, /first_phase_secs: 3\.0/);
  // Five phases close by 3 + 5 x (3 + 12) = 78 s; the timer (105 s) ends before the final seal (126 s).
  assert.match(squeeze, /close_secs: 12.0/);
  assert.equal(matchLength(squeeze), 105);
  const lcs = quickRuleSet(ruleset("last_color_standing"), QUICK_ENTRIES.find(([id]) => id === "last_color_standing")[1], 45);
  assert.match(lcs, /stalemate_secs: 23\.0/);
  assert.match(lcs, /stalemate_drain_secs: 0\.5/);
});

test("setRonField refuses a missing field", () => {
  assert.equal(setRonField("x: 1.0, match_secs: 300.0,", "match_secs", 20), "x: 1.0, match_secs: 20.0,");
  assert.throws(() => setRonField("x: 1.0", "match_secs", 20), /no match_secs/);
});

test("the rotation is read from the server log (public room only)", () => {
  const log = [
    "INFO ls_server::sim: match ready room=public entry=team_brawl.ron share_code=LS0C warmup=true build_ms=1",
    "INFO ls_server::sim: next room swapped in room=public warmup=false entry=team_brawl.ron share_code=LS0C players=8 moved=8 matches=1",
    "INFO ls_server::sim: tick stats room=public phase=\"match\" warmup=false players=8 p99_us=300 max_us=400 matches=1 craters=3",
    "INFO ls_server::sim: match over; building the next room room=public players=8 mode=\"team_brawl\" entry=team_brawl.ron winner_side=1 left_s=0.0 result=Ended { winner: Some(1) } next=squeeze.ron",
    "INFO ls_server::sim: next room swapped in room=public warmup=true entry=squeeze.ron players=8 moved=8 matches=1",
    "INFO ls_server::sim: next room swapped in room=R-ABC123 warmup=false entry=duel players=2 moved=2 matches=1",
    "WARN ls_server::sim: player not accepted by the next room room=public name=x",
  ].join("\n");
  const r = rotationFromLog(log);
  assert.equal(r.swaps.length, 2, "the private room's swap is not the rotation's");
  assert.equal(r.overs.length, 1);
  assert.equal(r.overs[0].mode, "team_brawl");
  assert.equal(r.overs[0].winner_side, "1");
  assert.equal(r.entryOf.get("1"), "team_brawl.ron");
  assert.equal(r.windows[0].p99_us, "300");
  assert.equal(r.refused.length, 1);
});

test("bot totals parse, with and without snapshot kinds", () => {
  const t = botTotals("totals: joins 8 (connect failures 0), disconnects 0, match restarts 152, inputs sent 99, snapshots 1000 (990 delta, 10 full, 0 stale), starved 0");
  assert.deepEqual([t.joins, t.disconnects, t.restarts, t.snapshots, t.full], [8, 0, 152, 1000, 10]);
  const old = botTotals("totals: joins 7 (connect failures 1), disconnects 2, match restarts 0, inputs sent 5, snapshots 6");
  assert.equal(old.full, null);
  assert.equal(botTotals("nothing"), null);
  assert.deepEqual(parseFields('a=1 b="x y" c=z'), { a: "1", b: "x y", c: "z" });
});

test("private rooms parse from rules:bots pairs", () => {
  assert.deepEqual(parsePrivateRooms(""), []);
  assert.deepEqual(parsePrivateRooms("duel:2, team_brawl:4"), [
    { rules: "duel", bots: 2 },
    { rules: "team_brawl", bots: 4 },
  ]);
  assert.throws(() => parsePrivateRooms("duel"), /bad entry/);
  assert.throws(() => parsePrivateRooms("duel:0"), /bad entry/);
});

test("welcome times and a private room's lines are read from the logs", () => {
  const bots = [
    "INFO ls_bots::net: joined bot=probe1-1 player=8 team=0 code=LS0C room= transport=\"websocket\" edits=12 welcome_ms=3.4",
    "INFO ls_bots::net: joined bot=bot1 player=0 team=0 code=LS0C room= transport=\"websocket\" edits=0",
    "INFO ls_bots::net: progress bot=bot1",
  ].join("\n");
  assert.deepEqual(welcomeTimes(bots), [3.4]);
  const server = [
    "INFO ls_server::sim: tick stats room=public players=8 p99_us=300",
    "INFO ls_server::sim: tick stats room=R-ABC players=2 p99_us=200",
  ].join("\n");
  const lines = roomLines(server, "R-ABC", "tick stats");
  assert.equal(lines.length, 1);
  assert.equal(lines[0].p99_us, "200");
});

test("the quick playlist file: Absurdity 100, The Architect, Belt Up only for up to 8 racers", () => {
  const dir = mkdtempSync(join(tmpdir(), "e2e-quick-"));
  try {
    const plan = writeQuickPlaylist(dir, 45, 8);
    const text = readFileSync(plan.file, "utf8");
    assert.match(text, /rules: "absurdity_100\.ron", world: RandomSliders\(absurdity: \(100, 100\)\)/);
    assert.match(text, /rules: "the_architect\.ron", world: Random, min_players: 2/);
    assert.match(text, /rules: "belt_up\.ron", world: Random, max_players: 8/);
    assert.equal(plan.entries.length, QUICK_ENTRIES.length);
    // Shortened: 3 rounds of 15 + 3 s, 3 heats of 3 + 20 + 3 s.
    assert.equal(plan.entries.find((e) => e.source === "the_architect.ron").secs, 54);
    assert.equal(plan.entries.find((e) => e.source === "belt_up.ron").secs, 78);
    const crowded = writeQuickPlaylist(dir, 45, 9);
    assert.ok(!crowded.entries.some((e) => e.source === "belt_up.ron"), "no Belt Up for 9 players");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("--event-schedule defaults to forced events with the quick playlist", () => {
  assert.equal(eventScheduleArg("", "quick"), QUICK_EVENT_SCHEDULE);
  assert.equal(eventScheduleArg("", ""), "");
  assert.equal(eventScheduleArg("off", "quick"), "");
  assert.equal(eventScheduleArg("10 duck_parade", ""), "10 duck_parade");
});

test("comedy events are read from the server log per room and match", () => {
  const log = [
    'INFO ls_server::sim: event schedule: events forced in this match room=public events=bored_level@5.0s,tax_drone@6.0s',
    'INFO ls_server::sim: comedy event room=public event="bored_level" phase=Announce target=-1 tick=300',
    'INFO ls_server::sim: comedy event room=public event="bored_level" phase=Active target=-1 tick=480',
    'INFO ls_server::sim: comedy event room=public event="bored_level" phase="Over" tick=840',
    'INFO ls_server::sim: match over; building the next room room=public players=6 mode="team_brawl"',
    'INFO ls_server::sim: event schedule: events forced in this match room=public events=gravity_coffee_break@5.0s,rubber_chickens@6.0s',
    'INFO ls_server::sim: comedy event room=R-1 event="hat_rain" phase=Announce target=-1 tick=3000',
  ].join("\n");
  const rooms = eventsFromLog(log);
  const pub = rooms.get("public");
  assert.equal(pub.matches.length, 2);
  assert.deepEqual(pub.matches[0].forced, [{ id: "bored_level", secs: 5 }, { id: "tax_drone", secs: 6 }]);
  assert.deepEqual(pub.matches[0].announced, ["bored_level"]);
  assert.deepEqual(pub.matches[1].announced, []);
  assert.equal(pub.matches[0].finished, true);
  assert.equal(pub.matches[1].finished, false);
  assert.ok(pub.active.has("bored_level") && pub.over.has("bored_level"));
  assert.deepEqual(rooms.get("R-1").announced, ["hat_rain"]);
});
