import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import { HULL_RADIUS } from "../renderer/models/ships.ts";
import type { ParticleSpec } from "../renderer/particles.ts";
import { mulberry32, SPRITE } from "../renderer/spriteAtlas.ts";
import { CUE, NO_SHIP, type Cue } from "./mechanics.ts";
import { buildScene } from "./scene.ts";
import {
  CUE_HIT,
  engineMountsOf,
  explosionRadius,
  hidesMesh,
  IMPACT_RANGE,
  shapeVfx,
  Vfx,
  VFX_SHAPES,
  type VfxFrame,
} from "./vfx.ts";
import { count, explosion, lin, type VfxSink } from "./vfxLibrary.ts";
import type { ProjectileState, ShipState, VisualDef } from "./views.ts";

class Recorder implements VfxSink {
  now = 0;
  density = 1;
  spawned: ParticleSpec[] = [];
  immediates: ParticleSpec[] = [];
  spawn(p: ParticleSpec): void {
    this.spawned.push(p);
  }
  immediate(p: ParticleSpec): void {
    this.immediates.push(p);
  }
  beginFrame(): void {
    this.immediates = [];
  }
  take(): ParticleSpec[] {
    const s = this.spawned;
    this.spawned = [];
    return s;
  }
}

const VISUALS: VisualDef[] = [
  { shape: "bolt", color: [1, 0.3, 0.2], scale: 1 },
  { shape: "rocket", color: [1, 0.55, 0.25], scale: 1 },
  { shape: "orb", color: [0.35, 0.8, 1], scale: 1 },
  { shape: "never-seen", color: [1, 1, 1], scale: 1 },
];
const BOLT = 0;
const ROCKET = 1;

const ORIGIN: Vec3 = { x: 0, y: 0, z: 0 };

function ship(id: number, position: Vec3, over: Partial<ShipState> = {}): ShipState {
  return { id, team: 1, alive: true, boosting: false, position, orientation: q.identity(), shipType: 1, radius: 0.5, gadgetActive: false, ...over };
}

function shot(owner: number, visual: number, position: Vec3, velocity: Vec3, state = 0): ProjectileState {
  return { kind: 0, owner, position, velocity, visual, state };
}

function frame(time: number, over: Partial<VfxFrame> = {}): VfxFrame {
  return {
    time,
    dt: 1 / 60,
    camera: ORIGIN,
    player: 0,
    pose: { position: ORIGIN, orientation: q.identity(), velocity: ORIGIN, alive: true },
    ships: [],
    shots: [],
    entities: [],
    visuals: VISUALS,
    cues: [],
    beams: [],
    flashes: [],
    ...over,
  };
}

/** A fresh Vfx with a seeded RNG that has seen one empty frame (primed). */
function setup(): { vfx: Vfx; sink: Recorder } {
  const sink = new Recorder();
  const vfx = new Vfx(sink, mulberry32(7));
  vfx.update(frame(0));
  sink.take();
  return { vfx, sink };
}

test("every known shape has effects; energy shots replace their mesh, rockets keep theirs", () => {
  for (const shape of ["bolt", "pellet", "spike", "rocket", "missile", "mine", "orb", "blob", "arc", "grenade", "harpoon", "paint", "charge"]) {
    assert.ok(VFX_SHAPES[shape], shape);
  }
  assert.ok(hidesMesh(VISUALS[BOLT]));
  assert.ok(!hidesMesh(VISUALS[ROCKET]));
  // Unknown shapes still get an energy look (data-driven: no client code for new weapons).
  assert.equal(shapeVfx(VISUALS[3]).impact, "sparks");
  assert.ok(explosionRadius(VFX_SHAPES["rocket"]!, 1.8) > 2 * explosionRadius(VFX_SHAPES["rocket"]!, 1), "heavier rockets blow up bigger");
});

