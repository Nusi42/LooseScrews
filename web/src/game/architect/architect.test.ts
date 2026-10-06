import assert from "node:assert/strict";
import { test } from "node:test";

import * as q from "../../math/quat.ts";
import { emptyInput } from "../../input/input.ts";
import { emptyModeState, emptyMood, OBJECTIVE, PLAYER_FLAG, VIEW_FLAG, isOut, type ModeState, type Objective } from "../modeState.ts";
import type { VfxSink } from "../vfxLibrary.ts";
import type { ParticleSpec } from "../../renderer/particles.ts";
import { ArchitectCamera, CUT_ABOVE, OVERVIEW_DISTANCE, clampTarget } from "./camera.ts";
import {
  ArchitectTracker,
  architectView,
  DEFAULT_ARCHITECT_RULES,
  parseArchitectRules,
  REFUSE_TICKS,
  refusalText,
  strokeCost,
  strokeDef,
  strokeForSlider,
  strokeProblem,
  STROKES,
  telegraphProgress,
  type StrokeCheck,
} from "./model.ts";
import { aimPoint, AIM_FALLBACK, worldSliders } from "./index.ts";
import { ArchitectFx, architectLights, architectVolumes, BAD_COLOR, EXIT_COLOR, type ArchitectSceneInput } from "./visuals.ts";
import { architectHudModel, cardState, radialSector, type ArchitectHudInput } from "../../ui/architect.ts";
import { modeHudModel, objectiveMarkers } from "../../ui/modeHud.ts";
import { resultsModel } from "../../ui/results.ts";
import * as mat4 from "../../math/mat4.ts";

const BOUNDS = [0, 0, 0, 256, 128, 256];
const ARCH = 4;

function obj(id: number, kind: number, pos = { x: 100, y: 40, z: 100 }, extra: Partial<Objective> = {}): Objective {
  return { id, kind, team: -1, holder: ARCH, state: 0, timerEnd: 0, radius: 0, pos, ...extra };
}

/** An Architect mode state: round `round` of 3, the architect `ARCH` with `budget`, runners 0-2. */
function architectMode(o: { round?: number; over?: boolean; won?: boolean; budget?: number; strokes?: Objective[]; finished?: number[] } = {}): ModeState {
  const m = emptyModeState();
  m.mode = "the_architect";
  m.kind = 6;
  m.teams = 0;
  m.stage = o.round ?? 0;
  m.flags = VIEW_FLAG.lives;
  m.objectives = [obj(0, OBJECTIVE.exit, { x: 230, y: 40, z: 128 }, { radius: 6, state: (3 << 4) | (o.over ? 1 : 0) | (o.won ? 2 : 0), timerEnd: 9000 }), ...(o.strokes ?? [])];
  m.players = [0, 1, 2].map((id) => ({ id, side: id, score: 0, lives: 2, meter: id === 1 ? 2 : 0, flags: o.finished?.includes(id) ? PLAYER_FLAG.finished : 0 }));
  m.players.push({ id: ARCH, side: ARCH, score: 1, lives: 2, meter: o.budget ?? 100, flags: PLAYER_FLAG.architect });
  return m;
}

function stroke(id: number, index: number, active: boolean, timerEnd: number, pos = { x: 100, y: 40, z: 100 }, radius = 16): Objective {
  return obj(id, OBJECTIVE.stroke, pos, { radius, state: index | ((active ? 1 : 0) << 4), timerEnd });
}

test("the eight strokes are in wire order with the data file's costs", () => {
  assert.deepEqual(
    STROKES.map((s) => s.id),
    ["enclosure_plus", "enclosure_minus", "density_plus", "gravity_chaos_plus", "instability_plus", "absurdity_plus", "palette", "verticality"],
  );
  assert.deepEqual(DEFAULT_ARCHITECT_RULES.costs, [25, 15, 20, 30, 35, 60, 15, 80]);
  assert.equal(strokeDef(8), null);
  assert.equal(strokeForSlider("enclosure", 5)?.index, 0);
  assert.equal(strokeForSlider("enclosure", -5)?.index, 1);
  assert.equal(strokeForSlider("density", -5), null);
  assert.equal(strokeForSlider("palette", -3)?.index, 6);
  assert.equal(strokeForSlider("verticality", 9)?.index, 7);
});

