// M4 C3: the spectacle events' client timelines and state packing (Duck
// parade, Hat rain, Cat paw, Disco inferno, Tired announcer's kill-cams),
// their models and materials, and the `?event=` switch.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import * as q from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import { emptyMood, type MoodState } from "../modeState.ts";
import type { Cue } from "../mechanics.ts";
import type { EntityState, ShipState } from "../views.ts";
import type { ParticleSpec } from "../../renderer/particles.ts";
import { emptyScene } from "../../renderer/renderer.ts";
import { validateMesh } from "../../renderer/mesh.ts";
import { hatMeshKind, MESH_BUILDERS, SPECTACLE_MESH_KINDS } from "../../renderer/models/index.ts";
import { HAT_HEIGHTS, HAT_MODELS, HAT_STYLES, PAW_TOES } from "../../renderer/models/spectacleProps.ts";
import { MODEL_SHADING_WGSL } from "../../renderer/models/shading.ts";
import { SPECTACLE_PART_MAX, SPECTACLE_PART_MIN } from "../../renderer/models/spectacleShading.ts";
import { PART_GLOW, PART_METAL } from "../../renderer/models/kit.ts";
import { DUCK_RADIUS, DuckParadeFx, duckPose, MAX_STRETCH, MIN_SQUASH, springStep, squashTarget, stretchFor } from "./duckParade.ts";
import { growStack, HAT_SEAT, HatRainFx, looseHatPose, REVEAL_HATS, stackPoses, swayStep } from "./hatRain.ts";
import { axisDir, clawExtension, clawTips, CLAW_GAP, findWall, PAW_PHASE, pawFrame, CatPawFx } from "./catPaw.ts";
import { beamDirection, BEAM_RANGE, DiscoInfernoFx, SPECKS, speckDirections, TRAIL_SECS } from "./discoInferno.ts";
import { hash01, hatCounts, LoopVoice, slerp, type SpectacleInput, type SpectacleSoundEngine } from "./spectacleCommon.ts";
import { SPECTACLE_EVENTS, SPECTACLE_KINDS, SpectacleVisuals } from "./spectacles.ts";
import { forcedEvent } from "./forceEvent.ts";
import {
  AFTER_KILL,
  blastAt,
  clipClock,
  clipFrame,
  KillCamDirector,
  killCamPose,
  KillCamRecorder,
  KILLCAM_CUE,
  KILLCAM_SECS,
  NARRATED_ONLY,
  RECORD_SECS,
} from "../killcam.ts";

const UP: Vec3 = { x: 0, y: 1, z: 0 };
const near = (a: number, b: number, eps = 1e-6, msg = ""): void => assert.ok(Math.abs(a - b) <= eps, `${msg} ${a} vs ${b}`);
const nearV = (a: Vec3, b: Vec3, eps = 1e-5, msg = ""): void => assert.ok(v3.length(v3.sub(a, b)) <= eps, `${msg} ${JSON.stringify(a)} vs ${JSON.stringify(b)}`);

/** A particle sink that records what it is given. */
function recorder(): { now: number; density: number; spawned: ParticleSpec[]; imm: ParticleSpec[]; spawn(p: ParticleSpec): void; immediate(p: ParticleSpec): void; beginFrame(): void } {
  return {
    now: 0,
    density: 1,
    spawned: [],
    imm: [],
    spawn(p) {
      this.spawned.push(p);
    },
    immediate(p) {
      this.imm.push(p);
    },
    beginFrame() {
      this.imm = [];
    },
  };
}

/** A sound engine that records plays and loops. */
function sounds(): SpectacleSoundEngine & { played: string[]; loops: Map<string, { gain: number; stopped: boolean }> } {
  const loops = new Map<string, { gain: number; stopped: boolean }>();
  const played: string[] = [];
  return {
    played,
    loops,
    play(id) {
      played.push(id);
      return true;
    },
    loop(id) {
      const l = { gain: 0, stopped: false };
      loops.set(id, l);
      return {
        get playing() {
          return !l.stopped;
        },
        setGain(g: number) {
          l.gain = g;
        },
        setPosition() {},
        stop() {
          l.stopped = true;
        },
      };
    },
  };
}

function ent(id: number, kind: string, position: Vec3, state: [number, number, number, number], over: Partial<EntityState> = {}): EntityState {
  return { id, kind, visual: 0, owner: 255, team: 255, position, velocity: { x: 0, y: 0, z: 0 }, rotation: q.identity(), state, radius: 0, ...over };
}