test("with vfx the scene drops the mesh effects: bolt meshes, trails, flash and glow spheres", () => {
  const ships = [ship(1, { x: 0, y: 0, z: -10 }, { gadgetActive: true })];
  const shots = [shot(1, BOLT, { x: 0, y: 0, z: -8 }, { x: 0, y: 0, z: 90 }), shot(1, ROCKET, { x: 1, y: 0, z: -8 }, { x: 0, y: 0, z: 40 })];
  const flashes = [{ position: ORIGIN, color: [1, 1, 1] as const, radius: 2, strength: 1, ring: false }];
  const legacy = buildScene(ships, shots, [], VISUALS, () => 1, 0, { flashes, gadgetOf: () => "shield" });
  const vfx = buildScene(ships, shots, [], VISUALS, () => 1, 0, { flashes, gadgetOf: () => "shield", vfx: true });
  assert.equal(legacy.opaque.bolt?.length, 1);
  assert.equal(vfx.opaque.bolt, undefined, "the bolt is a particle core");
  assert.equal(vfx.opaque.rocket?.length, 1, "rockets keep their mesh");
  assert.ok((legacy.translucent.bolt?.length ?? 0) >= 2, "legacy trails");
  assert.equal(vfx.translucent.bolt, undefined, "no stretched-mesh trails");
  // Only the shield bubble stays a sphere (fainter); the flash is a sprite.
  assert.equal(vfx.translucent.sphere?.length, 1);
  assert.ok((vfx.translucent.sphere?.[0]?.emissive ?? 1) < (legacy.translucent.sphere?.[0]?.emissive ?? 0));
});

test("a new shot next to its owner flashes the muzzle once; shots appearing far away do not", () => {
  const { vfx, sink } = setup();
  const owner = ship(3, { x: 0, y: 0, z: -20 });
  const fan = [0, 1, 2].map((i) => shot(3, BOLT, { x: i * 0.2, y: 0, z: -19 }, { x: 0, y: 0, z: 90 }));
  vfx.update(frame(1, { ships: [owner], shots: fan }));
  assert.equal(vfx.counts.muzzles, 1, "one flash per owner and visual per frame (a fan)");
  assert.ok(sink.take().some((p) => p.sprite === SPRITE.flare), "a star flare");
  vfx.update(frame(1 + 1 / 60, { ships: [owner], shots: [shot(9, BOLT, { x: 0, y: 0, z: -40 }, { x: 0, y: 0, z: 90 })] }));
  assert.equal(vfx.counts.muzzles, 1, "no owner nearby");
});

test("no muzzle flash on the first frame (shots already in flight when the match view starts)", () => {
  const sink = new Recorder();
  const vfx = new Vfx(sink, mulberry32(1));
  vfx.update(frame(0, { ships: [ship(3, { x: 0, y: 0, z: -20 })], shots: [shot(3, BOLT, { x: 0, y: 0, z: -19 }, { x: 0, y: 0, z: 90 })] }));
  assert.equal(vfx.counts.muzzles, 0);
});

test("shots are tracked frame to frame: trails behind, an impact where one disappears", () => {
  const { vfx, sink } = setup();
  const dt = 1 / 60;
  let t = 1;
  let z = -30;
  for (let i = 0; i < 10; i++) {
    vfx.update(frame(t, { shots: [shot(5, BOLT, { x: 2, y: 0, z }, { x: 0, y: 0, z: 90 })] }));
    t += dt;
    z += 90 * dt;
  }
  const trail = sink.take().filter((p) => p.sprite === SPRITE.streak);
  assert.ok(trail.length >= 9 * 6, `ribbon sections every frame (${trail.length})`);
  // Sections are spawned along the whole path, with birth times spread over each frame.
  const zs = trail.map((p) => p.pos.z).sort((a, b) => a - b);
  const gaps = zs.slice(1).map((v, i) => v - (zs[i] ?? v));
  assert.ok(Math.max(...gaps) < 0.4, `a continuous ribbon (largest gap ${Math.max(...gaps).toFixed(2)} u)`);
  assert.equal(vfx.counts.impacts, 0);
  // The core is drawn as immediate sprites, not spawned.
  assert.ok(sink.immediates.length >= 2);
  vfx.update(frame(t, { shots: [] }));
  assert.equal(vfx.counts.impacts, 1, "it hit something");
  const burst = sink.take();
  assert.ok(burst.some((p) => p.sprite === SPRITE.spark), "sparks");
  assert.ok(burst.some((p) => p.blend === "alpha"), "rock dust (no ship nearby)");
});

