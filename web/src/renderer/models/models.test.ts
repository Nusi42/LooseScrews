// VQ4 modelling kit, hulls, wrecks and props.

import assert from "node:assert/strict";
import { test } from "node:test";
import { validateMesh, type MeshData } from "../mesh.ts";
import { packInstances, INSTANCE_FLOATS } from "../renderer.ts";
import { LEGACY_MESH_BUILDERS, MESH_BUILDERS, MESH_KINDS, SHIP_MESH_KINDS, shipMeshKind, WRECK_MESH_KINDS, wreckMeshKind } from "./index.ts";
import {
  hullRing,
  Model,
  PART_CANOPY,
  PART_ENGINE,
  PART_GLOW,
  PART_HULL,
  PART_LIVERY,
  PART_METAL,
  PART_NAV_PORT,
  PART_NAV_STARBOARD,
  PART_NAV_TAIL,
  PART_PLAIN,
  PART_SCORCH,
  signedVolume,
} from "./kit.ts";
import { crateModel, gadgetModel, harpoonModel, mineModel, rocketModel, upgradeModel } from "./props.ts";
import { MODEL_SHADING_WGSL, NAV_FLASH, NAV_PERIOD, NAV_STEADY, navBlink } from "./shading.ts";
import { HULL_BUILDERS, HULL_RADIUS, hullOf } from "./ships.ts";
import { wreckModel } from "./wrecks.ts";

/** Hitbox radii by ship type (data/ships/*.ron) and the Tinker's (instance scale 1). */
const RADII = [0.4, 0.5, 0.65, 0.55];
const TINKER_RADIUS = 0.5;
const NAMES = ["Skitter", "Tinker", "Hauler", "Tug"];

function parts(m: MeshData): Map<number, number> {
  const out = new Map<number, number>();
  for (const p of m.team) out.set(p, (out.get(p) ?? 0) + 1);
  return out;
}

/** Mean vertex position of the vertices with `part`. */
function centroidOf(m: MeshData, part: number): [number, number, number] {
  const c: [number, number, number] = [0, 0, 0];
  let n = 0;
  m.team.forEach((p, i) => {
    if (p !== part) return;
    c[0] += m.positions[i * 3]!;
    c[1] += m.positions[i * 3 + 1]!;
    c[2] += m.positions[i * 3 + 2]!;
    n++;
  });
  return [c[0] / n, c[1] / n, c[2] / n];
}

/** Silhouette of a model projected onto axes (u, v) as a 48 x 48 occupancy grid over [-0.5, 0.5]^2. */
function silhouette(m: Model, u: number, v: number): Uint8Array {
  const n = 48;
  const grid = new Uint8Array(n * n);
  for (const t of m.tris) {
    const pts = [t.a, t.b, t.c].map((p) => [(p[u]! + 0.5) * n, (p[v]! + 0.5) * n] as const);
    const [p0, p1, p2] = pts as [readonly [number, number], readonly [number, number], readonly [number, number]];
    const area = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (p1[1] - p0[1]);
    if (Math.abs(area) < 1e-12) continue;
    const x0 = Math.max(0, Math.floor(Math.min(p0[0], p1[0], p2[0]))), x1 = Math.min(n - 1, Math.ceil(Math.max(p0[0], p1[0], p2[0])));
    const y0 = Math.max(0, Math.floor(Math.min(p0[1], p1[1], p2[1]))), y1 = Math.min(n - 1, Math.ceil(Math.max(p0[1], p1[1], p2[1])));
    for (let y = y0; y <= y1; y++) {
      for (let x = x0; x <= x1; x++) {
        const px = x + 0.5, py = y + 0.5;
        const w0 = (p1[0] - px) * (p2[1] - py) - (p2[0] - px) * (p1[1] - py);
        const w1 = (p2[0] - px) * (p0[1] - py) - (p0[0] - px) * (p2[1] - py);
        const w2 = (p0[0] - px) * (p1[1] - py) - (p1[0] - px) * (p0[1] - py);
        if ((w0 >= 0 && w1 >= 0 && w2 >= 0) || (w0 <= 0 && w1 <= 0 && w2 <= 0)) grid[y * n + x] = 1;
      }
    }
  }
  return grid;
}

function iou(a: Uint8Array, b: Uint8Array): number {
  let inter = 0, union = 0;
  for (let i = 0; i < a.length; i++) {
    if (a[i] && b[i]) inter++;
    if (a[i] || b[i]) union++;
  }
  return inter / union;
}

