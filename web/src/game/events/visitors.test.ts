// Visitor event visuals (M4 C2): cue decoding, timelines and packing of the
// sim's state bytes, the strips' maths, and that every visitor draws its
// meshes, hides what it should and asks for sounds that exist.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import * as q from "../../math/quat.ts";
import { MESH_KINDS } from "../../renderer/models/index.ts";
import { emptyScene } from "../../renderer/renderer.ts";
import { VISITOR_SOUNDS } from "../../audio/sounds/visitors.ts";
import { EVENT_IDS } from "../../audio/announcer/lines.ts";
import { emptyMood, type MoodState } from "../modeState.ts";
import type { Cue } from "../mechanics.ts";
import type { EntityState, VisualDef } from "../views.ts";
import { markerSpot } from "../../ui/visitorHud.ts";
import { PANEL, ratingSound, starsText, starWave, starX, SURVEY_EVENT } from "./customerSurvey.ts";
import { faceGrowth, gossipLine, GROW_SECS, LINE_SECS, SINK_SECS, whereFrom } from "./gossipWall.ts";
import { HOP_HEIGHT, hopShape, lidAngle, MIMIC_FLAG, peekAngle, pickedPickups, pickedPoints } from "./mimicPickups.ts";
import { DOORBELL_RANGE, SLICES, slicesFor } from "./pizzaDelivery.ts";
import { receiptLength, sirenPhase } from "./taxDrone.ts";
import { attach, CUE_EVENT_OUTCOME, fromZUp, hoverPose, outcomes, OUTCOME_SHOT_DOWN, type SoundCall } from "./visitorKit.ts";
import { parsePoints, VISITOR_KINDS, VisitorVisuals, wantsVisitors, type VisitorFrameInput } from "./visitors.ts";

const ron = (id: string): string => readFileSync(new URL(`../../../../data/events/${id}.ron`, import.meta.url), "utf8");
const param = (text: string, key: string): number => {
  const m = new RegExp(`"${key}":\\s*([0-9.]+)`).exec(text) ?? new RegExp(`${key}:\\s*([0-9.]+)`).exec(text);
  assert.ok(m, `${key} in data`);
  return Number(m[1]);
};
const close = (a: number, b: number, eps = 1e-6): void => assert.ok(Math.abs(a - b) <= eps, `${a} vs ${b}`);
const v = (x: number, y: number, z: number) => ({ x, y, z });

function ent(over: Partial<EntityState>): EntityState {
  return { id: 1, kind: "drone", visual: 0, owner: 255, team: 255, position: v(0, 0, 0), velocity: v(0, 0, 0), rotation: q.identity(), state: [0, 0, 1, 100], radius: 0.9, ...over };
}

function mood(eventId: string, phase: string, over: Partial<MoodState> = {}): MoodState {
  return { ...emptyMood(), eventId, phase, phaseEndTick: 600, ...over };
}

function cue(event: string, byte: number, over: Partial<Cue> = {}): Cue {
  return { kind: CUE_EVENT_OUTCOME, source: 255, target: 255, position: v(0, 0, 0), value: (EVENT_IDS.indexOf(event as (typeof EVENT_IDS)[number]) << 8) | byte, tick: 0, ...over };
}

function frame(over: Partial<VisitorFrameInput> = {}): VisitorFrameInput {
  return {
    time: 10,
    dt: 1 / 60,
    nowTick: 600,
    camera: v(0, 0, 10),
    orientation: q.identity(),
    up: v(0, 1, 0),
    mood: emptyMood(),
    cues: [],
    entities: [],
    ships: [],
    own: 0,
    ownPosition: v(0, 0, 10),
    names: new Map([[1, "Rita"], [2, "Bo"]]),
    visuals: [{ shape: "crate", color: [1, 0.5, 0.1], scale: 1 } satisfies VisualDef],
    pickupPoints: undefined,
    quiet: false,
    ...over,
  };
}