test("architectRules JSON (ls-wasm architect_rules_json) parses, broken fields take the defaults", () => {
  // The exact string the Rust test in crates/ls-wasm/src/modes.rs checks.
  const r = parseArchitectRules('{"costs":[25,15,20,30,35,60,15,80],"minRadius":12,"maxRadius":24,"cooldownSecs":2,"telegraphSecs":2,"maxStrokes":4,"budgetStart":100,"budgetPerSec":4,"budgetMax":250,"keepClear":6,"wellSecs":30,"paletteSecs":20,"roundSecs":240,"roundResultsSecs":6,"runnerLives":2,"absurdity":100,"spawns":[[1,2.5,-3]]}');
  assert.ok(r);
  assert.equal(strokeCost(r, 5), 60);
  assert.deepEqual(r.spawns, [{ x: 1, y: 2.5, z: -3 }]);
  const broken = parseArchitectRules('{"costs":[1],"minRadius":"x"}');
  assert.equal(broken?.costs[0], 1);
  assert.equal(broken?.costs[1], 15);
  assert.equal(broken?.minRadius, 12);
  assert.equal(parseArchitectRules(undefined), null);
  assert.equal(parseArchitectRules("not json"), null);
});

test("architectView reads rounds, the architect's budget, the exit and the strokes", () => {
  assert.equal(architectView(emptyModeState()), null);
  const m = architectMode({ round: 1, budget: 137, finished: [2], strokes: [stroke(1, 3, false, 500), stroke(2, 0, true, 600)] });
  const v = architectView(m);
  assert.ok(v);
  assert.equal(v.round, 1);
  assert.equal(v.rounds, 3);
  assert.equal(v.architect, ARCH);
  assert.equal(v.budget, 137);
  assert.equal(v.roundOver, false);
  assert.deepEqual(v.exit, { pos: { x: 230, y: 40, z: 128 }, radius: 6 });
  assert.deepEqual(
    v.strokes.map((s) => [s.def.id, s.phase, s.timerEnd]),
    [
      ["gravity_chaos_plus", "telegraph", 500],
      ["enclosure_plus", "active", 600],
    ],
  );
  assert.deepEqual(
    v.runners.map((r) => [r.id, r.finished, r.architectPoints]),
    [
      [0, false, 0],
      [1, false, 2],
      [2, true, 0],
    ],
  );
  const over = architectView(architectMode({ over: true, won: true }));
  assert.equal(over?.roundOver, true);
  assert.equal(over?.runnersWon, true);
  // The architect and finished runners have no ship to fly: the client spectates.
  assert.equal(isOut(m.players.find((p) => p.id === ARCH)), true);
  assert.equal(isOut(m.players.find((p) => p.id === 2)), true);
  assert.equal(isOut(m.players.find((p) => p.id === 0)), false);
});

function check(over: Partial<StrokeCheck> = {}): StrokeCheck {
  const view = over.view ?? architectView(architectMode({ budget: 100 }))!;
  return {
    view,
    rules: { ...DEFAULT_ARCHITECT_RULES, spawns: [{ x: 20, y: 40, z: 128 }] },
    own: ARCH,
    nowTick: 1000,
    stroke: 0,
    center: { x: 120, y: 50, z: 120 },
    radius: 16,
    bounds: BOUNDS,
    cooldownUntil: 0,
    verticalityUsed: false,
    mood: emptyMood(),
    ...over,
  };
}

