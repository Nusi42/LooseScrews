// Rule set codes (`LR...`) in TypeScript: a bit-exact mirror of
// crates/ls-sim/src/rulecode.rs (format 5, full or diff against the mode's default rule
// set, every mode, comedy event bans, pigment pickups; the M3 format 4 and the Paint the
// Void-only M2 formats 2 = full and 3 = diff still decode; see the module docs there). The Node tests check this
// file against the vectors the Rust side generates into catalog.ts, so a code made
// in the browser is exactly the code the server decodes.

import {
  AVAILABILITIES,
  type Availability,
  type Catalog,
  type Loadout,
  type ModeInfo,
  type ModeKindName,
  type ModeRules,
  type PickupRule,
  type PigmentPickupRule,
  type RuleSetData,
  type ScrapPickupRule,
  type ShieldInteraction,
  type ShipTypeName,
  type WeaponTuning,
} from "./types.ts";

export const RULE_CODE_PREFIX = "LR";
/** The format this build prints (full and diff, every mode, event bans, pigment). */
export const CODE_VERSION = 5;
/** The M3 format (every mode, mutators), read only. */
export const LEGACY_V4_VERSION = 4;
/** Absurdity of every built-in rule set before M4 (the base of format 2-4 diff codes). */
export const LEGACY_ABSURDITY = 20;
/** The M2 formats (Paint the Void only), read only. */
export const LEGACY_FULL_VERSION = 2;
export const LEGACY_DIFF_VERSION = 3;
const MODE_BITS = 4;
export const MAX_NAME = 31;
const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const NAME_CHARS = " ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-";
const WEAPON_BITS = 6;
const GADGET_BITS = 4;
const EVENT_BITS = 5;

/** The registry ids a code covers (a prefix of each registry). */
interface Ids {
  weapons: readonly string[];
  gadgets: readonly string[];
  events: readonly string[];
}

/** Encoding or decoding failed; `message` is meant for players. */
export class RuleCodeError extends Error {
  override name = "RuleCodeError";
}

function fail(message: string): never {
  throw new RuleCodeError(message);
}

class Bits {
  bits: number[] = [];

  push(v: number, n: number): void {
    for (let i = n - 1; i >= 0; i--) this.bits.push(Math.floor(v / 2 ** i) % 2);
  }

  flag(v: boolean): void {
    this.bits.push(v ? 1 : 0);
  }
}

function pack(bits: readonly number[]): number[] {
  const out = new Array<number>(Math.ceil(bits.length / 8)).fill(0);
  bits.forEach((b, i) => {
    if (b) out[i >> 3] = (out[i >> 3] ?? 0) | (0x80 >> (i % 8));
  });
  return out;
}

class Reader {
  pos = 0;
  readonly bits: readonly number[];

  constructor(bits: readonly number[]) {
    this.bits = bits;
  }

  read(n: number): number {
    if (this.pos + n > this.bits.length) fail("code is too short");
    let v = 0;
    for (let i = 0; i < n; i++) v = v * 2 + (this.bits[this.pos++] ?? 0);
    return v;
  }

  flag(): boolean {
    return this.read(1) === 1;
  }
}

export function crc16(bytes: readonly number[]): number {
  let crc = 0xffff;
  for (const b of bytes) {
    crc ^= b << 8;
    for (let i = 0; i < 8; i++) crc = crc & 0x8000 ? ((crc << 1) ^ 0x1021) & 0xffff : (crc << 1) & 0xffff;
  }
  return crc;
}

/** `v` as an exact multiple of `1/steps` in `0..=max` steps (Rust `fixed`). */
function fixed(v: number, steps: number, max: number, what: string): number {
  if (!Number.isFinite(v) || v < 0) fail(`${what} ${v} must be >= 0`);
  const q = Math.round(v * steps);
  if (q > max || q / steps !== v) fail(`${what} ${v} is not a multiple of ${1 / steps} up to ${max / steps}`);
  return q;
}

function indexOf(ids: readonly string[], id: string, what: string): number {
  const i = ids.indexOf(id);
  if (i < 0) fail(`${what} \`${id}\` has no rule set code index`);
  return i;
}

function checkInt(v: number, max: number, what: string): number {
  if (!Number.isInteger(v) || v < 0 || v > max) fail(`${what} must be a whole number 0..${max}`);
  return v;
}

function writeRule(b: Bits, r: PickupRule, what: string): void {
  b.push(fixed(r.weight, 4, 255, what), 8);
  b.push(fixed(r.respawn_secs, 1, 1023, what), 10);
  if (r.ammo > 63 || r.max_on_map > 15) fail(`${what}: ammo <= 63 and max_on_map <= 15`);
  checkInt(r.ammo, 63, `${what} ammo`);
  checkInt(r.max_on_map, 15, `${what} max_on_map`);
  b.flag(r.ammo !== 0);
  if (r.ammo !== 0) b.push(r.ammo, 6);
  b.flag(r.max_on_map !== 0);
  if (r.max_on_map !== 0) b.push(r.max_on_map, 4);
}

function readRule(r: Reader): PickupRule {
  const weight = r.read(8) / 4;
  const respawn_secs = r.read(10);
  const ammo = r.flag() ? r.read(6) : 0;
  const max_on_map = r.flag() ? r.read(4) : 0;
  return { weight, respawn_secs, ammo, max_on_map };
}

function writeName(b: Bits, name: string): void {
  if (new TextEncoder().encode(name).length > MAX_NAME) fail(`name longer than ${MAX_NAME} characters`);
  b.push(name.length, 5);
  for (const c of name) {
    const i = NAME_CHARS.indexOf(c);
    if (i < 0) fail("name may only use letters, digits, spaces and '-'");
    b.push(i, 6);
  }
}