/** Records sound calls through a fake engine. */
function engine(): { calls: string[]; loops: string[]; e: ConstructorParameters<typeof VisitorVisuals>[2] } {
  const calls: string[] = [];
  const loops: string[] = [];
  return {
    calls,
    loops,
    e: {
      play: (id) => {
        calls.push(id);
        return true;
      },
      loop: (id) => {
        loops.push(id);
        return { playing: true, setGain: () => {}, setPosition: () => {}, stop: () => {} };
      },
    },
  };
}

test("outcome cues decode the event index and byte (ls_sim drone::outcome: kind << 8 | byte)", () => {
  const out = outcomes([cue("tax_drone", 3, { target: 2 }), { kind: 5, source: 0, target: 0, position: v(0, 0, 0), value: 0x1fff, tick: 0 }, cue("mimic_pickups", OUTCOME_SHOT_DOWN)]);
  assert.equal(out.length, 2);
  assert.deepEqual([out[0]!.event, out[0]!.byte, out[0]!.target], ["tax_drone", 3, 2]);
  assert.deepEqual([out[1]!.event, out[1]!.byte], ["mimic_pickups", 0xf0]);
  // The cue kind is ls_sim's CueKind::EventOutcome.
  const behavior = readFileSync(new URL("../../../../crates/ls-sim/src/behavior.rs", import.meta.url), "utf8");
  assert.match(behavior, new RegExp(`EventOutcome\\s*=\\s*${CUE_EVENT_OUTCOME}\\b`));
});

test("the visitors' entity kinds and wasm names", () => {
  const lib = readFileSync(new URL("../../../../crates/ls-wasm/src/lib.rs", import.meta.url), "utf8");
  for (const k of VISITOR_KINDS) assert.match(lib, new RegExp(`=> "${k}"`), k);
  assert.equal(wantsVisitors({ search: "?visitors=off" }), false);
  assert.equal(wantsVisitors({ search: "" }), true);
  assert.deepEqual(parsePoints([1, 2, 3, 4, 5, 6, 7]), [v(1, 2, 3), v(4, 5, 6)]);
});

test("tax drone: the receipt grows a stretch per touch up to the fifth; the siren always has one dome lit", () => {
  assert.equal(param(ron("tax_drone"), "max_touches"), 5);
  for (let n = 0; n < 5; n++) assert.ok(receiptLength(n + 1) > receiptLength(n));
  assert.equal(receiptLength(6), receiptLength(5));
  for (let t = 0; t < 2; t += 0.01) {
    const s = sirenPhase(t);
    assert.ok(Math.max(s.red, s.blue) >= 0.56, `t ${t}`);
    assert.ok(Math.min(s.red, s.blue) >= 0.12 - 1e-9);
  }
  // They take turns.
  assert.ok(sirenPhase(0.125).red > 0.9 && sirenPhase(0.375).blue > 0.9);
});

test("pizza: slices for the hold progress byte, all twelve when full, the doorbell inside the hold radius", () => {
  assert.equal(slicesFor(0), 0);
  assert.equal(slicesFor(1), 1);
  assert.equal(slicesFor(128), 7);
  assert.equal(slicesFor(254), SLICES);
  assert.equal(slicesFor(255), SLICES);
  assert.equal(DOORBELL_RANGE, param(ron("pizza_delivery"), "hold_radius"));
});

test("survey: stars follow the sim's columns (1 on the leader's left = the panel's +X), wave, text and stings", () => {
  const data = ron("customer_survey");
  const half = param(data, "panel_half_width");
  // ls_sim customer_survey::star_at: five equal columns across 2 x half width, star 1 on the leader's left.
  // The panel faces the leader, so the leader's left is the panel's +X: column centres at +X.
  for (let k = 1; k <= 5; k++) {
    const leaderX = -starX(k); // leader-space x of the star
    const col = Math.floor(((leaderX / (2 * half)) + 0.5) * 5) + 1;
    assert.equal(col, k, `star ${k}`);
  }
  for (let k = 1; k <= 5; k++) for (let t = 0; t < 3; t += 0.05) assert.ok(starWave(k, t) >= 0.35 && starWave(k, t) <= 1);
  assert.equal(starsText(2), "★★☆☆☆");
  assert.deepEqual([1, 2, 3, 4, 5].map(ratingSound), ["visitor.ratingBad", "visitor.ratingBad", "visitor.ratingMeh", "visitor.ratingGood", "visitor.ratingGood"]);
  assert.ok(PANEL.every((c) => c > 0.6));
});

