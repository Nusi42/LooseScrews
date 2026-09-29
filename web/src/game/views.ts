// Decodes ls-wasm's flat state arrays (layouts in wasm/api.ts) into poses,
// ships, projectiles, entities, visuals, players and the weapon loadout.
// game/scene.ts turns them into renderer instances.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { Rgb } from "../renderer/renderer.ts";
import {
  ENTITY_STRIDE,
  LOADOUT_INFO_STRIDE,
  PLAYER_STRIDE,
  POSE_LEN,
  PROJECTILE_STRIDE,
  SHIP_STRIDE,
  VISUAL_STRIDE,
  type LoadoutHandle,
} from "../wasm/api.ts";

export interface LocalPose {
  position: Vec3;
  orientation: Quat;
  velocity: Vec3;
  alive: boolean;
}

export interface ShipState {
  id: number;
  team: number;
  alive: boolean;
  boosting: boolean;
  position: Vec3;
  orientation: Quat;
  /** ls-sim ShipType index (Skitter 0, Tinker 1, Hauler 2, Tug 3). */
  shipType: number;
  /** Hitbox radius of the ship type (u). */
  radius: number;
  /** Its gadget (Shield, Cloak) is on. */
  gadgetActive: boolean;
}

export interface ProjectileState {
  kind: number;
  owner: number;
  position: Vec3;
  velocity: Vec3;
  /** Visual id (index into the match's visual table). */
  visual: number;
  /** Replicated projectile flags (PROJ_* below). */
  state: number;
}

/** ls-sim `proj_flags` low byte (replicated). */
export const PROJ_STUCK = 1 << 0;
export const PROJ_ARMED = 1 << 1;
export const PROJ_RETURNING = 1 << 4;

/** A replicated entity (pickup, scrap pile, wreck, debris, decoy, ghost, dust cloud, marker). */
export interface EntityState {
  id: number;
  /** Kind name (ls-wasm entityKindNames(), e.g. "weaponPickup"), "" if unknown. */
  kind: string;
  visual: number;
  /** Owner ship id, 255 for none. */
  owner: number;
  /** Team, 255 for none. */
  team: number;
  position: Vec3;
  velocity: Vec3;
  rotation: Quat;
  state: [number, number, number, number];
  /** Collision / volume radius (u), 0 when unknown. */
  radius: number;
}

/** One entry of the match's visual table. */
export interface VisualDef {
  /** Shape name (ls-wasm visualShapeNames(), e.g. "rocket"). */
  shape: string;
  color: Rgb;
  scale: number;
}

export interface PlayerState {
  id: number;
  team: number;
  shipType: number;
  /** Gadget id, 255 for none. */
  gadget: number;
  name: string;
}

export interface WeaponSlotView {
  name: string;
  ammo: string;
  secondary: boolean;
  selected: boolean;
  /** Number key 1-10 (10 = the 0 key), 0 = none. */
  key: number;
  level: number;
  maxLevel: number;
}

export interface LoadoutState {
  primaries: WeaponSlotView[];
  secondaries: WeaponSlotView[];
  /** Gadget name, "" for none. */
  gadget: string;
  gadgetActive: boolean;
}

export function emptyLoadout(): LoadoutState {
  return { primaries: [], secondaries: [], gadget: "", gadgetActive: false };
}

/** Ship type names by index (ls-sim ShipType). */
export const SHIP_TYPE_NAMES: readonly string[] = ["Skitter", "Tinker", "Hauler", "Tug"];

/** Hitbox radius of the Tinker (the ship mesh is drawn at scale radius / this). */
export const TINKER_RADIUS = 0.5;

function num(a: ArrayLike<number>, i: number): number {
  return a[i] ?? 0;
}

function vec(a: ArrayLike<number>, i: number): Vec3 {
  return { x: num(a, i), y: num(a, i + 1), z: num(a, i + 2) };
}

function quat(a: ArrayLike<number>, i: number): Quat {
  return q.normalize({ x: num(a, i), y: num(a, i + 1), z: num(a, i + 2), w: num(a, i + 3) });
}