test("a rocket that disappears explodes: fireball, smoke, sparks, ring and a camera shake nearby", () => {
  const { vfx, sink } = setup();
  const at: Vec3 = { x: 0, y: 0, z: -12 };
  vfx.update(frame(1, { shots: [shot(4, ROCKET, at, { x: 0, y: 0, z: -40 })] }));
  sink.take();
  vfx.update(frame(1 + 1 / 60));
  assert.equal(vfx.counts.explosions, 1);
  const blast = sink.take();
  assert.ok(blast.filter((p) => p.sprite === SPRITE.flame && p.hot).length >= 10, "fireball");
  assert.ok(blast.filter((p) => p.blend === "alpha").length >= 8, "smoke and dust");
  assert.ok(blast.some((p) => p.sprite === SPRITE.ring), "shockwave ring");
  assert.ok(blast.some((p) => (p.t0 ?? 0) > 1 + 1 / 60), "smoke follows the fire (delayed births)");
  assert.ok(vfx.shake(1 + 1 / 60) > 0.01, "the camera shakes");
  assert.equal(vfx.shake(5), 0, "and settles");
});

test("impacts are skipped beyond IMPACT_RANGE and inside the own ship", () => {
  const { vfx } = setup();
  vfx.update(frame(1, { shots: [shot(4, ROCKET, { x: 0, y: 0, z: -(IMPACT_RANGE + 20) }, { x: 0, y: 0, z: -40 }), shot(4, BOLT, { x: 0, y: 0, z: -0.2 }, { x: 0, y: 0, z: 1 })] }));
  vfx.update(frame(1 + 1 / 60));
  assert.equal(vfx.counts.impacts, 0);
});

test("a shot that vanishes at a shielded ship ripples the bubble instead of sparking the hull", () => {
  const { vfx, sink } = setup();
  const target = ship(2, { x: 0, y: 0, z: -15 }, { gadgetActive: true });
  const gadgetOf = (id: number): string => (id === 2 ? "shield" : "");
  vfx.update(frame(1, { ships: [target], shots: [shot(6, BOLT, { x: 0, y: 0, z: -16.6 }, { x: 0, y: 0, z: 30 })], gadgetOf }));
  sink.take();
  vfx.update(frame(1 + 1 / 60, { ships: [target], gadgetOf }));
  assert.equal(vfx.counts.ripples, 1);
  const ripple = sink.take();
  const rings = ripple.filter((p) => p.sprite === SPRITE.ring && p.orient === "plane");
  assert.ok(rings.length >= 1, "a ring lying on the bubble");
  // The ring sits on the bubble on the side the shot came from.
  const r = rings[0]!;
  assert.ok(r.pos.z < -15 - 0.5 * 1.7 * 0.5, `on the far side (z ${r.pos.z.toFixed(2)})`);
});

test("a ship that dies explodes with hull debris; a respawn shimmers", () => {
  const { vfx, sink } = setup();
  const p: Vec3 = { x: 3, y: 0, z: -20 };
  vfx.update(frame(1, { ships: [ship(8, p)] }));
  sink.take();
  vfx.update(frame(1 + 1 / 60, { ships: [ship(8, p, { alive: false })] }));
  assert.equal(vfx.counts.deaths, 1);
  const blast = sink.take();
  assert.ok(blast.some((p) => p.sprite === SPRITE.shard), "hull shards");
  vfx.update(frame(2, { ships: [ship(8, { x: 50, y: 0, z: 0 })] }));
  assert.ok(sink.take().some((p) => p.sprite === SPRITE.ring), "respawn ring");
});