function readName(r: Reader): string {
  const len = r.read(5);
  let name = "";
  for (let i = 0; i < len; i++) name += NAME_CHARS[r.read(6)];
  return name;
}

/** Paint the Void settings in formats 2 and 3. */
function legacyWriteMode(b: Bits, m: ModeRules): void {
  if (m.kind !== "PaintTheVoid") fail("format 2 carries only Paint the Void");
  if (!Number.isInteger(m.teams) || m.teams < 0 || m.teams > 7) fail("teams must be <= 7");
  b.push(m.teams, 3);
  b.push(fixed(m.match_secs, 1, 65535, "match_secs"), 16);
  b.push(fixed(m.respawn_secs, 10, 255, "respawn_secs"), 8);
}

function legacyReadMode(r: Reader, catalog: Catalog): ModeRules {
  const teams = r.read(3);
  const match_secs = r.read(16);
  const respawn_secs = r.read(8) / 10;
  return { ...catalog.defaults.PaintTheVoid.mode, teams, match_secs, respawn_secs };
}

/** The catalog entry of a mode. */
export function modeInfo(kind: string, catalog: Catalog): ModeInfo {
  return catalog.modes.find((m) => m.kind === kind) ?? fail(`unknown mode ${kind}`);
}

function modeField(m: ModeRules, name: string): number {
  const v = m[name];
  return typeof v === "number" ? v : NaN;
}

/** Format 4 mode settings: every field of the mode, in order (Rust `write_mode_fields`). */
function writeModeFields(b: Bits, m: ModeRules, catalog: Catalog): void {
  for (const f of modeInfo(m.kind, catalog).fields) {
    b.push(fixed(modeField(m, f.name), f.steps, 2 ** f.bits - 1, f.name), f.bits);
  }
}

function readModeFields(r: Reader, info: ModeInfo, catalog: Catalog): ModeRules {
  const m: ModeRules = { ...catalog.defaults[info.kind].mode };
  for (const f of info.fields) m[f.name] = r.read(f.bits) / f.steps;
  return m;
}

function percent(v: number, what: string): number {
  if (!Number.isInteger(v) || v < 0 || v > 100) fail(`${what} must be <= 100`);
  return v;
}

function shipMask(types: readonly ShipTypeName[], catalog: Catalog): number {
  let m = 0;
  for (const t of types) {
    const i = catalog.shipTypes.indexOf(t);
    if (i < 0) fail(`unknown ship type ${t}`);
    m |= 1 << i;
  }
  return m;
}

function shipsFromMask(mask: number, catalog: Catalog): ShipTypeName[] {
  return catalog.shipTypes.filter((_, i) => mask & (1 << i));
}

function mutatorMask(mutators: readonly string[], catalog: Catalog): number {
  let m = 0;
  for (const x of mutators) {
    const i = catalog.mutators.indexOf(x);
    if (i < 0) fail(`unknown mutator ${x}`);
    m |= 1 << i;
  }
  return m;
}

function mutatorsFromMask(mask: number, catalog: Catalog): string[] {
  return catalog.mutators.filter((_, i) => mask & (1 << i));
}

function knownMutators(catalog: Catalog): number {
  return (1 << catalog.mutators.length) - 1;
}

function writeLoadout(b: Bits, lo: Loadout, weapons: readonly string[], gadgets: readonly string[]): void {
  if (lo.primaries.length > 31 || lo.secondaries.length > 15) fail("loadout too large");
  b.push(lo.primaries.length, 5);
  for (const id of lo.primaries) b.push(indexOf(weapons, id, "weapon"), WEAPON_BITS);
  b.push(lo.secondaries.length, 4);
  for (const [id, n] of lo.secondaries) {
    b.push(indexOf(weapons, id, "weapon"), WEAPON_BITS);
    if (!Number.isInteger(n) || n < 0 || n > 255) fail("loadout counts must be <= 255");
    b.push(n, 8);
  }
  b.push(lo.gadget === null ? 0 : indexOf(gadgets, lo.gadget, "gadget") + 1, 3);
}

function readLoadout(r: Reader, weapons: readonly string[], gadgets: readonly string[]): Loadout {
  const weaponId = (i: number): string => weapons[i] ?? fail("weapon index out of range");
  const primaries: string[] = [];
  for (let n = r.read(5); n > 0; n--) primaries.push(weaponId(r.read(WEAPON_BITS)));
  const secondaries: [string, number][] = [];
  for (let n = r.read(4); n > 0; n--) {
    const id = weaponId(r.read(WEAPON_BITS));
    secondaries.push([id, r.read(8)]);
  }
  const g = r.read(3);
  const gadget = g === 0 ? null : (gadgets[g - 1] ?? fail("gadget index out of range"));
  return { primaries, secondaries, gadget };
}

function writeScrap(b: Bits, s: ScrapPickupRule): void {
  b.push(fixed(s.weight, 4, 255, "scrap weight"), 8);
  b.push(fixed(s.respawn_secs, 1, 1023, "scrap respawn"), 10);
  b.push(fixed(s.amount, 1, 255, "scrap amount"), 8);
}

function readScrap(r: Reader): ScrapPickupRule {
  const weight = r.read(8) / 4;
  const respawn_secs = r.read(10);
  const amount = r.read(8);
  return { weight, respawn_secs, amount };
}

function writePigment(b: Bits, p: PigmentPickupRule): void {
  checkInt(p.respawn_secs, 1023, "pigment respawn");
  checkInt(p.players_per_pickup, 15, "pigment players per pickup");
  checkInt(p.min_on_map, 15, "pigment minimum on the map");
  b.push(p.respawn_secs, 10);
  b.push(p.players_per_pickup, 4);
  b.push(p.min_on_map, 4);
}