function ship(id: number, position: Vec3, over: Partial<ShipState> = {}): ShipState {
  return { id, team: 0, alive: true, boosting: false, position, orientation: q.identity(), shipType: 1, radius: 0.5, gadgetActive: false, ...over };
}

function input(over: Partial<SpectacleInput> = {}): SpectacleInput {
  return {
    time: 0,
    dt: 1 / 60,
    nowTick: 0,
    camera: { x: 0, y: 0, z: 30 },
    orientation: q.identity(),
    mood: emptyMood(),
    cues: [],
    ships: [],
    own: null,
    entities: [],
    shots: [],
    shipExt: [],
    colorOf: () => [1, 0.5, 0.2],
    nameOf: (id) => `P${id}`,
    up: UP,
    quiet: false,
    ...over,
  };
}

const mood = (eventId: string, phase: string, phaseEndTick = 0): MoodState => ({ ...emptyMood(), eventId, phase, phaseEndTick });

// ----- models and materials ------------------------------------------------------------

test("spectacle props build as valid meshes within a triangle budget, in spectacle or kit materials", () => {
  for (const kind of SPECTACLE_MESH_KINDS) {
    const m = MESH_BUILDERS[kind]();
    validateMesh(m);
    const tris = m.indices.length / 3;
    assert.ok(tris >= 30 && tris <= 9000, `${kind}: ${tris} triangles`);
    for (const p of m.team) assert.ok((p >= SPECTACLE_PART_MIN && p <= SPECTACLE_PART_MAX) || p === PART_METAL || p === PART_GLOW, `${kind}: part ${p}`);
  }
  assert.equal(HAT_MODELS.length, 8);
  assert.equal(HAT_STYLES.length, 8);
  assert.equal(HAT_HEIGHTS.length, 8);
  assert.equal(hatMeshKind(3), "hat_3");
  assert.equal(hatMeshKind(11), "hat_3", "styles wrap");
  assert.equal(hatMeshKind(-1), "hat_7");
});

test("the model shader routes every spectacle part to shade_spectacle before the plain fallback", () => {
  const route = MODEL_SHADING_WGSL.indexOf("return shade_spectacle(");
  const plain = MODEL_SHADING_WGSL.indexOf("if (part > PART_LAMP)");
  assert.ok(route > 0 && plain > route, "the spectacle branch comes first");
  assert.match(MODEL_SHADING_WGSL, new RegExp(`const SPECTACLE_PART_MIN = ${SPECTACLE_PART_MIN}u;`));
  assert.match(MODEL_SHADING_WGSL, new RegExp(`const SPECTACLE_PART_MAX = ${SPECTACLE_PART_MAX}u;`));
  assert.ok(SPECTACLE_PART_MIN > 11 && SPECTACLE_PART_MAX < 255, "clear of the kit parts and PART_PLAIN");
});

// ----- Duck parade ------------------------------------------------------------------------

test("duck squash: 1 in the open, thinner (never below the minimum) where the tunnel is narrow; longer when thinner, capped", () => {
  assert.equal(squashTarget(25.5), 1);
  assert.equal(squashTarget(DUCK_RADIUS), 1);
  near(squashTarget(DUCK_RADIUS * 0.8), 0.8);
  assert.equal(squashTarget(0), MIN_SQUASH);
  assert.equal(stretchFor(1), 1);
  assert.ok(stretchFor(0.8) > 1 && stretchFor(0.8) < MAX_STRETCH);
  assert.equal(stretchFor(MIN_SQUASH), MAX_STRETCH);
});

test("duck squash spring: overshoots on the way back (the wobble) and settles on its target", () => {
  let [s, v] = [0.6, 0];
  let max = 0;
  for (let i = 0; i < 60; i++) {
    [s, v] = springStep(s, v, 1, 1 / 60);
    max = Math.max(max, s);
  }
  assert.ok(max > 1.02, `overshoot ${max}`);
  for (let i = 0; i < 240; i++) [s, v] = springStep(s, v, 1, 1 / 60);
  near(s, 1, 1e-3, "settled");
});

