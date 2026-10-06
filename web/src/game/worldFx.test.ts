import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import * as q from "../math/quat.ts";
import { MOOD_IDS, emptyMood, parseModeState, parseMood } from "./modeState.ts";
import { easeMood, MOOD_LOOKS, moodGrade, moodMotes, moodWeights, MoodFx } from "./moodFx.ts";
import {
  caveInShake,
  coffeeDim,
  drawnByWorldFx,
  INK_GREY,
  parseWells,
  pigmentedIds,
  pigmentStyle,
  pullModel,
  rubbleFresh,
  SHIP_PULL_CLAMP,
  SHIP_THRUST_ACCEL,
  shipPull,
  TEAMMATE_KEEP,
  WELL_PASS_MAX,
  WELL_SOFTENING,
  wellActive,
  wellLook,
  wellPull,
  WorldFx,
  WorldVisuals,
  WORLD_CUE,
  type WellInfo,
} from "./worldFx.ts";
import { NEUTRAL_GRADE } from "../renderer/post.ts";
import type { ParticleSpec } from "../renderer/particles.ts";
import type { Cue } from "./mechanics.ts";

const well = (over: Partial<WellInfo> = {}): WellInfo => ({ position: { x: 0, y: 0, z: 0 }, range: 30, peak: 12, core: 2, startTick: 0, endTick: -1, ...over });
const len = (v: { x: number; y: number; z: number }): number => Math.hypot(v.x, v.y, v.z);

test("the well force law mirrors ls_sim::gravity: peak at s / sqrt 2, zero at the range, toward the centre", () => {
  const w = well();
  const at = (r: number) => wellPull(w, { x: r, y: 0, z: 0 });
  // Untapered peak is `peak` at r = s / sqrt 2; the taper takes a little off.
  const rPeak = WELL_SOFTENING / Math.SQRT2;
  const taper = (1 - (rPeak * rPeak) / (w.range * w.range)) ** 2;
  assert.ok(Math.abs(len(at(rPeak)) - w.peak * taper) < 1e-3);
  assert.ok(len(at(rPeak)) > len(at(rPeak * 0.5)) && len(at(rPeak)) > len(at(rPeak * 2)));
  assert.equal(len(at(30)), 0);
  assert.equal(len(at(45)), 0);
  assert.ok(at(5).x < 0, "pulls toward the centre");
  assert.equal(len(at(0)), 0, "no pull at the centre");
});

test("ship pull: dynamic wells only between their ticks, the total clamped to 40 % of the ship's thrust", () => {
  const dyn = well({ startTick: 100, endTick: 200, core: 0 });
  assert.ok(!wellActive(dyn, 99) && wellActive(dyn, 100) && !wellActive(dyn, 200));
  assert.ok(wellActive(well(), 1e9));
  const p = { x: 2, y: 0, z: 0 };
  assert.equal(len(shipPull([dyn], p, 50)), 0);
  assert.ok(len(shipPull([dyn], p, 150)) > 0);
  // Three stacked wells never pull past the clamp.
  const three = [well(), well({ position: { x: 4, y: 0, z: 0 } }), well({ position: { x: 2, y: 3, z: 0 } })];
  for (const t of [0, 1, 2, 3]) {
    const a = shipPull(three, { x: 2 + t * 0.3, y: 1, z: 0 }, 0, t);
    assert.ok(len(a) <= SHIP_PULL_CLAMP * SHIP_THRUST_ACCEL[t]! + 1e-9);
  }
});

test("the thrust table matches data/ships", () => {
  const order = ["skitter", "tinker", "hauler", "tug"];
  order.forEach((name, i) => {
    const ron = readFileSync(new URL(`../../../data/ships/${name}.ron`, import.meta.url), "utf8");
    const m = /thrust_accel:\s*([0-9.]+)/.exec(ron);
    assert.ok(m, name);
    assert.equal(Number(m[1]), SHIP_THRUST_ACCEL[i], name);
  });
});

test("the gravity numbers match data/world/gravity.ron", () => {
  const ron = readFileSync(new URL("../../../data/world/gravity.ron", import.meta.url), "utf8");
  assert.equal(Number(/softening:\s*([0-9.]+)/.exec(ron)?.[1]), WELL_SOFTENING);
  assert.equal(Number(/ship_clamp:\s*([0-9.]+)/.exec(ron)?.[1]), SHIP_PULL_CLAMP);
});

