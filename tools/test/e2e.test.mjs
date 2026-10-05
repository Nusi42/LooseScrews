// Tests of tools/e2e.mjs's helpers: the quick playlist's rule sets, log parsing of the rotation.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { QUICK_ENTRIES, botTotals, matchLength, parseFields, quickRuleSet, rotationFromLog, setRonField } from "../e2e.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const ruleset = (id) => readFileSync(join(root, "data", "rulesets", `${id}.ron`), "utf8");

test("the quick playlist shortens every built-in mode rule set it names", () => {
  const modes = new Set();
  for (const [id, overrides] of QUICK_ENTRIES) {
    const text = quickRuleSet(ruleset(id), overrides, 45);
    modes.add(/mode:\s*(\w+)/.exec(text)[1]);
    assert.ok(matchLength(text) <= 110, `${id}: ${matchLength(text)} s`);
  }
  // Every PvP mode of the default rotation.
  for (const m of ["TeamBrawl", "TheSqueeze", "PaintTheVoid", "HotReactor", "Heist", "LastColorStanding"]) assert.ok(modes.has(m), m);
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