function readPigment(r: Reader): PigmentPickupRule {
  const respawn_secs = r.read(10);
  const players_per_pickup = r.read(4);
  const min_on_map = r.read(4);
  return { respawn_secs, players_per_pickup, min_on_map };
}

function writeShield(b: Bits, s: ShieldInteraction): void {
  const [kind, v] =
    s.kind === "Blocked"
      ? [0, 0]
      : s.kind === "Wears"
        ? [1, fixed(s.value, 100, 255, "shield wear")]
        : [2, fixed(s.value, 100, 100, "shield pass-through")];
  b.push(kind, 2);
  b.push(v, 8);
}

function readShield(r: Reader): ShieldInteraction {
  const kind = r.read(2);
  const value = r.read(8) / 100;
  if (kind === 0) return { kind: "Blocked", value: 0 };
  if (kind === 1) return { kind: "Wears", value };
  if (kind === 2) return { kind: "PassThrough", value };
  fail("unknown shield interaction");
}

function writeTuning(b: Bits, t: WeaponTuning): void {
  b.push(fixed(t.damage, 20, 255, "damage multiplier"), 8);
  b.push(fixed(t.fire_rate, 20, 255, "fire rate multiplier"), 8);
  b.push(fixed(t.ammo, 20, 255, "ammo multiplier"), 8);
}

function readTuning(r: Reader): WeaponTuning {
  const damage = r.read(8) / 20;
  const fire_rate = r.read(8) / 20;
  const ammo = r.read(8) / 20;
  return { damage, fire_rate, ammo };
}

// ---------------------------------------------------------------------------
// Equality and canonical form

export function eqRule(a: PickupRule | undefined, b: PickupRule | undefined): boolean {
  if (!a || !b) return a === b;
  return a.weight === b.weight && a.respawn_secs === b.respawn_secs && a.ammo === b.ammo && a.max_on_map === b.max_on_map;
}

function eqScrap(a: ScrapPickupRule | null, b: ScrapPickupRule | null): boolean {
  if (!a || !b) return a === b;
  return a.weight === b.weight && a.respawn_secs === b.respawn_secs && a.amount === b.amount;
}

function eqPigment(a: PigmentPickupRule | null, b: PigmentPickupRule | null): boolean {
  if (!a || !b) return a === b;
  return a.respawn_secs === b.respawn_secs && a.players_per_pickup === b.players_per_pickup && a.min_on_map === b.min_on_map;
}

/** Whether the rule set bans comedy event `id`. */
export function eventBanned(r: RuleSetData, id: string): boolean {
  return r.events[id] === "Banned";
}

/** Bans (or allows) a comedy event. */
export function setEventBanned(r: RuleSetData, id: string, banned: boolean): void {
  if (banned) r.events[id] = "Banned";
  else delete r.events[id];
}

function eqShield(a: ShieldInteraction | undefined, b: ShieldInteraction | undefined): boolean {
  if (!a || !b) return a === b;
  // Rust compares the enum: Blocked carries no value.
  return a.kind === b.kind && (a.kind === "Blocked" || a.value === b.value);
}

function eqTuning(a: WeaponTuning | undefined, b: WeaponTuning | undefined): boolean {
  if (!a || !b) return a === b;
  return a.damage === b.damage && a.fire_rate === b.fire_rate && a.ammo === b.ammo;
}

/** Same mode with the same settings (every key of either side). */
export function eqMode(a: ModeRules, b: ModeRules): boolean {
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
  for (const k of keys) if (a[k] !== b[k]) return false;
  return true;
}

export function eqLoadout(a: Loadout, b: Loadout): boolean {
  return (
    a.gadget === b.gadget &&
    a.primaries.length === b.primaries.length &&
    a.primaries.every((p, i) => p === b.primaries[i]) &&
    a.secondaries.length === b.secondaries.length &&
    a.secondaries.every(([id, n], i) => id === b.secondaries[i]?.[0] && n === b.secondaries[i]?.[1])
  );
}

function eqMap<T>(a: Record<string, T>, b: Record<string, T>, eq: (x: T | undefined, y: T | undefined) => boolean): boolean {
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
  for (const k of keys) if (!eq(a[k], b[k])) return false;
  return true;
}

/** Deep equality of two rule sets (as Rust's `PartialEq`, maps unordered). */
export function rulesEqual(a: RuleSetData, b: RuleSetData): boolean {
  return (
    a.name === b.name &&
    eqMode(a.mode, b.mode) &&
    eqMap(a.weapons, b.weapons, (x, y) => x === y) &&
    eqMap(a.gadgets, b.gadgets, (x, y) => x === y) &&
    eqLoadout(a.starting_loadout, b.starting_loadout) &&
    eqMap(a.pickups, b.pickups, eqRule) &&
    eqMap(a.upgrade_pickups, b.upgrade_pickups, eqRule) &&
    eqMap(a.gadget_pickups, b.gadget_pickups, eqRule) &&
    eqScrap(a.scrap_pickup, b.scrap_pickup) &&
    eqMap(a.shield_overrides, b.shield_overrides, eqShield) &&
    eqMap(a.tuning, b.tuning, eqTuning) &&
    a.friendly_fire_percent === b.friendly_fire_percent &&
    a.allowed_ship_types.length === b.allowed_ship_types.length &&
    a.allowed_ship_types.every((t, i) => t === b.allowed_ship_types[i]) &&
    a.absurdity === b.absurdity &&
    a.mutators.length === b.mutators.length &&
    a.mutators.every((m, i) => m === b.mutators[i]) &&
    eqMap(a.events, b.events, (x, y) => (x ?? "On") === (y ?? "On")) &&
    eqPigment(a.pigment_pickup, b.pigment_pickup)
  );
}

/** A deep copy. */
export function cloneRules(r: RuleSetData): RuleSetData {
  return structuredClone(r);
}

