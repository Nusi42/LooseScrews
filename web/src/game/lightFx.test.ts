import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import {
  ENGINE_LIGHT,
  LIGHT_PRESETS,
  LightFx,
  MAX_PULSES,
  MINE_BLINK,
  objectiveLight,
  pulseLevel,
  pulseLight,
  sceneLights,
  SHAPE_LIGHTS,
} from "./lightFx.ts";
import { OBJECTIVE, type Objective } from "./modeState.ts";
import { PROJ_ARMED, type ProjectileState, type ShipState, type VisualDef } from "./views.ts";

const at = (x: number, y: number, z: number): Vec3 => ({ x, y, z });
const VISUALS: VisualDef[] = [
  { shape: "bolt", color: [0.3, 1, 0.4], scale: 1 },
  { shape: "rocket", color: [0.6, 0.6, 0.6], scale: 1 },
  { shape: "mine", color: [0.5, 0.5, 0.5], scale: 1 },
  { shape: "harpoon", color: [0.5, 0.5, 0.5], scale: 1 },
];
const shot = (visual: number, position: Vec3, velocity: Vec3 = at(0, 0, -50), owner = 1, state = 0): ProjectileState => ({
  kind: 0,
  owner,
  position,
  velocity,
  visual,
  state,
});
const ship = (id: number, extra: Partial<ShipState> = {}): ShipState => ({
  id,
  team: 0,
  alive: true,
  boosting: false,
  position: at(0, 0, 0),
  orientation: q.identity(),
  shipType: 1,
  radius: 0.5,
  gadgetActive: false,
  ...extra,
});

test("pulse curves: dark outside their life, rise over the attack, decay to 0", () => {
  for (const [name, p] of Object.entries(LIGHT_PRESETS)) {
    assert.equal(pulseLevel(p, -0.01), 0, name);
    assert.equal(pulseLevel(p, p.duration), 0, name);
    const peakAt = "attack" in p ? p.attack : 0;
    assert.ok(pulseLevel(p, peakAt) > 0.9, `${name} peaks`);
    assert.ok(pulseLevel(p, p.duration * 0.9) < pulseLevel(p, p.duration * 0.2), `${name} decays`);
  }
  // A flash drops fast, then glows on.
  const ex = LIGHT_PRESETS.explosion;
  assert.ok(pulseLevel(ex, ex.duration * 0.25) < 0.4);
  assert.ok(pulseLevel(ex, ex.duration * 0.6) > 0.01);
  // Explosions cool from white-hot toward orange and grow.
  const early = pulseLight(ex, at(0, 0, 0), 0.03)!;
  const late = pulseLight(ex, at(0, 0, 0), ex.duration * 0.7)!;
  assert.ok(late.color[1] < early.color[1], "cooler");
  assert.ok((late.range ?? 0) > (early.range ?? 0), "grows");
  assert.equal(pulseLight(ex, at(0, 0, 0), ex.duration + 1), null);
});

test("LightFx: emitted pulses live for their duration, the oldest drop at the cap", () => {
  const fx = new LightFx();
  fx.random = () => 0.5;
  fx.emit("explosion", at(1, 2, 3), 10);
  fx.emit({ color: [0, 1, 0], intensity: 3, duration: 1 }, at(0, 0, 0), 10);
  assert.equal(fx.frame(10.1).length, 2);
  const lights = fx.frame(10.5);
  assert.equal(lights.length, 2);
  assert.ok(lights[0]!.intensity < LIGHT_PRESETS.explosion.intensity);
  assert.equal(fx.frame(10.8).length, 1, "the explosion is over");
  assert.equal(fx.frame(11.1).length, 0);
  assert.equal(fx.live, 0);
  for (let i = 0; i < MAX_PULSES + 20; i++) fx.emit("spark", at(i, 0, 0), 20);
  assert.equal(fx.live, MAX_PULSES);
  // Colour and scale options.
  const one = new LightFx();
  one.emit("muzzle", at(0, 0, 0), 0, { color: [0, 0, 1], scale: 2 });
  const l = one.frame(0)[0]!;
  assert.deepEqual(l.color, [0, 0, 1]);
  assert.ok(Math.abs(l.intensity - LIGHT_PRESETS.muzzle.intensity * 2) < 1e-9);
});

test("observe: a vanished rocket explodes, a bolt sparks, a shot still in flight does not", () => {
  const fx = new LightFx();
  fx.random = () => 0.5;
  const rocket = shot(1, at(0, 0, -10), at(0, 0, -40));
  const bolt = shot(0, at(5, 0, -10), at(0, 0, -100), 2);
  fx.observe([rocket, bolt], VISUALS, 1.0);
  assert.equal(fx.live, 0, "the first call only learns the shots");
  // 1/60 s later both moved on as predicted.
  const dt = 1 / 60;
  const moved = [shot(1, at(0, 0, -10 - 40 * dt), at(0, 0, -40)), shot(0, at(5, 0, -10 - 100 * dt), at(0, 0, -100), 2)];
  fx.observe(moved, VISUALS, 1.0 + dt);
  assert.equal(fx.live, 0, "matched, nothing emitted");
  // Then both vanish: an explosion and a spark (tinted with the bolt colour).
  fx.observe([], VISUALS, 1.0 + 2 * dt);
  const lights = fx.frame(1.0 + 3 * dt);
  assert.equal(lights.length, 2);
  const boom = lights.find((l) => l.intensity > 30)!;
  assert.ok(boom, "explosion");
  assert.ok(Math.abs(boom.position.z - (-10 - 40 * dt - 40 * dt * 0.5)) < 1e-6, "between its last position and the next");
  const spark = lights.find((l) => l !== boom)!;
  assert.equal(spark.color[1], 1, "spark takes the bolt's (brightened) colour");
});