export function parsePose(a: ArrayLike<number> | undefined): LocalPose | null {
  if (!a || a.length < POSE_LEN) return null;
  return { position: vec(a, 0), orientation: quat(a, 3), velocity: vec(a, 7), alive: num(a, 10) !== 0 };
}

export function parseShips(a: ArrayLike<number>): ShipState[] {
  const out: ShipState[] = [];
  for (let i = 0; i + SHIP_STRIDE <= a.length; i += SHIP_STRIDE) {
    out.push({
      id: num(a, i),
      team: num(a, i + 1),
      alive: num(a, i + 2) !== 0,
      boosting: num(a, i + 3) !== 0,
      position: vec(a, i + 4),
      orientation: quat(a, i + 7),
      shipType: num(a, i + 11),
      radius: num(a, i + 12) || TINKER_RADIUS,
      gadgetActive: num(a, i + 13) !== 0,
    });
  }
  return out;
}

export function parseProjectiles(a: ArrayLike<number>): ProjectileState[] {
  const out: ProjectileState[] = [];
  for (let i = 0; i + PROJECTILE_STRIDE <= a.length; i += PROJECTILE_STRIDE) {
    out.push({
      kind: num(a, i),
      owner: num(a, i + 1),
      position: vec(a, i + 2),
      velocity: vec(a, i + 5),
      visual: num(a, i + 8),
      state: num(a, i + 9),
    });
  }
  return out;
}

export function parseEntities(a: ArrayLike<number>, kindNames: readonly string[]): EntityState[] {
  const out: EntityState[] = [];
  for (let i = 0; i + ENTITY_STRIDE <= a.length; i += ENTITY_STRIDE) {
    out.push({
      id: num(a, i),
      kind: kindNames[num(a, i + 1)] ?? "",
      visual: num(a, i + 2),
      owner: num(a, i + 3),
      team: num(a, i + 4),
      position: vec(a, i + 5),
      velocity: vec(a, i + 8),
      rotation: quat(a, i + 11),
      state: [num(a, i + 15), num(a, i + 16), num(a, i + 17), num(a, i + 18)],
      radius: num(a, i + 19),
    });
  }
  return out;
}

export function parseVisuals(a: ArrayLike<number>, shapeNames: readonly string[]): VisualDef[] {
  const out: VisualDef[] = [];
  for (let i = 0; i + VISUAL_STRIDE <= a.length; i += VISUAL_STRIDE) {
    out.push({
      shape: shapeNames[num(a, i)] ?? "",
      color: [num(a, i + 1), num(a, i + 2), num(a, i + 3)],
      scale: num(a, i + 4) || 1,
    });
  }
  return out;
}

export function parsePlayers(info: ArrayLike<number>, names: readonly string[]): PlayerState[] {
  const out: PlayerState[] = [];
  for (let i = 0, n = 0; i + PLAYER_STRIDE <= info.length; i += PLAYER_STRIDE, n++) {
    const id = num(info, i);
    out.push({
      id,
      team: num(info, i + 1),
      shipType: num(info, i + 2),
      gadget: num(info, i + 3),
      name: names[n] || `Player ${id + 1}`,
    });
  }
  return out;
}

/** Copies the wasm loadout object into plain data and frees it. */
export function loadoutState(h: LoadoutHandle): LoadoutState {
  try {
    const out = emptyLoadout();
    out.gadget = h.gadget;
    out.gadgetActive = h.gadgetActive;
    const info = h.info;
    const maxLevels = h.maxLevels;
    h.names.forEach((name, i) => {
      const o = i * LOADOUT_INFO_STRIDE;
      const slot: WeaponSlotView = {
        name,
        ammo: h.ammo[i] ?? "",
        secondary: (info[o] ?? 0) !== 0,
        selected: (info[o + 1] ?? 0) !== 0,
        key: info[o + 2] ?? 0,
        level: info[o + 3] ?? 1,
        maxLevel: maxLevels[i] ?? 1,
      };
      (slot.secondary ? out.secondaries : out.primaries).push(slot);
    });
    return out;
  } finally {
    h.free();
  }
}