test("duck parade: ducks follow their entities (state = index, clearance in tenths), stay upright, squeak per swallowed shot and per shove", () => {
  const fx = new DuckParadeFx();
  const sink = recorder();
  const engine = sounds();
  fx.sink = sink;
  fx.engine = engine;
  const along = { x: 1, y: 0, z: 0 };
  const ducks = [0, 1].map((i) => ent(10 + i, "duck", { x: -9 * i, y: 0, z: 0 }, [i, 255, 0, 0], { velocity: v3.scale(along, 5), radius: DUCK_RADIUS }));
  for (let k = 0; k < 30; k++) fx.update(input({ time: k / 60, entities: ducks }));
  assert.ok(fx.active);
  assert.ok(engine.loops.get("duck.kazoo") && engine.loops.get("duck.kazoo")!.gain > 0, "the kazoo marches");
  const scene = emptyScene();
  fx.addToScene(scene, 0.5, UP);
  assert.equal(scene.opaque.rubber_duck?.length, 2);
  assert.equal(scene.opaque.hat_0?.length, 1, "the lead duck's top hat");
  for (const inst of scene.opaque.rubber_duck ?? []) {
    nearV(v3.normalize(q.forward(inst.rotation)), along, 0.2, "faces along its route");
    assert.ok(v3.dot(q.up(inst.rotation), UP) > 0.9, "upright (only a little waddle)");
  }
  // A shot plops into duck 1.
  fx.update(input({ time: 0.6, entities: ducks, cues: [{ kind: 20, source: 255, target: 255, position: { x: -9, y: 0, z: 2 }, value: 12, tick: 36 }] }));
  assert.ok(engine.played.includes("duck.squeak"));
  // A ship brushing the lead duck squeaks it too (once per pair and 0.8 s).
  const before = engine.played.length;
  const shoved = [ship(3, { x: 0, y: 0, z: DUCK_RADIUS + 0.6 })];
  fx.update(input({ time: 2, entities: ducks, ships: shoved }));
  fx.update(input({ time: 2.1, entities: ducks, ships: shoved }));
  assert.equal(engine.played.length, before + 1);
  assert.ok(sink.spawned.some((p) => p.sprite !== undefined), "bubbles and pops");
  // Narrow tunnel: the duck squashes over a few frames.
  const narrow = [ent(10, "duck", { x: 0, y: 0, z: 0 }, [0, 20, 0, 0], { velocity: v3.scale(along, 5), radius: DUCK_RADIUS })];
  for (let k = 0; k < 90; k++) fx.update(input({ time: 3 + k / 60, entities: narrow }));
  const sc = emptyScene();
  fx.addToScene(sc, 4.5, UP);
  const d = sc.opaque.rubber_duck?.[0];
  assert.ok(d && (d.stretch ?? 1) > 1.3, `squashed and stretched (${d?.stretch})`);
  // Gone: the kazoo stops.
  fx.update(input({ time: 5, entities: [] }));
  assert.ok(!fx.active);
  assert.ok(engine.loops.get("duck.kazoo")!.stopped);
});

test("duck pose: waddles about its route with a bob along up, and squishes for half a second after a hit", () => {
  const facing = q.lookRotation({ x: 0, y: 0, z: -1 }, UP);
  const a = duckPose({ pos: { x: 0, y: 0, z: 0 }, facing, s: 1, hitAt: -9, index: 0 }, 0.3, UP);
  assert.ok(a.position.y >= 0 && a.position.y <= 0.2);
  const hit = duckPose({ pos: { x: 0, y: 0, z: 0 }, facing, s: 1, hitAt: 1, index: 0 }, 1.01, UP);
  assert.ok(hit.scale < 1, "squished");
  const later = duckPose({ pos: { x: 0, y: 0, z: 0 }, facing, s: 1, hitAt: 1, index: 0 }, 1.7, UP);
  assert.equal(later.scale, 1, "back after the squish");
});

// ----- Hat rain ---------------------------------------------------------------------------

test("hat stacks: styles from the hats a ship touched, then seeded ones; shrink on death", () => {
  assert.deepEqual(growStack([], 2, [4, 6], 7), [4, 6]);
  const seeded = growStack([1], 3, [], 9);
  assert.equal(seeded[0], 1);
  assert.equal(seeded.length, 3);
  assert.deepEqual(growStack([1], 3, [], 9), seeded, "deterministic");
  assert.deepEqual(growStack([1, 2, 3], 1, [], 9), [1]);
  for (const s of growStack([], 20, [], 3)) assert.ok(s >= 0 && s < 8);
});

