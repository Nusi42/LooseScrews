// Tests of the comedy event visuals (M4 C1): the eventFx parsing and the
// parameter decoders, every event's timeline, the hub (one telegraph per
// run, modules linger and end, output merging), the shot tracker and the
// pixel look.

import { test } from "node:test";
import assert from "node:assert/strict";
import { emptyMood } from "../modeState.ts";
import type { ProjectileState, ShipState } from "../views.ts";
import { PROJ_CHICKEN } from "../views.ts";
import { CUE_HIT } from "../vfx.ts";
import { inflation, stepPupil } from "./bigHeadDay.ts";
import { screenAngle, zzzAmount } from "./boredLevel.ts";
import { CAKE_MAX_RADIUS, CAKE_RECEDE_SECS, cakeWave } from "./cake.ts";
import { mugPose } from "./coffeeBreak.ts";
import { clockOf, elasticOut, fxFromMood, NO_EVENT_FX, parseEventFx, unpackDir, unpackPos, type EventFx } from "./fx.ts";
import { bannerModel, EVENT_VISUALS, EventVisuals, runKey } from "./index.ts";
import { eventInfo, hasOwnIcon } from "./info.ts";
import { bump, crossed, popLook } from "./levelHiccups.ts";
import { boxPose, MOVE_SECS } from "./movingDay.ts";
import { chickenLook, isChicken } from "./rubberChickens.ts";
import { ShotTracker } from "./shots.ts";
import { emptyOutput, type EventInput } from "./types.ts";
import { blendCamera, glitch, sideAmount, sideCamera, SIDE_DISTANCE, CUT_MARGIN } from "./wrongGenre.ts";
import { nearestPaletteColor, packPixel, PIXEL_PALETTE, pixelActive, pixelLookAt, quarterCell } from "../../renderer/pixelPost.ts";
import { packCake } from "../../renderer/renderer.ts";
import * as q from "../../math/quat.ts";

const near = (a: number, b: number, eps = 1e-4): boolean => Math.abs(a - b) <= eps;

test("eventFx JSON parses (and broken input means no event)", () => {
  const fx = parseEventFx(
    JSON.stringify({ event: "level_hiccups", phase: "active", now: 700, activeTick: 600, phaseEndTick: 1500, wellScale: 1, hitScale: 1, chicken: false, craterScale: 1, scavengeScale: 1, up: null, hitPlane: null, bored: null, hiccups: { ticks: [780, 950], liftTicks: 15, up: [0, 1, 0] }, cake: null }),
  );
  assert.equal(fx.event, "level_hiccups");
  assert.equal(fx.phase, "active");
  assert.deepEqual(fx.hiccups?.ticks, [780, 950]);
  assert.equal(fx.hiccups?.liftTicks, 15);
  assert.deepEqual(parseEventFx("{broken"), NO_EVENT_FX);
  assert.deepEqual(parseEventFx(undefined), NO_EVENT_FX);
  const cake = parseEventFx(JSON.stringify({ event: "everything_is_cake", phase: "announce", activeTick: 900, cake: { origin: [1, 2.5, -3] }, chicken: true }));
  assert.deepEqual(cake.cake?.origin, { x: 1, y: 2.5, z: -3 });
  assert.equal(cake.chicken, true);
});

test("parameter bytes decode like ls_sim::events::params", () => {
  // pack_dir(+X) = [127, 0, 0]; pack_dir(-Y) = [0, 129 (-127), 0]; zeros = +Y.
  assert.deepEqual(unpackDir(127, 0, 0), { x: 1, y: 0, z: 0 });
  assert.deepEqual(unpackDir(0, 129, 0), { x: 0, y: -1, z: 0 });
  assert.deepEqual(unpackDir(0, 0, 0), { x: 0, y: 1, z: 0 });
  // pack_pos: i16 little-endian in 1/64 u: 10 u = 640 = [128, 2]; -3 u = -192 = [64, 255].
  assert.deepEqual(unpackPos([128, 2, 32, 0, 64, 255]), { x: 10, y: 0.5, z: -3 });
  const mood = { ...emptyMood(), eventId: "everything_is_cake", phase: "active", activeTick: 100, phaseEndTick: 1300, params: [128, 2, 32, 0, 64, 255, 0, 0] };
  const fx = fxFromMood(mood, 200);
  assert.deepEqual(fx.cake?.origin, { x: 10, y: 0.5, z: -3 });
  assert.equal(fxFromMood({ ...emptyMood(), eventId: "rubber_chickens", phase: "active" }, 1).chicken, true);
  assert.equal(fxFromMood({ ...emptyMood(), eventId: "rubber_chickens", phase: "announce" }, 1).chicken, false);
});

