import { test } from "node:test";
import assert from "node:assert/strict";
import * as q from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { RaceState } from "../modeState.ts";
import { azimuth, prograde, ringPoint, type Belt } from "./course.ts";
import {
  gantryPose,
  gateLooks,
  GO_SECS,
  lapNumber,
  PEBBLE_RANGE,
  PebbleField,
  RaceVisuals,
  startLights,
  TickClock,
  type RaceFxInput,
} from "./raceFx.ts";
import type { Instance, SceneInstances } from "../../renderer/renderer.ts";
import { packBelt, signCorners, SIGN_WORLD_HEIGHT } from "../../renderer/belt.ts";
import { BELT_SLOT, BELT_UNIFORM_FLOATS, BELT_VOLUME_DIVISOR, SIGN_WORLD_WIDTH } from "../../renderer/shaders/belt.ts";

const BELT: Belt = {
  center: { x: 160, y: 48, z: 160 },
  innerRadius: 95,
  outerRadius: 145,
  halfThickness: 12,
  cocoonRadius: 30,
  exitRadius: 6,
  gateInnerRadius: 87,
  gateOuterRadius: 153,
  slots: Array.from({ length: 8 }, (_, k) => ({ pos: ringPoint({ center: { x: 160, y: 48, z: 160 } } as Belt, (k * Math.PI) / 4 + 0.05, 120), rot: q.identity() })),
  pickupPoints: [],
  boulders: [{ radius: 120, height: 2, phase: 0.3, angularSpeed: 0.02, size: 3, tumbleAxis: { x: 0, y: 1, z: 0 }, tumbleSpeed: 0.5 }],
};

function race(phase: RaceState["phase"], phaseEndTick: number, gates = 0, finishTick = 0, heat = 0): RaceState {
  return { heat, heats: 3, phase, phaseEndTick, racers: [{ id: 0, gates, nextGate: 1, finishTick, place: 1, holdEndTick: 0, slot: 0 }] };
}

test("start lights: five lamps come on one by one, then all go out and flash green", () => {
  const end = 1000;
  const r = race("lights", end);
  // 3 s of lights (180 ticks), a lamp every 30 ticks from tick 30.
  const lit = [0, 29, 30, 60, 90, 120, 150, 179].map((t) => startLights(r, end - 180 + t, end - 180, 0).lit);
  assert.deepEqual(lit, [0, 0, 1, 2, 3, 4, 5, 5]);
  assert.equal(startLights(r, end - 100, 0, 0).show, true, "the lights' start is estimated when not seen");
  const go = startLights(race("racing", end + 10800), end + 30, end - 180, end);
  assert.equal(go.go, true);
  assert.ok(Math.abs(go.sinceGo - 0.5) < 1e-9);
  assert.equal(startLights(race("racing", end + 10800), end + Math.ceil(GO_SECS * 60) + 1, end - 180, end).show, false);
  assert.equal(startLights(null, 0, 0, 0).show, false);
});

test("gates in the own lap order: next bright, passed in the own colour, the own gate last", () => {
  assert.equal(lapNumber(1, 0), 1);
  assert.equal(lapNumber(0, 0), 8, "the own gate closes the lap");
  assert.equal(lapNumber(5, 3), 2);
  const own: [number, number, number] = [1, 0.5, 0];
  const g = gateLooks({ slot: 2, gates: 3 }, own);
  assert.equal(g.length, 8);
  // Gates 3, 4, 5 passed; gate 6 next; 7, 0, 1, 2 later (gate 2 is number 8).
  assert.deepEqual(g.map((x) => x.state), [0, 0, 0, 2, 2, 2, 1, 0]);
  assert.deepEqual(g[3]?.color, own);
  assert.equal(g[6]?.pulse, 1);
  assert.equal(g[2]?.number, 8);
  assert.ok((g[6]?.intensity ?? 0) > (g[7]?.intensity ?? 0));
  // Without an own racer every gate looks alike, numbered 1-8.
  assert.deepEqual(gateLooks(null, own).map((x) => x.number), [1, 2, 3, 4, 5, 6, 7, 8]);
});

test("the tick clock runs smoothly between server ticks", () => {
  const c = new TickClock();
  assert.equal(c.update(100, 10), 100);
  assert.ok(Math.abs(c.update(100, 10 + 0.5 / 60) - 100.5) < 1e-9);
  assert.equal(c.update(100, 11), 101.5, "capped while no new tick comes");
  assert.equal(c.update(101, 11), 101);
});

