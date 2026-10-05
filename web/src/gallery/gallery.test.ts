import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import { MeshBuilder, NO_TEAM } from "../renderer/mesh.ts";
import { appendBox } from "../renderer/primitives.ts";
import { durationsFromTicks, TimingWindow } from "../renderer/gpuTimer.ts";
import { powerPreferenceFrom } from "../renderer/gpu.ts";
import { formatDebugLines, formatGpuLine, type DebugInfo } from "../ui/debugOverlay.ts";
import { pickPose, pickSunPose, sceneContent, wantsGallery, LINEUP_RADII } from "./gallery.ts";
import {
  pickCloseup,
  pickLongest,
  pickVista,
  probeAround,
  raycast,
  rayMesh,
  sphereDirections,
  type Probe,
} from "./raycast.ts";
import { BUDGET_TOLERANCE, checkBudget, FRAME_BUDGET_MS, PASS_BUDGETS_MS, sceneById, SCENES } from "./scenes.ts";

// ----- GPU timer --------------------------------------------------------------------------

test("timestamps become milliseconds per label, summed over passes that share one", () => {
  const ticks = [1_000_000n, 3_000_000n, 3_000_000n, 3_500_000n, 4_000_000n, 4_250_000n];
  const d = durationsFromTicks(ticks, ["terrain", "post", "post"]);
  assert.equal(d.get("terrain"), 2);
  assert.equal(d.get("post"), 0.75);
});

test("a pass whose end precedes its begin counts as zero", () => {
  const d = durationsFromTicks([5n, 2n], ["objects"]);
  assert.equal(d.get("objects"), 0);
});

test("the timing window averages per pass, orders the known passes and keeps the worst frame", () => {
  const w = new TimingWindow(3);
  assert.equal(w.summary(), null);
  w.push(new Map([["post", 1], ["terrain", 2]]));
  w.push(new Map([["post", 1], ["terrain", 4]]));
  const s = w.summary();
  assert.ok(s);
  assert.deepEqual(s.passes, [["terrain", 3], ["post", 1]]);
  assert.equal(s.total, 4);
  assert.equal(s.worst, 5);
  assert.equal(s.frames, 2);
  // The window slides: the oldest frames drop out.
  w.push(new Map([["terrain", 1]]));
  w.push(new Map([["terrain", 1]]));
  w.push(new Map([["terrain", 1]]));
  assert.equal(w.summary()?.total, 1);
  assert.equal(w.length, 3);
});

test("the debug overlay shows the GPU line only when the renderer reports one", () => {
  const base: DebugInfo = {
    fps: 60,
    worstFrameMs: 17,
    mode: "practice",
    transport: null,
    droppedSends: null,
    stats: {
      pingMs: null, synced: true, corrected: 0, snapped: 0, accepted: 0, lastError: 0,
      shotsPredicted: 0, shotsConfirmed: 0, shotsDropped: 0, shotsUnpredicted: 0,
    },
    chunks: { meshedTotal: 0, pending: 0, shown: 0, drawn: 0, triangles: 0 },
    shareCode: "LS",
    speed: 0,
    gpuError: null,
  };
  assert.ok(!formatDebugLines(base).some((l) => l.startsWith("gpu")));
  const lines = formatDebugLines({
    ...base,
    gpu: { passes: [["terrain", 1.234], ["post", 0.5]], total: 1.734, worst: 2.5, frames: 60 },
  });
  assert.equal(lines[1], "gpu 1.73 ms (worst 2.50) · terrain 1.23 · post 0.50");
  assert.equal(formatGpuLine(null, "high"), "gpu: no timestamp-query · high");
});

test("?gpu=low-power asks for the integrated adapter", () => {
  assert.equal(powerPreferenceFrom("?gpu=low-power"), "low-power");
  assert.equal(powerPreferenceFrom("?gallery=fight"), "high-performance");
  assert.equal(powerPreferenceFrom(""), "high-performance");
});

// ----- budget -----------------------------------------------------------------------------

