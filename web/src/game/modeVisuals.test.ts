import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import type { ParticleSpec } from "../renderer/particles.ts";
import { NEUTRAL_GRADE } from "../renderer/post.ts";
import { mulberry32 } from "../renderer/spriteAtlas.ts";
import { emptyModeState, OBJECTIVE, PLAYER_FLAG, VIEW_FLAG, type ModeState, type Objective } from "./modeState.ts";
import {
  addModeProps,
  closeStep,
  colourMeterStyle,
  CORE_STATE,
  easeMonochrome,
  fuseFraction,
  modeLights,
  ModeVfx,
  modeVolumes,
  monochromeGrade,
  REACTOR_STATE,
  SHELL_STEP,
  shipStyleFor,
  snap,
  squeezeView,
  ZONE_WALLS,
  type ModeSceneInput,
} from "./modeVisuals.ts";
import { emptyScene } from "../renderer/renderer.ts";
import type { ShipState } from "./views.ts";
import type { VfxSink } from "./vfxLibrary.ts";

function obj(id: number, kind: number, extra: Partial<Objective> = {}): Objective {
  return { id, kind, team: -1, holder: -1, state: 0, timerEnd: 0, radius: 0, pos: { x: 0, y: 0, z: 0 }, ...extra };
}

function mode(over: Partial<ModeState>): ModeState {
  return { ...emptyModeState(), ...over };
}

function input(m: ModeState, over: Partial<ModeSceneInput> = {}): ModeSceneInput {
  return { mode: m, nowTick: 1000, time: 1, own: 0, ownPos: { x: 0, y: 0, z: 0 }, ships: [], camera: { x: 0, y: 0, z: 0 }, ...over };
}

function ship(id: number, team: number, x: number): ShipState {
  return { id, team, alive: true, boosting: false, position: { x, y: 0, z: 0 }, orientation: q.identity(), shipType: 1, radius: 0.5, gadgetActive: false };
}

/** A sink that only counts spawns. */
function countingSink(): VfxSink & { spawned: ParticleSpec[]; immediates: number } {
  const s = {
    now: 0,
    density: 1,
    spawned: [] as ParticleSpec[],
    immediates: 0,
    spawn(p: ParticleSpec) {
      s.spawned.push(p);
    },
    immediate() {
      s.immediates++;
    },
  };
  return s;
}

const squeeze = (now: number, phase: "before" | "warn" | "close" | "after"): ModeState => {
  const safe = obj(0, OBJECTIVE.safeZone, { radius: 60, pos: { x: 10.2, y: 0, z: -3.6 }, timerEnd: 2000, state: phase === "before" ? 0 : ZONE_WALLS });
  const next = obj(1, OBJECTIVE.nextZone, { radius: 40, pos: { x: 20, y: 5, z: 0 }, timerEnd: 3800 });
  void now;
  return mode({ mode: "the_squeeze", teams: 0, stage: 2, objectives: phase === "before" ? [safe] : [safe, next] });
};

test("The Squeeze: the shell follows the sim's schedule in SHELL_STEP steps", () => {
  // closeStep: endpoints and quantization.
  const from = { center: { x: 0, y: 0, z: 0 }, radius: 60 };
  const to = { center: { x: 10, y: 0, z: 0 }, radius: 40 };
  assert.deepEqual(closeStep(from, to, 100, 200, 50), from);
  assert.deepEqual(closeStep(from, to, 100, 200, 200), to);
  // Travel 10 + 20 = 30 u, 60 steps over 100 ticks: every step moves the surface at most 0.5 u.
  let prev = closeStep(from, to, 100, 200, 100);
  for (let t = 101; t <= 200; t++) {
    const z = closeStep(from, to, 100, 200, t);
    const moved = Math.hypot(z.center.x - prev.center.x, z.center.y - prev.center.y, z.center.z - prev.center.z) + Math.abs(z.radius - prev.radius);
    assert.ok(moved <= 2 * SHELL_STEP + 1e-9, `tick ${t}: ${moved}`);
    prev = z;
  }
  assert.deepEqual(snap({ x: 1.5, y: -0.5, z: 2.49 }), { x: 2, y: 0, z: 2 });

  assert.equal(squeezeView(mode({ mode: "heist" }), 0), null, "other modes");
  const before = squeezeView(squeeze(1000, "before"), 1000)!;
  assert.equal(before.shell, null, "no walls before the first close");
  const warn = squeezeView(squeeze(1000, "warn"), 1000)!;
  assert.ok(warn.warning && !warn.closing);
  assert.deepEqual(warn.shell, { center: { x: 10, y: 0, z: -4 }, radius: 60 }, "walls at the snapped current zone");
  const mid = squeezeView(squeeze(2900, "close"), 2900)!;
  assert.ok(mid.closing && !mid.warning);
  assert.ok(mid.shell!.radius < 60 && mid.shell!.radius > 40);
  assert.ok(Math.abs(mid.progress - 0.5) < 1e-9);
  const after = squeezeView(squeeze(3800, "after"), 3800)!;
  assert.deepEqual(after.shell, { center: { x: 20, y: 5, z: 0 }, radius: 40 });
  assert.ok(!after.closing);
});