test("four distinct hulls of 600-2000 triangles, mapped by ship type", () => {
  assert.equal(HULL_BUILDERS.length, 4);
  const extents: number[][] = [];
  for (let t = 0; t < 4; t++) {
    const hull = hullOf(t);
    assert.equal(hull.name, NAMES[t]);
    const mesh = MESH_BUILDERS[shipMeshKind(t)]();
    validateMesh(mesh);
    const tris = mesh.indices.length / 3;
    assert.ok(tris >= 600 && tris <= 2000, `${hull.name}: ${tris} triangles`);
    const b = hull.model.bounds();
    extents.push([b.max[0] - b.min[0], b.max[1] - b.min[1], b.max[2] - b.min[2]]);
  }
  assert.deepEqual(SHIP_MESH_KINDS.map((_, t) => shipMeshKind(t)), ["ship_skitter", "ship_tinker", "ship_hauler", "ship_tug"]);
  assert.equal(shipMeshKind(99), "ship_tinker", "unknown types draw the Tinker");
  // Distinct silhouettes: seen from above or from the front, no two hulls overlap by more than 80 %.
  const views = [0, 1, 2, 3].map((t) => [silhouette(hullOf(t).model, 0, 2), silhouette(hullOf(t).model, 0, 1)]);
  for (let a = 0; a < 4; a++) {
    for (let b = a + 1; b < 4; b++) {
      const top = iou(views[a]![0]!, views[b]![0]!), front = iou(views[a]![1]!, views[b]![1]!);
      assert.ok(Math.min(top, front) < 0.8, `${NAMES[a]} vs ${NAMES[b]}: top ${top.toFixed(2)}, front ${front.toFixed(2)}`);
    }
  }
  // The Skitter is the slimmest, the Tug the widest, the Hauler the tallest.
  const width = extents.map((e) => e[0]!), height = extents.map((e) => e[1]!);
  assert.equal(width.indexOf(Math.max(...width)), 3, `widths ${width}`);
  assert.equal(height.indexOf(Math.max(...height)), 2, `heights ${height}`);
  assert.ok(height[0]! < height[2]! && width[0]! < width[3]!);
});

test("every hull stays inside its collision sphere (radius x 1.05)", () => {
  for (let t = 0; t < 4; t++) {
    const mesh = MESH_BUILDERS[shipMeshKind(t)]();
    // The scene draws hulls at scale radius / TINKER_RADIUS (game/scene.ts addShips).
    const s = RADII[t]! / TINKER_RADIUS;
    let max = 0;
    for (let i = 0; i < mesh.positions.length; i += 3) {
      max = Math.max(max, Math.hypot(mesh.positions[i]!, mesh.positions[i + 1]!, mesh.positions[i + 2]!) * s);
    }
    assert.ok(max <= RADII[t]! * 1.05, `${NAMES[t]}: farthest vertex ${max.toFixed(4)} u, radius ${RADII[t]}`);
    // ...and fills it (the hull is the size cue for "do I fit").
    assert.ok(max >= RADII[t]! * 0.95, `${NAMES[t]} fills its sphere: ${max.toFixed(4)}`);
  }
  assert.ok(Math.abs(hullOf(1).model.maxRadius() - HULL_RADIUS) < 1e-9);
});

test("hulls carry livery, canopy, rear engines and port/starboard nav lights", () => {
  for (let t = 0; t < 4; t++) {
    const mesh = MESH_BUILDERS[shipMeshKind(t)]();
    const p = parts(mesh);
    for (const part of [PART_HULL, PART_LIVERY, PART_CANOPY, PART_ENGINE, PART_NAV_PORT, PART_NAV_STARBOARD, PART_NAV_TAIL, PART_METAL]) {
      assert.ok((p.get(part) ?? 0) > 0, `${NAMES[t]} has part ${part}`);
    }
    assert.equal(p.get(PART_PLAIN) ?? 0, 0, `${NAMES[t]}: no plain (untinted) vertices`);
    assert.ok(centroidOf(mesh, PART_ENGINE)[2] > 0.25, `${NAMES[t]}: engines at the back (+Z)`);
    assert.ok(centroidOf(mesh, PART_CANOPY)[2] < 0 && centroidOf(mesh, PART_CANOPY)[1] > 0, `${NAMES[t]}: canopy on top, in front`);
    assert.ok(centroidOf(mesh, PART_NAV_PORT)[0] < -0.1, `${NAMES[t]}: red light on the port (-X) side`);
    assert.ok(centroidOf(mesh, PART_NAV_STARBOARD)[0] > 0.1, `${NAMES[t]}: green light on the starboard (+X) side`);
  }
});