export function availabilityOf(r: RuleSetData, id: string): Availability {
  return r.weapons[id] ?? "Banned";
}

export function gadgetAvailabilityOf(r: RuleSetData, id: string): Availability {
  return r.gadgets[id] ?? "Banned";
}

export function asPickup(a: Availability): boolean {
  return a === "LoadoutAndPickup" || a === "PickupOnly";
}

export function inLoadout(a: Availability): boolean {
  return a === "LoadoutAndPickup" || a === "LoadoutOnly";
}

/** What decoding returns: banned entries and events that are on left out, ship types in index order. */
export function canonical(r: RuleSetData, catalog: Catalog): RuleSetData {
  const c = cloneRules(r);
  for (const [k, a] of Object.entries(c.weapons)) if (a === "Banned") delete c.weapons[k];
  for (const [k, a] of Object.entries(c.gadgets)) if (a === "Banned") delete c.gadgets[k];
  for (const [k, e] of Object.entries(c.events)) if (e === "On") delete c.events[k];
  c.allowed_ship_types = shipsFromMask(shipMask(r.allowed_ship_types, catalog), catalog);
  c.mutators = mutatorsFromMask(mutatorMask(r.mutators, catalog), catalog);
  return c;
}

function checkIds(r: RuleSetData, ids: Ids): void {
  const { weapons, gadgets } = ids;
  for (const id of Object.keys(r.events)) indexOf(ids.events, id, "event");
  const lo = r.starting_loadout;
  for (const id of [
    ...Object.keys(r.weapons),
    ...Object.keys(r.pickups),
    ...Object.keys(r.upgrade_pickups),
    ...Object.keys(r.shield_overrides),
    ...Object.keys(r.tuning),
    ...lo.primaries,
    ...lo.secondaries.map(([w]) => w),
  ]) {
    indexOf(weapons, id, "weapon");
  }
  for (const id of [...Object.keys(r.gadgets), ...Object.keys(r.gadget_pickups), ...(lo.gadget === null ? [] : [lo.gadget])]) {
    indexOf(gadgets, id, "gadget");
  }
}

function pick<T>(map: Record<string, T>, keep: (id: string) => boolean): Record<string, T> {
  const out: Record<string, T> = {};
  for (const [k, v] of Object.entries(map)) if (keep(k)) out[k] = v;
  return out;
}

/** The mode default restricted to the covered weapons, gadgets and events (Rust `base_for`). */
function baseFor(kind: ModeKindName, ids: Ids, catalog: Catalog): RuleSetData {
  const { weapons, gadgets } = ids;
  const base = cloneRules(catalog.defaults[kind]);
  base.events = pick(base.events, (id) => ids.events.includes(id));
  const w = (id: string): boolean => weapons.includes(id);
  const g = (id: string): boolean => gadgets.includes(id);
  base.weapons = pick(base.weapons, w);
  base.pickups = pick(base.pickups, w);
  base.upgrade_pickups = pick(base.upgrade_pickups, w);
  base.shield_overrides = pick(base.shield_overrides, w);
  base.tuning = pick(base.tuning, w);
  base.gadgets = pick(base.gadgets, g);
  base.gadget_pickups = pick(base.gadget_pickups, g);
  const lo = base.starting_loadout;
  lo.primaries = lo.primaries.filter(w);
  lo.secondaries = lo.secondaries.filter(([id]) => w(id));
  if (lo.gadget !== null && !g(lo.gadget)) lo.gadget = null;
  return canonical(base, catalog);
}

/** The base of format 2-4 diff codes: the M3 mode default (Rust `legacy_base_for`). */
function legacyBaseFor(kind: ModeKindName, ids: Ids, catalog: Catalog): RuleSetData {
  const base = baseFor(kind, ids, catalog);
  base.absurdity = LEGACY_ABSURDITY;
  base.pigment_pickup = null;
  base.events = {};
  return base;
}

/** A format 2-4 decode gets today's pigment default and no event bans (Rust `with_m4_fields`). */
function withM4Fields(r: RuleSetData, catalog: Catalog): RuleSetData {
  const p = catalog.defaults[r.mode.kind].pigment_pickup;
  r.pigment_pickup = p === null ? null : { ...p };
  r.events = {};
  return r;
}

/** Drops pickups and loadout entries the availability no longer allows (Rust `implied_by_availability`). */
export function impliedByAvailability(r: RuleSetData): void {
  const wa = (id: string): Availability => availabilityOf(r, id);
  const ga = (id: string): Availability => gadgetAvailabilityOf(r, id);
  r.pickups = pick(r.pickups, (id) => asPickup(wa(id)));
  r.upgrade_pickups = pick(r.upgrade_pickups, (id) => asPickup(wa(id)));
  r.gadget_pickups = pick(r.gadget_pickups, (id) => asPickup(ga(id)));
  const lo = r.starting_loadout;
  lo.primaries = lo.primaries.filter((id) => inLoadout(wa(id)));
  lo.secondaries = lo.secondaries.filter(([id]) => inLoadout(wa(id)));
  if (lo.gadget !== null && !inLoadout(ga(lo.gadget))) lo.gadget = null;
}

// ---------------------------------------------------------------------------
// Encoding

function headerCounts(b: Bits, ids: Ids, version: number): void {
  if (ids.weapons.length > 63 || ids.gadgets.length > 15 || ids.events.length > 31) {
    fail("too many weapons, gadgets or events for the code format");
  }
  b.push(ids.weapons.length, 6);
  b.push(ids.gadgets.length, 4);
  if (version >= CODE_VERSION) b.push(ids.events.length, EVENT_BITS);
}

