// Tests of tools/gen-weapon-docs.mjs: the generated mode reference covers every built-in rule
// set, explains every mode setting, and reads the mode list of the generated rules catalog.
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { catalogModes, duration, generate } from "../gen-weapon-docs.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

test("the mode reference lists every built-in rule set and every mode", () => {
  const md = generate()["modes.md"];
  assert.ok(md, "modes.md is generated");
  for (const f of readdirSync(join(root, "data/rulesets")).filter((f) => f.endsWith(".ron"))) {
    assert.ok(md.includes(`(\`${f.replace(/\.ron$/, "")}\`)`), `rule set ${f} listed`);
  }
  for (const name of ["Paint the Void", "Team Brawl", "The Squeeze", "Heist", "Hot Reactor", "Last Color Standing"]) {
    assert.ok(md.includes(`\n## ${name}\n`), `section for ${name}`);
  }
  assert.ok(md.includes("**Instagib**"), "the Instagib mutator is described");
});

test("every mode setting has a meaning (a new ModeRules field needs a text in MODE_FIELDS)", () => {
  const md = generate()["modes.md"];
  const unexplained = md.split("\n").filter((l) => /^\| `[a-z_0-9]+` \|/.test(l) && / – \|$/.test(l));
  assert.deepEqual(unexplained, []);
});

test("every mode's allowed team counts come from the rules catalog", () => {
  const modes = catalogModes(readFileSync(join(root, "web/src/rules/catalog.ts"), "utf8"));
  assert.deepEqual([...modes.keys()], ["PaintTheVoid", "TeamBrawl", "TheSqueeze", "Heist", "HotReactor", "LastColorStanding", "TheArchitect"]);
  assert.deepEqual(modes.get("TeamBrawl").teams, [0, 2, 3, 4]);
  assert.ok(!generate()["modes.md"].includes("**Teams the mode allows:** –"));
});

test("durations print as minutes and seconds", () => {
  assert.equal(duration(300), "5 min");
  assert.equal(duration(90), "1 min 30 s");
  assert.equal(duration(45), "45 s");
  assert.equal(duration(0), "–");
});