test("hull pieces are closed, outward-wound solids, 3-4 per hull", () => {
  for (let t = 0; t < 4; t++) {
    const hull = hullOf(t);
    assert.ok(hull.pieces.length >= 3 && hull.pieces.length <= 4, hull.name);
    let total = 0;
    for (const piece of hull.pieces) {
      assert.ok(signedVolume(piece.tris) > 0, `${hull.name}: piece wound outward`);
      total += piece.triangleCount;
    }
    assert.equal(total, hull.model.triangleCount, `${hull.name}: the ship is exactly its pieces`);
    assert.ok(signedVolume(hull.model.tris) > 0);
  }
});

test("wrecks break the hull of the dead ship type into its pieces, lights out, burnt", () => {
  assert.deepEqual(WRECK_MESH_KINDS.map((_, t) => wreckMeshKind(t)), ["wreck_skitter", "wreck_tinker", "wreck_hauler", "wreck_tug"]);
  for (let t = 0; t < 4; t++) {
    const wreck = wreckModel(t);
    assert.equal(wreck.triangleCount, hullOf(t).model.triangleCount, "every piece of the hull is there");
    const mesh = MESH_BUILDERS[wreckMeshKind(t)]();
    validateMesh(mesh);
    const p = parts(mesh);
    for (const dead of [PART_ENGINE, PART_NAV_PORT, PART_NAV_STARBOARD, PART_NAV_TAIL, PART_CANOPY]) {
      assert.equal(p.get(dead) ?? 0, 0, `wreck ${t}: part ${dead} is dead`);
    }
    assert.ok((p.get(PART_SCORCH) ?? 0) > 0, "charred");
    assert.equal(p.get(PART_HULL) ?? 0, 0, "all paint burnt");
    assert.ok((p.get(PART_METAL) ?? 0) > 0, "bare metal stays");
    assert.ok(wreck.maxRadius() < 0.5 && wreck.maxRadius() > 0.3, `wreck ${t} radius ${wreck.maxRadius()}`);
    // Not the intact ship: pieces moved.
    const hullB = hullOf(t).model.bounds(), wb = wreck.bounds();
    assert.notDeepEqual(hullB, wb);
  }
});

test("props: reworked rocket, harpoon, mine, crate, upgrade and gadget", () => {
  const props: [string, Model, number[]][] = [
    ["rocket", rocketModel(), [PART_PLAIN, PART_METAL, PART_ENGINE, PART_GLOW]],
    ["harpoon", harpoonModel(), [PART_PLAIN, PART_METAL]],
    ["mine", mineModel(), [PART_PLAIN, PART_METAL, PART_GLOW]],
    ["crate", crateModel(), [PART_PLAIN, PART_GLOW, PART_METAL]],
    ["upgrade", upgradeModel(), [PART_GLOW, PART_METAL]],
    ["gadget", gadgetModel(), [PART_PLAIN, PART_GLOW, PART_METAL]],
  ];
  for (const [name, model, want] of props) {
    const mesh = model.build();
    validateMesh(mesh);
    const p = parts(mesh);
    for (const part of want) assert.ok((p.get(part) ?? 0) > 0, `${name} has part ${part}`);
    const r = model.maxRadius();
    assert.ok(r > 0.35 && r <= 0.62, `${name} about 1 u across: radius ${r}`);
    const tris = mesh.indices.length / 3;
    assert.ok(tris >= 60 && tris <= 1500, `${name}: ${tris} triangles`);
    // Richer than the primitive it replaces.
    assert.ok(tris > LEGACY_MESH_BUILDERS[name as "rocket"]().indices.length / 3, `${name} has more detail than before`);
  }
});

test("legacy set: the pre-VQ4 primitives for every kind, all plain", () => {
  for (const kind of MESH_KINDS) {
    const m = LEGACY_MESH_BUILDERS[kind]();
    validateMesh(m);
    assert.ok(m.team.every((p) => p === PART_PLAIN), kind);
  }
  assert.equal(LEGACY_MESH_BUILDERS.ship_hauler().indices.length, 24, "one 8-triangle dart");
});

