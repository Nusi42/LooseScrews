// Rule set shapes for the browser rule set editor. They mirror `ls_sim::RuleSet`
// (crates/ls-sim/src/ruleset.rs) field by field, with the Rust field names, so the
// generated catalog (catalog.ts) can carry rule sets as plain JSON.

export type Availability = "LoadoutAndPickup" | "PickupOnly" | "LoadoutOnly" | "Banned";

/** In the order of `Availability::index` (rule set codes). */
export const AVAILABILITIES: readonly Availability[] = ["LoadoutAndPickup", "PickupOnly", "LoadoutOnly", "Banned"];

export type ShipTypeName = "Skitter" | "Tinker" | "Hauler" | "Tug";

export interface PickupRule {
  weight: number;
  respawn_secs: number;
  /** Ammo granted for count weapons (ignored otherwise). */
  ammo: number;
  /** At most this many on the map at once (0 = no limit). */
  max_on_map: number;
}

export interface ScrapPickupRule {
  weight: number;
  respawn_secs: number;
  amount: number;
}

/** Pigment pickups on monochrome levels (`ls_sim::PigmentPickupRule`). */
export interface PigmentPickupRule {
  /** Whole seconds, 0..=1023. */
  respawn_secs: number;
  /** One pickup per this many players, 1..=15. */
  players_per_pickup: number;
  /** At least this many, 0..=15. */
  min_on_map: number;
}

/** Whether the comedy director may roll an event (`ls_sim::EventRule`). */
export type EventRule = "On" | "Banned";

export type ShieldKind = "Blocked" | "Wears" | "PassThrough";

export interface ShieldInteraction {
  kind: ShieldKind;
  /** Wears: energy factor (>= 0); PassThrough: fraction 0..=1; Blocked: 0. */
  value: number;
}

export interface WeaponTuning {
  damage: number;
  fire_rate: number;
  ammo: number;
}

/** `ls_sim::ModeKind` names, in mode index order (rule set codes). */
export type ModeKindName = "PaintTheVoid" | "TeamBrawl" | "TheSqueeze" | "Heist" | "HotReactor" | "LastColorStanding" | "TheArchitect";

/**
 * A mode and its settings: `kind` plus one number per field of the mode's settings
 * struct (`ModeRules::fields`; every mode has `teams`, `match_secs`, `respawn_secs`).
 */
export interface ModeRules {
  kind: ModeKindName;
  /** 0 = free-for-all, else 2-4 (see `ModeInfo.teams`). */
  teams: number;
  match_secs: number;
  respawn_secs: number;
  [field: string]: number | string;
}

/** One mode setting as rule set codes carry it (`ls_sim::ModeField`). */
export interface ModeFieldInfo {
  name: string;
  /** Steps per unit (1 = whole numbers, 10 = tenths). */
  steps: number;
  bits: number;
}

/** A mode as the editor and the codes see it. */
export interface ModeInfo {
  kind: ModeKindName;
  /** Mode index in rule set codes. */
  index: number;
  name: string;
  /** Supported team counts (0 = free-for-all). */
  teams: number[];
  /** Lives per player (0 = unlimited). */
  lives: number;
  /** Settings in code order. */
  fields: ModeFieldInfo[];
}

export interface Loadout {
  primaries: string[];
  secondaries: [string, number][];
  gadget: string | null;
}

export interface RuleSetData {
  name: string;
  mode: ModeRules;
  /** Unlisted = Banned. */
  weapons: Record<string, Availability>;
  gadgets: Record<string, Availability>;
  starting_loadout: Loadout;
  pickups: Record<string, PickupRule>;
  upgrade_pickups: Record<string, PickupRule>;
  gadget_pickups: Record<string, PickupRule>;
  scrap_pickup: ScrapPickupRule | null;
  shield_overrides: Record<string, ShieldInteraction>;
  tuning: Record<string, WeaponTuning>;
  /** 0 = off, otherwise 10..=100. */
  friendly_fire_percent: number;
  allowed_ship_types: ShipTypeName[];
  /** Cap of the comedy director's Absurdity (0-100; it runs at min(world slider, cap)). */
  absurdity: number;
  mutators: string[];
  /** Comedy events by id; unlisted = On. */
  events: Record<string, EventRule>;
  pigment_pickup: PigmentPickupRule | null;
}

export interface WeaponInfo {
  id: string;
  name: string;
  family: string;
  slot: "primary" | "secondary";
  /** Ammo-count weapon (ordnance): pickups grant ammo, loadout entries have counts. */
  count: boolean;
  maxLevel: number;
}

export interface GadgetInfo {
  id: string;
  name: string;
}

/** A comedy event (`data/events/<id>.ron`). */
export interface EventInfo {
  id: string;
  name: string;
}

/** A rule set with the code and validity the Rust side computed (Node tests). */
export interface CodeVector {
  label: string;
  /** `RuleSet::resolve` succeeds. */
  valid: boolean;
  /** `RuleSet::to_code`, or null when it fails. */
  code: string | null;
  /** `RuleSet::to_full_code`, or null when it fails. */
  full: string | null;
  /** Canonical form (what decoding returns). */
  rules: RuleSetData;
}

/** Everything the editor needs from the game data (generated from Rust). */
export interface Catalog {
  /** Registry order (rule set code indices). */
  weapons: WeaponInfo[];
  /** Registry order. */
  gadgets: GadgetInfo[];
  /** Comedy events in registry order (rule set code bits). */
  events: EventInfo[];
  /** `ShipType::index` order. */
  shipTypes: ShipTypeName[];
  /** Every mode, in mode index order. */
  modes: ModeInfo[];
  /** Mutator names in mask bit order. */
  mutators: string[];
  /** Canonical default rule set per mode (the base of diff codes). */
  defaults: Record<ModeKindName, RuleSetData>;
  vectors: CodeVector[];
  /** Codes of the older formats (2-4) with what they decode to. */
  legacyVectors: { code: string; rules: RuleSetData }[];
}
