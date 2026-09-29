import assert from "node:assert/strict";
import { test } from "node:test";
import { CATALOG } from "./catalog.ts";
import {
  availabilityOf,
  canonical,
  codeVersion,
  decodeRules,
  DIFF_VERSION,
  encodeFullRules,
  encodeRules,
  formatRuleCode,
  FULL_VERSION,
  normalizeRuleCode,
  RuleCodeError,
  rulesEqual,
} from "./code.ts";
import {
  defaultRules,
  inviteLink,
  loadoutCount,
  parseInvite,
  rulesSummary,
  serverCommand,
  setGadgetAvailability,
  setLoadoutPrimary,
  setLoadoutSecondary,
  setPickup,
  setShipTypeAllowed,
  setTuning,
  setWeaponAvailability,
} from "./model.ts";
import { rulesToCode, validateRules } from "./validate.ts";
import type { RuleSetData } from "./types.ts";

const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

function roundTrip(r: RuleSetData): string {
  const code = encodeRules(r, CATALOG);
  const back = decodeRules(code, CATALOG);
  assert.ok(rulesEqual(back, canonical(r, CATALOG)), `round trip of ${code}`);
  assert.equal(encodeRules(back, CATALOG), code);
  return code;
}

test("codes match the Rust encoder for every vector", () => {
  assert.ok(CATALOG.vectors.length > 20);
  for (const v of CATALOG.vectors) {
    if (v.code === null) {
      assert.throws(() => encodeRules(v.rules, CATALOG), RuleCodeError, v.label);
      continue;
    }
    assert.equal(encodeRules(v.rules, CATALOG), v.code, v.label);
    assert.ok(rulesEqual(decodeRules(v.code, CATALOG), v.rules), `decode ${v.label}`);
    assert.ok(v.full !== null, v.label);
    assert.equal(encodeFullRules(v.rules, CATALOG), v.full, `full ${v.label}`);
    assert.ok(rulesEqual(decodeRules(v.full, CATALOG), v.rules), `decode full ${v.label}`);
  }
});

test("validation agrees with the server for every vector", () => {
  for (const v of CATALOG.vectors) {
    const problems = validateRules(v.rules, CATALOG);
    assert.equal(problems.length === 0, v.valid, `${v.label}: ${JSON.stringify(problems)}`);
    const res = rulesToCode(v.rules, CATALOG);
    assert.equal(res.ok, v.valid && v.code !== null, v.label);
    if (res.ok) assert.equal(res.code, v.code);
    else assert.ok(res.problems.length > 0, v.label);
  }
});

test("default rules are a short diff code; the full code is the long fallback", () => {
  const code = roundTrip(defaultRules(CATALOG));
  assert.equal(codeVersion(code), DIFF_VERSION);
  assert.ok(code.length <= 20, code);
  const full = encodeFullRules(defaultRules(CATALOG), CATALOG);
  assert.equal(codeVersion(full), FULL_VERSION);
  assert.ok(full.length > 200, `${full.length}`);
  const fallback = CATALOG.vectors.find((v) => v.label === "full fallback");
  assert.ok(fallback?.code);
  assert.equal(codeVersion(fallback.code), FULL_VERSION, "picked when shorter");
});

test("codes decode leniently and are formatted in groups", () => {
  const code = encodeRules(defaultRules(CATALOG), CATALOG);
  const messy = ` ${code.toLowerCase().replace(/-/g, " ")} `;
  assert.ok(rulesEqual(decodeRules(messy, CATALOG), decodeRules(code, CATALOG)));
  assert.equal(formatRuleCode(code.replace(/-/g, "")), code);
  assert.equal(normalizeRuleCode("lr0o-il"), "LR0011");
});

test("typos, truncation and garbage are rejected", () => {
  const codes = [
    encodeRules(defaultRules(CATALOG), CATALOG),
    ...CATALOG.vectors.filter((v) => v.code !== null).map((v) => v.code as string),
  ];
  let tested = 0;
  for (const code of codes.slice(0, 6)) {
    const chars = [...code];
    for (let i = 2; i < chars.length; i++) {
      if (chars[i] === "-") continue;
      for (const repl of [ALPHABET[(ALPHABET.indexOf(chars[i] as string) + 1) % 32], ALPHABET[(ALPHABET.indexOf(chars[i] as string) + 17) % 32]]) {
        const c = [...chars];
        c[i] = repl as string;
        assert.throws(() => decodeRules(c.join(""), CATALOG), RuleCodeError, `typo at ${i} in ${code}`);
        tested++;
      }
    }
    assert.throws(() => decodeRules(code.slice(0, -3), CATALOG), RuleCodeError);
    assert.throws(() => decodeRules(`${code}0`, CATALOG), RuleCodeError);
  }
  assert.ok(tested > 100);
  for (const bad of ["", "LR", "LS04-0000", "LR!!", "hello"]) {
    assert.throws(() => decodeRules(bad, CATALOG), RuleCodeError, bad);
  }
});

