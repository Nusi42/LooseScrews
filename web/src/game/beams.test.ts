// Leech Arc beams (VQP): the own beam drawn locally (ls-wasm `localBeam`)
// and beams that follow their locked target.

import assert from "node:assert/strict";
import { test } from "node:test";
import type { Vec3 } from "../math/vec3.ts";
import { BEAM_COLOR, CUE, EffectsState, NO_SHIP, parseLocalBeam, type Cue, type EffectsContext } from "./mechanics.ts";

const ME = 3;

function ctx(ships: Record<number, Vec3>): EffectsContext {
  return { player: ME, camera: { x: 0, y: 0, z: 0 }, shipPos: (id) => ships[id] };
}

test("parseLocalBeam decodes the wasm array and treats empty as no beam", () => {
  assert.equal(parseLocalBeam(undefined), null);
  assert.equal(parseLocalBeam(new Float32Array(0)), null);
  const b = parseLocalBeam(new Float32Array([1, 2, 3, 4, 5, 6, 7]));
  assert.deepEqual(b, { from: { x: 1, y: 2, z: 3 }, to: { x: 4, y: 5, z: 6 }, target: 7 });
  assert.equal(parseLocalBeam(new Float32Array([0, 0, 0, 0, 0, -9, 255]))?.target, NO_SHIP);
});

test("the own local beam draws at full strength from the own ship to its lock", () => {
  const fx = new EffectsState();
  const ships = { [ME]: { x: 0, y: 0, z: 0 }, 5: { x: 0, y: 1, z: -6 } };
  // The wasm aimed at the target's position of its tick; the drawn ship has moved on.
  const local = { from: { x: 0.1, y: 0, z: 0 }, to: { x: 0, y: 0, z: -5 }, target: 5 };
  const f = fx.frame(1, ctx(ships), local);
  assert.equal(f.beams.length, 1);
  assert.deepEqual(f.beams[0], { from: ships[ME], to: ships[5], color: BEAM_COLOR, strength: 1 });
  // Without a lock it runs to the wasm's end point.
  const free = fx.frame(1, ctx(ships), { ...local, target: NO_SHIP });
  assert.deepEqual(free.beams[0]?.to, local.to);
  assert.equal(fx.frame(1, ctx(ships), null).beams.length, 0);
});

test("a remote beam cue with a lock follows the target as it moves", () => {
  const fx = new EffectsState();
  const cue: Cue = { kind: CUE.beam, source: 4, target: 6, position: { x: 0, y: 0, z: -4 }, value: 18, tick: 1 };
  const ships: Record<number, Vec3> = { 4: { x: 0, y: 0, z: 0 }, 6: { x: 0, y: 0, z: -4 } };
  fx.apply([cue], 1, ctx(ships));
  ships[6] = { x: 2, y: 0, z: -4 };
  const f = fx.frame(1.1, ctx(ships));
  assert.deepEqual(f.beams[0]?.to, { x: 2, y: 0, z: -4 });
  assert.ok((f.beams[0]?.strength ?? 0) > 0 && (f.beams[0]?.strength ?? 0) < 1);
});
