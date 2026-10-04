// Editing operations of the rule set editor, invite links and the host's
// command line. Pure functions over `RuleSetData` (no DOM), so Node tests cover
// them. Edits keep the rule set consistent the way the server expects: banning a
// weapon also takes it out of the pickups and the loadout (like
// `RuleSet::ban_weapon`), making it a pickup again restores its default pickup.

import { asPickup, availabilityOf, cloneRules, eqRule, gadgetAvailabilityOf, inLoadout, normalizeRuleCode } from "./code.ts";
import type { Availability, Catalog, ModeKindName, PickupRule, RuleSetData, ShipTypeName, WeaponTuning } from "./types.ts";

/** A fresh copy of a mode's default rules (Paint the Void by default). */
export function defaultRules(catalog: Catalog, kind: ModeKindName = "PaintTheVoid"): RuleSetData {
  return cloneRules(catalog.defaults[kind]);
}

/** The default rules of the rule set's mode. */
function modeDefaults(r: RuleSetData, catalog: Catalog): RuleSetData {
  return catalog.defaults[r.mode.kind] ?? catalog.defaults.PaintTheVoid;
}

/** The pickup a weapon gets when it becomes a pickup again: the default one, else a plain rule. */
function defaultPickup(r: RuleSetData, catalog: Catalog, id: string): PickupRule {
  const d = modeDefaults(r, catalog).pickups[id];
  if (d) return { ...d };
  const w = catalog.weapons.find((x) => x.id === id);
  return { weight: 1, respawn_secs: 30, ammo: w?.count ? 2 : 0, max_on_map: 0 };
}

export function setWeaponAvailability(r: RuleSetData, catalog: Catalog, id: string, a: Availability): void {
  if (a === "Banned") delete r.weapons[id];
  else r.weapons[id] = a;
  if (asPickup(a)) {
    r.pickups[id] ??= defaultPickup(r, catalog, id);
    const up = modeDefaults(r, catalog).upgrade_pickups[id];
    if (up && !r.upgrade_pickups[id]) r.upgrade_pickups[id] = { ...up };
  } else {
    delete r.pickups[id];
    delete r.upgrade_pickups[id];
  }
  if (!inLoadout(a)) {
    const lo = r.starting_loadout;
    lo.primaries = lo.primaries.filter((w) => w !== id);
    lo.secondaries = lo.secondaries.filter(([w]) => w !== id);
  }
}

export function setGadgetAvailability(r: RuleSetData, catalog: Catalog, id: string, a: Availability): void {
  if (a === "Banned") delete r.gadgets[id];
  else r.gadgets[id] = a;
  if (asPickup(a)) {
    const d = modeDefaults(r, catalog).gadget_pickups[id];
    r.gadget_pickups[id] ??= d ? { ...d } : { weight: 0.5, respawn_secs: 45, ammo: 0, max_on_map: 0 };
  } else {
    delete r.gadget_pickups[id];
  }
  if (!inLoadout(a) && r.starting_loadout.gadget === id) r.starting_loadout.gadget = null;
}

/** Adds or removes a primary from the starting loadout. */
export function setLoadoutPrimary(r: RuleSetData, id: string, on: boolean): void {
  const lo = r.starting_loadout;
  lo.primaries = lo.primaries.filter((w) => w !== id);
  if (on) lo.primaries.push(id);
}

/** Sets a secondary's starting count (0 removes it), keeping its position. */
export function setLoadoutSecondary(r: RuleSetData, id: string, n: number): void {
  const lo = r.starting_loadout;
  const i = lo.secondaries.findIndex(([w]) => w === id);
  if (n <= 0) {
    if (i >= 0) lo.secondaries.splice(i, 1);
  } else if (i >= 0) {
    lo.secondaries[i] = [id, n];
  } else {
    lo.secondaries.push([id, n]);
  }
}

export function loadoutCount(r: RuleSetData, id: string): number {
  return r.starting_loadout.secondaries.find(([w]) => w === id)?.[1] ?? 0;
}

export function setLoadoutGadget(r: RuleSetData, id: string | null): void {
  r.starting_loadout.gadget = id;
}

/** Changes fields of a weapon's pickup rule (no-op if it is not a pickup). */
export function setPickup(r: RuleSetData, id: string, patch: Partial<PickupRule>): void {
  const p = r.pickups[id];
  if (p) r.pickups[id] = { ...p, ...patch };
}

export function setUpgradePickup(r: RuleSetData, id: string, patch: Partial<PickupRule> | null): void {
  if (patch === null) {
    delete r.upgrade_pickups[id];
    return;
  }
  r.upgrade_pickups[id] = { ...(r.upgrade_pickups[id] ?? { weight: 1, respawn_secs: 30, ammo: 0, max_on_map: 0 }), ...patch };
}