test("the clock runs from -3 s (announced) through the active phase", () => {
  const announce: EventFx = { ...NO_EVENT_FX, event: "wrong_genre", phase: "announce", activeTick: 1000, phaseEndTick: 1000 };
  assert.ok(near(clockOf(announce, 910).t, -1.5));
  // Interpolated ticks past the snapshot's announce end stay at 0 until the core says active.
  assert.equal(clockOf(announce, 1010).t, 0);
  const active: EventFx = { ...announce, phase: "active", phaseEndTick: 1600 };
  assert.ok(near(clockOf(active, 1300).t, 5));
  assert.equal(clockOf(active, 1300).activeSecs, 10);
});

test("banner: announce counts down 3 s, active counts the event down; targets named", () => {
  const fx: EventFx = { ...NO_EVENT_FX, event: "tax_drone", phase: "announce", activeTick: 1180, phaseEndTick: 1180 };
  const b = bannerModel(fx, 1000, "Rivet");
  assert.ok(b && b.stage === "announce" && near(b.secsLeft, 3) && b.total === 3 && b.target === "Rivet");
  const a = bannerModel({ ...fx, phase: "active", activeTick: 1180, phaseEndTick: 1180 + 25 * 60 }, 1180 + 600, "");
  assert.ok(a && a.stage === "active" && near(a.secsLeft, 15) && near(a.total, 25));
  assert.equal(bannerModel(NO_EVENT_FX, 1, ""), null);
  // Every event has a name; C1's eight their own icons.
  for (const id of Object.keys(EVENT_VISUALS)) assert.ok(hasOwnIcon(id), id);
  assert.equal(eventInfo("duck_parade").name, "Duck parade");
  assert.ok(eventInfo("something_new").icon.includes("<path"));
});

test("Big head day: three puffs while announced, a pop to full size, a burst at the end", () => {
  assert.ok(inflation(-3, null) < 0.2);
  const steps = [-2.0, -1.5, -0.8, -0.1].map((t) => inflation(t, null));
  for (let i = 1; i < steps.length; i++) assert.ok((steps[i] ?? 0) > (steps[i - 1] ?? 0), `inflates: ${steps}`);
  assert.ok((steps[3] ?? 1) < 0.7);
  assert.ok(near(inflation(2, null), 1, 1e-3));
  assert.ok(Math.max(...[0.1, 0.2, 0.3].map((t) => inflation(t, null))) > 1, "overshoots when it pops to size");
  assert.ok(inflation(0, 0.05) > 1, "swells as it bursts");
  assert.equal(inflation(0, 0.1), 0);
  // Googly pupils stay inside the eye and settle on their target.
  const p = { o: { x: 0, y: 0 }, v: { x: 0, y: 0 } };
  stepPupil(p, { x: 0, y: 0 }, { x: 900, y: 0 }, 0.05);
  assert.ok(Math.hypot(p.o.x, p.o.y) <= 0.1 + 1e-9);
  for (let i = 0; i < 300; i++) stepPupil(p, { x: 0.05, y: 0 }, { x: 0, y: 0 }, 1 / 60);
  assert.ok(near(p.o.x, 0.05, 2e-3) && near(p.o.y, 0, 2e-3));
});

test("Rubber chickens: chickens by the state bit, sized by shape, the Charge an egg", () => {
  const shot = (state: number): ProjectileState => ({ kind: 1, owner: 2, position: { x: 0, y: 0, z: 0 }, velocity: { x: 1, y: 0, z: 0 }, visual: 0, state });
  assert.ok(isChicken(shot(PROJ_CHICKEN | 3)));
  assert.ok(!isChicken(shot(3)));
  const bolt = chickenLook({ shape: "bolt", color: [1, 0, 0], scale: 1 });
  const rocket = chickenLook({ shape: "rocket", color: [1, 0, 0], scale: 1 });
  const charge = chickenLook({ shape: "charge", color: [1, 0, 0], scale: 2 });
  assert.equal(bolt.kind, "chicken");
  assert.ok(rocket.scale > bolt.scale);
  assert.equal(charge.kind, "giant_egg");
  assert.ok(charge.scale > 2);
});