test("strokeProblem mirrors the server's checks", () => {
  assert.equal(strokeProblem(check()), null);
  assert.match(strokeProblem(check({ own: 1 })) ?? "", /Only the architect/);
  assert.match(strokeProblem(check({ view: architectView(architectMode({ over: true }))! })) ?? "", /round is over/);
  assert.match(strokeProblem(check({ radius: 30 })) ?? "", /Size 12-24/);
  assert.match(strokeProblem(check({ center: { x: -5, y: 50, z: 120 } })) ?? "", /Outside/);
  assert.match(strokeProblem(check({ center: { x: 215, y: 40, z: 128 } })) ?? "", /exit/);
  // Only rock strokes keep clear of the exit and the spawns.
  assert.equal(strokeProblem(check({ center: { x: 215, y: 40, z: 128 }, stroke: 3 })), null);
  assert.match(strokeProblem(check({ center: { x: 30, y: 40, z: 128 } })) ?? "", /start/);
  assert.match(strokeProblem(check({ cooldownUntil: 1090 })) ?? "", /Recharging · 1\.5 s/);
  assert.match(strokeProblem(check({ view: architectView(architectMode({ budget: 20 }))! })) ?? "", /Needs 25 · you have 20/);
  const four = [1, 2, 3, 4].map((i) => stroke(i, 3, false, 2000));
  assert.match(strokeProblem(check({ view: architectView(architectMode({ strokes: four }))! })) ?? "", /4 strokes at once/);
  assert.match(strokeProblem(check({ stroke: 7, verticalityUsed: true })) ?? "", /once a round/);
  assert.match(strokeProblem(check({ stroke: 5, mood: { ...emptyMood(), eventId: "hat_rain" } })) ?? "", /event is already/);
  assert.match(strokeProblem(check({ stroke: 5, rules: { ...DEFAULT_ARCHITECT_RULES, absurdity: 0 } })) ?? "", /off/);
  assert.match(refusalText(null, STROKES[1]!), /aim into rock/);
  assert.equal(refusalText("Too close to the exit", STROKES[0]!), "Too close to the exit");
});

test("the tracker turns snapshots into events, our cooldown and the round history", () => {
  const t = new ArchitectTracker();
  const rules = DEFAULT_ARCHITECT_RULES;
  let ev = t.update(architectView(architectMode())!, ARCH, 100, rules);
  assert.deepEqual(ev.map((e) => e.kind), ["roundStart"]);
  t.sent(7, 100);
  assert.equal(t.waiting, true);
  ev = t.update(architectView(architectMode({ strokes: [stroke(1, 7, false, 220)] }))!, ARCH, 102, rules);
  assert.deepEqual(ev.map((e) => e.kind), ["telegraph"]);
  assert.equal(ev[0]?.kind === "telegraph" && ev[0].ours, true);
  assert.equal(t.waiting, false);
  assert.equal(t.cooldownUntil, 102 + 120);
  assert.equal(t.verticalityUsed, true);
  ev = t.update(architectView(architectMode({ strokes: [stroke(1, 7, true, 280)] }))!, ARCH, 220, rules);
  assert.deepEqual(ev.map((e) => e.kind), ["act"]);
  ev = t.update(architectView(architectMode({ finished: [1] }))!, ARCH, 300, rules);
  assert.deepEqual(ev.map((e) => e.kind).sort(), ["end", "escaped"]);
  // A request nobody answers is refused after REFUSE_TICKS.
  t.sent(1, 300, null);
  assert.deepEqual(t.update(architectView(architectMode({ finished: [1] }))!, ARCH, 300 + REFUSE_TICKS, rules), []);
  ev = t.update(architectView(architectMode({ finished: [1] }))!, ARCH, 301 + REFUSE_TICKS, rules);
  assert.equal(ev[0]?.kind, "refused");
  assert.match(ev[0]?.kind === "refused" ? ev[0].reason : "", /aim into rock/);
  ev = t.update(architectView(architectMode({ over: true, finished: [1] }))!, ARCH, 400, rules);
  assert.equal(ev[0]?.kind, "roundOver");
  assert.deepEqual(t.history, [{ round: 0, architect: ARCH, runners: 3, escaped: 1, runnersWon: false }]);
  // The next round clears Verticality.
  ev = t.update(architectView(architectMode({ round: 1 }))!, ARCH, 800, rules);
  assert.equal(ev[0]?.kind, "roundStart");
  assert.equal(t.verticalityUsed, false);
  t.reset();
  assert.equal(t.history.length, 0);
});

