import { test } from "node:test";
import assert from "node:assert/strict";
import * as mat4 from "../math/mat4.ts";
import * as q from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import { courseLeft, gapSecs, gateAzimuth, ringPoint, WEDGE, type Belt } from "../game/race/course.ts";
import type { MatchResults, ModeState, RaceState } from "../game/modeState.ts";
import { emptyModeState } from "../game/modeState.ts";
import { formatGap, formatRaceTime, HEAT_POINTS, MINI_RING, MINI_SIZE, miniPoint, raceHudModel, screenMarker, type RaceHudInput } from "./raceHud.ts";
import { beltUpStage } from "./modeHud.ts";
import { resultsModel } from "./results.ts";

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

const viewProj = (pos: Vec3, look: Vec3): mat4.Mat4 => {
  const d = { x: look.x - pos.x, y: look.y - pos.y, z: look.z - pos.z };
  const l = Math.hypot(d.x, d.y, d.z);
  const o = q.lookRotation({ x: d.x / l, y: d.y / l, z: d.z / l }, { x: 0, y: 1, z: 0 });
  return mat4.multiply(mat4.perspective(1.18, 16 / 9, 0.05, 600), mat4.viewFromPose(pos, o));
};

function hudInput(race: RaceState, positions: Map<number, Vec3>, over: Partial<RaceHudInput> = {}): RaceHudInput {
  const own = positions.get(0) ?? null;
  return {
    belt: BELT,
    race,
    own: 0,
    names: new Map([
      [1, "Rivet"],
      [2, "Sprocket"],
    ]),
    sides: new Map([
      [0, 0],
      [1, 1],
      [2, 2],
    ]),
    points: new Map([
      [0, 10],
      [1, 8],
      [2, 6],
    ]),
    positions,
    ownPos: own,
    ownVel: { x: 0, y: 0, z: -20 },
    nowTick: 900 + 60 * 30,
    raceStart: 900,
    dose: 0,
    hull: 80,
    instagib: false,
    lights: { show: false, lit: 0, go: false, sinceGo: 0 },
    wrongWay: false,
    viewProj: viewProj(own ?? BELT.center, BELT.center),
    width: 1920,
    height: 1080,
    time: 0,
    spectating: false,
    ...over,
  };
}

/** Three racers in slot 0's lap: the own racer second, one ahead, one behind. */
function threeRacers(): { race: RaceState; positions: Map<number, Vec3> } {
  const race: RaceState = {
    heat: 1,
    heats: 3,
    phase: "racing",
    phaseEndTick: 900 + 180 * 60,
    racers: [
      { id: 0, gates: 3, nextGate: 4, finishTick: 0, place: 2, holdEndTick: 0, slot: 0 },
      { id: 1, gates: 3, nextGate: 4, finishTick: 0, place: 1, holdEndTick: 0, slot: 0 },
      { id: 2, gates: 2, nextGate: 3, finishTick: 0, place: 3, holdEndTick: 0, slot: 0 },
    ],
  };
  const positions = new Map<number, Vec3>([
    [0, ringPoint(BELT, gateAzimuth(3) + 0.3, 118)],
    [1, ringPoint(BELT, gateAzimuth(3) + 0.6, 125)],
    [2, ringPoint(BELT, gateAzimuth(2) + 0.5, 110)],
  ]);
  return { race, positions };
}

test("formats: race time, gaps, the minimap point", () => {
  assert.equal(formatRaceTime(72.43), "1:12.4");
  assert.equal(formatRaceTime(5), "0:05.0");
  assert.equal(formatGap(1.44), "+1.4 s");
  assert.equal(formatGap(-0.81), "−0.8 s");
  assert.equal(formatGap(0.01), "+0.0 s");
  // The middle of the belt maps onto the ring; prograde is counter-clockwise on the map.
  const p = miniPoint(BELT, ringPoint(BELT, 0, 120));
  assert.ok(Math.abs(p.x - (MINI_SIZE / 2 + MINI_RING)) < 1e-9 && Math.abs(p.y - MINI_SIZE / 2) < 1e-9);
  const p2 = miniPoint(BELT, ringPoint(BELT, Math.PI / 2, 120));
  assert.ok(p2.y < MINI_SIZE / 2 - MINI_RING + 1e-6, "a quarter lap on: the top of the map");
});

