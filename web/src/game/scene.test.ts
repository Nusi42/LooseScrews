// Data-driven entity and projectile rendering (scene.ts), the M2 decoders
// (views.ts), the weapon panel, name tags and the new input bindings.

import assert from "node:assert/strict";
import { test } from "node:test";
import { KEY_BINDINGS, WEAPON_ACTIONS, wheelStep, WHEEL_STEP } from "../input/bindings.ts";
import * as mat4 from "../math/mat4.ts";
import * as q from "../math/quat.ts";
import { expandTeams, SURFACE_CHARGED, SURFACE_HOT, validateMesh, MeshError } from "../renderer/mesh.ts";
import { MESH_BUILDERS, MESH_KINDS } from "../renderer/primitives.ts";
import { DEFAULT_SETTINGS } from "../renderer/renderer.ts";
import { loadoutLines } from "../ui/hud.ts";
import { nameTagModel, projectToScreen } from "../ui/nameTags.ts";
import { stubLoadout } from "../wasm/stub.ts";
import {
  buildScene,
  DEFAULT_STYLE,
  dustDensityAt,
  dustFog,
  DUST_FOG_DENSITY,
  scrapScale,
  SHAPE_STYLES,
  styleFor,
  TEAM_HULL,
} from "./scene.ts";
import {
  loadoutState,
  parseEntities,
  parsePlayers,
  parseVisuals,
  PROJ_ARMED,
  type EntityState,
  type ProjectileState,
  type ShipState,
  type VisualDef,
} from "./views.ts";

const KINDS = ["debris", "weaponPickup", "upgradePickup", "gadgetPickup", "scrapPile", "wreck", "decoy", "ghost", "dustCloud", "marker"];
const SHAPES = ["bolt", "pellet", "spike", "rocket", "missile", "mine", "orb", "blob", "arc", "grenade", "harpoon", "paint", "charge",
  "debris", "scrap", "wreck", "crate", "upgrade", "gadget", "decoy", "ghost", "dust", "duck"];

const vis = (shape: string, color: [number, number, number] = [1, 0, 0], scale = 1): VisualDef => ({ shape, color, scale });

function entity(over: Partial<EntityState>): EntityState {
  return {
    id: 1,
    kind: "debris",
    visual: 0,
    owner: 255,
    team: 255,
    position: { x: 0, y: 0, z: 0 },
    velocity: { x: 0, y: 0, z: 0 },
    rotation: q.identity(),
    state: [0, 1, 0, 0],
    radius: 0,
    ...over,
  };
}

function shot(over: Partial<ProjectileState>): ProjectileState {
  return { kind: 0, owner: 1, position: { x: 0, y: 0, z: 0 }, velocity: { x: 0, y: 0, z: -30 }, visual: 0, state: 0, ...over };
}

test("every ls-sim shape has a style whose mesh exists", () => {
  for (const shape of SHAPES) {
    const style = SHAPE_STYLES[shape];
    assert.ok(style, `style for ${shape}`);
    assert.ok(MESH_KINDS.includes(style.mesh), `${shape} -> ${style.mesh}`);
    assert.ok(style.size > 0);
  }
  assert.equal(styleFor(vis("not-a-shape")), DEFAULT_STYLE, "unknown shapes still draw");
  assert.equal(styleFor(undefined), DEFAULT_STYLE);
});

test("procedural meshes are valid and about one unit across", () => {
  for (const kind of MESH_KINDS) {
    const m = MESH_BUILDERS[kind]();
    validateMesh(m);
    assert.ok(m.indices.length >= 12, kind);
    let max = 0;
    for (const v of m.positions) max = Math.max(max, Math.abs(v));
    assert.ok(max > 0.2 && max < 1.0, `${kind} extent ${max}`);
  }
});