test("Level hiccups: the bump lands on the hiccup tick and settles; HIC! pops and fades", () => {
  assert.deepEqual(crossed([780, 950, 1100], 779.5, 951), [780, 950]);
  assert.deepEqual(crossed([780], 780, 790), []);
  assert.equal(bump(-0.1).lift, 0);
  assert.ok(Math.abs(bump(0.05).lift) > 0.05);
  assert.equal(bump(0.7).lift, 0);
  assert.ok(popLook(0.3).alpha === 1 && popLook(0.3).scale > 0.9);
  assert.equal(popLook(2).alpha, 0);
});

test("Bored level: Z's while announced; the down arrow points down the screen", () => {
  assert.equal(zzzAmount(-3.2), 0);
  assert.equal(zzzAmount(-1.5), 1);
  assert.equal(zzzAmount(0.5), 0);
  const cam = { position: { x: 0, y: 0, z: 0 }, orientation: q.identity(), fovY: 1, near: 0.1, far: 100 };
  assert.ok(near(screenAngle(cam, { x: 0, y: -1, z: 0 }).angle, 0));
  // Down to the left of the screen: a quarter turn clockwise.
  assert.ok(near(screenAngle(cam, { x: -1, y: 0, z: 0 }).angle, Math.PI / 2));
  assert.ok(screenAngle(cam, { x: 0, y: 0, z: -1 }).inPlane < 1e-6);
});

test("Gravity coffee break: the mug drops in while announced, sits on the core, lifts off after", () => {
  const w = { center: { x: 0, y: 10, z: 0 }, core: 2, range: 30, dynamic: false };
  const up = { x: 0, y: 1, z: 0 };
  assert.equal(mugPose(w, up, -2.9, null, 1), null);
  const falling = mugPose(w, up, -2.4, null, 1);
  const resting = mugPose(w, up, -0.5, null, 1);
  assert.ok(falling && resting && falling.position.y > resting.position.y + 1);
  // Resting on top of the core stone.
  assert.ok(resting.position.y - 10 > 2 && resting.position.y - 10 < 2 + resting.scale);
  const leaving = mugPose(w, up, 0, 0.8, 1);
  assert.ok(leaving && leaving.position.y > resting.position.y && leaving.scale < resting.scale);
  assert.equal(mugPose(w, up, 0, 2, 1), null);
});

test("Moving day: boxes pop up, shut, hop along an arc to the new point and unpack", () => {
  const from = { x: 0, y: 0, z: 0 };
  const to = { x: 10, y: 0, z: 0 };
  const up = { x: 0, y: 1, z: 0 };
  assert.equal(boxPose(from, to, up, -2.5, 1), null);
  const open = boxPose(from, to, up, -1.7, 1);
  const shut = boxPose(from, to, up, -0.4, 1);
  assert.ok(open && shut && open.flaps > 1 && shut.flaps < 0.05 && shut.tape > 0.5);
  const mid = boxPose(from, to, up, MOVE_SECS / 2, 1);
  assert.ok(mid && near(mid.position.x, 5, 0.01) && mid.position.y > 2.5, "arcs up over the middle");
  const landed = boxPose(from, to, up, MOVE_SECS + 0.05, 1);
  assert.ok(landed && near(landed.position.x, 10) && near(landed.position.y, 0));
  const gone = boxPose(from, to, up, MOVE_SECS + 0.69, 1);
  assert.ok(gone && gone.scale < 0.1 && gone.flaps > 2);
});

test("Everything is cake: the wave grows from the origin, covers the level, recedes after the end", () => {
  const o = { x: 1, y: 2, z: 3 };
  const up = { x: 0, y: 1, z: 0 };
  assert.equal(cakeWave(o, up, -1, null), null);
  const a = cakeWave(o, up, 1, null);
  const b = cakeWave(o, up, 3, null);
  assert.ok(a && b && b.radius > a.radius && a.amount === 1);
  assert.equal(cakeWave(o, up, 19, null)?.radius, CAKE_MAX_RADIUS);
  const back = cakeWave(o, up, 20, CAKE_RECEDE_SECS / 2);
  assert.ok(back && back.radius < CAKE_MAX_RADIUS);
  assert.equal(cakeWave(o, up, 20, CAKE_RECEDE_SECS), null);
  // Packed into the terrain params: origin, radius, up, amount; off = zeros.
  assert.deepEqual([...packCake(b)], [1, 2, 3, b.radius, 0, 1, 0, 1].map((x) => Math.fround(x)));
  assert.deepEqual([...packCake(null)], [0, 0, 0, 0, 0, 0, 0, 0]);
});