test("hat sway: lags the hull's acceleration (tips back when it speeds up), springs back, never past the limit", () => {
  let s = { x: 0, z: 0, vx: 0, vz: 0 };
  for (let i = 0; i < 120; i++) s = swayStep(s, 0, 30, 1 / 60);
  near(s.x, 0.54, 0.02, "forward 30 u/s^2 tips back");
  for (let i = 0; i < 120; i++) s = swayStep(s, 1000, 1000, 1 / 60);
  assert.ok(Math.abs(s.x) <= 0.75 && Math.abs(s.z) <= 0.75, "clamped");
  for (let i = 0; i < 600; i++) s = swayStep(s, 0, 0, 1 / 60);
  near(s.x, 0, 1e-3);
});

test("worn stack: the first hat sits on the hull's top, each next one higher along the hull's up", () => {
  const poses = stackPoses([0, 3, 4], { x: 0, y: 0, z: 0 }, q.identity(), 0.5, { x: 0, z: 0 }, 0);
  near(poses[0]!.position.y, HAT_SEAT * 0.5, 1e-9);
  assert.ok(poses[1]!.position.y > poses[0]!.position.y && poses[2]!.position.y > poses[1]!.position.y);
  // On a hull turned upside down the stack hangs below it.
  const flipped = stackPoses([0, 3], { x: 0, y: 0, z: 0 }, q.fromAxisAngle({ x: 0, y: 0, z: 1 }, Math.PI), 0.5, { x: 0, z: 0 }, 0);
  assert.ok(flipped[1]!.position.y < flipped[0]!.position.y);
});

test("hat rain: hats taken show on the hull with a pop, resting hats sit on the rock, the own count feeds the HUD chip", () => {
  const fx = new HatRainFx();
  const engine = sounds();
  const sink = recorder();
  fx.engine = engine;
  fx.sink = sink;
  const s = ship(5, { x: 0, y: 0, z: 0 });
  const hats = [ent(100, "hat", { x: 0.6, y: 0.2, z: 0 }, [6, 0, 0, 0]), ent(101, "hat", { x: 20, y: 1, z: 0 }, [2, 1, 0, 0])];
  const floor = (o: Vec3, d: Vec3, max: number): number => (d.y < -0.5 && o.y - 0.4 <= max ? o.y - 0.4 : -1);
  fx.update(input({ ships: [s], entities: hats, shipExt: [5, 0, 0], raycast: floor }));
  // Ship 5 touched hat 100: it vanishes and the count goes up.
  fx.update(input({ time: 0.1, ships: [s], entities: [hats[1]!], shipExt: [5, 0, 1], raycast: floor }));
  assert.deepEqual(fx.stackOf(5), [6], "wears the fez it touched");
  assert.ok(engine.played.includes("hat.pop"));
  assert.ok(sink.spawned.length > 0, "confetti");
  const scene = emptyScene();
  fx.addToScene(scene, [hats[1]!], [s], 0.1, UP);
  assert.equal(scene.opaque.hat_6?.length, 1, "the fez on the hull");
  const resting = scene.opaque.hat_2?.[0];
  assert.ok(resting, "the resting party cone");
  near(resting.position.y, 0.44, 1e-6, "seated just above the rock below it (at y 0.4)");
  // The own hats: the HUD chip, and REVEAL_HATS give the pilot away while the event runs.
  const own = { id: 0, team: 0, position: { x: 50, y: 0, z: 0 }, orientation: q.identity(), shipType: 1, radius: 0.5 };
  const vis = new SpectacleVisuals(null, null, true, null, () => {});
  const f = vis.update(input({ own, shipExt: [0, 0, REVEAL_HATS], mood: mood("hat_rain", "active") }));
  assert.equal(f.ownHats, REVEAL_HATS);
  assert.ok(f.revealed);
  assert.ok(!vis.update(input({ own, shipExt: [0, 0, REVEAL_HATS], mood: emptyMood() })).revealed, "not after the event");
  assert.deepEqual([...hatCounts([1, 0, 2, 7, 1, 0])], [[1, 2], [7, 0]]);
});