test("telegraph progress fills over the telegraph", () => {
  const v = architectView(architectMode({ strokes: [stroke(1, 0, false, 1120)] }))!;
  const s = v.strokes[0]!;
  assert.equal(telegraphProgress(s, 1000, 2), 0);
  assert.equal(telegraphProgress(s, 1060, 2), 0.5);
  assert.equal(telegraphProgress(s, 1200, 2), 1);
});

test("aimPoint: on the rock for growth, into it for Enclosure-, back into the open for the rest", () => {
  const o = { x: 100, y: 50, z: 100 };
  const d = { x: 1, y: 0, z: 0 };
  assert.deepEqual(aimPoint(o, d, 20, 0, 16, BOUNDS), { x: 120, y: 50, z: 100 });
  assert.equal(aimPoint(o, d, 20, 1, 16, BOUNDS).x, 120 + 16 * 0.35);
  assert.equal(aimPoint(o, d, 20, 3, 16, BOUNDS).x, 112);
  assert.equal(aimPoint(o, d, -1, 3, 16, BOUNDS).x, 100 + AIM_FALLBACK);
  assert.equal(aimPoint(o, d, 500, 0, 16, BOUNDS).x, 256);
});

test("the share code's sliders feed the slider panel", () => {
  assert.equal(worldSliders("not a code"), null);
});

test("architect camera: free flight, the map view over the point below, the cut", () => {
  const cam = new ArchitectCamera({ position: { x: 100, y: 60, z: 100 }, orientation: q.identity() });
  const fly = { ...emptyInput(), thrustForward: 1 };
  cam.update(1, fly, 0.002, BOUNDS);
  assert.ok(Math.abs(cam.position.z - 70) < 1e-6, `${cam.position.z}`);
  assert.equal(cam.cutaway(BOUNDS), null);
  cam.toggle(BOUNDS);
  assert.equal(cam.mode, "overview");
  assert.deepEqual(cam.target, { x: 100, y: 60, z: 70 });
  const cut = cam.cutaway(BOUNDS);
  assert.equal(cut?.max.y, 60 + CUT_ABOVE);
  // The camera looks at the target from OVERVIEW_DISTANCE, heading where the free camera looked (-Z).
  const toTarget = Math.hypot(cam.position.x - 100, cam.position.y - 60, cam.position.z - 70);
  assert.ok(Math.abs(toTarget - OVERVIEW_DISTANCE) < 1e-6);
  const f = q.forward(cam.orientation);
  assert.ok(f.y < -0.5 && f.z < 0, JSON.stringify(f));
  // W pans along the heading, Space raises the cut.
  cam.update(1, { ...emptyInput(), thrustForward: 1, thrustUp: 1 }, 0.002, BOUNDS);
  assert.ok(cam.target.z < 70 && cam.target.y > 60);
  assert.deepEqual(clampTarget({ x: -9, y: -9, z: 999 }, BOUNDS), { x: 0, y: 2, z: 256 });
  cam.toggle(BOUNDS);
  assert.equal(cam.mode, "free");
});

test("the wheel picks a sector clockwise from the top", () => {
  assert.equal(radialSector(0, 0), -1);
  assert.equal(radialSector(0, -80), 0);
  assert.equal(radialSector(80, 0), 2);
  assert.equal(radialSector(0, 80), 4);
  assert.equal(radialSector(-80, 0), 6);
  assert.equal(radialSector(60, -60), 1);
});

function hudInput(over: Partial<ArchitectHudInput> = {}): ArchitectHudInput {
  return {
    view: architectView(architectMode({ budget: 50 }))!,
    rules: DEFAULT_ARCHITECT_RULES,
    own: ARCH,
    nowTick: 1000,
    names: new Map([[ARCH, "Gasket"]]),
    armed: 0,
    radius: 18,
    problem: null,
    aimDistance: 30,
    cooldownUntil: 0,
    verticalityUsed: false,
    mood: emptyMood(),
    overview: false,
    pointerLocked: true,
    radial: { open: false, selected: -1 },
    banner: null,
    toast: null,
    waiting: false,
    sliders: null,
    ...over,
  };
}