test("pebbles: in the belt, near the camera, capped, stable per seed", () => {
  const f = new PebbleField(BELT, 7, 3000);
  const g = new PebbleField(BELT, 7, 3000);
  assert.deepEqual(f.data, g.data);
  const cam = ringPoint(BELT, 1.0, 120);
  const fwd = prograde(1.0);
  const out: Instance[] = [];
  f.instances(cam, fwd, 0, out, 400);
  assert.ok(out.length > 20 && out.length <= 400, String(out.length));
  for (const p of out) {
    const d = v3.length(v3.sub(p.position, cam));
    assert.ok(d <= PEBBLE_RANGE && d * d >= 12, `pebble at ${d}`);
    const r = Math.hypot(p.position.x - 160, p.position.z - 160);
    assert.ok(r > 85 && r < 155 && Math.abs(p.position.y - 48) < 25);
    // In front of the camera (or just behind it).
    assert.ok(v3.dot(v3.sub(p.position, cam), fwd) >= -2);
  }
  // Denser toward the inner edge.
  let inner = 0;
  let outer = 0;
  for (let i = 0; i < f.count; i++) {
    const rad = f.data[i * 10 + 1] ?? 0;
    if (rad < 112) inner++;
    else if (rad > 128) outer++;
  }
  assert.ok(inner > outer * 1.3, `${inner} vs ${outer}`);
});

test("the gantry hangs ahead of the slot, its lamps facing the racer", () => {
  const slot = BELT.slots[3]!.pos;
  const g = gantryPose(BELT, slot);
  const ahead = prograde(azimuth(BELT, slot));
  assert.ok(v3.dot(v3.sub(g.position, slot), ahead) > 5);
  assert.ok(g.position.y > slot.y + 1);
  // The mesh's lamps face -Z: toward the slot.
  const lampDir = q.rotate(g.rotation, { x: 0, y: 0, z: -1 });
  assert.ok(v3.dot(lampDir, ahead) < -0.99);
});

/** A sound engine fake that records plays. */
function fakeEngine(): { plays: string[]; loops: string[]; engine: NonNullable<ConstructorParameters<typeof RaceVisuals>[1]> } {
  const plays: string[] = [];
  const loops: string[] = [];
  return {
    plays,
    loops,
    engine: {
      play: (id) => {
        plays.push(id);
        return true;
      },
      loop: (id) => {
        loops.push(id);
        return { playing: true, setGain: () => {}, setPosition: () => {}, stop: () => {} };
      },
    },
  };
}

function input(over: Partial<RaceFxInput>): RaceFxInput {
  return {
    time: 0,
    dt: 1 / 60,
    nowTick: 0,
    camera: ringPoint(BELT, 0.3, 120),
    orientation: q.identity(),
    belt: BELT,
    race: null,
    own: 0,
    ownPose: null,
    ownColor: [1, 0.5, 0],
    ships: [],
    quiet: false,
    seed: 1,
    ...over,
  };
}

test("race sounds: a beep per lamp, the go tone and 'Belt up!' once, a chime per gate, the finish once", () => {
  const f = fakeEngine();
  const said: string[] = [];
  const fx = new RaceVisuals(null, f.engine, () => 0.5);
  const end = 600;
  // The whole start lights and a second of racing, a frame per tick.
  for (let t = end - 180; t <= end + 60; t++) {
    const r = t < end ? race("lights", end) : race("racing", end + 10800);
    fx.update(input({ nowTick: t, time: t / 60, race: r, say: (x) => said.push(x) }));
  }
  assert.equal(f.plays.filter((p) => p === "race.lightOn").length, 5);
  assert.equal(f.plays.filter((p) => p === "race.go").length, 1);
  assert.deepEqual(said, ["beltUp"]);
  assert.ok(f.loops.includes("race.portal"));
  // Gates: 0 -> 1 -> 2 chimes; 8 is the lap sting; the finish once.
  for (const [g, fin] of [[0, 0], [1, 0], [2, 0], [8, 0], [8, 900], [8, 900]] as const) {
    fx.update(input({ nowTick: 800, time: 20, race: race("racing", end + 10800, g, fin) }));
  }
  assert.equal(f.plays.filter((p) => p === "race.gate").length, 2);
  assert.equal(f.plays.filter((p) => p === "race.lapDone").length, 1);
  assert.equal(f.plays.filter((p) => p === "race.finish").length, 1);
  // Quiet (overlays): nothing new.
  const n = f.plays.length;
  fx.update(input({ nowTick: 801, time: 20.1, race: race("racing", end + 10800, 8, 900), quiet: true }));
  assert.equal(f.plays.length, n);
});