test("terrain surface states pack above the team byte", () => {
  const packed = expandTeams(new Uint8Array([0, 255, 1]), new Uint8Array([SURFACE_HOT, 0, SURFACE_CHARGED | SURFACE_HOT]));
  assert.deepEqual([...packed], [0 | (1 << 8), 255, 1 | (3 << 8)]);
  assert.deepEqual([...expandTeams(new Uint8Array([2]), new Uint8Array(0))], [2], "no surface states");
  const mesh = { positions: new Float32Array(9), normals: new Float32Array(9), indices: new Uint32Array([0, 1, 2]), team: new Uint8Array(3) };
  validateMesh({ ...mesh, surface: new Uint8Array(0) });
  assert.throws(() => validateMesh({ ...mesh, surface: new Uint8Array(2) }), MeshError);
});

test("projectiles: visual colour and mesh, trails, glow, team paint, blinking mines", () => {
  const visuals = [vis("rocket", [1, 0.5, 0], 2), vis("paint", [1, 1, 1]), vis("orb", [0, 0, 1]), vis("mine", [1, 0, 0]), vis("zzz")];
  const teamOf = (id: number): number | undefined => (id === 7 ? 1 : undefined);
  const scene = buildScene(
    [],
    [shot({ visual: 0 }), shot({ visual: 1, owner: 7 }), shot({ visual: 2 }), shot({ visual: 3, state: PROJ_ARMED, velocity: { x: 0, y: 0, z: 0 } }), shot({ visual: 4 })],
    [],
    visuals,
    teamOf,
    0.1,
  );
  const rocket = scene.opaque.rocket?.[0];
  assert.ok(rocket);
  assert.deepEqual(rocket.color, [1, 0.5, 0]);
  assert.ok(Math.abs(rocket.scale - (SHAPE_STYLES["rocket"]!.size * 2)) < 1e-9, "visual scale applies");
  assert.ok(q.forward(rocket.rotation).z < -0.99, "oriented along the velocity");
  const orbs = scene.opaque.orb ?? [];
  assert.deepEqual(orbs[0]!.color, TEAM_HULL[1], "paint takes the shooter's team colour");
  // Trails: translucent bolts stretched behind moving projectiles; none for the resting mine.
  const trails = scene.translucent.bolt ?? [];
  assert.equal(trails.length, 3, "rocket, paint and orb trail; the unknown default style has none");
  const trail = trails[0]!;
  assert.ok((trail.stretch ?? 1) > 1);
  assert.ok(trail.position.z > 0, "behind the rocket");
  assert.ok((trail.opacity ?? 1) < 1);
  assert.equal(scene.translucent.sphere?.length, 1, "the orb glows");
  const mine = scene.opaque.mine?.[0];
  assert.ok(mine);
  assert.ok(mine.emissive === 1 || mine.emissive === 0.2, "armed mines blink");
  assert.equal(scene.opaque.bolt?.length, 1, "unknown shapes fall back to a bolt");
});

test("entities: pickups bob and spin, scrap grows, debris and dust use their radius", () => {
  const visuals = [vis("debris", [0.5, 0.5, 0.5]), vis("crate", [0, 1, 0]), vis("scrap"), vis("dust", [0.6, 0.5, 0.4]), vis("decoy"), vis("ghost")];
  const ents = [
    entity({ id: 1, kind: "debris", visual: 0, radius: 1.2 }),
    entity({ id: 2, kind: "weaponPickup", visual: 1, position: { x: 5, y: 0, z: 0 } }),
    entity({ id: 3, kind: "scrapPile", visual: 2, state: [100, 0, 0, 0] }),
    entity({ id: 4, kind: "dustCloud", visual: 3, radius: 6 }),
    entity({ id: 5, kind: "decoy", visual: 4, team: 0 }),
    entity({ id: 6, kind: "ghost", visual: 5 }),
  ];
  const a = buildScene([], [], ents, visuals, () => undefined, 0.3);
  const b = buildScene([], [], ents, visuals, () => undefined, 1.1);
  assert.ok(Math.abs(a.opaque.debris![0]!.scale - 2.4) < 1e-9, "debris diameter from its radius");
  const crateA = a.opaque.crate![0]!, crateB = b.opaque.crate![0]!;
  assert.deepEqual(crateA.color, [0, 1, 0], "weapon pickups take the weapon colour");
  assert.notEqual(crateA.position.y, crateB.position.y, "pickups bob");
  assert.notDeepEqual(crateA.rotation, crateB.rotation, "pickups spin");
  assert.ok(a.opaque.scrap![0]!.scale > SHAPE_STYLES["scrap"]!.size, "a big pile is bigger");
  assert.ok(scrapScale(1e6) <= 2.5 && scrapScale(0) > 0);
  const dust = a.translucent.sphere?.find((i) => Math.abs(i.scale - 12) < 1e-9);
  assert.ok(dust, "dust is a translucent volume of its diameter");
  assert.ok((dust.opacity ?? 1) < 1);
  const ships = a.opaque.ship ?? [];
  assert.equal(ships.length, 1, "the decoy looks like a ship");
  assert.deepEqual(ships[0]!.color, TEAM_HULL[0], "in its team colour");
  assert.equal(a.translucent.ship?.length, 1, "the ghost is translucent");
});