test("mimics: picked points from the mood params, pickups there hidden, hops squash on landing and lids snap on a bite", () => {
  const points = [v(0, 0, 0), v(10, 0, 0), v(20, 0, 0)];
  const m = mood("mimic_pickups", "active", { params: [2, 0, 255, 255, 255, 255, 255, 255] });
  const picked = pickedPoints({ mood: m, pickupPoints: points });
  assert.deepEqual(picked, [v(20, 0, 0), v(0, 0, 0)]);
  assert.deepEqual(pickedPoints({ mood: mood("tax_drone", "active", { params: [0] }), pickupPoints: points }), []);
  const pickups = [ent({ id: 5, kind: "weaponPickup", position: v(0.3, 0, 0) }), ent({ id: 6, kind: "weaponPickup", position: v(10, 0, 0) }), ent({ id: 7, kind: "scrapPile", position: v(20, 0, 0) })];
  assert.deepEqual(pickedPickups(pickups, picked).map((e) => e.id), [5]);
  // Landing (0, π) squashes, mid-air stretches and lifts.
  assert.ok(hopShape(0, 1).stretch < 0.8 && hopShape(0, 1).lift === 0);
  assert.ok(hopShape(Math.PI / 2, 1).lift > HOP_HEIGHT * 0.99);
  assert.equal(hopShape(1, 0).stretch, 1);
  assert.ok(lidAngle(MIMIC_FLAG.biting | MIMIC_FLAG.hunting, 0, 1) > 1);
  assert.ok(lidAngle(MIMIC_FLAG.hunting, 0.3, 1) > 0.35);
  assert.ok(lidAngle(0, 0.3, 1) < 0.15);
  let peeks = 0;
  for (let t = 0; t < 13; t += 0.05) if (peekAngle(t, 3) > 0.3) peeks++;
  assert.ok(peeks > 10 && peeks < 60, `${peeks}`);
});

test("gossip: the face grows over the announcement, sinks over the ending, gossips about each pilot in turn", () => {
  const data = ron("gossip_wall");
  assert.equal(SINK_SECS, param(data, "ending_secs"));
  assert.equal(GROW_SECS, 3);
  assert.equal(faceGrowth("announce", 180, 0), 0);
  assert.equal(faceGrowth("announce", 180, 180), 1);
  assert.equal(faceGrowth("active", 0, 100), 1);
  assert.equal(faceGrowth("ending", 60, 0), 1);
  assert.equal(faceGrowth("ending", 60, 60), 0);
  const up = v(0, 1, 0);
  assert.equal(whereFrom(v(0, -40, 0), v(0, 0, 0), q.identity(), up), "40 u below you");
  assert.equal(whereFrom(v(0, 0, 12), v(0, 0, 0), q.identity(), up), "right behind you, 12 u");
  assert.equal(whereFrom(v(9, 0, 0), v(0, 0, 0), q.identity(), up), "9 u to your right");
  const ships = [{ id: 0, alive: true, position: v(0, 0, 0) }, { id: 2, alive: true, position: v(0, 40, 0) }, { id: 1, alive: true, position: v(0, 0, -30) }];
  const name = (id: number) => ["Me", "Rita", "Bo"][id] ?? "?";
  const a = gossipLine(0, ships, 0, v(0, 0, 0), q.identity(), up, name)!;
  const b = gossipLine(LINE_SECS, ships, 0, v(0, 0, 0), q.identity(), up, name)!;
  assert.deepEqual([a.about, b.about], [1, 2]);
  assert.match(a.text, /Rita/);
  assert.match(a.text, /30 u ahead of you/);
  assert.match(b.text, /Bo/);
  assert.match(b.text, /40 u above you/);
  assert.equal(gossipLine(0, [ships[0]!], 0, null, q.identity(), up, name), null);
});