test("place, heat, points, clock, gaps and distance follow the course maths", () => {
  const { race, positions } = threeRacers();
  const m = raceHudModel(hudInput(race, positions));
  assert.equal(m.visible, true);
  assert.deepEqual(m.place, { place: 2, of: 3 });
  assert.equal(m.heat, "HEAT 2/3");
  assert.equal(m.points, 10);
  assert.equal(m.clock, "0:30.0");
  // Gaps: the course difference over the own average speed, as course.ts computes them.
  const left = (id: number): number => courseLeft(BELT, race.racers.find((r) => r.id === id)!, positions.get(id)!);
  const avg = (2 * Math.PI * 120 + 114 - left(0)) / 30;
  const own = { id: 0, place: 2, left: left(0), finishTick: 0 };
  assert.equal(m.ahead?.gap, formatGap(gapSecs(own, { id: 1, place: 1, left: left(1), finishTick: 0 }, avg, 0)));
  assert.equal(m.behind?.gap, formatGap(gapSecs(own, { id: 2, place: 3, left: left(2), finishTick: 0 }, avg, 0)));
  assert.ok(m.ahead?.gap.startsWith("+") && m.behind?.gap.startsWith("−"));
  assert.equal(m.ahead?.name, "Rivet");
  assert.equal(m.distance, `${Math.round(left(0))} u to the EXIT`);
  assert.equal(m.next, "GATE 4/8");
  assert.ok(m.marker && m.marker.label === "GATE 4/8");
  assert.equal(m.results, null);
  assert.equal(m.finish, null);
  // Minimap: three dots (the own one last, on top) and 8 gates, the next one marked.
  assert.equal(m.mini?.dots.length, 3);
  assert.equal(m.mini?.dots[2]?.own, true);
  assert.equal(m.mini?.gates.filter((g) => g.state === "next").length, 1);
  assert.equal(m.mini?.gates.filter((g) => g.state === "passed").length, 3);
});

test("after the lap the marker points at the EXIT; a finish shows the banner", () => {
  const { race, positions } = threeRacers();
  race.racers[0]!.gates = 8;
  const m = raceHudModel(hudInput(race, positions));
  assert.equal(m.next, "EXIT");
  assert.equal(m.marker?.exit, true);
  race.racers[0]!.finishTick = 900 + 60 * 75;
  race.racers[0]!.place = 1;
  const f = raceHudModel(hudInput(race, positions, { nowTick: 900 + 60 * 80 }));
  assert.deepEqual(f.finish, { place: 1, time: "1:15.0" });
  assert.equal(f.clock, "1:15.0", "the clock stops at the finish");
  assert.equal(f.marker, null);
});

test("markers clamp to the screen edge with an arrow when off screen or behind", () => {
  const cam = { x: 0, y: 0, z: 0 };
  const vp = viewProj(cam, { x: 0, y: 0, z: -10 });
  const ahead = screenMarker({ x: 0, y: 0, z: -50 }, vp, 1920, 1080);
  assert.ok(ahead.onScreen && Math.abs(ahead.x - 960) < 1 && Math.abs(ahead.y - 540) < 1);
  const right = screenMarker({ x: 200, y: 0, z: -10 }, vp, 1920, 1080);
  assert.ok(!right.onScreen && Math.abs(right.x - (1920 - 48)) < 1 && Math.abs(right.angle) < 0.1);
  const behind = screenMarker({ x: -5, y: 0, z: 50 }, vp, 1920, 1080);
  assert.equal(behind.onScreen, false);
  assert.ok(behind.x <= 48 + 1, "behind and to the left: the left edge");
});