test("dust thickens the fog inside a cloud", () => {
  const visuals = [vis("dust")];
  const cloud = entity({ kind: "dustCloud", radius: 10, position: { x: 0, y: 0, z: 0 } });
  assert.equal(dustDensityAt({ x: 20, y: 0, z: 0 }, [cloud], visuals), 0);
  assert.ok(Math.abs(dustDensityAt({ x: 5, y: 0, z: 0 }, [cloud], visuals) - 0.5) < 1e-9);
  assert.equal(dustDensityAt({ x: 0, y: 0, z: 0 }, [cloud], visuals), 1);
  assert.deepEqual(dustFog(DEFAULT_SETTINGS, 0), { fogDensity: DEFAULT_SETTINGS.fogDensity, fogColor: DEFAULT_SETTINGS.fogColor });
  assert.equal(dustFog(DEFAULT_SETTINGS, 1).fogDensity, DUST_FOG_DENSITY);
});

test("M2 decoders: entities, visuals, players and the loadout", () => {
  const ents = parseEntities(
    [300, 5, 2, 255, 255, 1, 2, 3, 0, 0, 0, 0, 0, 0, 1, 12, 0, 0, 0, 0.8, /* truncated: */ 1, 2],
    KINDS,
  );
  assert.equal(ents.length, 1);
  assert.equal(ents[0]!.kind, "wreck");
  assert.deepEqual(ents[0]!.state, [12, 0, 0, 0]);
  assert.ok(Math.abs(ents[0]!.radius - 0.8) < 1e-6);
  assert.equal(parseEntities([0, 99, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0], KINDS)[0]!.kind, "", "unknown kind");

  const visuals = parseVisuals([3, 1, 0.5, 0, 2, 99, 0, 0, 0, 0], SHAPES);
  assert.deepEqual(visuals[0], { shape: "rocket", color: [1, 0.5, 0], scale: 2 });
  assert.equal(visuals[1]!.shape, "", "unknown shape index");
  assert.equal(visuals[1]!.scale, 1, "zero scale falls back to 1");

  const players = parsePlayers([2, 1, 3, 255, 5, 0, 1, 0], ["Ada"]);
  assert.deepEqual(players[0], { id: 2, team: 1, shipType: 3, gadget: 255, name: "Ada" });
  assert.equal(players[1]!.name, "Player 6", "missing names get a fallback");

  let freed = false;
  const l = loadoutState(
    stubLoadout({
      names: ["Scrap Laser", "Rivet Gun", "Bolt Rocket"],
      ammo: ["∞", "12 shots", "3"],
      info: new Uint8Array([0, 0, 1, 3, 0, 1, 2, 1, 1, 1, 0, 1]),
      maxLevels: new Uint8Array([6, 1, 1]),
      gadget: "Shield",
      gadgetActive: true,
      free: () => {
        freed = true;
      },
    }),
  );
  assert.ok(freed);
  assert.equal(l.primaries.length, 2);
  assert.equal(l.secondaries.length, 1);
  assert.deepEqual(l.primaries[0], { name: "Scrap Laser", ammo: "∞", secondary: false, selected: false, key: 1, level: 3, maxLevel: 6 });
  assert.equal(l.primaries[1]!.selected, true);

  const lines = loadoutLines(l);
  assert.deepEqual(
    lines.map((x) => [x.key, x.name, x.detail, x.selected, x.kind]),
    [
      ["1", "Scrap Laser", "L3/6 · ∞", false, "primary"],
      ["2", "Rivet Gun", "12 shots", true, "primary"],
      ["X", "Bolt Rocket", "3", true, "secondary"],
      ["G", "Shield", "on", true, "gadget"],
    ],
  );
  const tenth = loadoutLines({ ...l, primaries: [{ ...l.primaries[0]!, key: 10 }], secondaries: [], gadget: "" });
  assert.equal(tenth[0]!.key, "0", "the tenth primary is on the 0 key");
});

