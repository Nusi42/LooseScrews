import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  averageSpeed,
  courseDone,
  courseLeft,
  courseLength,
  gapSecs,
  gateAzimuth,
  nextGate,
  nextTarget,
  parseBelt,
  raceProgress,
  RaceClock,
  ringPoint,
  WEDGE,
  WrongWay,
  type Belt,
} from "./course.ts";

/** The fixture's belt (crates/ls-sim/tests/race_progress.rs). */
const BELT: Belt = {
  center: { x: 160, y: 48, z: 160 },
  innerRadius: 95,
  outerRadius: 145,
  halfThickness: 12,
  cocoonRadius: 30,
  exitRadius: 6,
  gateInnerRadius: 87,
  gateOuterRadius: 153,
  slots: [],
  pickupPoints: [],
  boulders: [],
};

test("raceProgress matches the simulation's progress function (Rust fixture)", () => {
  const text = readFileSync(new URL("../../../../crates/ls-sim/tests/fixtures/race_progress.txt", import.meta.url), "utf8");
  let n = 0;
  for (const line of text.split(/\r?\n/)) {
    if (!line || line.startsWith("#")) continue;
    const [slot, gates, x, y, z, g, k] = line.split(" ").map(Number);
    const [pg, pk] = raceProgress(BELT, { slot: slot!, gates: gates! }, { x: x!, y: y!, z: z! });
    assert.equal(pg, g, line);
    assert.ok(Math.abs(pk - k!) < 2e-4, `${line}: key ${pk}`);
    n++;
  }
  assert.equal(n, 96);
});

test("courseLeft: a lap at the line radius plus the dive, then straight to the exit", () => {
  const mid = 120;
  // At slot 0 (3 degrees past gate 0), no gate passed: 8 wedges less 3 degrees of arc, then the dive.
  const a = (3 * Math.PI) / 180;
  const at = ringPoint(BELT, a, mid);
  const left = courseLeft(BELT, { slot: 0, gates: 0 }, at);
  assert.ok(Math.abs(left - ((8 * WEDGE - a) * mid + (mid - 6))) < 1e-6, String(left));
  // Halfway through the lap (4 gates, half a sector in).
  const half = ringPoint(BELT, gateAzimuth(4) + WEDGE / 2, mid);
  assert.ok(Math.abs(courseLeft(BELT, { slot: 0, gates: 4 }, half) - (3.5 * WEDGE * mid + 114)) < 1e-6);
  // Measured on the middle line: the same at any radius and height in the sector.
  const inner = courseLeft(BELT, { slot: 0, gates: 4 }, ringPoint(BELT, gateAzimuth(4) + WEDGE / 2, 95));
  const high = courseLeft(BELT, { slot: 0, gates: 4 }, ringPoint(BELT, gateAzimuth(4) + WEDGE / 2, 200, 30));
  assert.ok(Math.abs(inner - high) < 1e-9);
  // Lap done: straight to the exit sphere.
  assert.equal(courseLeft(BELT, { slot: 3, gates: 8 }, { x: 160 + 40, y: 48, z: 160 }), 34);
  assert.equal(courseLeft(BELT, { slot: 3, gates: 8 }, { x: 162, y: 48, z: 160 }), 0);
  // Progress share: 0 at the slot, 1 in the exit.
  assert.ok(courseDone(BELT, { slot: 0, gates: 0 }, at) < 0.01);
  assert.equal(courseDone(BELT, { slot: 0, gates: 8 }, BELT.center), 1);
  assert.ok(Math.abs(courseLength(BELT) - (8 * WEDGE * 120 + 114)) < 1e-9);
});

test("courseLeft orders racers like the simulation's progress", () => {
  // Same sector: further along = less left; one more gate beats any key.
  const p1 = ringPoint(BELT, gateAzimuth(2) + 0.2, 120);
  const p2 = ringPoint(BELT, gateAzimuth(2) + 0.5, 120);
  assert.ok(courseLeft(BELT, { slot: 0, gates: 2 }, p2) < courseLeft(BELT, { slot: 0, gates: 2 }, p1));
  const p3 = ringPoint(BELT, gateAzimuth(3) + 0.01, 145);
  assert.ok(courseLeft(BELT, { slot: 0, gates: 3 }, p3) < courseLeft(BELT, { slot: 0, gates: 2 }, p2));
  // The sim orders the same way.
  const [g2, k2] = raceProgress(BELT, { slot: 0, gates: 2 }, p2);
  const [g1, k1] = raceProgress(BELT, { slot: 0, gates: 2 }, p1);
  assert.ok(g2 === g1 && k2 > k1);
});