test("radiation meter: dose rate and how long the hull lasts; Instagib shows the dose left", () => {
  const { race, positions } = threeRacers();
  const m = raceHudModel(hudInput(race, positions, { dose: 6, hull: 42 }));
  assert.equal(m.rad?.rate, "6.0 hull/s");
  assert.equal(m.rad?.lasts, "hull lasts 7 s");
  assert.equal(m.rad?.danger, true);
  assert.ok(Math.abs((m.rad?.level ?? 0) - 6 / 16) < 1e-9);
  const ig = raceHudModel(hudInput(race, positions, { dose: 6, hull: 30, hullMax: 120, instagib: true }));
  assert.equal(ig.rad?.lasts, "dose left 25%");
  assert.equal(raceHudModel(hudInput(race, positions, { dose: 0 })).rad, null);
  assert.equal(raceHudModel(hudInput(race, positions, { dose: 5, spectating: true })).rad, null);
});

test("heat results: places, times, points gained and totals", () => {
  const { race, positions } = threeRacers();
  race.phase = "results";
  race.racers[0]!.finishTick = 900 + 4500;
  race.racers[1]!.finishTick = 900 + 4380;
  race.racers[0]!.place = 2;
  race.racers[1]!.place = 1;
  const m = raceHudModel(hudInput(race, positions));
  assert.equal(m.results?.title, "HEAT 2 OF 3 · RESULTS");
  assert.deepEqual(
    m.results?.rows.map((r) => [r.place, r.name, r.time, r.gain, r.own]),
    [
      [1, "Rivet", "1:13.0", `+${HEAT_POINTS[0]}`, false],
      [2, "You", "1:15.0", `+${HEAT_POINTS[1]}`, true],
      [3, "Sprocket", "DNF", `+${HEAT_POINTS[2]}`, false],
    ],
  );
  assert.equal(m.distance, "");
});

test("start lights in the HUD; nothing without a race or belt", () => {
  const { race, positions } = threeRacers();
  race.phase = "lights";
  const m = raceHudModel(hudInput(race, positions, { lights: { show: true, lit: 3, go: false, sinceGo: 0 } }));
  assert.deepEqual(m.lights, { show: true, lit: 3, go: false });
  assert.equal(m.clock, "0:00.0");
  assert.equal(raceHudModel(hudInput(race, positions, { belt: null })).visible, false);
  assert.equal(raceHudModel({ ...hudInput(race, positions), race: null }).visible, false);
});

test("Belt Up lines: the mode HUD's stage and the results podium", () => {
  const race: RaceState = { heat: 0, heats: 3, phase: "lights", phaseEndTick: 600, racers: [] };
  assert.equal(beltUpStage(race, 500), "Heat 1/3 · start lights");
  assert.equal(beltUpStage({ ...race, phase: "racing", phaseEndTick: 600 + 60 * 100 }, 600), "Heat 1/3 · 1:40 left");
  assert.equal(beltUpStage({ ...race, phase: "results", phaseEndTick: 900 }, 600), "Heat 1/3 results · next heat in 5 s");
  assert.equal(beltUpStage({ ...race, heat: 2, phase: "results", phaseEndTick: 900 }, 600), "Heat 3/3 results · podium next");
  const results: MatchResults = {
    tick: 0,
    kind: 7,
    mode: "belt_up",
    teams: 0,
    winner: 1,
    teamScores: [],
    rows: [
      { id: 1, side: 1, kills: 0, deaths: 1, score: 26, name: "Rivet" },
      { id: 0, side: 0, kills: 2, deaths: 0, score: 24, name: "Me" },
    ],
    awards: [],
  };
  const m = resultsModel({ results, own: 0, ownSide: 0, online: true, modeFlags: 0, scoreTotal: 0, lobby: null, secsLeft: 0 });
  assert.equal(m.subtitle, "Rivet rules the belt with 26 points");
  assert.equal(m.scoreUnit, " pts");
  assert.equal(m.podium.length, 2);
  const other: ModeState = emptyModeState();
  assert.equal(resultsModel({ results: { ...results, mode: other.mode || "team_brawl" }, own: 0, ownSide: 0, online: true, modeFlags: 0, scoreTotal: 0, lobby: null, secsLeft: 0 }).scoreUnit, "");
});

test("a gate is WEDGE apart (sanity of the shared constants)", () => {
  assert.ok(Math.abs(gateAzimuth(2) - 2 * WEDGE) < 1e-12);
});