test("name tags: projection, range, facing and names", () => {
  const view = mat4.viewFromPose({ x: 0, y: 0, z: 0 }, q.identity());
  const vp = mat4.multiply(mat4.perspective(Math.PI / 2, 2, 0.1, 500), view);
  const centre = projectToScreen(vp, { x: 0, y: 0, z: -10 }, 800, 400);
  assert.ok(centre);
  assert.ok(Math.abs(centre.x - 400) < 1e-3 && Math.abs(centre.y - 200) < 1e-3);
  const up = projectToScreen(vp, { x: 0, y: 2, z: -10 }, 800, 400)!;
  assert.ok(up.y < 200, "up is towards the top of the screen");
  assert.equal(projectToScreen(vp, { x: 0, y: 0, z: 10 }, 800, 400), null, "behind the camera");

  const ship = (id: number, team: number, z: number, alive = true): ShipState => ({
    id, team, alive, boosting: false, position: { x: 0, y: 0, z }, orientation: q.identity(), shipType: 1, radius: 0.5, gadgetActive: false,
  });
  const players = [{ id: 1, team: 0, shipType: 1, gadget: 255, name: "Mate" }, { id: 2, team: 1, shipType: 1, gadget: 255, name: "Foe" }];
  const tags = nameTagModel(
    [ship(1, 0, -80), ship(2, 1, -20), ship(3, 1, -60), ship(4, 1, -5, false), ship(5, 0, 30)],
    players,
    { x: 0, y: 0, z: 0 },
    vp,
    800,
    400,
    0,
  );
  assert.deepEqual(tags.map((t) => t.name), ["Foe", "Mate"], "nearest first; far enemies, dead and hidden ships skipped");
  assert.equal(tags[1]!.teammate, true);
  assert.ok(tags[0]!.opacity > tags[1]!.opacity, "far labels fade");
});

test("M2 key scheme: 1-0, X, G and the wheel", () => {
  assert.equal(KEY_BINDINGS["Digit0"], "weapon10");
  assert.equal(KEY_BINDINGS["Digit7"], "weapon7");
  assert.equal(KEY_BINDINGS["KeyX"], "cycleSecondary");
  assert.equal(KEY_BINDINGS["KeyG"], "gadget");
  assert.equal(KEY_BINDINGS["KeyF"], "overdrive", "F stays overdrive");
  assert.equal(WEAPON_ACTIONS.length, 10);
  assert.equal(WEAPON_ACTIONS.indexOf("weapon10") + 1, 10);
  assert.deepEqual(wheelStep(WHEEL_STEP * 3), [1, 0], "one step per frame, the rest dropped");
  assert.deepEqual(wheelStep(-WHEEL_STEP), [-1, 0]);
  assert.deepEqual(wheelStep(10), [0, 10], "small movements accumulate");
});