test("Wrong genre: glitches while announced, swings to the side view, cuts the rock in front of the plane", () => {
  assert.equal(sideAmount(-1, null), 0);
  assert.equal(sideAmount(5, null), 1);
  assert.equal(sideAmount(0, 2), 0);
  assert.ok(sideAmount(0, 0.2) > 0 && sideAmount(0, 0.2) < 1);
  assert.equal(glitch(-2), 0);
  assert.ok([-1.3, -1.1, -0.9, -0.7, -0.5, -0.3, -0.1].some((t) => glitch(t) > 0));
  const side = sideCamera({ x: 4, y: 5, z: 10 }, { x: 0, y: 1, z: 0 });
  assert.deepEqual(side.position, { x: 4, y: 5, z: 10 + SIDE_DISTANCE });
  const f = q.forward(side.orientation);
  assert.ok(near(f.z, -1, 1e-6));
  // +x is screen right, the level's up is screen up.
  assert.ok(near(q.right(side.orientation).x, 1, 1e-6) && near(q.up(side.orientation).y, 1, 1e-6));
  const game = { position: { x: 4, y: 5, z: 10 }, orientation: q.identity(), fovY: 1.2, near: 0.05, far: 600 };
  const full = blendCamera(game, side, 1, 10);
  assert.ok(near(full.near, SIDE_DISTANCE - CUT_MARGIN, 1e-4));
  assert.equal(blendCamera(game, side, 0, 10).near, 0.05);
});

test("pixel look: steps in whole pixels to a quarter resolution in 16 colours", () => {
  assert.equal(PIXEL_PALETTE.length, 16);
  assert.equal(quarterCell(1080), 4);
  assert.equal(quarterCell(2160), 8);
  assert.equal(quarterCell(400), 2);
  assert.equal(pixelActive(pixelLookAt(0, 4)), false);
  const cells = [0.1, 0.3, 0.6, 1].map((a) => pixelLookAt(a, 4).cell);
  assert.deepEqual(cells, [1, 2, 3, 4]);
  assert.deepEqual(packPixel(undefined), [1, 0, 0, 0]);
  assert.deepEqual(packPixel({ cell: 4, palette: 1, dither: 1 }), [4, 1, 1, 1]);
  // Blues stay blue, reds red, greys grey.
  assert.deepEqual(nearestPaletteColor([0.25, 0.65, 1]), PIXEL_PALETTE[12]);
  assert.deepEqual(nearestPaletteColor([1, 0.05, 0.3]), PIXEL_PALETTE[8]);
  assert.deepEqual(nearestPaletteColor([0.02, 0.02, 0.02]), PIXEL_PALETTE[0]);
});

test("the shot tracker follows shots by their predicted position, reports new and gone ones", () => {
  const t = new ShotTracker();
  const s = (x: number, owner = 1): ProjectileState => ({ kind: 1, owner, position: { x, y: 0, z: 0 }, velocity: { x: 30, y: 0, z: 0 }, visual: 0, state: 0 });
  const a = t.update([s(0), s(100, 2)], 0, 0);
  assert.equal(a.born.length, 2);
  const b = t.update([s(0.5), s(100.5, 2)], 1 / 60, 1 / 60);
  assert.equal(b.born.length, 0);
  assert.deepEqual(b.ids, a.ids);
  const c = t.update([s(1)], 1 / 60, 2 / 60);
  assert.equal(c.gone.length, 1);
  assert.equal(c.gone[0]?.owner, 2);
});

function input(fx: EventFx, time: number, extra: Partial<EventInput> = {}): EventInput {
  return {
    time,
    dt: 1 / 60,
    tick: fx.now,
    fx,
    mood: { ...emptyMood(), eventId: fx.event, phase: fx.phase, target: 3 },
    camera: { position: { x: 0, y: 0, z: 0 }, orientation: q.identity(), fovY: 1.2, near: 0.05, far: 600 },
    own: null,
    ships: [],
    shots: [],
    entities: [],
    visuals: [],
    cues: [],
    wells: [],
    up: { x: 0, y: 1, z: 0 },
    colorOf: () => [1, 0.5, 0],
    quiet: false,
    screenHeight: 1080,
    ...extra,
  };
}