test("pull indicator: hidden when weak, sideways pulls give a screen direction, pulls along the view a dot", () => {
  const id = q.identity();
  assert.equal(pullModel({ x: 0.1, y: 0, z: 0 }, id), null);
  const right = q.right(id);
  const side = pullModel({ x: right.x * 6, y: right.y * 6, z: right.z * 6 }, id, 1);
  assert.ok(side && side.screen && Math.abs(side.screen.x - 1) < 1e-9);
  assert.ok(side && Math.abs(side.strength - 6 / (0.4 * 30)) < 1e-9 && Math.abs(side.ofThrust - 0.2) < 1e-9);
  const f = q.forward(id);
  const ahead = pullModel({ x: f.x * 5, y: f.y * 5, z: f.z * 5 }, id);
  assert.ok(ahead && ahead.screen === null && ahead.ahead);
  const behind = pullModel({ x: -f.x * 5, y: -f.y * 5, z: -f.z * 5 }, id);
  assert.ok(behind && !behind.ahead);
});

test("wells parse from ls-wasm floats and dim through the Gravity coffee break", () => {
  const ws = parseWells([1, 2, 3, 25, 9, 1.8, 0, -1, 4, 5, 6, 16, 3.6, 0, 60, 420]);
  assert.equal(ws.length, 2);
  assert.deepEqual(ws[1], { position: { x: 4, y: 5, z: 6 }, range: 16, peak: 3.6, core: 0, startTick: 60, endTick: 420 });
  assert.deepEqual(parseWells(undefined), []);
  const m = { ...emptyMood(), eventId: "gravity_coffee_break", phase: "active", activeTick: 1000, phaseEndTick: 2200 };
  assert.equal(coffeeDim(null, 0), 1);
  assert.equal(coffeeDim({ ...m, phase: "announce" }, 900), 1);
  assert.ok(Math.abs(coffeeDim(m, 1030) - 0.54) < 1e-9, "half way through the 1 s fade");
  assert.ok(Math.abs(coffeeDim(m, 1200) - 0.08) < 1e-9);
  assert.ok(Math.abs(coffeeDim({ ...m, phase: "ending" }, 2200) - 1) < 1e-9);
  const lit = wellLook(well(), 0, 1);
  const dim = wellLook(well(), 0, 0.08);
  assert.ok(lit.kind === "well" && dim.kind === "well" && dim.intensity < lit.intensity * 0.1 && dim.lens < lit.lens);
  assert.ok(lit.kind === "well" && lit.lens > lit.core, "the photon ring sits outside the stone");
});

test("cave-ins: a warning cue makes cracks, the collapse leaves rubble that keeps a coat", () => {
  const fx = new WorldFx();
  const spawned: ParticleSpec[] = [];
  fx.sink = { now: 0, density: 1, spawn: (p) => spawned.push(p), immediate: () => {} };
  const cue: Cue = { kind: WORLD_CUE.caveInWarning, source: 255, target: 255, position: { x: 10, y: 5, z: 0 }, value: 90, tick: 0 };
  const base = { dt: 1 / 60, nowTick: 0, camera: { x: 0, y: 5, z: 0 }, orientation: q.identity(), wells: [], mood: null, up: { x: 0, y: 1, z: 0 }, own: null, ships: [], entities: [], monochrome: false, pigmented: new Set<number>(), teams: 2, raycast: () => 4 };
  const f0 = fx.update({ ...base, time: 10, cues: [cue] });
  assert.equal(f0.warnings.length, 1);
  assert.deepEqual(f0.warnings[0]!.ceiling, { x: 10, y: 9, z: 0 }, "the cracks open on the ceiling above");
  assert.ok(f0.volumes.some((v) => v.kind === "crack"));
  const f1 = fx.update({ ...base, time: 11, cues: [] });
  const crack = f1.volumes.find((v) => v.kind === "crack");
  assert.ok(crack && crack.kind === "crack" && Math.abs(crack.progress - 2 / 3) < 1e-6);
  assert.ok(f1.shake > 0);
  const f2 = fx.update({ ...base, time: 11.6, cues: [] });
  assert.equal(f2.collapses.length, 1);
  assert.equal(fx.rubbleSites.length, 1);
  assert.ok(f2.volumes.some((v) => v.kind === "rubble") && !f2.volumes.some((v) => v.kind === "crack"));
  assert.ok(spawned.length > 40, "dust, pebbles and the collapse burst");
  assert.ok(rubbleFresh(0) === 1 && rubbleFresh(60) < 0.1);
  const c = f2.collapses[0]!;
  assert.ok(caveInShake(c, c.end + 0.05, 5) > caveInShake(c, c.start, 5), "the collapse shakes harder than the warning");
  assert.equal(caveInShake(c, c.end, 100), 0);
  // The same spot warned twice is one cave-in.
  const g = new WorldFx();
  g.update({ ...base, time: 0, cues: [cue, cue] });
  assert.equal(g.activeCaveIns.length, 1);
});