test("HUD markers sit on their ship on screen and pin to the edge toward it off screen", () => {
  const on = markerSpot({ x: 0, y: 0, w: 5 }, 800, 600);
  assert.deepEqual([on.x, on.y, on.edge], [400, 300, false]);
  const right = markerSpot({ x: 20, y: 0, w: 5 }, 800, 600);
  assert.equal(right.edge, true);
  close(right.x, 800 - 28);
  close(right.y, 300);
  // Behind the camera: mirrored, so a ship behind-left pins to the left.
  const behind = markerSpot({ x: 3, y: 0, w: -2 }, 800, 600);
  assert.ok(behind.edge && behind.x < 400);
});

test("hover and attached poses: drones fly level and bank, parts follow their parent", () => {
  const e = ent({ rotation: q.lookRotation(v(0, -1, -1), v(0, 1, 0)) });
  const p = hoverPose(e, 0, v(0, 1, 0));
  const f = q.forward(p.rotation);
  assert.ok(Math.abs(f.y) < 1e-6, "level");
  const child = attach({ position: v(1, 2, 3), rotation: q.fromAxisAngle(v(0, 1, 0), Math.PI / 2), scale: 2 }, v(1, 0, 0));
  close(child.position.x, 1);
  close(child.position.z, 1);
  // A Z-up model's +Z stands along the parent's up.
  const zu = q.rotate(fromZUp(q.identity()), v(0, 0, 1));
  close(zu.y, 1);
});

