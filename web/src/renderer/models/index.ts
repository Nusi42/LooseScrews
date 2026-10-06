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
import { balloonFaceModel, balloonModel, boxFlapModel, cardboardBoxModel, coffeeMugModel, giantEggModel, pupilModel, rubberChickenModel, tapeStripModel } from "./eventProps.ts";
import { VISITOR_MESH_KINDS, visitorModel, type VisitorMeshKind } from "./visitors.ts";
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
  // M4 C1 event props (eventProps.ts).
  | "chicken"
  | "giant_egg"
  | "moving_box"
  | "box_flap"
  | "box_tape"
  | "balloon"
  | "balloon_face"
  | "pupil"
  | "coffee_mug"
  | VisitorMeshKind;

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
  // M4 C1: comedy event props (eventProps.ts).
  chicken: buildProp(rubberChickenModel),
  giant_egg: buildProp(giantEggModel),
  moving_box: buildProp(cardboardBoxModel),
  box_flap: buildProp(boxFlapModel),
  box_tape: buildProp(tapeStripModel),
  balloon: buildProp(balloonModel),
  balloon_face: buildProp(balloonFaceModel),
  pupil: buildProp(pupilModel),
  coffee_mug: buildProp(coffeeMugModel),
  // M4 C2: the event visitors (visitors.ts).
  ...visitorBuilders((k) => () => visitorModel(k).build()),
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
  chicken: () => sphereMesh(1),
  giant_egg: () => sphereMesh(1),
  moving_box: () => sphereMesh(1),
  box_flap: () => sphereMesh(1),
  box_tape: () => sphereMesh(1),
  balloon: () => sphereMesh(1),
  balloon_face: () => sphereMesh(1),
  pupil: () => sphereMesh(1),
  coffee_mug: () => sphereMesh(1),
  ...visitorBuilders(() => () => sphereMesh(1)),
};

function visitorBuilders(f: (k: VisitorMeshKind) => () => MeshData): Record<VisitorMeshKind, () => MeshData> {
  return Object.fromEntries(VISITOR_MESH_KINDS.map((k) => [k, f(k)])) as Record<VisitorMeshKind, () => MeshData>;
}

export const MESH_KINDS = Object.keys(MESH_BUILDERS) as MeshKind[];
