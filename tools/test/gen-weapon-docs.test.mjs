// Tests of tools/gen-weapon-docs.mjs: the generated mode reference covers every built-in rule
// set, explains every mode setting, and reads the mode list of the generated rules catalog; the
// event reference lists every comedy event; the world reference covers every world physics file
// and mirrors the simulation's radiation and cave-in laws.
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { capDistance, catalogModes, caveInThreshold, doseRate, duration, generate } from "../gen-weapon-docs.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

test("the mode reference lists every built-in rule set and every mode", () => {
  const md = generate()["modes.md"];
  assert.ok(md, "modes.md is generated");
  for (const f of readdirSync(join(root, "data/rulesets")).filter((f) => f.endsWith(".ron"))) {
    assert.ok(md.includes(`(\`${f.replace(/\.ron$/, "")}\`)`), `rule set ${f} listed`);
  }
  for (const name of ["Paint the Void", "Team Brawl", "The Squeeze", "Heist", "Hot Reactor", "Last Color Standing", "The Architect", "Belt Up"]) {
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
  assert.deepEqual([...modes.keys()], ["PaintTheVoid", "TeamBrawl", "TheSqueeze", "Heist", "HotReactor", "LastColorStanding", "TheArchitect", "BeltUp"]);
  assert.deepEqual(modes.get("TeamBrawl").teams, [0, 2, 3, 4]);
  assert.ok(!generate()["modes.md"].includes("**Teams the mode allows:** –"));
});

test("the event reference lists every comedy event and every built-in rule set", () => {
  const md = generate()["events.md"];
  const events = readdirSync(join(root, "data/events")).filter((f) => f.endsWith(".ron") && f !== "director.ron");
  assert.equal(events.length, 18);
  for (const f of events) {
    const row = md.split("\n").find((l) => l.includes(`(\`${f.replace(/\.ron$/, "")}\`)`));
    assert.ok(row, `event ${f} listed`);
    assert.ok(!row.endsWith("|  |"), `event ${f} has a description (EVENT_TEXT)`);
  }
  for (const f of readdirSync(join(root, "data/rulesets")).filter((f) => f.endsWith(".ron"))) {
    assert.ok(md.includes(`(\`${f.replace(/\.ron$/, "")}\`) |`), `rule set ${f} has an Absurdity row`);
  }
});

test("the world reference covers every world physics file and mirrors the sim's laws", () => {
  const md = generate()["world.md"];
  for (const title of ["Gravity wells", "Instability and cave-ins", "Pigment", "Radiation (asteroid belts)", "Drifting boulders (asteroid belts)"]) {
    assert.ok(md.includes(`\n## ${title}\n`), `section ${title}`);
  }
  assert.equal(readdirSync(join(root, "data/world")).filter((f) => f.endsWith(".ron")).length, 5, "a new data/world file needs a section");
  assert.ok(!md.includes("undefined") && !md.includes("NaN"), "every number resolves");
  // ls_sim::radiation::RadiationSettings::rate_at_distance with the C5 tuning.
  const r = { deep: 4, surface_rate: 0.8, slope: 0.2, knee: 4, far_slope: 0.6, cap: 7 };
  assert.equal(doseRate(r, -4), 0);
  assert.equal(doseRate(r, -2), 0.4);
  assert.equal(doseRate(r, 0), 0.8);
  assert.ok(Math.abs(doseRate(r, 4) - 1.6) < 1e-9);
  assert.ok(Math.abs(doseRate(r, 6) - 2.8) < 1e-9);
  assert.equal(doseRate(r, 50), 7);
  assert.ok(Math.abs(capDistance(r) - 13) < 1e-9);
  // ls_sim::instability::InstabilitySettings::threshold: never at 0, the base value at 100.
  const s = { threshold_at_100: 35, threshold_exponent: 0.18 };
  assert.equal(caveInThreshold(s, 0), Infinity);
  assert.equal(caveInThreshold(s, 100), 35);
  assert.ok(caveInThreshold(s, 30) > 35);
});

test("durations print as minutes and seconds", () => {
  assert.equal(duration(300), "5 min");
  assert.equal(duration(90), "1 min 30 s");
  assert.equal(duration(45), "45 s");
  assert.equal(duration(0), "–");
});