test("observe: new shots flash at the muzzle, not stuck ones; gaps and harpoons emit nothing", () => {
  const fx = new LightFx();
  fx.random = () => 0.5;
  fx.observe([], VISUALS, 0);
  fx.observe([shot(0, at(0, 0, -1)), shot(3, at(1, 0, -1)), shot(0, at(2, 0, -1), at(0, 0, 0), 1, 1)], VISUALS, 0.016);
  assert.equal(fx.live, 2, "bolt and harpoon flash; the stuck one does not");
  const fx2 = new LightFx();
  fx2.observe([shot(1, at(0, 0, 0))], VISUALS, 0);
  fx2.observe([], VISUALS, 2);
  assert.equal(fx2.live, 0, "after a long gap nothing is emitted");
  const fx3 = new LightFx();
  fx3.watchProjectiles = false;
  fx3.observe([shot(1, at(0, 0, 0))], VISUALS, 0);
  fx3.observe([], VISUALS, 0.016);
  assert.equal(fx3.live, 0, "watching off (an effect system emits impacts itself)");
  assert.equal(SHAPE_LIGHTS["harpoon"]!.impact, null);
});

test("observeShips: a ship dying emits a death blast; clear forgets", () => {
  const fx = new LightFx();
  fx.observeShips([ship(1), ship(2)], 0);
  fx.observeShips([ship(1, { alive: false }), ship(2)], 0.1);
  assert.equal(fx.live, 1);
  fx.observeShips([ship(1, { alive: false }), ship(2)], 0.2);
  assert.equal(fx.live, 1, "only once");
  fx.clear();
  assert.equal(fx.live, 0);
});

test("sceneLights: engines behind ships (brighter boosting, none cloaked), shots, mines, flashes, beams", () => {
  const lights = sceneLights({
    ships: [ship(1), ship(2, { boosting: true, position: at(10, 0, 0) }), ship(3, { gadgetActive: true }), ship(4, { alive: false })],
    gadgetOf: (id) => (id === 3 ? "cloak" : ""),
    shots: [shot(0, at(0, 0, -5)), shot(2, at(0, 0, -6), at(0, 0, 0), 1, PROJ_ARMED), shot(1, at(0, 0, -20), at(0, 0, -40))],
    visuals: VISUALS,
    flashes: [{ position: at(3, 3, 3), color: [1, 1, 1], radius: 2, strength: 1, ring: false }],
    beams: [{ from: at(0, 0, 0), to: at(0, 0, -8), color: [0.5, 1, 0.5], strength: 1 }],
    time: 0.1,
  });
  const engines = lights.filter((l) => l.color === ENGINE_LIGHT.idle.color || l.color === ENGINE_LIGHT.boost.color);
  assert.equal(engines.length, 2, "living, uncloaked ships only");
  const idle = engines.find((l) => l.color === ENGINE_LIGHT.idle.color)!;
  const boost = engines.find((l) => l.color === ENGINE_LIGHT.boost.color)!;
  assert.ok(boost.intensity > idle.intensity * 2);
  assert.ok(idle.position.z > 0, "behind the ship (its nose points to -z)");
  // Bolt light in its colour, the armed mine blinks (on at t = 0.1), the rocket's motor sits behind it.
  assert.ok(lights.some((l) => l.position.z === -5 && l.color[1] === 1));
  assert.ok(lights.some((l) => l.position.z === -6 && l.intensity === MINE_BLINK));
  assert.ok(lights.some((l) => l.position.z > -20 && l.position.z < -19.3));
  // Flash and beam (end and middle).
  assert.ok(lights.some((l) => l.position.x === 3 && l.intensity > 5));
  assert.equal(lights.filter((l) => l.color[1] === 1 && l.color[0] === 0.5).length, 2);
});

test("objective lights: reactor pulses faster near detonation, core and extraction glow, zones do not", () => {
  const obj = (kind: number, timerEnd = 0): Objective => ({ id: 1, kind, team: -1, holder: -1, state: 0, timerEnd, radius: 5, pos: at(0, 0, 0) });
  const series = (o: Objective, nowTick: number): number[] => Array.from({ length: 40 }, (_, i) => objectiveLight(o, i * 0.05, nowTick)!.intensity);
  const crossings = (xs: number[]): number => xs.slice(1).filter((x, i) => (x > 9) !== (xs[i]! > 9)).length;
  const calm = series(obj(OBJECTIVE.reactor, 60 * 60), 0);
  const close = series(obj(OBJECTIVE.reactor, 60 * 60), 60 * 58);
  assert.ok(crossings(close) > crossings(calm), "faster pulse in the last seconds");
  assert.ok(objectiveLight(obj(OBJECTIVE.core), 0, 0)!.intensity > 0);
  assert.ok(objectiveLight(obj(OBJECTIVE.extraction), 0, 0)!.intensity > 0);
  assert.equal(objectiveLight(obj(OBJECTIVE.safeZone), 0, 0), null);
  assert.ok((objectiveLight(obj(OBJECTIVE.core), 0, 0)!.priority ?? 1) > 1, "objectives win the light budget");
});
