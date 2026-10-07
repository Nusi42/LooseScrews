import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { CLAW_GAP } from "../game/events/catPaw.ts";
import { findPawWall, GROOVE_INSET, GROOVE_RADIUS, grooveEdits, type PawWall } from "./spectacleRun.ts";

const pawRon = readFileSync(new URL("../../../data/events/cat_paw.ron", import.meta.url), "utf8");
const param = (name: string): number => {
  const m = new RegExp(`"${name}":\\s*([0-9.]+)`).exec(pawRon);
  assert.ok(m, name);
  return Number(m[1]);
};

test("cat paw groove numbers match data/events/cat_paw.ron", () => {
  assert.equal(CLAW_GAP, param("claw_gap"));
  assert.equal(GROOVE_RADIUS, param("groove_radius"));
  assert.equal(GROOVE_INSET, param("groove_inset"));
  // Three separate grooves (D5): a rock ridge stays between neighbours.
  assert.ok(CLAW_GAP > 2 * GROOVE_RADIUS);
});

/** A ray against the plane z = `wallZ` (rock beyond it), like ls-wasm `Game.raycast`. */
const planeRay = (wallZ: number) => (o: Vec3, d: Vec3, max: number): number => {
  if (o.z >= wallZ) return 0;
  if (d.z <= 0) return -1;
  const t = (wallZ - o.z) / d.z;
  return t <= max ? t : -1;
};

test("aftermath grooves: three lanes on the rock the ray finds, each sphere GROOVE_INSET inside it", () => {
  const n = { x: 0, y: 0, z: 1 };
  const t = { x: 1, y: 0, z: 0 };
  const w: PawWall = { face: 5, n, t, b: v3.cross(t, n), wall: { x: 0, y: 0, z: 100 } };
  // The rock is 2 u further in than the wall point: the grooves follow it.
  const edits = grooveEdits(w, planeRay(102));
  assert.ok(edits.length >= 30);
  for (const e of edits) {
    assert.equal(e.radius, GROOVE_RADIUS);
    assert.ok(Math.abs(e.center.z - (102 + GROOVE_INSET)) < 1e-4, `${e.center.z}`);
  }
  const lanes = [...new Set(edits.map((e) => Math.round(e.center.y)))].sort((a, b) => a - b);
  const groups: number[][] = [];
  for (const y of lanes) {
    const g = groups[groups.length - 1];
    if (g && y - g[g.length - 1]! <= 1) g.push(y);
    else groups.push([y]);
  }
  assert.equal(groups.length, 3, `${lanes}`);
  // No rock in reach: nothing carved.
  assert.equal(grooveEdits(w, () => -1).length, 0);
});

test("the paw's wall is generated rock, never the bare sealing wall", () => {
  // A 100 u box whose +z side has rock 1 u (the sealing wall alone) or 8 u deep.
  const bounds = [0, 0, 0, 100, 100, 100];
  const box = (rockFromFace: number) => (o: Vec3, d: Vec3, max: number): number => {
    const ts: number[] = [];
    const walls: Array<[keyof Vec3, number, number]> = [
      ["x", 0, -1],
      ["x", 100, 1],
      ["y", 0, -1],
      ["y", 100, 1],
      ["z", 0, -1],
      ["z", 100 - rockFromFace, 1],
    ];
    for (const [a, plane, sign] of walls) {
      const dv = d[a] * sign;
      if (dv <= 0) continue;
      ts.push(((plane - o[a]) * sign) / dv);
    }
    const hit = Math.min(...ts.filter((x) => x >= 0));
    return hit <= max ? hit : -1;
  };
  const sealed = findPawWall(bounds, box(1));
  assert.equal(sealed, null);
  const rock = findPawWall(bounds, box(8));
  assert.ok(rock);
  assert.ok(rock.wall.z <= 97, `${rock.wall.z}`);
});