test("wells: the pass gets the nearest few, swirl motes go out as immediates, lights per well", () => {
  const fx = new WorldFx();
  let immediates = 0;
  fx.sink = { now: 0, density: 1, spawn: () => {}, immediate: () => immediates++ };
  const wells = [0, 1, 2, 3, 4].map((i) => well({ position: { x: i * 12, y: 0, z: 0 } }));
  const f = fx.update({ time: 5, dt: 0, nowTick: 300, camera: { x: 0, y: 3, z: 0 }, orientation: q.identity(), wells, cues: [], mood: null, up: { x: 0, y: 1, z: 0 }, own: { position: { x: 0, y: 3, z: 0 }, shipType: 1 }, ships: [], entities: [], monochrome: false, pigmented: new Set(), teams: 2 });
  assert.equal(f.volumes.filter((v) => v.kind === "well").length, WELL_PASS_MAX);
  assert.equal(f.lights.length, 5);
  assert.ok(immediates > 300);
  assert.ok(f.pull && f.pull.strength > 0);
  // Off: nothing.
  const off = new WorldFx();
  off.enabled = false;
  const g = off.update({ time: 5, dt: 0, nowTick: 300, camera: { x: 0, y: 3, z: 0 }, orientation: q.identity(), wells, cues: [], mood: null, up: { x: 0, y: 1, z: 0 }, own: null, ships: [], entities: [], monochrome: false, pigmented: new Set(), teams: 2 });
  assert.equal(g.volumes.length + g.lights.length, 0);
});

test("pigment: side colour kept through the ink when pigmented, a tint for grey teammates, ink grey for the rest", () => {
  assert.deepEqual([...pigmentedIds([3, 1, 0, 4, 0, 2, 7, 1, 1])], [3, 7]);
  assert.equal(pigmentStyle(false, new Set([1]), 0, 2), null, "only monochrome levels");
  const style = pigmentStyle(true, new Set([1]), 0, 2)!;
  assert.equal(style({ id: 1, team: 1 }).keepColor, 1);
  assert.equal(style({ id: 2, team: 0 }).keepColor, TEAMMATE_KEEP);
  assert.deepEqual(style({ id: 3, team: 1 }), { color: INK_GREY, keepColor: 0 });
  assert.ok(drawnByWorldFx({ kind: "pigmentPickup" }) && !drawnByWorldFx({ kind: "weaponPickup" }));
  // Pickups and splashes become kept-colour drops; pigmented ships drip.
  const fx = new WorldFx();
  const scene = { opaque: {}, translucent: {} } as Parameters<WorldFx["addToScene"]>[0];
  const ent = (id: number, splash: boolean) => ({ id, kind: "pigmentPickup", visual: 0, owner: 255, team: 255, position: { x: 0, y: 0, z: 0 }, velocity: { x: 0, y: 0, z: 0 }, rotation: q.identity(), state: [splash ? 1 : 0, 1, 0, 0] as [number, number, number, number], radius: 0 });
  const ship = { id: 1, team: 1, alive: true, boosting: false, position: { x: 0, y: 0, z: 0 }, orientation: q.identity(), shipType: 1, radius: 0.5, gadgetActive: false };
  for (let i = 0; i <= 30; i++) fx.update({ time: i / 60, dt: 1 / 60, nowTick: i, camera: { x: 0, y: 0, z: -5 }, orientation: q.identity(), wells: [], cues: [], mood: null, up: { x: 0, y: 1, z: 0 }, own: null, ships: [ship], entities: [], monochrome: true, pigmented: new Set([1]), teams: 2 });
  const lights: unknown[] = [];
  fx.addToScene(scene, [ent(1, false), ent(2, true)], 0.5, 2, lights as never);
  const drops = scene.opaque.pigment ?? [];
  assert.ok(drops.length >= 3 + 3 + 3, "three primaries, a splash and its two droplets, drips");
  assert.ok(drops.every((d) => d.keepColor === 1));
  assert.equal(lights.length, 2);
});