test("loose hats: flutter while falling (around their position), rest upright on the level's up", () => {
  const falling = looseHatPose({ id: 3, position: { x: 0, y: 10, z: 0 }, state: [1, 0, 0, 0] }, 1.2, UP);
  assert.ok(v3.length(v3.sub(falling.position, { x: 0, y: 10, z: 0 })) < 0.5);
  const sideways = { x: 1, y: 0, z: 0 };
  const resting = looseHatPose({ id: 3, position: { x: 0, y: 0, z: 0 }, state: [1, 1, 0, 0] }, 1.2, sideways);
  assert.ok(v3.dot(q.up(resting.rotation), sideways) > 0.95, "upright along a turned up (Bored level)");
});

// ----- Cat paw ----------------------------------------------------------------------------

test("cat paw frame: claws and claw row as ls-sim's Sweep::claws, the palm toward the sweep, the arm out of the wrist", () => {
  // ls-sim: rot columns (along, z x along, z) with z = -fingers; mode 0 claws toward b = t x n = -local Y.
  const n = { x: 1, y: 0, z: 0 };
  const t = { x: 0, y: 0, z: 1 };
  const rot = q.lookRotation(v3.scale(n, -1), v3.cross(n, t));
  const b = v3.cross(t, n);
  const m0 = pawFrame(rot, 0);
  nearV(m0.along, t, 1e-6, "sweep");
  nearV(m0.dir, b, 1e-6, "mode 0: claws toward t x n");
  nearV(m0.spread, n, 1e-6, "claw row toward the shoulder (outward)");
  const m1 = pawFrame(rot, 1);
  nearV(m1.dir, v3.scale(b, -1), 1e-6, "mode 1: the other side");
  assert.equal(m1.mesh, "cat_paw_up");
  const m2 = pawFrame(rot, 2);
  nearV(m2.dir, n, 1e-6, "mode 2: claws toward the wall");
  nearV(m2.spread, b, 1e-6, "mode 2: claw row along t x n");
  for (const f of [m0, m1, m2]) {
    // The drawn paw's toes point where the claws do.
    const toes = q.rotate(f.paw, { x: 0, y: f.mesh === "cat_paw_up" ? 1 : -1, z: 0 });
    nearV(toes, f.dir, 1e-6, "toes toward the claws");
    nearV(q.rotate(f.paw, { x: 1, y: 0, z: 0 }), f.along, 1e-6, "palm toward the sweep");
  }
  const tips = clawTips({ x: 0, y: 0, z: 0 }, m0.dir, m0.spread, 10);
  nearV(tips[1]!, v3.scale(b, 10), 1e-6);
  near(v3.length(v3.sub(tips[2]!, tips[1]!)), CLAW_GAP, 1e-6);
  assert.equal(PAW_TOES.length, 4);
});

test("cat paw state: face codes, claws out for the sweep and in for the withdraw", () => {
  assert.deepEqual(axisDir(0), { x: -1, y: 0, z: 0 });
  assert.deepEqual(axisDir(3), { x: 0, y: 1, z: 0 });
  assert.deepEqual(axisDir(5), { x: 0, y: 0, z: 1 });
  assert.equal(clawExtension(PAW_PHASE.tell, 1), 0);
  assert.equal(clawExtension(PAW_PHASE.reach, 1), 0);
  assert.equal(clawExtension(PAW_PHASE.sweep, 0), 0);
  assert.equal(clawExtension(PAW_PHASE.sweep, 0.5), 1);
  assert.equal(clawExtension(PAW_PHASE.withdraw, 0), 1);
  assert.equal(clawExtension(PAW_PHASE.withdraw, 0.6), 0);
});

test("cat paw tell: the wall is found from open space inside the face, not from inside the rock", () => {
  // A wall slab: rock for x in [80, 100); the paw waits outside at x = 114.
  const ray = (o: Vec3, d: Vec3, max: number): number => {
    if (o.x >= 80) return 0;
    if (d.x <= 0) return -1;
    const t = (80 - o.x) / d.x;
    return t <= max ? t : -1;
  };
  const w = findWall({ x: 114, y: 0, z: 0 }, { x: 1, y: 0, z: 0 }, 10, ray);
  assert.ok(w);
  near(w.x, 80, 1e-6);
  assert.equal(findWall({ x: 114, y: 0, z: 0 }, { x: 1, y: 0, z: 0 }, 10, undefined), null);
});