test("kit: lofts wind outward whatever the ring order, boxes have their volume", () => {
  const rings = [hullRing(-0.5, 0.1, 0.1, 0.1, 8), hullRing(0, 0.2, 0.1, 0.1, 8), hullRing(0.5, 0.1, 0.1, 0.1, 8)];
  const fwd = new Model().loft(rings, { capStart: true, capEnd: true });
  const back = new Model().loft([...rings].reverse(), { capStart: true, capEnd: true });
  assert.ok(signedVolume(fwd.tris) > 0);
  assert.ok(Math.abs(signedVolume(fwd.tris) - signedVolume(back.tris)) < 1e-12);
  // A pointed nose (ring collapsed to one point) adds no degenerate triangles.
  const nose = new Model().loft([hullRing(-0.5, 0, 0, 0, 8), rings[1]!], { capEnd: true });
  assert.equal(nose.triangleCount, 8 + 8);
  const box = new Model().box([1, 2, 3], [0.1, 0.2, 0.3]);
  assert.ok(Math.abs(signedVolume(box.tris) - 8 * 0.1 * 0.2 * 0.3) < 1e-9);
  const bevelled = new Model().box([0, 0, 0], [0.1, 0.2, 0.3], { bevel: 0.02 });
  const v = signedVolume(bevelled.tris);
  assert.ok(v > 0 && v < 8 * 0.1 * 0.2 * 0.3, "bevels cut the corners");
});

test("kit: mirror keeps the winding and swaps the nav light sides", () => {
  const m = new Model();
  m.sphere([0.3, 0, 0], 0.05, 1, PART_NAV_STARBOARD);
  const both = new Model().add(m).addMirrorX(m);
  assert.ok(signedVolume(both.tris) > 0);
  const mesh = both.build();
  assert.ok(centroidOf(mesh, PART_NAV_PORT)[0] < -0.25);
  assert.ok(centroidOf(mesh, PART_NAV_STARBOARD)[0] > 0.25);
});

test("kit: smooth groups share normals below the crease angle, flat groups do not", () => {
  const ring = (z: number) => hullRing(z, 0.2, 0.2, 0.2, 16, 2);
  const smooth = new Model();
  smooth.loft([ring(-0.2), ring(0.2)], { group: smooth.smooth(40) });
  const flat = new Model().loft([ring(-0.2), ring(0.2)]);
  const sm = smooth.build(), fm = flat.build();
  // Smooth: one vertex per ring point (normals agree); flat: each face its own vertices.
  assert.equal(sm.positions.length / 3, 32);
  assert.ok(fm.positions.length / 3 > 32);
  // Smooth normals point radially.
  for (let i = 0; i < sm.positions.length; i += 3) {
    const r = Math.hypot(sm.positions[i]!, sm.positions[i + 1]!);
    const dotr = (sm.normals[i]! * sm.positions[i]! + sm.normals[i + 1]! * sm.positions[i + 1]!) / r;
    assert.ok(dotr > 0.99, `radial normal ${dotr}`);
  }
});

test("nav lights blink out of phase; shader parts match the kit", () => {
  assert.equal(navBlink(0, 0), NAV_STEADY + NAV_FLASH, "flash at the start of the period");
  assert.equal(navBlink(NAV_PERIOD / 2, 0), NAV_STEADY, "steady between flashes");
  assert.equal(navBlink(NAV_PERIOD * 3 + 0.01, 0), NAV_STEADY + NAV_FLASH, "periodic");
  assert.notEqual(navBlink(0.05, 0), navBlink(0.05, 0.5), "phase shifts the blink");
  assert.equal(navBlink(0.35, 0, true), NAV_STEADY + NAV_FLASH, "the tail strobe double-flashes");
  for (const [name, value] of [["PART_HULL", PART_HULL], ["PART_ENGINE", PART_ENGINE], ["PART_NAV_TAIL", PART_NAV_TAIL], ["PART_SCORCH", PART_SCORCH]] as const) {
    assert.ok(MODEL_SHADING_WGSL.includes(`const ${name} = ${value}u;`), name);
  }
});

test("instances carry engine glow and nav phase in the padding floats", () => {
  const data = packInstances([
    { position: { x: 0, y: 0, z: 0 }, rotation: { x: 0, y: 0, z: 0, w: 1 }, scale: 1, color: [1, 1, 1], emissive: 0, glow: 1, blink: 0.25 },
    { position: { x: 0, y: 0, z: 0 }, rotation: { x: 0, y: 0, z: 0, w: 1 }, scale: 1, color: [1, 1, 1], emissive: 0 },
  ]);
  assert.equal(data[14], 1);
  assert.equal(data[15], 0.25);
  assert.equal(data[INSTANCE_FLOATS + 14], 0);
  assert.equal(data[INSTANCE_FLOATS + 15], 0);
});