test("banning a weapon removes it everywhere and costs a few symbols", () => {
  const r = defaultRules(CATALOG);
  const base = encodeRules(r, CATALOG);
  setWeaponAvailability(r, CATALOG, "bolt_rocket", "Banned");
  assert.equal(availabilityOf(r, "bolt_rocket"), "Banned");
  assert.equal(r.pickups["bolt_rocket"], undefined);
  assert.equal(loadoutCount(r, "bolt_rocket"), 0);
  assert.deepEqual(validateRules(r, CATALOG), []);
  const code = roundTrip(r);
  assert.ok(code.length <= base.length + 4, code);
  // Matches the Rust vector for the same edit.
  const r2 = defaultRules(CATALOG);
  setWeaponAvailability(r2, CATALOG, "railspike", "Banned");
  setWeaponAvailability(r2, CATALOG, "tectonic_charge", "Banned");
  assert.equal(encodeRules(r2, CATALOG), CATALOG.vectors.find((v) => v.label === "ban railspike and tectonic")?.code);
  // Un-banning restores the default pickup: back to the default code.
  setWeaponAvailability(r2, CATALOG, "railspike", "PickupOnly");
  setWeaponAvailability(r2, CATALOG, "tectonic_charge", "PickupOnly");
  assert.equal(encodeRules(r2, CATALOG), base);
});

test("editor operations keep the rules valid", () => {
  const r = defaultRules(CATALOG);
  // Scrap Laser becomes loadout only: its pickups and upgrades go.
  setWeaponAvailability(r, CATALOG, "scrap_laser", "LoadoutOnly");
  assert.equal(r.pickups["scrap_laser"], undefined);
  assert.equal(r.upgrade_pickups["scrap_laser"], undefined);
  assert.deepEqual(validateRules(r, CATALOG), []);
  // Back to loadout and pickup: upgrades return with their default.
  setWeaponAvailability(r, CATALOG, "scrap_laser", "LoadoutAndPickup");
  assert.ok(r.upgrade_pickups["scrap_laser"]);
  // Loadout edits.
  setWeaponAvailability(r, CATALOG, "railspike", "LoadoutAndPickup");
  setLoadoutPrimary(r, "railspike", true);
  setLoadoutSecondary(r, "bolt_rocket", 5);
  assert.equal(loadoutCount(r, "bolt_rocket"), 5);
  assert.deepEqual(validateRules(r, CATALOG), []);
  // A pickup-only primary in the loadout is refused like the server does.
  setLoadoutPrimary(r, "rivet_gun", true);
  assert.equal(validateRules(r, CATALOG)[0]?.field, "loadout");
  setLoadoutPrimary(r, "rivet_gun", false);
  // Pickup weights, respawn times, tuning.
  setPickup(r, "railspike", { weight: 2.5, respawn_secs: 12 });
  setTuning(r, "railspike", { damage: 0.75 });
  assert.deepEqual(r.tuning["railspike"], { damage: 0.75, fire_rate: 1, ammo: 1 });
  setTuning(r, "rivet_gun", { fire_rate: 1 });
  assert.equal(r.tuning["rivet_gun"], undefined, "neutral tuning is no entry");
  // Friendly fire and ship types.
  r.friendly_fire_percent = 0;
  setShipTypeAllowed(r, CATALOG, "Tug", false);
  setShipTypeAllowed(r, CATALOG, "Tug", true);
  assert.deepEqual(r.allowed_ship_types, CATALOG.shipTypes);
  setShipTypeAllowed(r, CATALOG, "Skitter", false);
  // The shield gadget is banned: it leaves the loadout.
  setGadgetAvailability(r, CATALOG, "shield", "Banned");
  assert.equal(r.starting_loadout.gadget, null);
  const res = rulesToCode(r, CATALOG);
  assert.ok(res.ok, JSON.stringify(res.problems));
  roundTrip(r);
  assert.match(rulesSummary(decodeRules(res.code, CATALOG), CATALOG), /friendly fire off/);
});

test("problems the code format cannot carry are reported, not encoded", () => {
  const r = defaultRules(CATALOG);
  setPickup(r, "rivet_gun", { weight: 0.3 });
  const res = rulesToCode(r, CATALOG);
  assert.equal(res.ok, false);
  assert.equal(res.problems[0]?.field, "code");
  r.friendly_fire_percent = 5;
  assert.ok(rulesToCode(r, CATALOG).problems.some((p) => p.field === "general"));
});

test("invite links carry the world and rule set codes", () => {
  const rules = encodeRules(defaultRules(CATALOG), CATALOG);
  const world = "LS08-0000-0000-000A-K4M6-8001-8F1T";
  const link = inviteLink("https://play.example/game/?x=1#frag", { world, rules });
  assert.equal(link, `https://play.example/game/?world=LS0800000000000AK4M680018F1T&rules=${rules.replace(/-/g, "")}`);
  const back = parseInvite(link);
  assert.equal(back.world, "LS0800000000000AK4M680018F1T");
  assert.ok(back.rules);
  assert.ok(rulesEqual(decodeRules(back.rules, CATALOG), defaultRules(CATALOG)));
  assert.deepEqual(parseInvite("https://play.example/"), { world: null, rules: null });
  assert.equal(inviteLink("http://localhost:7777/", { world: null, rules }), `http://localhost:7777/?rules=${rules.replace(/-/g, "")}`);
  assert.equal(serverCommand({ world, rules }), `ls-server --code ${world} --ruleset ${rules}`);
  assert.equal(serverCommand({ world: null, rules: null }), "ls-server");
});

test("the catalog lists every weapon and gadget in registry order", () => {
  const ids = CATALOG.weapons.map((w) => w.id);
  assert.equal(new Set(ids).size, ids.length);
  assert.equal(ids[0], "scrap_laser");
  assert.deepEqual(
    CATALOG.gadgets.map((g) => g.id),
    ["shield", "cloak", "decoy"],
  );
  assert.deepEqual(CATALOG.shipTypes, ["Skitter", "Tinker", "Hauler", "Tug"]);
  for (const id of Object.keys(CATALOG.defaults.PaintTheVoid.weapons)) assert.ok(ids.includes(id), id);
});
