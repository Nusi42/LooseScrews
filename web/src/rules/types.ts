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

export interface ModeRules {
  kind: "PaintTheVoid";
  teams: number;
  match_secs: number;
  respawn_secs: number;
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
  absurdity: number;
  mutators: string[];
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
  /** `ShipType::index` order. */
  shipTypes: ShipTypeName[];
  /** Canonical default rule set per mode (the base of diff codes). */
  defaults: { PaintTheVoid: RuleSetData };
  vectors: CodeVector[];
}