test("moods: parsed from the mode state, eased slowly, tint the grade and the motes", () => {
  const ms = parseModeState(JSON.stringify({ kind: 0, mood: { mood: 3, moodId: "giddy", eventId: "bored_level", phaseId: "active", target: -1, activeTick: 5, phaseEndTick: 9, params: [1, 2], up: [1, 0, 0] } }));
  assert.equal(ms.mood.mood, "giddy");
  assert.equal(ms.mood.eventId, "bored_level");
  assert.deepEqual(ms.mood.up, { x: 1, y: 0, z: 0 });
  assert.equal(parseMood(undefined).mood, "calm");
  assert.deepEqual(parseMood({ up: [0, 0, 0] }).up, { x: 0, y: 1, z: 0 });
  let w = moodWeights("calm");
  w = easeMood(w, "grumpy", 1);
  assert.ok(w.grumpy > 0.3 && w.grumpy < 0.5, "a mood change takes seconds");
  for (let i = 0; i < 20; i++) w = easeMood(w, "grumpy", 1);
  assert.ok(w.grumpy > 0.99);
  assert.ok(Math.abs(MOOD_IDS.reduce((s, id) => s + w[id], 0) - 1) < 1e-9);
  assert.deepEqual(moodGrade(NEUTRAL_GRADE, moodWeights("grumpy"), 0), NEUTRAL_GRADE);
  const bored = moodGrade(NEUTRAL_GRADE, moodWeights("bored"));
  assert.ok(bored.saturation < 0.8);
  assert.ok(moodGrade(NEUTRAL_GRADE, moodWeights("giddy")).saturation > 1.1);
  assert.equal(moodMotes(moodWeights("giddy")).rainbow, MOOD_LOOKS.giddy.rainbow);
  const fx = new MoodFx();
  fx.update("bored", 0, 0);
  assert.equal(fx.icon(1)?.announce, "The level is bored");
  assert.equal(fx.icon(10)?.announce, null);
  // The drones crossfade through the engine.
  const gains = new Map<string, number>();
  fx.engine = { loop: (id) => ({ playing: true, setGain: (g) => gains.set(id, g), stop: () => gains.delete(id) }) };
  fx.update("giddy", 10, 3);
  assert.ok((gains.get("mood.giddy") ?? 0) > 0 && (gains.get("mood.bored") ?? 0) > 0);
  fx.update("giddy", 40, 30);
  assert.equal(gains.has("mood.bored"), false);
});

test("the world visuals facade: grade untouched when off, pigment styles only without a mode's own colours", () => {
  const off = new WorldVisuals(null, false);
  assert.deepEqual(off.grade(NEUTRAL_GRADE), NEUTRAL_GRADE);
  assert.equal(off.skipKinds, undefined);
  const on = new WorldVisuals(null, true);
  assert.ok(on.skipKinds?.has("pigmentPickup"));
  const input = { time: 1, dt: 0.016, nowTick: 60, camera: { x: 0, y: 0, z: 0 }, orientation: q.identity(), wells: undefined, up: undefined, shipExt: [5, 1, 0], monochrome: true, modeStyles: false, cues: [], mood: emptyMood(), own: { id: 5, team: 0, position: { x: 0, y: 0, z: 0 }, shipType: 1 }, ownTeam: 0, ships: [], entities: [], teams: 2, quiet: false };
  const f = on.update(input, () => {});
  assert.equal(f.ownStyle?.keepColor, 1, "the own pigmented frame keeps its colour");
  assert.ok(f.shipStyle);
  assert.equal(on.update({ ...input, modeStyles: true }, () => {}).shipStyle, null, "Last Color Standing keeps its meters");
});