function writeAll<T>(b: Bits, ids: readonly string[], map: Record<string, T>, write: (b: Bits, v: T, id: string) => void): void {
  for (const id of ids) {
    const v = map[id];
    b.flag(v !== undefined);
    if (v !== undefined) write(b, v, id);
  }
}

/** Format 2 payload (legacy; only the fingerprints of format 3 codes use it). */
function legacyFullPayload(r: RuleSetData, ids: Ids, catalog: Catalog): Bits {
  const b = new Bits();
  b.push(LEGACY_FULL_VERSION, 4);
  headerCounts(b, ids, LEGACY_FULL_VERSION);
  writeName(b, r.name);
  b.push(0, 2);
  legacyWriteMode(b, r.mode);
  writeFullTail(b, r, ids, catalog, LEGACY_FULL_VERSION);
  return b;
}

/** Format 5 (or, for fingerprints of M3 codes, 4) full payload. */
function fullPayload(r: RuleSetData, ids: Ids, catalog: Catalog, version: number): Bits {
  const b = new Bits();
  b.push(version, 4);
  headerCounts(b, ids, version);
  b.flag(true);
  b.push(modeInfo(r.mode.kind, catalog).index, MODE_BITS);
  writeName(b, r.name);
  writeModeFields(b, r.mode, catalog);
  writeFullTail(b, r, ids, catalog, version);
  return b;
}

/** The full formats' fields after the mode settings (Rust `write_full_tail`). */
function writeFullTail(b: Bits, r: RuleSetData, ids: Ids, catalog: Catalog, version: number): void {
  const { weapons, gadgets } = ids;
  b.push(percent(r.friendly_fire_percent, "friendly fire"), 7);
  b.push(percent(r.absurdity, "absurdity"), 7);
  b.push(shipMask(r.allowed_ship_types, catalog), 4);
  b.push(mutatorMask(r.mutators, catalog), 4);
  checkIds(r, ids);
  for (const w of weapons) b.push(AVAILABILITIES.indexOf(availabilityOf(r, w)), 2);
  for (const g of gadgets) b.push(AVAILABILITIES.indexOf(gadgetAvailabilityOf(r, g)), 2);
  writeLoadout(b, r.starting_loadout, weapons, gadgets);
  writeAll(b, weapons, r.pickups, writeRule);
  writeAll(b, weapons, r.upgrade_pickups, writeRule);
  writeAll(b, gadgets, r.gadget_pickups, writeRule);
  b.flag(r.scrap_pickup !== null);
  if (r.scrap_pickup !== null) writeScrap(b, r.scrap_pickup);
  if (version >= CODE_VERSION) {
    b.flag(r.pigment_pickup !== null);
    if (r.pigment_pickup !== null) writePigment(b, r.pigment_pickup);
  }
  writeAll(b, weapons, r.shield_overrides, writeShield);
  writeAll(b, weapons, r.tuning, writeTuning);
  if (version >= CODE_VERSION) for (const e of ids.events) b.flag(eventBanned(r, e));
}

function writeChanges(b: Bits, ids: readonly string[], idxBits: number, changed: (id: string) => boolean, write: (id: string) => void): void {
  const list = ids.map((id, i) => [id, i] as const).filter(([id]) => changed(id));
  b.flag(list.length > 0);
  if (list.length === 0) return;
  b.push(list.length - 1, idxBits);
  for (const [id, i] of list) {
    b.push(i, idxBits);
    write(id);
  }
}

function writeMapDiff<T>(
  b: Bits,
  ids: readonly string[],
  idxBits: number,
  map: Record<string, T>,
  base: Record<string, T>,
  eq: (x: T | undefined, y: T | undefined) => boolean,
  write: (b: Bits, v: T, id: string) => void,
): void {
  writeChanges(
    b,
    ids,
    idxBits,
    (id) => !eq(map[id], base[id]),
    (id) => {
      const v = map[id];
      b.flag(v !== undefined);
      if (v !== undefined) write(b, v, id);
    },
  );
}

function fingerprint(base: RuleSetData, ids: Ids, catalog: Catalog, version: number): number {
  const b = fullPayload(base, ids, catalog, version);
  return crc16(pack(b.bits));
}

function legacyFingerprint(base: RuleSetData, ids: Ids, catalog: Catalog): number {
  const b = legacyFullPayload(base, ids, catalog);
  return crc16(pack(b.bits));
}

/** Format 5 diff payload. */
function diffPayload(rules: RuleSetData, ids: Ids, catalog: Catalog): Bits {
  checkIds(rules, ids);
  const r = canonical(rules, catalog);
  const info = modeInfo(r.mode.kind, catalog);
  const base = baseFor(info.kind, ids, catalog);
  const b = new Bits();
  b.push(CODE_VERSION, 4);
  headerCounts(b, ids, CODE_VERSION);
  b.flag(false);
  b.push(info.index, MODE_BITS);
  b.push(fingerprint(base, ids, catalog, CODE_VERSION), 16);

  b.flag(r.name !== base.name);
  if (r.name !== base.name) writeName(b, r.name);
  const modeChanged = !eqMode(r.mode, base.mode);
  b.flag(modeChanged);
  if (modeChanged) writeModeFields(b, r.mode, catalog);
  writeDiffTail(b, r, base, ids, catalog);
  return b;
}

