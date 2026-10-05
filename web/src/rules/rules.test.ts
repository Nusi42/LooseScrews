import assert from "node:assert/strict";
import { test } from "node:test";
import { CATALOG } from "./catalog.ts";
import {
  availabilityOf,
  canonical,
  CODE_VERSION,
  codeForm,
  codeVersion,
  decodeRules,
  encodeFullRules,
  encodeRules,
  formatRuleCode,
  eventBanned,
  LEGACY_ABSURDITY,
  LEGACY_DIFF_VERSION,
  LEGACY_FULL_VERSION,
  LEGACY_V4_VERSION,
  setEventBanned,
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
  assert.equal(codeVersion(code), CODE_VERSION);
  assert.equal(codeForm(code), "diff");
  assert.ok(code.length <= 21, code);
  const full = encodeFullRules(defaultRules(CATALOG), CATALOG);
  assert.equal(codeForm(full), "full");
  assert.ok(full.length > 200, `${full.length}`);
  const fallback = CATALOG.vectors.find((v) => v.label === "full fallback");
  assert.ok(fallback?.code);
  assert.equal(codeForm(fallback.code), "full", "picked when shorter");
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
  const world = "LS0C-0000-0000-000A-K4M6-8001-8F1Y";
  const link = inviteLink("https://play.example/game/?x=1#frag", { world, rules });
  assert.equal(link, `https://play.example/game/?world=LS0C00000000000AK4M680018F1Y&rules=${rules.replace(/-/g, "")}`);
  const back = parseInvite(link);
  assert.equal(back.world, "LS0C00000000000AK4M680018F1Y");
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

test("every mode's default is a short diff code, and mode settings round-trip", () => {
  for (const mode of CATALOG.modes) {
    const r = defaultRules(CATALOG, mode.kind);
    assert.equal(r.mode.kind, mode.kind);
    assert.deepEqual(validateRules(r, CATALOG), [], mode.kind);
    const code = roundTrip(r);
    assert.equal(codeForm(code), "diff");
    assert.ok(code.length <= 21, `${mode.kind}: ${code}`);
    for (const f of mode.fields) {
      const e = defaultRules(CATALOG, mode.kind);
      e.mode[f.name] = (2 ** f.bits - 1) / f.steps;
      const back = decodeRules(roundTrip(e), CATALOG);
      assert.equal(back.mode[f.name], e.mode[f.name], `${mode.kind}.${f.name}`);
    }
  }
});

test("M2 codes (formats 3 and 2) still decode", () => {
  // Against the M2/M3 default (Absurdity 20), with today's pigment and no event bans.
  const def = { ...defaultRules(CATALOG), absurdity: LEGACY_ABSURDITY };
  const v3 = "LR6V-69CM-G006-58R";
  assert.equal(codeVersion(v3), LEGACY_DIFF_VERSION);
  assert.ok(rulesEqual(decodeRules(v3, CATALOG), def));
  const v2 =
    "LR4V-6WGD-RX2W-0Q8K-W0PM-RVR8-0JR7-K4AF-05AN-A5AN-APAM-A40B-1506-A107-H0G3-RG61-W840-Y420-F20R-7H10-2GG8-1W86-0S44-0A8D-0G3T-2840-YGT0-GA42-G61X-1410-PR50-C3T2-840Y-GT0R-7M4G-25N0-RR21-D410-PR8G-42V0-M10P-R908-5MG4-2V14-30F0-0000-1085-MG42-T821-D4R0-7HW0-0000-0000-02B5-4";
  assert.equal(codeVersion(v2), LEGACY_FULL_VERSION);
  assert.ok(rulesEqual(decodeRules(v2, CATALOG), def));
  const banned = decodeRules("LR6V-69CM-G214-D7G1-P6B", CATALOG);
  assert.equal(availabilityOf(banned, "railspike"), "Banned");
  assert.equal(availabilityOf(banned, "tectonic_charge"), "Banned");
});

test("older codes (formats 2-4) decode like the server decodes them", () => {
  assert.ok(CATALOG.legacyVectors.length >= 6);
  let v4 = 0;
  for (const v of CATALOG.legacyVectors) {
    if (codeVersion(v.code) === LEGACY_V4_VERSION) v4++;
    assert.ok(rulesEqual(decodeRules(v.code, CATALOG), v.rules), v.code);
    // Re-encoded they print format 5.
    assert.equal(codeVersion(encodeRules(v.rules, CATALOG)), CODE_VERSION);
  }
  assert.ok(v4 >= 4);
});

test("comedy event bans and pigment pickups round-trip", () => {
  assert.equal(CATALOG.events.length, 18);
  const base = defaultRules(CATALOG);
  const short = encodeRules(base, CATALOG);
  const r = defaultRules(CATALOG);
  setEventBanned(r, "duck_parade", true);
  const code = roundTrip(r);
  assert.ok(code.length <= short.length + 3, `${short} -> ${code}`);
  assert.ok(eventBanned(decodeRules(code, CATALOG), "duck_parade"));
  assert.ok(!eventBanned(decodeRules(code, CATALOG), "cat_paw"));
  setEventBanned(r, "duck_parade", false);
  assert.equal(encodeRules(r, CATALOG), short);
  r.pigment_pickup = null;
  roundTrip(r);
  r.pigment_pickup = { respawn_secs: 45, players_per_pickup: 3, min_on_map: 1 };
  roundTrip(r);
  assert.equal(validateRules(r, CATALOG).length, 0);
  r.pigment_pickup.players_per_pickup = 0;
  assert.ok(validateRules(r, CATALOG).some((p) => p.field === "pigment"));
});