test("the hub: one telegraph per run, modules linger after the end and stop", () => {
  const said: string[] = [];
  const played: string[] = [];
  const hub = new EventVisuals({
    particles: { spawn: () => {}, immediate: () => {} },
    sound: { play: (id) => (played.push(id), true), loop: () => null },
    hud: null,
    announcer: { say: (t, v) => said.push(`${t}:${v?.["target"] ?? ""}`) },
    enabled: true,
  });
  const ann: EventFx = { ...NO_EVENT_FX, event: "big_head_day", phase: "announce", activeTick: 600, phaseEndTick: 600, now: 450 };
  const ship: ShipState = { id: 7, team: 0, alive: true, boosting: false, position: { x: 0, y: 0, z: -8 }, orientation: q.identity(), shipType: 1, radius: 0.5, gadgetActive: false };
  for (let k = 0; k < 30; k++) hub.update(input({ ...ann, now: 450 + k }, k / 60, { tick: 450 + k, ships: [ship] }), (id) => (id === 3 ? "Rivet" : ""));
  assert.deepEqual(said, ["event.big_head_day:Rivet"], "the line once, with the target's name");
  assert.equal(runKey(ann), "big_head_day@600");
  const act: EventFx = { ...ann, phase: "active", phaseEndTick: 600 + 900, hitScale: 3 };
  let out = hub.update(input({ ...act, now: 700 }, 2, { tick: 700, ships: [ship], cues: [{ kind: CUE_HIT, source: 1, target: 7, position: ship.position, value: 5, tick: 700 }] }));
  assert.ok(out.translucent.some(([k]) => k === "balloon") && out.opaque.some(([k]) => k === "balloon_face"));
  assert.equal(out.opaque.filter(([k]) => k === "pupil").length, 2);
  assert.ok(played.includes("bighead.honk"));
  // The event ends: the balloons burst (a pop), then the module stops.
  out = hub.update(input({ ...NO_EVENT_FX, now: 1501 }, 20, { tick: 1501, ships: [ship] }));
  assert.ok(played.includes("bighead.pop"));
  assert.ok(out.hud.pops.some((p) => p.text === "POP!"));
  hub.update(input({ ...NO_EVENT_FX, now: 1600 }, 22, { tick: 1600 }));
  out = hub.update(input({ ...NO_EVENT_FX, now: 1601 }, 22.1, { tick: 1601 }));
  assert.equal(out.opaque.length + out.translucent.length + out.hud.pops.length, 0);
  // A late join into a running event: no telegraph.
  hub.update(input({ ...NO_EVENT_FX, event: "moving_day", phase: "active", activeTick: 3000, phaseEndTick: 3120, now: 3010 }, 30, { tick: 3010 }));
  assert.equal(said.length, 1);
});

test("the hub hides chickens from the normal shot visuals and keeps the guided one hidden", () => {
  const hub = new EventVisuals({ particles: null, sound: null, hud: null, announcer: null, enabled: true });
  const chick: ProjectileState = { kind: 1, owner: 2, position: { x: 0, y: 0, z: -5 }, velocity: { x: 0, y: 0, z: -30 }, visual: 0, state: PROJ_CHICKEN };
  const plain: ProjectileState = { ...chick, state: 0 };
  const fx: EventFx = { ...NO_EVENT_FX, event: "rubber_chickens", phase: "active", activeTick: 100, phaseEndTick: 820, chicken: true, now: 200 };
  const out = hub.update(input(fx, 1, { tick: 200, shots: [chick, plain], visuals: [{ shape: "bolt", color: [1, 0, 0], scale: 1 }] }));
  assert.ok(out.opaque.some(([k]) => k === "chicken"));
  const hide = hub.hideShot((p) => p === plain);
  assert.ok(hide && hide(chick) && hide(plain));
  // Without the event nothing is hidden by it.
  const none = new EventVisuals({ particles: null, sound: null, hud: null, announcer: null, enabled: true });
  none.update(input(NO_EVENT_FX, 0));
  assert.equal(none.hideShot(undefined), undefined);
  assert.equal(emptyOutput().pixel, null);
});

test("elastic pop overshoots and settles", () => {
  assert.equal(elasticOut(0), 0);
  assert.equal(elasticOut(1), 1);
  assert.ok(Math.max(...[0.1, 0.15, 0.2, 0.25].map(elasticOut)) > 1);
});

