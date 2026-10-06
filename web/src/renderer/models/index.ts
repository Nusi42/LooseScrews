// Mesh kinds of the instanced objects and their builders. The detailed set
// (VQ4: modelling-kit hulls per ship type, wrecks made of the hull's pieces,
// reworked rocket, harpoon, mine, crate, upgrade and gadget) is the default;
// LEGACY_MESH_BUILDERS keeps the pre-VQ4 primitives for every kind (the
// "before" screenshots and a fallback for weak GPUs, `Renderer.create(...,
// { legacyModels: true })`, later RenderQuality.low).

import type { MeshData } from "../mesh.ts";
import {
  crateMesh,
  debrisMesh,
  duckMesh,
  gadgetMesh,
  harpoonMesh,
  mineMesh,
  projectileMesh,
  rocketMesh,
  scrapMesh,
  shipMesh,
  sphereMesh,
  upgradeMesh,
  wreckMesh,
} from "../primitives.ts";
import { buildProp, crateModel, gadgetModel, harpoonModel, mineModel, paperweightModel, rocketModel, upgradeModel } from "./props.ts";
import { coreModel, reactorModel } from "./modeProps.ts";
import { pigmentDropModel } from "./worldProps.ts";
import { boulderMesh, gantryMesh, pebbleMesh } from "./raceProps.ts";
import { hullOf } from "./ships.ts";
import { wreckModel } from "./wrecks.ts";

/** Hull mesh kinds in ls-sim ShipType order (Skitter 0, Tinker 1, Hauler 2, Tug 3). */
export const SHIP_MESH_KINDS = ["ship_skitter", "ship_tinker", "ship_hauler", "ship_tug"] as const;
/** Wreck mesh kinds in ShipType order. */
export const WRECK_MESH_KINDS = ["wreck_skitter", "wreck_tinker", "wreck_hauler", "wreck_tug"] as const;

export type ShipMeshKind = (typeof SHIP_MESH_KINDS)[number];
export type WreckMeshKind = (typeof WRECK_MESH_KINDS)[number];

/** Every instanced mesh kind the renderer knows. */
export type MeshKind =
  | "ship"
  | ShipMeshKind
  | "bolt"
  | "orb"
  | "sphere"
  | "rocket"
  | "harpoon"
  | "mine"
  | "crate"
  | "upgrade"
  | "gadget"
  | "debris"
  | "scrap"
  | "wreck"
  | WreckMeshKind
  | "duck"
  | "core"
  | "reactor"
  | "paperweight"
  | "pigment"
  | "boulder"
  | "pebble"
  | "gantry";

/** Hull mesh of a ship type (unknown types: the Tinker's). */
export function shipMeshKind(shipType: number): ShipMeshKind {
  return SHIP_MESH_KINDS[shipType] ?? "ship_tinker";
}

/** Wreck mesh of a ship type (unknown types: the Tinker's). */
export function wreckMeshKind(shipType: number): WreckMeshKind {
  return WRECK_MESH_KINDS[shipType] ?? "wreck_tinker";
}

const hullMesh = (t: number) => (): MeshData => hullOf(t).model.build();
const wreckMeshOf = (t: number) => (): MeshData => wreckModel(t).build();

/** Builders for every mesh kind (the renderer uploads each once). */
export const MESH_BUILDERS: Readonly<Record<MeshKind, () => MeshData>> = {
  ship: hullMesh(1),
  ship_skitter: hullMesh(0),
  ship_tinker: hullMesh(1),
  ship_hauler: hullMesh(2),
  ship_tug: hullMesh(3),
  bolt: projectileMesh,
  orb: () => sphereMesh(1),
  sphere: () => sphereMesh(2),
  rocket: buildProp(rocketModel),
  harpoon: buildProp(harpoonModel),
  mine: buildProp(mineModel),
  crate: buildProp(crateModel),
  upgrade: buildProp(upgradeModel),
  gadget: buildProp(gadgetModel),
  debris: debrisMesh,
  scrap: scrapMesh,
  wreck: wreckMeshOf(1),
  wreck_skitter: wreckMeshOf(0),
  wreck_tinker: wreckMeshOf(1),
  wreck_hauler: wreckMeshOf(2),
  wreck_tug: wreckMeshOf(3),
  duck: duckMesh,
  // Mode objectives (M3 C1, modeProps.ts).
  core: buildProp(coreModel),
  reactor: buildProp(reactorModel),
  // M4 B7.
  paperweight: buildProp(paperweightModel),
  // M4 B5: the pigment drop (worldProps.ts).
  pigment: buildProp(pigmentDropModel),
  // M4 C7: Belt Up's drifting boulders, the belt's pebbles and the start-light gantry (raceProps.ts).
  boulder: boulderMesh,
  pebble: pebbleMesh,
  gantry: gantryMesh,
};

/** The pre-VQ4 primitives for every kind (one dart for all hulls, one wreck). */
export const LEGACY_MESH_BUILDERS: Readonly<Record<MeshKind, () => MeshData>> = {
  ship: shipMesh,
  ship_skitter: shipMesh,
  ship_tinker: shipMesh,
  ship_hauler: shipMesh,
  ship_tug: shipMesh,
  bolt: projectileMesh,
  orb: () => sphereMesh(1),
  sphere: () => sphereMesh(2),
  rocket: rocketMesh,
  harpoon: harpoonMesh,
  mine: mineMesh,
  crate: crateMesh,
  upgrade: upgradeMesh,
  gadget: gadgetMesh,
  debris: debrisMesh,
  scrap: scrapMesh,
  wreck: wreckMesh,
  wreck_skitter: wreckMesh,
  wreck_tinker: wreckMesh,
  wreck_hauler: wreckMesh,
  wreck_tug: wreckMesh,
  duck: duckMesh,
  core: () => sphereMesh(1),
  reactor: () => sphereMesh(1),
  paperweight: () => sphereMesh(1),
  pigment: () => sphereMesh(1),
  boulder: debrisMesh,
  pebble: debrisMesh,
  gantry: crateMesh,
};

export const MESH_KINDS = Object.keys(MESH_BUILDERS) as MeshKind[];
