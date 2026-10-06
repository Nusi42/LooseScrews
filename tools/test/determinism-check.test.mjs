// Tests of tools/determinism-check.mjs's fixture parsing and the mode replay comparison.
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { compareModeReplays, parseFixture, parseModeFixture } from "../determinism-check.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

test("the checked-in mode replay fixture parses and names every built-in rule set", () => {
  const rows = parseModeFixture(readFileSync(join(root, "crates/ls-sim/tests/fixtures/mode_replays.txt"), "utf8"));
  const files = rows.map((r) => r.file);
  const ruleset = readFileSync(join(root, "crates/ls-sim/src/ruleset.rs"), "utf8");
  const builtin = [...ruleset.matchAll(/\("(rulesets\/[a-z_]+\.ron)", [A-Z_]+_RON\)/g)].map((m) => m[1]);
  assert.ok(builtin.length >= 9, `BUILTIN_RULESETS parsed: ${builtin.length}`);
  assert.deepEqual(files, builtin);
});

test("the worldgen fixture still parses", () => {
  const rows = parseFixture(readFileSync(join(root, "crates/ls-worldgen/tests/fixtures/golden_hashes.txt"), "utf8"));
  assert.ok(rows.length > 0);
});

test("parseModeFixture skips comments and rejects malformed lines", () => {
  assert.deepEqual(parseModeFixture("# c\r\n\r\nrulesets/a.ron 0x0123456789abcdef\n"), [
    { file: "rulesets/a.ron", hash: "0x0123456789abcdef" },
  ]);
  assert.throws(() => parseModeFixture("rulesets/a.ron 123"));
});

test("compareModeReplays reports differences, missing and extra rule sets", () => {
  const rows = [
    { file: "a", hash: "0x0000000000000001" },
    { file: "b", hash: "0x0000000000000002" },
  ];
  assert.deepEqual(compareModeReplays(["a 0x0000000000000001", "b 0x0000000000000002"], rows), []);
  const p = compareModeReplays(["a 0x0000000000000009", "c 0x0000000000000003"], rows);
  assert.deepEqual(p, [
    "a: wasm 0x0000000000000009 != native 0x0000000000000001",
    "b: missing in the WASM build",
    "c: not in the native fixture (re-bless it)",
  ]);
});

test("the checked-in mood replay fixture parses: world physics, all 18 events, The Architect and Belt Up", () => {
  const rows = parseModeFixture(readFileSync(join(root, "crates", "ls-sim", "tests", "fixtures", "mood_replays.txt"), "utf8"));
  const names = rows.map((r) => r.file);
  assert.equal(new Set(names).size, names.length, "no duplicate lines");
  const events = readdirSync(join(root, "data", "events"))
    .filter((f) => f.endsWith(".ron") && f !== "director.ron")
    .map((f) => `event/${f.slice(0, -4)}`);
  assert.equal(events.length, 18);
  for (const n of [...events, "world/gravity", "world/cave_in", "world/pigment", "world/radiation", "world/paperweight", "mode/the_architect", "mode/belt_up"]) {
    assert.ok(names.includes(n), `${n} in mood_replays.txt`);
  }
});