export function setGadgetPickup(r: RuleSetData, id: string, patch: Partial<PickupRule>): void {
  const p = r.gadget_pickups[id];
  if (p) r.gadget_pickups[id] = { ...p, ...patch };
}

export const NEUTRAL_TUNING: WeaponTuning = { damage: 1, fire_rate: 1, ammo: 1 };

export function tuningOf(r: RuleSetData, id: string): WeaponTuning {
  return r.tuning[id] ?? NEUTRAL_TUNING;
}

/** Sets tuning multipliers; all 1x removes the entry (shorter codes). */
export function setTuning(r: RuleSetData, id: string, patch: Partial<WeaponTuning>): void {
  const t = { ...tuningOf(r, id), ...patch };
  if (t.damage === 1 && t.fire_rate === 1 && t.ammo === 1) delete r.tuning[id];
  else r.tuning[id] = t;
}

/** Friendly fire choices: off, then 10% to 100%. */
export const FRIENDLY_FIRE_STEPS: readonly number[] = [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100];

export function setShipTypeAllowed(r: RuleSetData, catalog: Catalog, t: ShipTypeName, on: boolean): void {
  const set = new Set(r.allowed_ship_types);
  if (on) set.add(t);
  else set.delete(t);
  r.allowed_ship_types = catalog.shipTypes.filter((x) => set.has(x));
}

/** Whether the weapon's pickup rule differs from the mode default (editor highlight). */
export function pickupChanged(r: RuleSetData, catalog: Catalog, id: string): boolean {
  return !eqRule(r.pickups[id], modeDefaults(r, catalog).pickups[id]);
}

/** Weapons banned by the rule set, by name. */
export function bannedWeapons(r: RuleSetData, catalog: Catalog): string[] {
  return catalog.weapons.filter((w) => availabilityOf(r, w.id) === "Banned").map((w) => w.name);
}

/** A short human summary (e.g. for the code field's caption). */
export function rulesSummary(r: RuleSetData, catalog: Catalog): string {
  const banned = bannedWeapons(r, catalog);
  const ff = r.friendly_fire_percent === 0 ? "off" : `${r.friendly_fire_percent}%`;
  const minutes = r.mode.match_secs / 60;
  const gadgets = catalog.gadgets.filter((g) => gadgetAvailabilityOf(r, g.id) !== "Banned").length;
  return [
    `${r.name || "Unnamed"}: ${Number.isInteger(minutes) ? minutes : minutes.toFixed(1)} min`,
    `friendly fire ${ff}`,
    `ships: ${r.allowed_ship_types.join(", ") || "none"}`,
    banned.length ? `banned: ${banned.join(", ")}` : "no weapon banned",
    `${gadgets} gadget${gadgets === 1 ? "" : "s"}`,
  ].join(" · ");
}

// ---------------------------------------------------------------------------
// Invite links

/** Query parameter names of an invite link. */
export const INVITE_WORLD_PARAM = "world";
export const INVITE_RULES_PARAM = "rules";

export interface Invite {
  /** World share code (`LS...`), or null. */
  world: string | null;
  /** Rule set code (`LR...`), or null. */
  rules: string | null;
}

/** `https://host/path?world=LS..&rules=LR..` (codes without dashes). */
export function inviteLink(base: string, invite: Invite): string {
  const url = new URL(base);
  url.search = "";
  url.hash = "";
  const clean = (c: string): string => c.toUpperCase().replace(/[\s-]+/g, "");
  if (invite.world) url.searchParams.set(INVITE_WORLD_PARAM, clean(invite.world));
  if (invite.rules) url.searchParams.set(INVITE_RULES_PARAM, clean(invite.rules));
  return url.toString();
}

/** Reads the codes of an invite link (or any page URL); absent ones are null. */
export function parseInvite(url: string | URL): Invite {
  const u = typeof url === "string" ? new URL(url) : url;
  const get = (k: string): string | null => {
    const v = u.searchParams.get(k)?.trim();
    return v ? v : null;
  };
  const rules = get(INVITE_RULES_PARAM);
  return { world: get(INVITE_WORLD_PARAM), rules: rules === null ? null : normalizeRuleCode(rules) };
}

/** The command line a host runs to play these rules (and world). */
export function serverCommand(invite: Invite): string {
  const parts = ["ls-server"];
  if (invite.world) parts.push("--code", invite.world);
  if (invite.rules) parts.push("--ruleset", invite.rules);
  return parts.join(" ");
}