/** The diff format's fields after the mode settings (Rust `write_diff_tail`). */
function writeDiffTail(b: Bits, r: RuleSetData, base: RuleSetData, ids: Ids, catalog: Catalog): void {
  const { weapons, gadgets } = ids;
  for (const [v, bv, what] of [
    [r.friendly_fire_percent, base.friendly_fire_percent, "friendly fire"],
    [r.absurdity, base.absurdity, "absurdity"],
  ] as const) {
    b.flag(v !== bv);
    if (v !== bv) b.push(percent(v, what), 7);
  }
  for (const [v, bv] of [
    [shipMask(r.allowed_ship_types, catalog), shipMask(base.allowed_ship_types, catalog)],
    [mutatorMask(r.mutators, catalog), mutatorMask(base.mutators, catalog)],
  ] as const) {
    b.flag(v !== bv);
    if (v !== bv) b.push(v, 4);
  }
  const avail = (ids: readonly string[], bits: number, get: (x: RuleSetData, id: string) => Availability): void =>
    writeChanges(
      b,
      ids,
      bits,
      (id) => get(r, id) !== get(base, id),
      (id) => b.push(AVAILABILITIES.indexOf(get(r, id)), 2),
    );
  avail(weapons, WEAPON_BITS, availabilityOf);
  avail(gadgets, GADGET_BITS, gadgetAvailabilityOf);
  base = cloneRules(base);
  base.weapons = { ...r.weapons };
  base.gadgets = { ...r.gadgets };
  impliedByAvailability(base);
  const loChanged = !eqLoadout(r.starting_loadout, base.starting_loadout);
  b.flag(loChanged);
  if (loChanged) writeLoadout(b, r.starting_loadout, weapons, gadgets);
  writeMapDiff(b, weapons, WEAPON_BITS, r.pickups, base.pickups, eqRule, writeRule);
  writeMapDiff(b, weapons, WEAPON_BITS, r.upgrade_pickups, base.upgrade_pickups, eqRule, writeRule);
  writeMapDiff(b, gadgets, GADGET_BITS, r.gadget_pickups, base.gadget_pickups, eqRule, writeRule);
  const scrapChanged = !eqScrap(r.scrap_pickup, base.scrap_pickup);
  b.flag(scrapChanged);
  if (scrapChanged) {
    b.flag(r.scrap_pickup !== null);
    if (r.scrap_pickup !== null) writeScrap(b, r.scrap_pickup);
  }
  const pigmentChanged = !eqPigment(r.pigment_pickup, base.pigment_pickup);
  b.flag(pigmentChanged);
  if (pigmentChanged) {
    b.flag(r.pigment_pickup !== null);
    if (r.pigment_pickup !== null) writePigment(b, r.pigment_pickup);
  }
  writeMapDiff(b, weapons, WEAPON_BITS, r.shield_overrides, base.shield_overrides, eqShield, writeShield);
  writeMapDiff(b, weapons, WEAPON_BITS, r.tuning, base.tuning, eqTuning, writeTuning);
  writeChanges(
    b,
    ids.events,
    EVENT_BITS,
    (id) => eventBanned(r, id) !== eventBanned(base, id),
    () => {},
  );
}

function finish(b: Bits): string {
  const crc = crc16(pack(b.bits));
  b.push(crc, 16);
  while (b.bits.length % 5 !== 0) b.bits.push(0);
  let out = RULE_CODE_PREFIX;
  for (let i = 0; i * 5 < b.bits.length; i++) {
    if (i > 0 && i % 4 === 2) out += "-";
    let v = 0;
    for (let j = 0; j < 5; j++) v = v * 2 + (b.bits[i * 5 + j] ?? 0);
    out += ALPHABET[v];
  }
  return out;
}

function ids(catalog: Catalog): Ids {
  return {
    weapons: catalog.weapons.map((w) => w.id),
    gadgets: catalog.gadgets.map((g) => g.id),
    events: catalog.events.map((e) => e.id),
  };
}

/** The short code (diff against the mode default, or the full code when shorter). Throws `RuleCodeError`. */
export function encodeRules(r: RuleSetData, catalog: Catalog): string {
  const all = ids(catalog);
  const full = fullPayload(r, all, catalog, CODE_VERSION);
  const diff = diffPayload(r, all, catalog);
  return finish(diff.bits.length <= full.bits.length ? diff : full);
}

/** The full (format 5) code. Throws `RuleCodeError`. */
export function encodeFullRules(r: RuleSetData, catalog: Catalog): string {
  return finish(fullPayload(r, ids(catalog), catalog, CODE_VERSION));
}

// ---------------------------------------------------------------------------
// Decoding

/** Normalized code text: uppercase, no dashes/whitespace, O->0 and I/L->1 after the prefix. */
export function normalizeRuleCode(raw: string): string {
  const s = raw.toUpperCase().replace(/[\s-]+/g, "");
  if (!s.startsWith(RULE_CODE_PREFIX)) return s;
  return RULE_CODE_PREFIX + s.slice(2).replace(/O/g, "0").replace(/[IL]/g, "1");
}

/** Prints a normalized code in groups of four (`LRab-cdef-...`). */
export function formatRuleCode(raw: string): string {
  const n = normalizeRuleCode(raw);
  const body = n.slice(RULE_CODE_PREFIX.length);
  let out = RULE_CODE_PREFIX;
  for (let i = 0; i < body.length; i++) {
    if (i >= 2 && (i - 2) % 4 === 0) out += "-";
    out += body[i];
  }
  return out;
}

function symbols(code: string): number[] {
  const n = normalizeRuleCode(code);
  if (!n.startsWith(RULE_CODE_PREFIX)) fail("a rule set code starts with LR");
  const bits: number[] = [];
  for (const c of n.slice(RULE_CODE_PREFIX.length)) {
    const v = ALPHABET.indexOf(c);
    if (v < 0) fail(`invalid character \`${c}\``);
    for (let i = 4; i >= 0; i--) bits.push((v >> i) & 1);
  }
  return bits;
}

function readAll<T>(r: Reader, ids: readonly string[], read: (r: Reader) => T): Record<string, T> {
  const out: Record<string, T> = {};
  for (const id of ids) if (r.flag()) out[id] = read(r);
  return out;
}