test("gaps in seconds over the own average speed", () => {
  const own = { id: 1, place: 2, left: 500, finishTick: 0 };
  const ahead = { id: 2, place: 1, left: 380, finishTick: 0 };
  const behind = { id: 3, place: 3, left: 560, finishTick: 0 };
  assert.equal(gapSecs(own, ahead, 30, 1000), 4);
  assert.equal(gapSecs(own, behind, 30, 1000), -2);
  // Slow starts use at least 12 u/s.
  assert.equal(gapSecs(own, ahead, 2, 1000), 10);
  // A finished racer ahead: time since it finished plus the own way left.
  assert.equal(gapSecs(own, { id: 2, place: 1, left: 0, finishTick: 940 }, 50, 1000), 1 + 10);
  // Both finished: the finish ticks.
  assert.equal(gapSecs({ ...own, finishTick: 1300 }, { id: 2, place: 1, left: 0, finishTick: 1180 }, 50, 2000), 2);
  // Average speed: course done over the race time.
  const len = courseLength(BELT);
  assert.ok(Math.abs(averageSpeed(BELT, len - 600, 600, 600 + 60 * 20) - 30) < 1e-9);
  assert.equal(averageSpeed(BELT, len, 0, 5000), 0);
});

test("next gate and target, wrong way", () => {
  assert.equal(nextGate({ slot: 3, gates: 0 }), 4);
  assert.equal(nextGate({ slot: 3, gates: 7 }), 3);
  assert.equal(nextGate({ slot: 3, gates: 8 }), -1);
  const t = nextTarget(BELT, { slot: 0, gates: 1 });
  assert.ok(Math.abs(t.x - (160 + 120 * Math.cos(2 * WEDGE))) < 1e-9 && Math.abs(t.z - (160 - 120 * Math.sin(2 * WEDGE))) < 1e-9);
  assert.deepEqual(nextTarget(BELT, { slot: 0, gates: 8 }), BELT.center);
  const ww = new WrongWay();
  const pos = ringPoint(BELT, 0.3, 120);
  // Prograde at azimuth 0.3 is (-sin, 0, -cos); flying the other way for a second warns.
  const back = { x: Math.sin(0.3) * 20, y: 0, z: Math.cos(0.3) * 20 };
  assert.equal(ww.update(BELT, { slot: 0, gates: 0 }, pos, back, 10), false);
  assert.equal(ww.update(BELT, { slot: 0, gates: 0 }, pos, back, 10.5), false);
  assert.equal(ww.update(BELT, { slot: 0, gates: 0 }, pos, back, 10.9), true);
  // Turning around clears it; after the lap any direction is fine.
  assert.equal(ww.update(BELT, { slot: 0, gates: 0 }, pos, { x: -back.x, y: 0, z: -back.z }, 11), false);
  assert.equal(ww.update(BELT, { slot: 0, gates: 8 }, pos, back, 20), false);
  // A hold left over from the last heat does not carry into the next one (RaceClient resets it between heats).
  assert.equal(ww.update(BELT, { slot: 0, gates: 0 }, pos, back, 30), false);
  ww.reset();
  assert.equal(ww.update(BELT, { slot: 0, gates: 0 }, pos, back, 40), false, "the hold starts over after a reset");
});

test("race clock remembers the lights' end", () => {
  const c = new RaceClock();
  const race = { heat: 0, heats: 3, phase: "lights" as const, phaseEndTick: 900, racers: [] };
  assert.equal(c.update(race), 900);
  assert.equal(c.update({ ...race, phase: "racing", phaseEndTick: 900 + 180 * 60 }), 900);
  // Joined mid-race in another heat: estimated from the limit.
  const c2 = new RaceClock();
  assert.equal(c2.update({ ...race, heat: 1, phase: "racing", phaseEndTick: 20000 }), 20000 - 10800);
});

test("parseBelt reads ls-wasm's belt JSON", () => {
  const json = JSON.stringify({
    center: [160, 48, 160],
    innerRadius: 95,
    outerRadius: 145,
    halfThickness: 12,
    cocoonRadius: 30,
    exitRadius: 6,
    gateInnerRadius: 87,
    gateOuterRadius: 153,
    gateAzimuths: [0, 0.78],
    slots: [{ pos: [280, 48, 154], rot: [0, 0.02, 0, 0.99] }],
    pickupPoints: [[1, 2, 3]],
    boulders: [{ radius: 120, height: 2, phase: 0.5, angularSpeed: 0.02, size: 3, tumbleAxis: [0, 1, 0], tumbleSpeed: 0.4 }],
  });
  const b = parseBelt(json);
  assert.ok(b);
  assert.equal(b.slots.length, 1);
  assert.equal(b.boulders[0]?.size, 3);
  assert.deepEqual(b.pickupPoints[0], { x: 1, y: 2, z: 3 });
  assert.equal(parseBelt(undefined), null);
  assert.equal(parseBelt("{"), null);
});