test("cat paw: the tell glows on the wall and rumbles; the sweep lights the paw, sprays rock where the claws rake and shakes the camera", () => {
  const fx = new CatPawFx();
  const sink = recorder();
  const engine = sounds();
  fx.sink = sink;
  fx.engine = engine;
  const n = { x: 1, y: 0, z: 0 };
  const t = { x: 0, y: 0, z: 1 };
  const rot = q.lookRotation(v3.scale(n, -1), v3.cross(n, t));
  const ray = (o: Vec3, d: Vec3, max: number): number => {
    if (o.x >= 80) return 0;
    if (d.x <= 0) return -1;
    const tt = (80 - o.x) / d.x;
    return tt <= max ? tt : -1;
  };
  const tell = ent(7, "catPaw", { x: 114, y: 0, z: 0 }, [0, 0, 1, 2], { rotation: rot, radius: 10 });
  fx.update(input({ entities: [tell], raycast: ray, mood: mood("cat_paw", "announce", 90), camera: { x: 50, y: 0, z: 0 } }));
  assert.ok(engine.played.includes("paw.mrrp") && engine.played.includes("paw.rumble"));
  assert.ok(sink.imm.some((p) => p.orient === "plane"), "the print on the wall");
  assert.ok(fx.lights.length > 0);
  // Sweeping 1 u off the wall with the claws raking it.
  const sweep = ent(7, "catPaw", { x: 69, y: 0, z: 0 }, [2, 128, 1, 2], { rotation: rot, radius: 10, velocity: v3.scale(t, 45) });
  sink.spawned = [];
  fx.update(input({ entities: [sweep], raycast: ray, mood: mood("cat_paw", "active"), camera: { x: 50, y: 0, z: 0 }, nowTick: 200 }));
  assert.ok(engine.played.includes("paw.whoosh"));
  assert.ok(sink.spawned.length > 3, "rock spray");
  assert.ok(fx.shake > 0);
  assert.ok(engine.loops.get("paw.scrape") && engine.loops.get("paw.scrape")!.gain > 0, "claws scraping");
  const scene = emptyScene();
  fx.addToScene(scene);
  assert.equal(scene.opaque.cat_paw?.length, 1);
  assert.equal(scene.opaque.paw_arm?.length, 1);
  assert.equal(scene.opaque.paw_claw?.length, 4, "claws out");
});

// ----- Disco inferno ----------------------------------------------------------------------

test("disco beams point below the ball's horizon, unit length; reflection specks spread over the lower sphere", () => {
  for (let k = 0; k < 8; k++) {
    for (const t of [0, 1.3, 7.9]) {
      const d = beamDirection(k, t);
      near(v3.length(d), 1, 1e-9);
      assert.ok(d.y < -0.3, "down toward the floor and walls");
    }
  }
  const s = speckDirections();
  assert.equal(s.length, SPECKS);
  for (const d of s) near(v3.length(d), 1, 1e-9);
});

test("disco inferno: the light eases down to the ball's percent and back; beams hit rock; trails follow ships and fade after", () => {
  const fx = new DiscoInfernoFx();
  const engine = sounds();
  fx.engine = engine;
  const ball = ent(9, "discoBall", { x: 0, y: 10, z: 0 }, [8, 255, 0, 0], { radius: 1.5 });
  const ray = (o: Vec3, d: Vec3): number => (d.y < 0 ? o.y / -d.y : 40);
  const s = ship(2, { x: 0, y: 0, z: 0 });
  let t = 0;
  for (let k = 0; k < 180; k++) {
    s.position = { x: Math.cos(t) * 5, y: 0, z: Math.sin(t) * 5 };
    fx.update(input({ time: t, entities: [ball], ships: [s], raycast: ray, mood: mood("disco_inferno", "active"), nowTick: k }));
    t += 1 / 60;
  }
  near(fx.lightLevel, 0.08, 0.02, "lights down");
  assert.ok(engine.played.includes("disco.drop"));
  assert.ok(engine.loops.get("disco.loop")!.gain > 0);
  assert.equal(fx.lastBeams.length, 8);
  for (const b of fx.lastBeams) {
    assert.ok(b.hit && b.len < BEAM_RANGE);
    assert.ok(Math.abs(b.from.y + b.dir.y * b.len) < 1e-6, "the beam ends on the floor");
  }
  assert.ok(fx.lights.length >= 9, "eight spot pools, the ball's pin spot and the trail light");
  const scene = emptyScene();
  fx.addToScene(scene, t, UP);
  assert.equal(scene.opaque.disco_ball?.length, 1);
  const segs = scene.opaque.light_trail ?? [];
  assert.ok(segs.length > 20, `trail segments ${segs.length}`);
  assert.ok(segs.every((x) => x.keepColor === 1), "kept in colour through the ink");
  // The event ends: the light comes back, the trail runs out after TRAIL_SECS.
  for (let k = 0; k < Math.ceil((TRAIL_SECS + 0.5) * 60); k++) {
    fx.update(input({ time: t, entities: [], ships: [s], raycast: ray, mood: emptyMood(), nowTick: 200 + k }));
    t += 1 / 60;
  }
  assert.ok(fx.lightLevel > 0.9);
  const after = emptyScene();
  fx.addToScene(after, t, UP);
  assert.equal(after.opaque.light_trail?.length ?? 0, 0);
  assert.ok(engine.loops.get("disco.loop")!.stopped);
});