test("the budget check fails a pass more than 25 % over its share, and the frame over 8 ms", () => {
  const terrain = PASS_BUDGETS_MS["terrain"] ?? 0;
  assert.ok(checkBudget([["terrain", terrain * BUDGET_TOLERANCE * 0.99]]).ok);
  const over = checkBudget([["terrain", terrain * BUDGET_TOLERANCE * 1.01]]);
  assert.equal(over.ok, false);
  assert.match(over.failures[0] ?? "", /^terrain/);
  // Scaled for a slower GPU, the same time passes.
  assert.ok(checkBudget([["terrain", terrain * BUDGET_TOLERANCE * 1.01]], 1.75).ok);
  // Passes without a share count only toward the frame.
  assert.ok(checkBudget([["scene", 5]]).ok);
  const frame = checkBudget([["scene", FRAME_BUDGET_MS * BUDGET_TOLERANCE + 0.1]]);
  assert.equal(frame.ok, false);
  assert.match(frame.failures[0] ?? "", /^frame/);
});

test("the gallery has the track's scenes with unique ids", () => {
  const ids = SCENES.map((s) => s.id);
  assert.equal(new Set(ids).size, ids.length);
  for (const id of ["cave-closeup", "cave-tunnel", "open-sky", "mine-shaft", "ship-lineup", "fight"]) {
    assert.ok(sceneById(id), id);
  }
  assert.equal(sceneById("nope"), undefined);
  assert.ok(wantsGallery({ search: "?gallery=fight" }));
  assert.ok(!wantsGallery({ search: "?transport=ws" }));
});

// ----- ray casts and poses ------------------------------------------------------------------

/** A closed box room from -h to +h (walls are boxes 1 u thick around it). */
function room(h: number) {
  const b = new MeshBuilder();
  const t = () => NO_TEAM;
  appendBox(b, -h - 0.5, 0, 0, 0.5, h + 1, h + 1, t); // -x wall
  appendBox(b, h + 0.5, 0, 0, 0.5, h + 1, h + 1, t); // +x wall
  appendBox(b, 0, -h - 0.5, 0, h + 1, 0.5, h + 1, t);
  appendBox(b, 0, h + 0.5, 0, h + 1, 0.5, h + 1, t);
  appendBox(b, 0, 0, -h - 0.5, h + 1, h + 1, 0.5, t);
  appendBox(b, 0, 0, h + 0.5, h + 1, h + 1, 0.5, t);
  return rayMesh(b.build());
}

test("a ray hits the nearest wall with the normal facing back", () => {
  const m = room(10);
  const hit = raycast([m], { x: 0, y: 0, z: 0 }, { x: 1, y: 0, z: 0 }, 100);
  assert.ok(hit);
  assert.ok(Math.abs(hit.t - 10) < 1e-6, String(hit.t));
  assert.ok(Math.abs(hit.normal.x + 1) < 1e-6);
  assert.equal(raycast([m], { x: 0, y: 0, z: 0 }, { x: 1, y: 0, z: 0 }, 5), null, "out of range");
});

test("sphere directions are unit length, spread over both hemispheres and deterministic", () => {
  const d = sphereDirections(64);
  assert.equal(d.length, 64);
  for (const v of d) assert.ok(Math.abs(Math.hypot(v.x, v.y, v.z) - 1) < 1e-9);
  assert.ok(d.some((v) => v.y > 0.9) && d.some((v) => v.y < -0.9));
  assert.deepEqual(sphereDirections(64), d);
});

test("pose picks: close-up near the wanted distance, the longest horizontal view, a vista in range", () => {
  const p = (x: number, y: number, z: number, t: number | null): Probe => ({
    dir: { x, y, z },
    hit: t === null ? null : { t, normal: { x: -x, y: -y, z: -z } },
  });
  const probes = [p(1, 0, 0, 3), p(0, 0, 1, 8.5), p(0, 0, -1, 30), p(0, 1, 0, null), p(-1, 0, 0, 70)];
  assert.equal(pickCloseup(probes, 9, 5)?.hit?.t, 8.5);
  assert.equal(pickLongest(probes, 160)?.hit?.t, 70, "the miss straight up is too steep");
  assert.equal(pickVista(probes, 40, 150)?.hit?.t, 70);
  assert.equal(pickVista(probes, 20, 40)?.hit?.t, 30);
  assert.equal(pickCloseup([p(1, 0, 0, null)], 9, 5), null);
});