function readChanges(r: Reader, ids: readonly string[], idxBits: number, apply: (id: string) => void): void {
  if (!r.flag()) return;
  const n = r.read(idxBits) + 1;
  let last = -1;
  for (let k = 0; k < n; k++) {
    const i = r.read(idxBits);
    if (i <= last || i >= ids.length) fail("corrupt code (bad index list)");
    last = i;
    apply(ids[i] as string);
  }
}

function readMapDiff<T>(
  r: Reader,
  ids: readonly string[],
  idxBits: number,
  map: Record<string, T>,
  eq: (x: T | undefined, y: T | undefined) => boolean,
  read: (r: Reader) => T,
): void {
  readChanges(r, ids, idxBits, (id) => {
    const v = r.flag() ? read(r) : undefined;
    if (eq(v, map[id])) fail("corrupt code (an entry equals the default)");
    if (v === undefined) delete map[id];
    else map[id] = v;
  });
}

function readAvailabilityDiff(r: Reader, ids: readonly string[], idxBits: number, map: Record<string, Availability>): void {
  readChanges(r, ids, idxBits, (id) => {
    const a = AVAILABILITIES[r.read(2)] as Availability;
    if (a === (map[id] ?? "Banned")) fail("corrupt code (an entry equals the default)");
    if (a === "Banned") delete map[id];
    else map[id] = a;
  });
}

/** A value error found while parsing; reported only if the checksum matches. */
type Parsed = { rules: RuleSetData } | { error: string };

/** The full formats' fields after the mode settings (Rust `read_full_tail`). */
function readFullTail(r: Reader, ids: Ids, catalog: Catalog, version: number, name: string, mode: ModeRules): Parsed {
  const { weapons, gadgets } = ids;
  const friendly_fire_percent = r.read(7);
  const absurdity = r.read(7);
  const allowed_ship_types = shipsFromMask(r.read(4), catalog);
  const mutatorBits = r.read(4);
  const mutators = mutatorsFromMask(mutatorBits, catalog);
  const wmap: Record<string, Availability> = {};
  for (const w of weapons) {
    const a = AVAILABILITIES[r.read(2)] as Availability;
    if (a !== "Banned") wmap[w] = a;
  }
  const gmap: Record<string, Availability> = {};
  for (const g of gadgets) {
    const a = AVAILABILITIES[r.read(2)] as Availability;
    if (a !== "Banned") gmap[g] = a;
  }
  const starting_loadout = readLoadout(r, weapons, gadgets);
  const pickups = readAll(r, weapons, readRule);
  const upgrade_pickups = readAll(r, weapons, readRule);
  const gadget_pickups = readAll(r, gadgets, readRule);
  const scrap_pickup = r.flag() ? readScrap(r) : null;
  const pigment_pickup = version >= CODE_VERSION && r.flag() ? readPigment(r) : null;
  const shield_overrides = readAll(r, weapons, readShield);
  const tuning = readAll(r, weapons, readTuning);
  const events: Record<string, "Banned"> = {};
  if (version >= CODE_VERSION) for (const e of ids.events) if (r.flag()) events[e] = "Banned";
  if ((mutatorBits & ~knownMutators(catalog)) !== 0) return { error: "unknown mutator (code from a newer build?)" };
  return {
    rules: {
      name,
      mode,
      weapons: wmap,
      gadgets: gmap,
      starting_loadout,
      pickups,
      upgrade_pickups,
      gadget_pickups,
      scrap_pickup,
      shield_overrides,
      tuning,
      friendly_fire_percent,
      allowed_ship_types,
      absurdity,
      mutators,
      events,
      pigment_pickup,
    },
  };
}

function readFullV2(r: Reader, ids: Ids, catalog: Catalog): Parsed {
  const name = readName(r);
  const mode = r.read(2);
  const m = legacyReadMode(r, catalog);
  const parsed = readFullTail(r, ids, catalog, LEGACY_FULL_VERSION, name, m);
  if (mode !== 0) return { error: "unknown mode" };
  return parsed;
}

function readFullV45(r: Reader, info: ModeInfo, ids: Ids, catalog: Catalog, version: number): Parsed {
  const name = readName(r);
  const mode = readModeFields(r, info, catalog);
  return readFullTail(r, ids, catalog, version, name, mode);
}

/** The diff formats' fields after the mode settings; returns whether an unknown mutator bit was set. */
function readDiffTail(r: Reader, out: RuleSetData, ids: Ids, catalog: Catalog, version: number): boolean {
  const { weapons, gadgets } = ids;
  if (r.flag()) out.friendly_fire_percent = r.read(7);
  if (r.flag()) out.absurdity = r.read(7);
  if (r.flag()) out.allowed_ship_types = shipsFromMask(r.read(4), catalog);
  let unknownMutator = false;
  if (r.flag()) {
    const mask = r.read(4);
    unknownMutator = (mask & ~knownMutators(catalog)) !== 0;
    out.mutators = mutatorsFromMask(mask, catalog);
  }
  readAvailabilityDiff(r, weapons, WEAPON_BITS, out.weapons);
  readAvailabilityDiff(r, gadgets, GADGET_BITS, out.gadgets);
  impliedByAvailability(out);
  if (r.flag()) {
    const lo = readLoadout(r, weapons, gadgets);
    if (eqLoadout(lo, out.starting_loadout)) fail("corrupt code (an entry equals the default)");
    out.starting_loadout = lo;
  }
  readMapDiff(r, weapons, WEAPON_BITS, out.pickups, eqRule, readRule);
  readMapDiff(r, weapons, WEAPON_BITS, out.upgrade_pickups, eqRule, readRule);
  readMapDiff(r, gadgets, GADGET_BITS, out.gadget_pickups, eqRule, readRule);
  if (r.flag()) {
    const s = r.flag() ? readScrap(r) : null;
    if (eqScrap(s, out.scrap_pickup)) fail("corrupt code (an entry equals the default)");
    out.scrap_pickup = s;
  }
  if (version >= CODE_VERSION && r.flag()) {
    const p = r.flag() ? readPigment(r) : null;
    if (eqPigment(p, out.pigment_pickup)) fail("corrupt code (an entry equals the default)");
    out.pigment_pickup = p;
  }
  readMapDiff(r, weapons, WEAPON_BITS, out.shield_overrides, eqShield, readShield);
  readMapDiff(r, weapons, WEAPON_BITS, out.tuning, eqTuning, readTuning);
  if (version >= CODE_VERSION) readChanges(r, ids.events, EVENT_BITS, (id) => setEventBanned(out, id, !eventBanned(out, id)));
  return unknownMutator;
}