test("hit cues add up: below 30 % hull a ship smokes, above it does not", () => {
  const { vfx, sink } = setup();
  const s = ship(4, { x: 0, y: 0, z: -10 });
  const hit = (damage: number): Cue => ({ kind: CUE_HIT, source: 1, target: 4, position: s.position, value: damage, tick: 0 });
  const smoke = (): number => sink.take().filter((p) => p.blend === "alpha" && p.sprite >= SPRITE.smoke0 && p.sprite <= SPRITE.smoke3).length;
  vfx.update(frame(1, { ships: [s], cues: [hit(60)], hullOf: () => 100 }));
  for (let i = 1; i < 30; i++) vfx.update(frame(1 + i / 60, { ships: [s], hullOf: () => 100 }));
  assert.equal(smoke(), 0, "60 of 100 taken: no smoke");
  vfx.update(frame(2, { ships: [s], cues: [hit(20)], hullOf: () => 100 }));
  for (let i = 1; i < 30; i++) vfx.update(frame(2 + i / 60, { ships: [s], hullOf: () => 100 }));
  assert.ok(smoke() >= 4, "80 of 100 taken: smoke");
  // A respawn heals.
  vfx.update(frame(3, { ships: [{ ...s, alive: false }], hullOf: () => 100 }));
  vfx.update(frame(4, { ships: [s], hullOf: () => 100 }));
  sink.take();
  for (let i = 1; i < 30; i++) vfx.update(frame(4 + i / 60, { ships: [s], hullOf: () => 100 }));
  assert.equal(smoke(), 0);
});

test("engines: a flame per nozzle every frame, a denser and longer plume at boost, none while cloaked", () => {
  for (let t = 0; t < 4; t++) {
    const mounts = engineMountsOf(t);
    assert.ok(mounts.length >= 1, `ship type ${t} has engines`);
    for (const m of mounts) {
      assert.ok(m.pos.z > 0.2, "at the rear (+Z)");
      assert.ok(Math.hypot(m.pos.x, m.pos.y, m.pos.z) <= HULL_RADIUS * 1.05, "inside the hull radius");
      assert.ok(m.radius > 0.01 && m.radius < 0.1);
    }
  }
  assert.deepEqual([0, 1, 2, 3].map((t) => engineMountsOf(t).length), [1, 4, 4, 3], "Skitter 1, Tinker 4 (pods), Hauler 4, Tug 3 engines");
  const run = (over: Partial<ShipState>, gadget = ""): { imm: number; spawned: number } => {
    const { vfx, sink } = setup();
    let spawned = 0;
    for (let i = 0; i < 60; i++) {
      vfx.update(frame(1 + i / 60, { ships: [ship(2, { x: 0, y: 0, z: -10 }, { shipType: 2, radius: 0.65, ...over })], gadgetOf: () => gadget }));
      spawned += sink.take().length;
    }
    return { imm: sink.immediates.length, spawned };
  };
  const idle = run({});
  const boost = run({ boosting: true });
  assert.ok(idle.imm >= 2 * 4, "core and flame per nozzle");
  assert.ok(boost.spawned > 2 * idle.spawned, `boost ${boost.spawned} vs idle ${idle.spawned} per second`);
  const cloaked = run({ gadgetActive: true }, "cloak");
  assert.equal(cloaked.imm, 0, "a cloaked ship shows no flame");
});