// ----- kill-cam ---------------------------------------------------------------------------

test("kill-cam recorder keeps the last seconds only", () => {
  const r = new KillCamRecorder();
  for (let k = 0; k <= 600; k++) r.push({ time: k / 60, ships: [], shots: [], entities: [] });
  assert.ok(r.size <= RECORD_SECS * 60 + 2);
  const w = r.window(9, 10);
  assert.equal(w.length, 61);
  assert.ok(w[0]!.time >= 9);
});

test("kill-cams play one after another at the sim's delays (plus the blast), 3 s each; queue overflow is narrated only", () => {
  const d = new KillCamDirector();
  let t = 0;
  const step = (cues: Cue[] = []): ReturnType<KillCamDirector["update"]> => {
    d.recorder.push({ time: t, ships: [ship(1, { x: 0, y: 0, z: 0 }), ship(2, { x: 10, y: 0, z: 0 })], shots: [], entities: [] });
    const say = d.onCues(cues, t);
    if (cues.some((c) => c.value === NARRATED_ONLY)) assert.equal(say.length, 1);
    const out = d.update(t);
    t += 1 / 60;
    return out;
  };
  for (let k = 0; k < 240; k++) step();
  const kill = (value: number): Cue => ({ kind: KILLCAM_CUE, source: 1, target: 2, position: { x: 10, y: 0, z: 0 }, value, tick: 0 });
  const t0 = t;
  step([kill(0), kill(KILLCAM_SECS * 60), kill(NARRATED_ONLY)]);
  let firstStart = -1;
  let secondStart = -1;
  for (let k = 0; k < 60 * 8; k++) {
    const p = step();
    if (p?.started && firstStart < 0) firstStart = t - 1 / 60;
    else if (p?.started && secondStart < 0) secondStart = t - 1 / 60;
  }
  near(firstStart - t0, AFTER_KILL, 1 / 30, "the first starts after the blast is on tape");
  near(secondStart - firstStart, KILLCAM_SECS, 1 / 30, "the second right after the first");
});

test("kill-cam clip: frames and clock from 2.6 s before the kill to the blast after it; the camera rides behind the killer, then circles the victim of the level", () => {
  const killer = ship(1, { x: 0, y: 0, z: 0 }, { orientation: q.lookRotation({ x: 1, y: 0, z: 0 }, UP) });
  const frames = Array.from({ length: 200 }, (_, k) => ({ time: 7 + k / 60, ships: [killer, ship(2, { x: 10, y: 0, z: 0 })], shots: [], entities: [] }));
  const clip = { killer: 1, victim: 2, at: { x: 10, y: 0, z: 0 }, killTime: 9.6, start: 10, frames };
  near(clipFrame(clip, 0)!.time, 9.6 + AFTER_KILL - KILLCAM_SECS, 1 / 60);
  near(clipFrame(clip, 1)!.time, 9.6 + AFTER_KILL, 1 / 60);
  near(clipClock(1), AFTER_KILL);
  near(clipClock(0), AFTER_KILL - KILLCAM_SECS);
  const pose = killCamPose(frames[0]!, 1, 2, clip.at, 0);
  assert.ok(pose.position.x < 0, "behind the killer");
  assert.ok(v3.dot(q.forward(pose.orientation), { x: 1, y: 0, z: 0 }) > 0.9, "looking where it looks");
  const level = killCamPose(frames[0]!, 255, 2, clip.at, 0.5);
  near(v3.length(v3.sub(level.position, { x: 10, y: 3.5, z: 0 })), 9, 1e-6, "circling the victim");
  assert.equal(blastAt(-0.1), null);
  assert.ok(blastAt(0.1)!.fire > blastAt(0)!.fire, "the blast grows");
  assert.equal(blastAt(AFTER_KILL + 0.3), null);
});