test("every visitor draws its meshes, hides mimicked pickups, and asks only for sounds that exist", () => {
  const sounds = new Set(VISITOR_SOUNDS.map((s) => s.id));
  const { calls, loops, e } = engine();
  const vis = new VisitorVisuals(null, true, e);
  const points = [v(5, 0, 0)];
  const entities: EntityState[] = [
    ent({ id: 1, state: [0, 2, 1, 40] }),
    ent({ id: 2, state: [1, 120, 1, 100], owner: 2, position: v(2, 1, 0) }),
    ent({ id: 3, state: [2, 0, 1, 100], radius: 1.25 }),
    ent({ id: 4, kind: "mimic", visual: 0, state: [0, 0, MIMIC_FLAG.hunting | MIMIC_FLAG.biting, 100], position: v(3, 0, 0) }),
    ent({ id: 5, kind: "gossipFace", state: [1, 0, 0, 0], radius: 3, position: v(0, 0, -8) }),
    ent({ id: 6, kind: "weaponPickup", position: v(5, 0, 0) }),
  ];
  const ships = [{ id: 0, alive: true, position: v(0, 0, 10) }, { id: 1, alive: true, position: v(0, 2, 0) }, { id: 2, alive: true, position: v(2, -1, 0) }];
  const cues = [cue("tax_drone", 2, { target: 1 }), cue("pizza_delivery", 1, { target: 2 }), cue(SURVEY_EVENT, 4, { source: 0, target: 1 }), cue("mimic_pickups", 2, { target: 0 }), cue("mimic_pickups", OUTCOME_SHOT_DOWN, { source: 2 })];
  // Two events cannot run at once in the sim; the visitors only read their own entities and cues.
  const f = vis.update(frame({ entities, ships, cues, mood: mood("mimic_pickups", "active", { params: [0, 255, 255, 255, 255, 255, 255, 255] }), pickupPoints: points.flatMap((p) => [p.x, p.y, p.z]) }));
  assert.deepEqual([...f.hidden], [6], "the mimicked pickup is hidden");
  const scene = emptyScene();
  vis.addToScene(scene);
  for (const mesh of ["tax_drone", "tax_eyes", "tax_tie", "tax_receipt", "visitor_siren", "pizza_drone", "pizza_box", "pizza_slice", "pizza_crust", "survey_panel", "survey_star", "survey_drone", "mimic_body", "mimic_lid", "mimic_tongue", "visitor_eye"] as const) {
    assert.ok((scene.opaque[mesh]?.length ?? 0) > 0, `${mesh} drawn`);
    assert.ok(MESH_KINDS.includes(mesh));
  }
  assert.equal(scene.opaque.survey_star?.length, 10, "the live panel and the rated one tumbling away");
  assert.equal(scene.opaque.pizza_slice?.length, slicesFor(120));
  assert.ok(f.lights.length >= 3, "siren and box lights");
  // Toasts for the cues; the own bite is about the own pilot.
  assert.ok(f.hud.toasts.length >= 5);
  assert.ok(f.hud.toasts.some((t) => t.own && /bit you/.test(t.text)));
  assert.ok(f.hud.toasts.some((t) => t.stars === 4));
  for (const id of [...calls, ...loops]) assert.ok(sounds.has(id), `sound ${id} exists`);
  assert.ok(loops.includes("visitor.siren") && loops.includes("visitor.rotor") && loops.includes("visitor.snarl"));
  // The gossip face (its own event) draws with brows, eyes, lids and jaw; markers for every other pilot.
  const g = vis.update(frame({ entities: [entities[4]!], ships, mood: mood("gossip_wall", "active") }));
  const gs = emptyScene();
  vis.addToScene(gs);
  for (const mesh of ["gossip_face", "gossip_brow", "gossip_lid", "gossip_jaw", "visitor_eye"] as const) assert.ok((gs.opaque[mesh]?.length ?? 0) > 0, mesh);
  assert.deepEqual(g.hud.markers.map((m) => m.id).sort(), [1, 2]);
  assert.ok(g.hud.bubble && g.hud.bubble.text.length > 0);
  // Quiet (results, lobby): no loops, no HUD.
  loops.length = 0;
  const quiet = vis.update(frame({ entities, ships, quiet: true }));
  assert.equal(loops.length, 0);
  assert.equal(quiet.hud.markers.length, 0);
  // Off: nothing drawn, nothing skipped.
  const off = new VisitorVisuals(null, false, null);
  assert.equal(off.skipKinds, undefined);
  off.update(frame({ entities, ships }));
  const os = emptyScene();
  off.addToScene(os);
  assert.equal(Object.values(os.opaque).reduce((n, l) => n + (l?.length ?? 0), 0), 0);
});

test("a shot-down tax drone throws off its receipt, tie and sirens for a moment", () => {
  const vis = new VisitorVisuals(null, true, null);
  const ships = [{ id: 1, alive: true, position: v(0, 2, 0) }];
  vis.update(frame({ entities: [ent({ id: 1, state: [0, 4, 1, 10] })], ships }));
  vis.update(frame({ time: 10.02, entities: [], ships, cues: [cue("tax_drone", OUTCOME_SHOT_DOWN, { source: 1 })] }));
  const s = emptyScene();
  vis.addToScene(s);
  assert.ok((s.opaque.tax_receipt?.length ?? 0) > 1 && (s.opaque.tax_tie?.length ?? 0) === 1 && (s.opaque.visitor_siren?.length ?? 0) === 2);
  vis.update(frame({ time: 13, entities: [], ships }));
  const later = emptyScene();
  vis.addToScene(later);
  assert.equal(later.opaque.tax_receipt?.length ?? 0, 0, "gone after a few seconds");
});

test("sound definitions: unique ids, the loops loop", () => {
  const ids = VISITOR_SOUNDS.map((s) => s.id);
  assert.equal(new Set(ids).size, ids.length);
  for (const id of ["visitor.siren", "visitor.rotor", "visitor.snarl", "visitor.mumble"]) assert.equal(VISITOR_SOUNDS.find((s) => s.id === id)?.loop, true, id);
  const unused: SoundCall[] = [];
  assert.equal(unused.length, 0);
});