test("radiation: the dose drives the Geiger clicks, the screen edges and the hull sizzle", () => {
  const f = fakeEngine();
  const sparks: unknown[] = [];
  const glows: unknown[] = [];
  let seed = 7;
  const rnd = (): number => {
    seed = (seed * 16807) % 2147483647;
    return seed / 2147483647;
  };
  const fx = new RaceVisuals({ spawn: (p) => sparks.push(p), immediate: (p) => glows.push(p) }, f.engine, rnd);
  const hot = ringPoint(BELT, 0.5, 120, 40);
  let frame = fx.update(input({ race: race("racing", 20000), ownPose: { position: hot, velocity: { x: 0, y: 0, z: 0 } }, doseAt: () => 12, ships: [] }));
  for (let i = 0; i < 60; i++) {
    frame = fx.update(input({ time: i / 60, nowTick: i, race: race("racing", 20000), ownPose: { position: hot, velocity: { x: 0, y: 0, z: 0 } }, doseAt: () => 12, ships: [{ id: 3, team: 3, alive: true, boosting: false, position: hot, orientation: q.identity(), shipType: 1, radius: 0.5, gadgetActive: false }] }));
  }
  assert.equal(frame.dose, 12);
  assert.ok((frame.look?.radiation ?? 0) > 0.8, String(frame.look?.radiation));
  assert.ok(f.plays.filter((p) => p === "race.geiger").length > 20);
  assert.ok(sparks.length > 10 && glows.length > 0);
  // Inside the belt: no dose, the screen clears.
  for (let i = 0; i < 60; i++) frame = fx.update(input({ time: 1 + i / 60, race: race("racing", 20000), ownPose: { position: ringPoint(BELT, 0.5, 120), velocity: { x: 0, y: 0, z: 0 } }, doseAt: () => 0 }));
  assert.equal(frame.look?.radiation, 0);
});

test("the frame: boulders from the orbits, lights, the belt look; nothing on classic levels", () => {
  const fx = new RaceVisuals(null, null);
  const frame = fx.update(input({ race: race("racing", 20000, 8), nowTick: 300 }));
  assert.ok(frame.look);
  assert.equal(frame.look.gates.length, 8);
  assert.ok(frame.look.portal > 1, "the exit glows brighter once the own lap is done");
  assert.ok(frame.lights.length >= 2);
  const scene: SceneInstances = { opaque: {}, translucent: {} };
  fx.addToScene(scene);
  assert.equal(scene.opaque.boulder?.length, 1);
  assert.equal(scene.opaque.boulder?.[0]?.scale, 6, "1 u mesh scaled to the boulder's diameter");
  // From ls-wasm poses when given.
  const poses = new Float32Array([1, 2, 3, 0, 0, 0, 1, 2.5]);
  fx.update(input({ boulderPoses: () => poses }));
  const s2: SceneInstances = { opaque: {}, translucent: {} };
  fx.addToScene(s2);
  assert.deepEqual(s2.opaque.boulder?.[0]?.position, { x: 1, y: 2, z: 3 });
  // Start lights add a gantry and five lamps per racer.
  fx.update(input({ race: race("lights", 500), nowTick: 440 }));
  const counts = fx.counts;
  assert.equal(counts.gantries, 1);
  assert.equal(counts.lamps, 5);
  const none = fx.update(input({ belt: null }));
  assert.equal(none.look, null);
});

test("belt uniform packing and the sign's corners", () => {
  const out = new Float32Array(BELT_UNIFORM_FLOATS);
  const look = new RaceVisuals(null, null).update(input({ race: race("racing", 20000, 2) })).look!;
  packBelt(look, { position: { x: 1, y: 2, z: 3 }, orientation: q.identity(), fovY: 1, near: 0.05, far: 600 }, 16 / 9, 5, 0.003, { width: 1920, height: 1080 }, 1, { x: 0, y: 1, z: 0 }, [1, 1, 1], out);
  assert.deepEqual([...out.subarray(BELT_SLOT.center, BELT_SLOT.center + 4)], [160, 48, 160, 95]);
  assert.equal(out[BELT_SLOT.low], Math.ceil(1920 / BELT_VOLUME_DIVISOR));
  assert.equal(out[BELT_SLOT.low + 1], Math.ceil(1080 / BELT_VOLUME_DIVISOR));
  // Gate 3 (slot 0, 2 gates passed: lap numbers 1 and 2 behind) is next: state 1.
  assert.equal(out[BELT_SLOT.gateInfo + 4 * 3 + 3], 1);
  // The sign faces the camera, upright, SIGN_WORLD_WIDTH wide.
  const cam = ringPoint(BELT, 0.2, 120, 0);
  const [tl, tr, bl] = signCorners(look, cam);
  assert.ok(Math.abs(v3.length(v3.sub(tr!, tl!)) - SIGN_WORLD_WIDTH) < 1e-6);
  assert.ok(Math.abs(v3.length(v3.sub(tl!, bl!)) - SIGN_WORLD_HEIGHT) < 1e-6);
  const normal = v3.normalize(v3.cross(v3.sub(tr!, tl!), v3.sub(bl!, tl!)));
  const toCam = v3.normalize(v3.sub(cam, tl!));
  assert.ok(Math.abs(v3.dot(normal, toCam)) > 0.9);
  assert.ok(tl!.y > bl!.y);
});