// ----- the bundle -------------------------------------------------------------------------

test("spectacles: announce lines for their own five events only, props kept from the scene builder, nothing when switched off", () => {
  const said: string[] = [];
  const vis = new SpectacleVisuals(null, null, true, null, (t) => said.push(t));
  const ids = readFileSync(new URL("../../audio/announcer/lines.ts", import.meta.url), "utf8");
  for (const id of SPECTACLE_EVENTS) assert.ok(ids.includes(`"${id}"`));
  const announce = (index: number): Cue => ({ kind: 16, source: 255, target: 255, position: { x: 0, y: 0, z: 0 }, value: index, tick: 0 });
  vis.update(input({ cues: [announce(10), announce(0), announce(13)] }));
  assert.deepEqual(said, ["event.duck_parade", "event.disco_inferno"]);
  assert.deepEqual([...vis.skipKinds].sort(), [...SPECTACLE_KINDS].sort());
  const off = new SpectacleVisuals(null, null, false, null, () => {});
  assert.equal(off.skipKinds.size, 0, "off: the props draw as before");
  const f = off.update(input({ entities: [ent(1, "discoBall", { x: 0, y: 0, z: 0 }, [8, 255, 0, 0])] }));
  assert.equal(f.lightLevel, 1);
});

test("?event=<id> forces that event's EventKind index; unknown ids and none give null", () => {
  assert.equal(forcedEvent({ search: "?event=duck_parade" }), 10);
  assert.equal(forcedEvent({ search: "?event=bored_level" }), 0);
  assert.equal(forcedEvent({ search: "?event=nope" }), null);
  assert.equal(forcedEvent({ search: "" }), null);
  assert.equal(forcedEvent(undefined), null);
});

test("small helpers: slerp ends and stays unit, loop voices stop when silent, hashes are in [0, 1)", () => {
  const a = q.identity();
  const b = q.fromAxisAngle(UP, 1);
  nearV(q.forward(slerp(a, b, 1)), q.forward(b), 1e-9);
  const h = slerp(a, b, 0.5);
  near(Math.hypot(h.x, h.y, h.z, h.w), 1, 1e-9);
  const engine = sounds();
  const v = new LoopVoice("x.loop");
  v.update(engine, 1, null);
  assert.equal(engine.loops.get("x.loop")!.gain, 1);
  v.update(engine, 0, null);
  assert.ok(engine.loops.get("x.loop")!.stopped);
  for (let i = 0; i < 50; i++) {
    const x = hash01(i, i * 3);
    assert.ok(x >= 0 && x < 1);
  }
});

test("the level's light (Disco inferno) scales sun, ambient, haze, sky and shafts; 1 leaves the frame unchanged", async () => {
  const { LIGHTING_WGSL } = await import("../../renderer/shaders/lighting.ts");
  const { FRAME_WGSL } = await import("../../renderer/shaders/frame.ts");
  const { packSkyUniforms } = await import("../../renderer/sky.ts");
  const { atmosphereFor } = await import("../../renderer/atmosphere.ts");
  assert.match(LIGHTING_WGSL, /sun_vis_last = sun_visibility\(world_pos, n\) \* frame\.rock_color\.w;/);
  assert.match(LIGHTING_WGSL, /n\.y \* 0\.5 \+ 0\.5\) \* frame\.rock_color\.w;/);
  assert.match(FRAME_WGSL, /\* frame\.rock_color\.w;/);
  const a = atmosphereFor("LS0G-0000-0000-0000-1001-6RFX-250V", [0, 0, 0, 256, 128, 256]);
  const cam = { position: { x: 0, y: 0, z: 0 }, orientation: q.identity(), fovY: 1 };
  assert.equal(packSkyUniforms(a, cam, 100, 50, 0)[58], 1);
  near(packSkyUniforms(a, cam, 100, 50, 0, undefined, q.identity(), 0.08)[58] ?? 0, 0.08, 1e-7);
});