test("volumes: walls warn and grind, the next zone, Heist and Hot Reactor objectives", () => {
  let v = modeVolumes(input(squeeze(1000, "warn")));
  assert.deepEqual(v.map((x) => x.kind), ["wall", "zone"]);
  const wall = v[0]!;
  assert.ok(wall.kind === "wall" && wall.warn === 1 && wall.grind === 0);
  v = modeVolumes(input(squeeze(2900, "close"), { nowTick: 2900 }));
  assert.ok(v[0]!.kind === "wall" && v[0]!.grind === 1);
  assert.equal(modeVolumes(input(squeeze(1000, "warn"), { warmup: true })).length, 0, "nothing in the warm-up");

  const heist = (state: number, holder = -1): ModeState =>
    mode({
      mode: "heist",
      teams: 2,
      players: [{ id: 3, side: 1, score: 0, lives: -1, meter: 0, flags: PLAYER_FLAG.carrier }],
      objectives: [obj(0, OBJECTIVE.core, { state, holder, pos: { x: 5, y: 0, z: 0 } }), obj(1, OBJECTIVE.extraction, { team: 1, radius: 7, pos: { x: 30, y: 0, z: 0 } })],
    });
  assert.deepEqual(modeVolumes(input(heist(CORE_STATE.embedded))).map((x) => x.kind), ["zone", "beam", "zone", "beam"]);
  const carried = modeVolumes(input(heist(CORE_STATE.carried, 3), { ships: [ship(3, 1, 12)] }));
  assert.deepEqual(carried.map((x) => x.kind), ["halo", "beam", "zone", "beam"]);
  assert.deepEqual(carried[0]!.center, { x: 12, y: 0, z: 0 }, "the halo follows the rendered carrier, not the lagging marker");
  assert.equal(modeVolumes(input(heist(CORE_STATE.respawning))).length, 2, "a respawning core shows nothing");

  const reactor = mode({ mode: "hot_reactor", teams: 0, objectives: [obj(0, OBJECTIVE.reactor, { state: REACTOR_STATE.held, holder: 4, radius: 15, timerEnd: 1000 + 600 })] });
  const rv = modeVolumes(input(reactor, { ships: [ship(4, 4, 7)] }));
  assert.deepEqual(rv.map((x) => x.kind), ["halo", "zone"]);
  const fuse = rv[1]!;
  assert.ok(fuse.kind === "zone" && Math.abs((fuse.ring ?? 0) - 0.5) < 1e-9, "10 s of 20 left");
  assert.equal(fuseFraction(reactor.objectives[0]!, 1000 + 1200), 0);
  assert.equal(modeVolumes(input(mode({ ...reactor, objectives: [obj(0, OBJECTIVE.reactor, { state: REACTOR_STATE.waiting })] }))).length, 0);
});

test("props and lights: core and reactor meshes, wall lights near the camera", () => {
  const scene = emptyScene();
  const m = mode({
    mode: "hot_reactor",
    objectives: [obj(0, OBJECTIVE.reactor, { state: REACTOR_STATE.held, holder: 0, radius: 15, timerEnd: 1300 }), obj(1, OBJECTIVE.core, { state: CORE_STATE.free, pos: { x: 3, y: 0, z: 0 } })],
  });
  addModeProps(scene, input(m, { ownPos: { x: 1, y: 2, z: 3 } }));
  assert.equal(scene.opaque.reactor?.length, 1);
  assert.equal(scene.opaque.core?.length, 1);
  assert.ok(Math.abs(scene.opaque.reactor![0]!.position.y - 2) > 1, "the reactor floats over its holder");
  // Walls: a light at the wall nearest the camera while warning or closing.
  const lights = modeLights(input(squeeze(1000, "warn"), { camera: { x: 10, y: 0, z: 40 }, ownPos: null }));
  assert.equal(lights.length, 1);
  const d = Math.hypot(lights[0]!.position.x - 10, lights[0]!.position.y, lights[0]!.position.z + 4);
  assert.ok(Math.abs(d - 58.8) < 1e-6, `just inside the wall (${d})`);
  assert.equal(modeLights(input(squeeze(1000, "before"))).length, 0);
});