test("every event module starts each looping sound once per run and stops it (announce, active, ending, over)", () => {
  for (const id of Object.keys(EVENT_VISUALS)) {
    const starts: string[] = [];
    const live = new Set<{ id: string; stopped: boolean }>();
    const hub = new EventVisuals({
      particles: { spawn: () => {}, immediate: () => {} },
      sound: {
        play: () => true,
        loop: (sid) => {
          starts.push(sid);
          const l = { id: sid, stopped: false };
          live.add(l);
          return {
            get playing() {
              return !l.stopped;
            },
            setGain: () => {},
            setRate: () => {},
            setPosition: () => {},
            stop: () => {
              l.stopped = true;
              live.delete(l);
            },
          };
        },
      },
      hud: null,
      announcer: null,
      enabled: true,
    });
    const wells = [{ center: { x: 0, y: 0, z: -20 }, core: 2, range: 30, dynamic: false }];
    const own = { id: 1, team: 0, position: { x: 0, y: 0, z: 0 }, orientation: q.identity(), velocity: { x: 0, y: 0, z: 0 }, shipType: 1, radius: 0.5 };
    const base: EventFx = { ...NO_EVENT_FX, event: id, activeTick: 180 };
    let time = 0;
    for (let tick = 0; tick < 1200; tick++) {
      time += 1 / 60;
      const fx: EventFx =
        tick < 180
          ? { ...base, phase: "announce", phaseEndTick: 180, now: tick }
          : tick < 780
            ? { ...base, phase: "active", phaseEndTick: 780, now: tick }
            : tick < 840
              ? { ...base, phase: "ending", phaseEndTick: 840, now: tick }
              : { ...NO_EVENT_FX, now: tick };
      hub.update(input(fx, time, { tick, wells, own }));
    }
    const counts = new Map<string, number>();
    for (const s of starts) counts.set(s, (counts.get(s) ?? 0) + 1);
    for (const [sid, n] of counts) assert.equal(n, 1, `${id}: ${sid} started ${n} times`);
    assert.equal(live.size, 0, `${id}: loops still playing after the event: ${[...live].map((l) => l.id).join(", ")}`);
    hub.dispose();
  }
});

test("the hub runs C2's visitors: hidden pickups and lights in its output, props, HUD, skip kinds, reset and dispose", () => {
  const calls: string[] = [];
  let seen: { camera: unknown; own: number; ships: number } | null = null;
  const light = { position: { x: 1, y: 2, z: 3 }, color: [1, 0, 0] as [number, number, number], intensity: 1, range: 5 };
  const hud = { markers: [], bubble: null, survey: false, pizza: null, toasts: [] };
  const stub = {
    skipKinds: new Set(["drone", "mimic", "gossipFace"]),
    update: (f: { camera: unknown; own: number; ships: readonly unknown[] }) => {
      calls.push("update");
      seen = { camera: f.camera, own: f.own, ships: f.ships.length };
      return { hidden: new Set([42]), lights: [light], hud };
    },
    updateHud: (m: unknown) => void calls.push(m === hud ? "hud" : "hud?"),
    addToScene: () => void calls.push("scene"),
    reset: () => void calls.push("reset"),
    dispose: () => void calls.push("dispose"),
  };
  const hub = new EventVisuals({ particles: null, sound: null, hud: null, announcer: null, enabled: true, visitors: stub as unknown as import("./visitors.ts").VisitorVisuals });
  assert.deepEqual([...(hub.skipKinds ?? [])].sort(), ["drone", "gossipFace", "mimic"]);
  // Without the visitors' extras (the gallery's synthetic runs) the hub leaves them alone.
  hub.update(input(NO_EVENT_FX, 0));
  assert.deepEqual(calls, []);
  const extras = { own: 3, ownPosition: null, ships: [{ id: 3, alive: true, position: { x: 0, y: 0, z: 0 } }], names: new Map([[3, "Rivet"]]), pickupPoints: undefined };
  const out = hub.update(input(NO_EVENT_FX, 1, { visitors: extras }));
  assert.ok(out.hideEntities.has(42));
  assert.ok(out.lights.includes(light));
  assert.deepEqual(seen, { camera: { x: 0, y: 0, z: 0 }, own: 3, ships: 1 });
  const lights: typeof out.lights = [];
  hub.addToScene({ opaque: {}, translucent: {}, refract: {} } as never, lights, out);
  assert.ok(lights.includes(light));
  hub.updateHud(null, 800, 600, 1);
  hub.reset();
  hub.dispose();
  assert.deepEqual(calls, ["update", "scene", "hud", "reset", "dispose"]);
});