test("the HUD model: the architect's cards and budget, the runner's plate", () => {
  const m = architectHudModel(hudInput());
  assert.equal(m.role, "architect");
  assert.deepEqual(
    m.cards.map((c) => c.state),
    ["armed", "ready", "ready", "ready", "ready", "poor", "ready", "poor"],
  );
  assert.equal(m.budgetFill, 20);
  assert.equal(m.afterFill, 10);
  assert.equal(m.reticle?.status, "Click to place");
  assert.equal(architectHudModel(hudInput({ problem: "Too close to the exit" })).reticle?.ok, false);
  assert.equal(architectHudModel(hudInput({ pointerLocked: false })).showPanel, true);
  assert.equal(cardState(hudInput({ cooldownUntil: 1060 }), 1).state, "cooldown");
  assert.equal(cardState(hudInput({ verticalityUsed: true }), 7).state, "used");
  const runner = architectHudModel(hudInput({ own: 0 }));
  assert.equal(runner.role, "runner");
  assert.equal(runner.cards.length, 0);
  assert.equal(runner.architectName, "Gasket");
  assert.equal(runner.reticle, null);
  assert.equal(runner.runnersLine, "3 runners");
});

class FakeSink implements VfxSink {
  now = 0;
  density = 1;
  spawned: ParticleSpec[] = [];
  spawn(p: ParticleSpec): void {
    this.spawned.push(p);
  }
  immediate(p: ParticleSpec): void {
    this.spawned.push(p);
  }
}

function scene(m: ModeState, over: Partial<ArchitectSceneInput> = {}): ArchitectSceneInput {
  return {
    view: architectView(m)!,
    rules: DEFAULT_ARCHITECT_RULES,
    nowTick: 1000,
    time: 1,
    amArchitect: false,
    preview: null,
    runners: [],
    overviewTarget: null,
    camera: { x: 0, y: 0, z: 0 },
    ...over,
  };
}

test("volumes: preview first, telegraphs with a filling ring, the exit gate, at most 12", () => {
  const m = architectMode({ strokes: [stroke(1, 0, false, 1060)] });
  const vols = architectVolumes(scene(m, { amArchitect: true, preview: { def: STROKES[2]!, center: { x: 1, y: 2, z: 3 }, radius: 14, ok: false } }));
  assert.equal(vols[0]?.kind, "zone");
  assert.deepEqual(vols[0]?.kind === "zone" ? vols[0].color : null, BAD_COLOR);
  const tele = vols[1];
  assert.ok(tele?.kind === "zone" && Math.abs((tele.ring ?? 0) - 0.5) < 1e-9);
  assert.ok(vols.some((v) => v.kind === "zone" && v.color === EXIT_COLOR));
  const many = architectMode({ strokes: [1, 2, 3, 4].map((i) => stroke(i, i, false, 1100)) });
  const runners = Array.from({ length: 7 }, (_, i) => ({ id: i, side: i, pos: { x: i, y: 0, z: 0 } }));
  assert.ok(architectVolumes(scene(many, { amArchitect: true, runners })).length <= 12);
  // Runners' pillars only for the architect.
  assert.ok(!architectVolumes(scene(many, { runners })).some((v) => v.kind === "beam" && v.width === 0.3));
  const lights = architectLights(scene(m, { overviewTarget: { x: 5, y: 5, z: 5 } }));
  assert.equal(lights.length, 3);
});

test("particles: sparks into a telegraph, the ink cloud, bursts when a stroke acts", () => {
  const sink = new FakeSink();
  const fx = new ArchitectFx(sink, () => 0.5);
  const tele = architectMode({ strokes: [stroke(1, 0, false, 1060)] });
  for (let i = 0; i < 30; i++) fx.update(scene(tele), [], 1 / 60);
  const sparks = sink.spawned.length;
  assert.ok(sparks > 10, `${sparks} sparks`);
  sink.spawned = [];
  const ink = architectMode({ strokes: [stroke(1, 6, true, 2000)] });
  for (let i = 0; i < 120; i++) fx.update(scene(ink), [], 1 / 60);
  const puffs = sink.spawned.filter((p) => p.blend === "alpha");
  assert.ok(puffs.length >= 40, `${puffs.length} ink puffs in 2 s`);
  sink.spawned = [];
  const v = architectView(architectMode({ strokes: [stroke(1, 0, true, 1008)] }))!;
  fx.update(scene(architectMode({})), [{ kind: "act", stroke: v.strokes[0]! }], 1 / 60);
  assert.ok(sink.spawned.length > 30, `${sink.spawned.length} burst particles`);
});