test("Last Color Standing: monochrome grade and colour-meter ship tint", () => {
  const full = colourMeterStyle([1, 0.5, 0], 100);
  assert.deepEqual(full.color, [1, 0.5, 0]);
  assert.equal(full.keepColor, 1);
  const half = colourMeterStyle([1, 0.5, 0], 50);
  assert.ok(half.color[0] < 1 && half.color[2] > 0, "drained toward grey");
  assert.equal(colourMeterStyle([1, 0.5, 0], 0).keepColor, 0);
  const m = mode({ mode: "last_color_standing", teams: 0, flags: VIEW_FLAG.monochrome | VIEW_FLAG.meter, players: [{ id: 7, side: 7, score: 0, lives: -1, meter: 34, flags: 0 }] });
  const style = shipStyleFor(m, 0)!;
  assert.equal(style(ship(7, 7, 0))?.keepColor, 1);
  assert.equal(style(ship(8, 8, 0)), undefined, "unknown ships keep their side colour");
  assert.equal(shipStyleFor(mode({ mode: "team_brawl" }), 2), null);
  const g = monochromeGrade(NEUTRAL_GRADE, 1);
  assert.equal(g.monochrome, 1);
  assert.equal(monochromeGrade(NEUTRAL_GRADE, 0), NEUTRAL_GRADE);
  assert.equal(easeMonochrome(0, 1, 0.6), 0.5);
  assert.equal(easeMonochrome(0.9, 1, 1), 1);
  assert.equal(easeMonochrome(0.5, 0, 0.6), 0);
});

test("ModeVfx: grinding walls, carrier motes, reactor pass and detonation, deliveries", () => {
  const sink = countingSink();
  const fx = new ModeVfx(sink, mulberry32(3));
  const pulses: string[] = [];
  fx.lightSink = { emit: (p) => void pulses.push(String(p)) };
  // Closing walls near the camera: sparks and dust.
  for (let i = 0; i < 30; i++) fx.update({ ...input(squeeze(2900, "close"), { nowTick: 2900, camera: { x: 10, y: 0, z: 40 } }), dt: 1 / 60 });
  assert.ok(fx.counts.grind > 10, `${fx.counts.grind} grind spawns`);
  assert.ok(sink.spawned.length > 10);

  // Hot Reactor: held by 1, passed to 2, then gone at the fuse's end.
  const held = (holder: number, timerEnd: number): ModeState =>
    mode({ mode: "hot_reactor", objectives: [obj(0, OBJECTIVE.reactor, { state: REACTOR_STATE.held, holder, radius: 15, timerEnd })] });
  const ships = [ship(1, 1, 0), ship(2, 2, 5)];
  fx.update({ ...input(held(1, 1300), { ships }), dt: 1 / 60 });
  fx.update({ ...input(held(2, 1300), { ships, nowTick: 1100 }), dt: 1 / 60 });
  assert.equal(fx.counts.passes, 1);
  fx.update({ ...input(mode({ mode: "hot_reactor", objectives: [obj(0, OBJECTIVE.reactor, { state: REACTOR_STATE.waiting })] }), { ships, nowTick: 1300 }), dt: 1 / 60 });
  assert.equal(fx.counts.detonations, 1);
  assert.ok(pulses.includes("bigExplosion"));

  // Heist: carried, then respawning = a delivery.
  const core = (state: number, holder = -1): ModeState => mode({ mode: "heist", teams: 2, objectives: [obj(0, OBJECTIVE.core, { state, holder })] });
  fx.update({ ...input(core(CORE_STATE.carried, 1), { ships }), dt: 1 / 60 });
  fx.update({ ...input(core(CORE_STATE.respawning), { ships }), dt: 1 / 60 });
  assert.equal(fx.counts.deliveries, 1);
  fx.reset();
  fx.update({ ...input(core(CORE_STATE.respawning), { ships }), dt: 1 / 60 });
  assert.equal(fx.counts.deliveries, 1, "no delivery without a carrier first");
});
