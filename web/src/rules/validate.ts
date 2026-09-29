// Rule set validation for the editor: mirrors `RuleSet::resolve_with`
// (crates/ls-sim/src/ruleset.rs) check by check, so the editor never produces a code
// the server refuses. The Node tests run it over the Rust-generated vectors
// (catalog.ts), which carry the server's verdict for each rule set.

import { asPickup, availabilityOf, encodeRules, gadgetAvailabilityOf, inLoadout, RuleCodeError } from "./code.ts";
import type { Catalog, PickupRule, RuleSetData } from "./types.ts";

export interface RuleProblem {
  /** Which part of the rule set: `general`, `ships`, `weapon:<id>`, `gadget:<id>`, `loadout`, `scrap`, `code`. */
  field: string;
  message: string;
}

const finiteNonNeg = (v: number): boolean => Number.isFinite(v) && v >= 0;
const count = (v: number): boolean => Number.isInteger(v) && v >= 0;

/** Every reason the server would refuse the rule set (empty = valid). */
export function validateRules(r: RuleSetData, catalog: Catalog): RuleProblem[] {
  const out: RuleProblem[] = [];
  const add = (field: string, message: string): void => {
    out.push({ field, message });
  };
  const weapon = (id: string) => catalog.weapons.find((w) => w.id === id);
  const gadget = (id: string) => catalog.gadgets.find((g) => g.id === id);
  const weaponName = (id: string): string => weapon(id)?.name ?? id;

  const ff = r.friendly_fire_percent;
  if (!Number.isInteger(ff) || ff < 0 || ff > 100 || (ff >= 1 && ff < 10)) {
    add("general", `Friendly fire ${ff}% must be off (0) or 10-100%.`);
  }
  if (!Number.isInteger(r.absurdity) || r.absurdity < 0 || r.absurdity > 100) {
    add("general", `Absurdity ${r.absurdity} must be 0-100.`);
  }
  for (const m of r.mutators) add("general", `Mutator ${m} is not supported yet.`);
  if (r.allowed_ship_types.length === 0) add("ships", "Allow at least one ship type.");
  for (const t of r.allowed_ship_types) if (!catalog.shipTypes.includes(t)) add("ships", `Unknown ship type ${t}.`);
  if (r.mode.teams !== 2) add("general", `Paint the Void supports 2 teams, got ${r.mode.teams}.`);
  if (!(Number.isFinite(r.mode.match_secs) && r.mode.match_secs > 0) || !finiteNonNeg(r.mode.respawn_secs)) {
    add("general", "Match length must be > 0 and respawn time >= 0.");
  }
  for (const id of Object.keys(r.weapons)) if (!weapon(id)) add(`weapon:${id}`, `Unknown weapon ${id}.`);
  for (const id of Object.keys(r.gadgets)) if (!gadget(id)) add(`gadget:${id}`, `Unknown gadget ${id}.`);

  for (const [id, t] of Object.entries(r.tuning)) {
    if (!weapon(id)) add(`weapon:${id}`, `Unknown weapon ${id}.`);
    else if (!finiteNonNeg(t.damage) || !(Number.isFinite(t.fire_rate) && t.fire_rate > 0) || !finiteNonNeg(t.ammo)) {
      add(`weapon:${id}`, `${weaponName(id)}: tuning needs damage >= 0, fire rate > 0, ammo >= 0.`);
    }
  }
  for (const [id, s] of Object.entries(r.shield_overrides)) {
    if (!weapon(id)) add(`weapon:${id}`, `Unknown weapon ${id}.`);
    else if (s.kind === "PassThrough" && !(s.value >= 0 && s.value <= 1)) {
      add(`weapon:${id}`, `${weaponName(id)}: shield pass-through fraction must be 0-1.`);
    } else if (s.kind === "Wears" && !finiteNonNeg(s.value)) {
      add(`weapon:${id}`, `${weaponName(id)}: shield wear factor must be >= 0.`);
    }
  }

  const loadoutWeapon = (id: string, slot: "primary" | "secondary"): boolean => {
    const w = weapon(id);
    if (!w) {
      add("loadout", `Unknown weapon ${id}.`);
      return false;
    }
    const a = availabilityOf(r, id);
    if (a === "Banned") add("loadout", `${w.name} is banned and cannot be in the loadout.`);
    else if (!inLoadout(a)) add("loadout", `${w.name} is ${a} and cannot be in the loadout.`);
    else if (w.slot !== slot) add("loadout", `${w.name} is not a ${slot} weapon.`);
    else return true;
    return false;
  };
  const lo = r.starting_loadout;
  for (const id of lo.primaries) loadoutWeapon(id, "primary");
  const seen = new Set<string>();
  for (const [id, n] of lo.secondaries) {
    if (!loadoutWeapon(id, "secondary")) continue;
    if (seen.has(id)) add("loadout", `${weaponName(id)} is listed twice in the secondaries.`);
    seen.add(id);
    if (!count(n)) add("loadout", `${weaponName(id)}: the count must be a whole number.`);
  }
  if (lo.gadget !== null) {
    const g = gadget(lo.gadget);
    const a = gadgetAvailabilityOf(r, lo.gadget);
    if (!g) add("loadout", `Unknown gadget ${lo.gadget}.`);
    else if (!inLoadout(a)) add("loadout", `Gadget ${g.name} is ${a} and cannot be in the loadout.`);
  }

  const checkRule = (field: string, what: string, rule: PickupRule): void => {
    if (!finiteNonNeg(rule.weight) || !finiteNonNeg(rule.respawn_secs)) {
      add(field, `${what}: invalid spawn weight or respawn time.`);
    }
    if (!count(rule.ammo) || !count(rule.max_on_map)) add(field, `${what}: ammo and max on map must be whole numbers.`);
  };
  for (const [id, rule] of Object.entries(r.pickups)) {
    const w = weapon(id);
    if (!w) {
      add(`weapon:${id}`, `Unknown weapon ${id}.`);
      continue;
    }
    const a = availabilityOf(r, id);
    if (!asPickup(a)) add(`weapon:${id}`, `${w.name} is ${a} and cannot be a pickup.`);
    else checkRule(`weapon:${id}`, `${w.name} pickup`, rule);
  }
  for (const [id, rule] of Object.entries(r.upgrade_pickups)) {
    const w = weapon(id);
    if (!w) {
      add(`weapon:${id}`, `Unknown weapon ${id}.`);
      continue;
    }
    const a = availabilityOf(r, id);
    if (a === "Banned") add(`weapon:${id}`, `${w.name} is banned; its upgrades cannot be pickups.`);
    else if (!asPickup(a)) add(`weapon:${id}`, `${w.name} is ${a}; its upgrades cannot be pickups.`);
    else if (w.maxLevel <= 1) add(`weapon:${id}`, `${w.name} has no upgrade levels.`);
    else checkRule(`weapon:${id}`, `${w.name} upgrade`, rule);
  }
  for (const [id, rule] of Object.entries(r.gadget_pickups)) {
    const g = gadget(id);
    if (!g) {
      add(`gadget:${id}`, `Unknown gadget ${id}.`);
      continue;
    }
    const a = gadgetAvailabilityOf(r, id);
    if (!asPickup(a)) add(`gadget:${id}`, `Gadget ${g.name} is ${a} and cannot be a pickup.`);
    else checkRule(`gadget:${id}`, `${g.name} pickup`, rule);
  }
  const s = r.scrap_pickup;
  if (s && (!finiteNonNeg(s.weight) || !finiteNonNeg(s.respawn_secs) || !(Number.isFinite(s.amount) && s.amount > 0))) {
    add("scrap", "Scrap piles need weight >= 0, respawn >= 0 and an amount > 0.");
  }
  return out;
}

export type CodeResult = { ok: true; code: string; problems: RuleProblem[] } | { ok: false; problems: RuleProblem[] };

/**
 * Validates and encodes: a code only for rules the server accepts and the code
 * format can carry exactly.
 */
export function rulesToCode(r: RuleSetData, catalog: Catalog): CodeResult {
  const problems = validateRules(r, catalog);
  let code: string | null = null;
  try {
    code = encodeRules(r, catalog);
  } catch (err) {
    if (!(err instanceof RuleCodeError)) throw err;
    problems.push({ field: "code", message: `Not expressible as a code: ${err.message}.` });
  }
  return code !== null && problems.length === 0 ? { ok: true, code, problems } : { ok: false, problems };
}