test("beams crackle as immediate streak sections; timed glows replace the flash spheres", () => {
  const { vfx, sink } = setup();
  vfx.update(
    frame(1, {
      beams: [{ from: ORIGIN, to: { x: 0, y: 0, z: -12 }, color: [0.45, 1, 1], strength: 1 }],
      flashes: [{ position: { x: 1, y: 0, z: -5 }, color: [1, 1, 1], radius: 2, strength: 0.8, ring: false }],
    }),
  );
  const streaks = sink.immediates.filter((p) => p.sprite === SPRITE.streak && p.orient === "fixed");
  assert.ok(streaks.length >= 8, `${streaks.length} sections`);
  assert.ok(sink.immediates.some((p) => p.sprite === SPRITE.glow && p.pos.x === 1), "the flash glows");
});

test("shockwave cues explode at their radius; pickups sparkle and burst when taken", () => {
  const { vfx, sink } = setup();
  vfx.update(frame(1, { cues: [{ kind: CUE.shockwave, source: NO_SHIP, target: NO_SHIP, position: { x: 0, y: 0, z: -30 }, value: 60, tick: 0 }] }));
  assert.equal(vfx.counts.explosions, 1);
  const big = sink.take().filter((p) => p.sprite === SPRITE.ring);
  assert.ok(big.some((p) => (p.size1 ?? 0) >= 100), "a Tectonic-sized ring");
  const pickup = { id: 9, kind: "weaponPickup", visual: 2, owner: 255, team: 255, position: { x: 0, y: 0, z: -6 }, velocity: ORIGIN, rotation: q.identity(), state: [0, 0, 0, 0] as [number, number, number, number], radius: 0.5 };
  let glints = 0;
  for (let i = 0; i < 60; i++) {
    vfx.update(frame(2 + i / 60, { entities: [pickup] }));
    glints += sink.take().filter((p) => p.sprite === SPRITE.flare).length;
  }
  assert.ok(glints >= 1, "sparkle");
  vfx.update(frame(3.1, { entities: [] }));
  assert.ok(sink.take().filter((p) => p.sprite === SPRITE.spark).length >= 10, "taken: a burst");
});

test("seeded runs are identical; density scales the particle counts", () => {
  const runOnce = (density: number): number => {
    const sink = new Recorder();
    sink.density = density;
    explosion(sink, mulberry32(3), ORIGIN, 3, { terrain: true, hull: true });
    return sink.spawned.length;
  };
  assert.equal(runOnce(1), runOnce(1));
  assert.ok(runOnce(0.35) < 0.5 * runOnce(1), `${runOnce(0.35)} vs ${runOnce(1)}`);
  const sink = new Recorder();
  sink.density = 0.5;
  const rng = mulberry32(5);
  let n = 0;
  for (let i = 0; i < 1000; i++) n += count(sink, rng, 3);
  assert.ok(Math.abs(n / 1000 - 1.5) < 0.1, "fractional counts average out");
  assert.deepEqual(lin([1, 1, 1], 2), [2, 2, 2]);
});

test("effects give off short lights for VQ6: explosions bright and wide, muzzle flashes brief", () => {
  const { vfx } = setup();
  const owner = ship(3, { x: 0, y: 0, z: -20 });
  vfx.update(frame(1, { ships: [owner], shots: [shot(3, ROCKET, { x: 0, y: 0, z: -19 }, { x: 0, y: 0, z: 40 })] }));
  const muzzle = vfx.lights(1);
  assert.equal(muzzle.length, 1);
  assert.equal(muzzle[0]!.strength, 1);
  assert.equal(vfx.lights(1.2).length, 0, "a muzzle light lasts under 0.1 s");
  vfx.update(frame(1 + 1 / 60, { ships: [owner] }));
  const blast = vfx.lights(1 + 1 / 60);
  assert.ok(blast.some((l) => l.radius > 5 && l.color[0] > 1), "the rocket's explosion");
  const later = vfx.lights(1.3).find((l) => l.radius > 5)!;
  assert.ok(later.strength > 0 && later.strength < 1, "fading");
  vfx.reset();
  assert.equal(vfx.lights(1.3).length, 0);
});