test("mode HUD: Architect rounds, a runner warned inside a telegraph, exit and stroke markers", () => {
  const m = architectMode({ round: 1, strokes: [stroke(1, 0, false, 1060, { x: 100, y: 40, z: 100 }, 16)] });
  m.stageEndTick = 1000 + 90 * 60;
  const names = new Map<number, string>();
  const runner = modeHudModel({ mode: m, stage: "match", own: 0, ownSide: 0, nowTick: 1000, timeLeft: 90, names, ownPos: { x: 105, y: 40, z: 100 }, spectating: false, following: null });
  // The round line and the plate say it all: no target or own line under the chips.
  assert.equal(runner.target, "");
  assert.equal(runner.ownLine, "");
  assert.match(runner.stage, /^Round 2 of 3/);
  assert.ok(runner.alerts.some((a) => a.level === "danger" && /Enclosure\+ in 1\.0 s: rock grows here/.test(a.text)), JSON.stringify(runner.alerts));
  const arch = modeHudModel({ mode: m, stage: "match", own: ARCH, ownSide: ARCH, nowTick: 1000, timeLeft: 90, names, ownPos: null, spectating: true, following: null });
  assert.equal(arch.lives, -1, "the architect has no lives to show");
  assert.ok(!arch.alerts.some((a) => /Spectating/.test(a.text)), "the architect has its own HUD, not the spectator line");
  const marks = objectiveMarkers(m, 0, names, 1000, mat4.create(), { x: 0, y: 0, z: 0 }, 800, 600, 0);
  assert.deepEqual(
    marks.map((k) => k.label),
    ["Exit", "Enclosure+ · 1.0 s"],
  );
  const over = architectMode({ over: true, won: false });
  assert.match(modeHudModel({ mode: over, stage: "match", own: 0, ownSide: 0, nowTick: 1000, timeLeft: 0, names, ownPos: null, spectating: true, following: null }).stage, /to the architect/);
});

test("results: an Architect match splits the points and lists the rounds", () => {
  const mode = emptyModeState();
  mode.mode = "the_architect";
  mode.players = [
    { id: 0, side: 0, score: 4, lives: 0, meter: 2, flags: 0 },
    { id: 1, side: 1, score: 1, lives: 0, meter: 0, flags: 0 },
  ];
  const model = resultsModel({
    results: {
      tick: 0,
      kind: 6,
      mode: "the_architect",
      teams: 0,
      winner: 0,
      teamScores: [],
      rows: [
        { id: 0, side: 0, kills: 0, deaths: 1, score: 4, name: "Ana" },
        { id: 1, side: 1, kills: 0, deaths: 2, score: 1, name: "Bo" },
      ],
      awards: [],
    },
    own: 1,
    ownSide: 1,
    online: true,
    modeFlags: 0,
    scoreTotal: 0,
    lobby: null,
    secsLeft: 0,
    architect: { rounds: [{ round: 0, architect: 0, runners: 1, escaped: 0, runnersWon: false }, { round: 1, architect: 1, runners: 1, escaped: 1, runnersWon: true }], mode },
  });
  assert.equal(model.architectColumns, true);
  assert.deepEqual(
    model.rows.map((r) => [r.architectPoints, r.runnerPoints]),
    [
      [2, 2],
      [0, 1],
    ],
  );
  assert.deepEqual(
    model.rounds.map((r) => [r.text, r.winner, r.you]),
    [
      ["Ana built · 0 of 1 escaped", "architect", false],
      ["You built · 1 of 1 escaped", "runners", false],
    ],
  );
});