function diffChecks(out: RuleSetData, fp: number, baseFp: number, unknownMutator: boolean): Parsed {
  if (fp !== baseFp) return { error: "code made against different default rules (another game version); ask for the full code" };
  if (unknownMutator) return { error: "unknown mutator (code from a newer build?)" };
  return { rules: out };
}

function readDiffV3(r: Reader, ids: Ids, catalog: Catalog): Parsed {
  const mode = r.read(2);
  const fp = r.read(16);
  if (mode !== 0) return { error: "unknown mode" };
  const out = legacyBaseFor("PaintTheVoid", ids, catalog);
  const baseFp = legacyFingerprint(out, ids, catalog);
  if (r.flag()) out.name = readName(r);
  if (r.flag()) out.mode = legacyReadMode(r, catalog);
  const unknownMutator = readDiffTail(r, out, ids, catalog, LEGACY_DIFF_VERSION);
  return diffChecks(out, fp, baseFp, unknownMutator);
}

function readDiffV45(r: Reader, info: ModeInfo, ids: Ids, catalog: Catalog, version: number): Parsed {
  const fp = r.read(16);
  const out = version >= CODE_VERSION ? baseFor(info.kind, ids, catalog) : legacyBaseFor(info.kind, ids, catalog);
  const baseFp = fingerprint(out, ids, catalog, version);
  if (r.flag()) out.name = readName(r);
  if (r.flag()) {
    const m = readModeFields(r, info, catalog);
    if (eqMode(m, out.mode)) fail("corrupt code (an entry equals the default)");
    out.mode = m;
  }
  const unknownMutator = readDiffTail(r, out, ids, catalog, version);
  return diffChecks(out, fp, baseFp, unknownMutator);
}

/** Decodes a code of any format this build reads (2 to 5) into canonical rules. Throws `RuleCodeError`. */
export function decodeRules(code: string, catalog: Catalog): RuleSetData {
  const bits = symbols(code);
  const r = new Reader(bits);
  const version = r.read(4);
  if (version < LEGACY_FULL_VERSION || version > CODE_VERSION) {
    fail(`code version ${version}, this build reads versions ${LEGACY_FULL_VERSION} to ${CODE_VERSION}`);
  }
  const nw = r.read(6);
  const ng = r.read(4);
  const all = ids(catalog);
  if (nw > all.weapons.length || ng > all.gadgets.length) {
    fail(
      `code from a newer build (${nw} weapons, ${ng} gadgets; this build knows ${all.weapons.length} and ${all.gadgets.length})`,
    );
  }
  const ne = version >= CODE_VERSION ? r.read(EVENT_BITS) : 0;
  if (ne > all.events.length) fail(`code from a newer build (${ne} comedy events; this build knows ${all.events.length})`);
  // Weapons and gadgets the code does not cover stay banned; events it does not cover stay on.
  const covered: Ids = { weapons: all.weapons.slice(0, nw), gadgets: all.gadgets.slice(0, ng), events: all.events.slice(0, ne) };
  let parsed: Parsed;
  if (version === LEGACY_FULL_VERSION) parsed = readFullV2(r, covered, catalog);
  else if (version === LEGACY_DIFF_VERSION) parsed = readDiffV3(r, covered, catalog);
  else {
    const full = r.flag();
    const index = r.read(MODE_BITS);
    const info = catalog.modes.find((m) => m.index === index) ?? fail(`unknown mode ${index} (code from a newer build?)`);
    parsed = full ? readFullV45(r, info, covered, catalog, version) : readDiffV45(r, info, covered, catalog, version);
  }
  if (version < CODE_VERSION && "rules" in parsed) parsed = { rules: withM4Fields(parsed.rules, catalog) };
  const payload = r.pos;
  const crc = r.read(16);
  const rest = bits.slice(r.pos);
  if (rest.length >= 5 || rest.some((b) => b)) fail("trailing data after the checksum");
  if (crc16(pack(bits.slice(0, payload))) !== crc) fail("checksum mismatch (typo?)");
  if ("error" in parsed) fail(parsed.error);
  return parsed.rules;
}

/** The format version of a code (5; 4 for M3 codes, 2 and 3 for M2 codes), or null if it has none. */
export function codeVersion(code: string): number | null {
  try {
    return new Reader(symbols(code)).read(4);
  } catch {
    return null;
  }
}

/** Whether a code is a full code (format 4/5 full or format 2), a diff, or unreadable (null). */
export function codeForm(code: string): "full" | "diff" | null {
  try {
    const r = new Reader(symbols(code));
    const version = r.read(4);
    if (version === LEGACY_FULL_VERSION) return "full";
    if (version === LEGACY_DIFF_VERSION) return "diff";
    if (version !== CODE_VERSION && version !== LEGACY_V4_VERSION) return null;
    r.read(version === CODE_VERSION ? 15 : 10);
    return r.flag() ? "full" : "diff";
  } catch {
    return null;
  }
}
