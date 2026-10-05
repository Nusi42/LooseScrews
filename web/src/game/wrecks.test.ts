// VQ4: hull choice for ships, decoys, ghosts and wrecks (scene.ts, wrecks.ts).

import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import { buildScene } from "./scene.ts";
import type { EntityState, ShipState, VisualDef } from "./views.ts";
import { fallbackType, WRECK_MATCH_SECONDS, WreckTypes } from "./wrecks.ts";

const origin = { x: 0, y: 0, z: 0 };

function ship(over: Partial<ShipState>): ShipState {
  return { id: 1, team: 0, alive: true, boosting: false, position: origin, orientation: q.identity(), shipType: 1, radius: 0.5, gadgetActive: false, ...over };
}

function entity(over: Partial<EntityState>): EntityState {
  return { id: 40, kind: "wreck", visual: 0, owner: 255, team: 255, position: origin, velocity: origin, rotation: q.identity(), state: [20, 0, 0, 0], radius: 0, ...over };
}

test("each ship type draws its own hull; boosting lights the engines; nav lights out of phase", () => {
  const ships = [0, 1, 2, 3].map((t) => ship({ id: 10 + t, shipType: t, radius: [0.4, 0.5, 0.65, 0.55][t]!, boosting: t === 2 }));
  const scene = buildScene(ships, [], [], [], () => undefined, 0);
  for (const kind of ["ship_skitter", "ship_tinker", "ship_hauler", "ship_tug"] as const) {
    assert.equal(scene.opaque[kind]?.length, 1, kind);
  }
  assert.equal(scene.opaque.ship, undefined, "no generic dart any more");
  assert.equal(scene.opaque.ship_hauler![0]!.glow, 1, "boosting Hauler");
  assert.equal(scene.opaque.ship_tinker![0]!.glow, 0, "idle Tinker");
  assert.notEqual(scene.opaque.ship_skitter![0]!.blink, scene.opaque.ship_tug![0]!.blink);
  assert.equal(scene.opaque.ship_tinker![0]!.emissive, 0, "the hull is lit, not washed with its team colour");
  // Unknown ship types fall back to the Tinker.
  assert.equal(buildScene([ship({ shipType: 9 })], [], [], [], () => undefined, 0).opaque.ship_tinker?.length, 1);
});

test("decoys and ghosts take their owner's hull", () => {
  const visuals: VisualDef[] = [{ shape: "decoy", color: [1, 1, 1], scale: 1 }, { shape: "ghost", color: [1, 1, 1], scale: 1 }];
  const ents = [entity({ id: 1, kind: "decoy", visual: 0, owner: 7, team: 1 }), entity({ id: 2, kind: "ghost", visual: 1, owner: 8 })];
  // From the players list (the owner may be out of view or the own ship).
  const types = new Map([[7, 3], [8, 0]]);
  const a = buildScene([], [], ents, visuals, () => undefined, 0, { shipTypeOf: (id) => types.get(id) });
  assert.equal(a.opaque.ship_tug?.length, 1, "a Tug's decoy is a Tug");
  assert.equal(a.translucent.ship_skitter?.length, 1, "a Skitter's ghost is a Skitter");
  // Without a players list, from the ships in view.
  const b = buildScene([ship({ id: 7, shipType: 2, radius: 0.65 })], [], ents.slice(0, 1), visuals, () => undefined, 0);
  assert.equal(b.opaque.ship_hauler?.length, 2, "the Hauler and its decoy");
});

test("wrecks show the hull of the ship that died there", () => {
  const visuals: VisualDef[] = [{ shape: "wreck", color: [0.5, 0.5, 0.5], scale: 1 }];
  const wrecks = new WreckTypes();
  const at = { x: 10, y: 2, z: -3 };
  wrecks.observe([ship({ id: 3, shipType: 3, position: at })], [], 1.0);
  wrecks.observe([ship({ id: 3, shipType: 3, position: at, alive: false })], [], 1.1);
  const wreck = entity({ id: 99, position: { x: 10.5, y: 2, z: -3 } });
  wrecks.observe([ship({ id: 3, shipType: 3, position: at, alive: false })], [wreck], 1.2);
  assert.equal(wrecks.typeOf(wreck), 3, "the Tug died here");
  const scene = buildScene([], [], [wreck], visuals, () => undefined, 0, { wreckTypeOf: (e) => wrecks.typeOf(e) });
  assert.equal(scene.opaque.wreck_tug?.length, 1);
  // The type sticks while the wreck exists, even after the death is forgotten.
  wrecks.observe([], [wreck], 1.2 + WRECK_MATCH_SECONDS + 5);
  assert.equal(wrecks.typeOf(wreck), 3);
  // A wreck far from any death (joined late) gets a stable type from its id.
  const far = entity({ id: 123, position: { x: 500, y: 0, z: 0 } });
  wrecks.observe([], [wreck, far], 20);
  assert.equal(wrecks.typeOf(far), fallbackType(123));
  assert.ok(fallbackType(123) >= 0 && fallbackType(123) < 4);
  // A removed wreck is forgotten; an id reused later gets a fresh match.
  wrecks.observe([], [], 21);
  assert.equal(wrecks.typeOf(wreck), fallbackType(99));
  // Without a tracker the scene still draws a wreck (the Tinker's).
  assert.equal(buildScene([], [], [wreck], visuals, () => undefined, 0).opaque.wreck_tinker?.length, 1);
});

test("two deaths, two wrecks: each wreck takes the nearest death", () => {
  const wrecks = new WreckTypes();
  const a = ship({ id: 1, shipType: 0, position: { x: 0, y: 0, z: 0 } });
  const b = ship({ id: 2, shipType: 2, position: { x: 2, y: 0, z: 0 } });
  wrecks.observe([a, b], [], 0);
  wrecks.observe([{ ...a, alive: false }, { ...b, alive: false }], [], 0.1);
  const wa = entity({ id: 5, position: { x: 0.1, y: 0, z: 0 } });
  const wb = entity({ id: 6, position: { x: 2.1, y: 0, z: 0 } });
  wrecks.observe([{ ...a, alive: false }, { ...b, alive: false }], [wb, wa], 0.2);
  assert.equal(wrecks.typeOf(wa), 0);
  assert.equal(wrecks.typeOf(wb), 2);
});