test("pickPose stands off the wall for a close-up and looks down the longest line otherwise", () => {
  const m = room(10);
  const origin = { x: 0, y: 0, z: 0 };
  const probes = probeAround([m], origin, sphereDirections(96), 160);
  const closeup = pickPose(sceneById("cave-closeup")!, origin, q.identity(), probes);
  // About 3.5 u from a wall, inside the room.
  const pos = closeup.position;
  const toWall = Math.min(10 - Math.abs(pos.x), 10 - Math.abs(pos.y), 10 - Math.abs(pos.z));
  assert.ok(toWall > 1 && toWall < 5, `distance to the nearest wall ${toWall}`);
  const tunnel = pickPose(sceneById("cave-tunnel")!, origin, q.identity(), probes);
  assert.deepEqual(tunnel.position, origin);
  const f = q.forward(tunnel.orientation);
  assert.ok(Math.abs(f.y) <= 0.5 + 1e-9, "near-horizontal");
});

test("sun pose: behind a rock, with the sun just clear of its edge", () => {
  const h = 40;
  const b = new MeshBuilder();
  const t = () => NO_TEAM;
  appendBox(b, 0, 0, -h - 0.5, h + 1, h + 1, 0.5, t); // far wall behind the rock
  appendBox(b, 0, 0, -12, 2, 2, 2, t); // the rock
  const meshes = [rayMesh(b.build())];
  const origin = { x: 0, y: 0, z: 0 };
  const sun = { x: 0, y: 0, z: -1 };
  const probes = probeAround(meshes, origin, sphereDirections(400), 160);
  const room = { min: { x: -h, y: -h, z: -h }, max: { x: h, y: h, z: h } };
  const pose = pickSunPose(meshes, probes, origin, sun, room);
  assert.ok(pose);
  // The sun ray from the camera passes the rock (not blocked within the rock's reach) ...
  const clear = raycast(meshes, pose.position, sun, 30);
  assert.equal(clear, null);
  // ... but only just: the rock's edge is within 1.5 u of the line of sight.
  const off = Math.hypot(pose.position.x, pose.position.y);
  assert.ok(off > 2 && off < 3.5, `offset ${off}`);
  // Looking toward the sun, which sits off the screen centre.
  const f = q.forward(pose.orientation);
  const dot = -f.z;
  assert.ok(dot > 0.9 && dot < 0.999, `forward . sun ${dot}`);
  // No rock: no pose.
  assert.equal(pickSunPose([], [], origin, sun, room), null);
});

test("scene content: four hulls in the lineup, ships, shots and an explosion in the fight", () => {
  const cam = { position: { x: 0, y: 0, z: 0 }, orientation: q.identity() };
  const visuals = [
    { shape: "bolt", color: [1, 0.8, 0.2] as const, scale: 1 },
    { shape: "rocket", color: [1, 0.5, 0.1] as const, scale: 1 },
  ];
  // Ships draw the hull of their type (VQ4: mesh kinds ship_<hull>).
  const hulls = (batches: Partial<Record<string, readonly unknown[]>>) =>
    Object.entries(batches).filter(([kind]) => kind.startsWith("ship_"));
  const ships = (batches: Partial<Record<string, readonly unknown[]>>) =>
    hulls(batches).reduce((n, [, list]) => n + (list?.length ?? 0), 0);
  const lineup = sceneContent(sceneById("ship-lineup")!, cam, visuals);
  assert.equal(ships(lineup.opaque), LINEUP_RADII.length);
  assert.equal(hulls(lineup.opaque).length, 4, "four distinct hulls");
  const fight = sceneContent(sceneById("fight")!, cam, visuals, { x: 3, y: 0, z: -15 });
  assert.equal(ships(fight.opaque), 2);
  assert.ok((fight.opaque.bolt?.length ?? 0) >= 2, "bolts");
  assert.ok((fight.opaque.rocket?.length ?? 0) >= 1, "rocket");
  // The shield bubble plus the explosion flash and the muzzle flash.
  assert.ok((fight.translucent.sphere?.length ?? 0) >= 3, `spheres ${fight.translucent.sphere?.length}`);
  const none = sceneContent(sceneById("cave-tunnel")!, cam, visuals);
  assert.equal(Object.keys(none.opaque).length + Object.keys(none.translucent).length, 0);
});
