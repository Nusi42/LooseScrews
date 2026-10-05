import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { mulberry32 } from "../renderer/spriteAtlas.ts";
import type { VisualDef } from "../game/views.ts";
import { catmullRom, coneDirection, DEFAULT_PATH, FlightPath, sphereDirs, type LevelProbe } from "./flightPath.ts";
import { CinematicCamera, DEFAULT_CINEMATIC, nlerp, pathFrame, RESPAWN_SECS, Squadron } from "./cinematic.ts";
import { BAY_DEPTH, BAY_FOV_Y, bayLayout, PanelBay, studioLights, turntable, type BaySlot } from "./shipBay.ts";
import { describeWorld, worldKind } from "./worlds.ts";
import { BACKDROP_WORLDS, frameYawFor } from "./backdrop.ts";
import { OWN_SHIP_MIN_DISTANCE, LobbyCinematic } from "./lobby.ts";

/** An analytic level: a closed box (half size `half`) with spherical rocks. */
function level(half: number, rocks: Array<{ c: Vec3; r: number }>) {
  const sdf = (p: Vec3): number => {
    let d = Math.min(half - Math.abs(p.x), half - Math.abs(p.y), half - Math.abs(p.z));
    for (const k of rocks) d = Math.min(d, v3.length(v3.sub(p, k.c)) - k.r);
    return d;
  };
  const march = (o: Vec3, d: Vec3, radius: number, max: number): number => {
    let t = 0;
    for (let i = 0; i < 400 && t < max; i++) {
      const s = sdf(v3.addScaled(o, d, t)) - radius;
      if (s < 0.01) return Math.max(0, t);
      t += s;
    }
    return max;
  };
  const probe: LevelProbe = { ray: (o, d, max) => march(o, d, 0, max), sweep: (o, d, r, max) => march(o, d, r, max) };
  return { sdf, probe };
}

function rockField(seed: number): Array<{ c: Vec3; r: number }> {
  const rng = mulberry32(seed);
  const rocks = [];
  for (let i = 0; i < 60; i++) {
    const c = { x: (rng() - 0.5) * 160, y: (rng() - 0.5) * 80, z: (rng() - 0.5) * 160 };
    if (v3.length(c) < 14) continue; // keep the start clear
    rocks.push({ c, r: 3 + rng() * 9 });
  }
  return rocks;
}

test("catmull-rom passes through its inner points; cone directions stay in the cone", () => {
  const p = [
    { x: 0, y: 0, z: 0 },
    { x: 10, y: 0, z: 0 },
    { x: 20, y: 5, z: 3 },
    { x: 30, y: 5, z: 9 },
  ] as const;
  const a = catmullRom(p[0], p[1], p[2], p[3], 0);
  const b = catmullRom(p[0], p[1], p[2], p[3], 1);
  assert.ok(v3.length(v3.sub(a, p[1])) < 1e-9);
  assert.ok(v3.length(v3.sub(b, p[2])) < 1e-9);
  const axis = v3.normalize({ x: 0.3, y: 0.2, z: -1 });
  const rng = mulberry32(3);
  for (let i = 0; i < 200; i++) {
    const d = coneDirection(axis, 0.6, rng(), rng());
    assert.ok(Math.abs(v3.length(d) - 1) < 1e-9);
    assert.ok(v3.dot(d, axis) >= Math.cos(0.6) - 1e-9);
  }
  assert.equal(sphereDirs(14).length, 14);
});

test("the flight path keeps clear of rock, is continuous and never ends", () => {
  for (const seed of [1, 2, 3]) {
    const { sdf, probe } = level(100, rockField(seed));
    const path = new FlightPath(probe, { x: 0, y: 0, z: 0 }, { x: 0, y: 0, z: -1 }, mulberry32(seed));
    let prev = path.sample(0).position;
    let worst = Infinity;
    for (let s = 0; s <= 1500; s += 0.5) {
      path.ensure(s + 30);
      path.forget(s - 10);
      const here = path.sample(s);
      assert.ok(Number.isFinite(here.position.x + here.position.y + here.position.z));
      assert.ok(Math.abs(v3.length(here.tangent) - 1) < 1e-6);
      // Arc-length sampling: half a unit of path is about half a unit of travel.
      const step = v3.length(v3.sub(here.position, prev));
      assert.ok(step <= 0.51, `step ${step} at ${s}`);
      if (s > 0) assert.ok(step > 0.3, `stalled at ${s} (${step})`);
      prev = here.position;
      worst = Math.min(worst, sdf(here.position));
    }
    // The tube is DEFAULT_PATH.clearance wide; corners of the spline may cut in a little.
    assert.ok(worst > DEFAULT_PATH.clearance * 0.5, `seed ${seed}: came within ${worst.toFixed(2)} u of rock`);
    assert.ok(path.waypoints.length < 20, "old waypoints are dropped");
  }
});

test("the path leans toward an attractor", () => {
  const { probe } = level(400, []);
  const toward = v3.normalize({ x: 1, y: 0, z: 0 });
  const path = new FlightPath(probe, { x: 0, y: 0, z: 0 }, { x: 0, y: 0, z: -1 }, mulberry32(9), { ...DEFAULT_PATH, toward, towardWeight: 0.6 });
  path.ensure(200);
  assert.ok(path.sample(200).position.x > 60, `x ${path.sample(200).position.x}`);
});

test("cinematic camera eases in, cruises, looks along the flight and frames it right of centre", () => {
  const { probe } = level(300, []);
  const path = new FlightPath(probe, { x: 0, y: 0, z: 0 }, { x: 0, y: 0, z: -1 }, mulberry32(1));
  const cam = new CinematicCamera(path);
  cam.update(0);
  assert.equal(cam.s, 0);
  for (let i = 0; i < 60 * 10; i++) cam.update(1 / 60);
  assert.ok(Math.abs(cam.speed - DEFAULT_CINEMATIC.speed) < 1e-9);
  assert.ok(cam.s > 60 && cam.s < 85, `s ${cam.s}`);
  const pose = cam.update(1 / 60);
  const ahead = path.sample(cam.s + DEFAULT_CINEMATIC.lookAhead).position;
  const look = v3.normalize(v3.sub(ahead, pose.position));
  assert.ok(v3.dot(q.forward(pose.orientation), look) > 0.95);

  const framed = new CinematicCamera(new FlightPath(probe, { x: 0, y: 0, z: 0 }, { x: 0, y: 0, z: -1 }, mulberry32(1)), { ...DEFAULT_CINEMATIC, frameYaw: 0.2 });
  for (let i = 0; i < 60 * 10; i++) framed.update(1 / 60);
  const f = framed.update(1 / 60);
  const target = framed.path.sample(framed.s + 9).position;
  // The flight ahead is right of the view axis.
  assert.ok(v3.dot(v3.sub(target, f.position), q.right(f.orientation)) > 0);
  assert.equal(frameYawFor(16 / 9), 0.2);
  assert.equal(frameYawFor(9 / 16), 0);
});

test("nlerp and the path frame are well formed", () => {
  const a = q.fromAxisAngle({ x: 0, y: 1, z: 0 }, 0.2);
  const b = q.fromAxisAngle({ x: 0, y: 1, z: 0 }, 1.2);
  const m = nlerp(a, b, 0.5);
  assert.ok(Math.abs(Math.hypot(m.x, m.y, m.z, m.w) - 1) < 1e-9);
  const { right, up } = pathFrame({ x: 0, y: 1, z: 0 });
  assert.ok(Math.abs(v3.length(right) - 1) < 1e-9 && Math.abs(v3.length(up) - 1) < 1e-9);
});

test("the squadron flies ahead of the camera, fights, loses ships and brings them back", () => {
  const { sdf, probe } = level(150, rockField(4));
  const path = new FlightPath(probe, { x: 0, y: 0, z: 0 }, { x: 0, y: 0, z: -1 }, mulberry32(4));
  const visuals: VisualDef[] = [
    { shape: "bolt", color: [1, 0.3, 0.2], scale: 1 },
    { shape: "orb", color: [0.3, 0.6, 1], scale: 1 },
    { shape: "rocket", color: [1, 0.6, 0.2], scale: 1 },
  ];
  const cam = new CinematicCamera(path);
  const squad = new Squadron(path, probe, visuals, mulberry32(4));
  const dt = 1 / 60;
  let hits = 0;
  let closest = Infinity;
  let deadSeen = false;
  const ids = new Set<number>();
  for (let i = 0; i < 60 * 90; i++) {
    const pose = cam.update(dt);
    const f = squad.update(i * dt, dt, cam.s, pose.position);
    hits += f.cues.length;
    assert.equal(f.ships.length, 4);
    for (const s of f.ships) {
      ids.add(s.id);
      if (!s.alive) {
        deadSeen = true;
        continue;
      }
      assert.ok(Number.isFinite(s.position.x + s.position.y + s.position.z));
      closest = Math.min(closest, sdf(s.position) - s.radius);
      // In front of the camera, not on it.
      if (i > 60 * 5) assert.ok(v3.length(v3.sub(s.position, pose.position)) > 2, "a ship flew into the camera");
    }
    for (const p of f.shots) assert.ok(p.visual >= 0 && p.visual < visuals.length);
  }
  assert.ok(squad.shotsFired > 50, `${squad.shotsFired} shots`);
  assert.ok(hits > 0, "no hits");
  assert.ok(squad.kills >= 1 && deadSeen, `${squad.kills} kills`);
  // A ship back from the dead flies under a new id (no respawn burst from the effect library).
  assert.ok(ids.size > 4);
  assert.ok(RESPAWN_SECS > 0);
  assert.ok(closest > -0.3, `a hull went ${(-closest).toFixed(2)} u into rock`);
});

test("bay layout puts each hull behind its box, sized to it", () => {
  const cw = 800;
  const ch = 200;
  const centre = bayLayout(400, 100, 100, 80, cw, ch);
  assert.ok(Math.abs(centre.position.x) < 1e-9 && Math.abs(centre.position.y) < 1e-9);
  assert.equal(centre.position.z, -BAY_DEPTH);
  const tanY = Math.tan(BAY_FOV_Y / 2);
  for (const [cx, cy] of [[100, 50], [700, 150], [250, 120]] as const) {
    const p = bayLayout(cx, cy, 120, 90, cw, ch).position;
    // Project back to pixels.
    const ndcX = p.x / (-p.z * tanY * (cw / ch));
    const ndcY = p.y / (-p.z * tanY);
    assert.ok(Math.abs((ndcX + 1) * 0.5 * cw - cx) < 1e-6);
    assert.ok(Math.abs((1 - ndcY) * 0.5 * ch - cy) < 1e-6);
  }
  // The hull's projected length (2 radii) stays inside its box.
  const r = bayLayout(400, 100, 100, 80, cw, ch).radius;
  const px = (2 * r) / ((2 * BAY_DEPTH * tanY) / ch);
  assert.ok(px <= 100 && px > 60, `${px} px`);
  const t = turntable(1.3, 2, true);
  assert.ok(Math.abs(Math.hypot(t.x, t.y, t.z, t.w) - 1) < 1e-9);
  assert.equal(studioLights(centre.position, 1, false).length, 2);
});

test("worlds are named for captions", () => {
  assert.equal(worldKind(0), "Open space");
  assert.equal(worldKind(36), "Caverns open to the sky");
  assert.equal(worldKind(100), "Mine shafts");
  assert.equal(describeWorld("not a code"), "not a code");
  for (const w of BACKDROP_WORLDS) assert.ok(w.enclosure > 30 && w.enclosure < 50, "backdrop worlds show caves and sky");
});

test("lobby cinematic needs probes, starts from the view and hides the own ship up close", () => {
  const lobby = new LobbyCinematic();
  const view = { position: { x: 0, y: 0, z: 0 }, orientation: q.identity(), fovY: 1, near: 0.05, far: 600 };
  assert.equal(lobby.update(1 / 60, {}, true, view), null);
  const { probe } = level(200, []);
  const game = {
    raycast: (ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, max: number) => {
      const t = probe.ray({ x: ox, y: oy, z: oz }, { x: dx, y: dy, z: dz }, max);
      return t >= max ? -1 : t;
    },
    sweepSphere: (ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, r: number, max: number) => {
      const t = probe.sweep({ x: ox, y: oy, z: oz }, { x: dx, y: dy, z: dz }, r, max);
      return t >= max ? new Float32Array(0) : new Float32Array([t, 0, 0, 0, 0, 0, 0]);
    },
  };
  const pose = { position: { x: 0, y: 0, z: 0 }, orientation: q.identity(), velocity: { x: 0, y: 0, z: 0 }, alive: true };
  const first = lobby.update(1 / 60, game, true, view);
  assert.ok(first && v3.length(first.position) < 0.1);
  assert.equal(lobby.ownShip(1, 0, pose, 1, 0.5).alive, false);
  for (let i = 0; i < 60 * 3; i++) lobby.update(1 / 60, game, true, view);
  assert.ok(lobby.ownShip(1, 0, pose, 1, 0.5).alive, `camera within ${OWN_SHIP_MIN_DISTANCE} u after 3 s`);
  assert.equal(lobby.update(1 / 60, game, false, view), null);
  assert.equal(lobby.active, false);
});

test("the in-game ship bay repaints its hulls in the new team's livery after a match transition", async () => {
  const g = globalThis as Record<string, unknown>;
  const savedRaf = g["requestAnimationFrame"];
  g["requestAnimationFrame"] = () => 0;
  try {
    const shown: BaySlot[][] = [];
    const view = {
      canvas: { classList: { add: () => {} } } as unknown as HTMLCanvasElement,
      setSlots: (s: readonly BaySlot[]) => void shown.push([...s]),
      show: () => {},
      dispose: () => {},
    };
    const host = { classList: { add: () => {} } } as unknown as HTMLElement;
    const bay = new PanelBay(host, {} as GPUDevice, 1, "medium", async () => view);
    const el = {} as HTMLElement;
    bay.update([{ el, shipType: 0, allowed: true, selected: true }]);
    await new Promise((r) => setTimeout(r, 0));
    assert.equal(shown.at(-1)?.[0]?.team, 1);
    bay.setTeam(0);
    assert.equal(bay.team, 0);
    assert.equal(shown.at(-1)?.[0]?.team, 0, "the showing hulls repaint at once");
    bay.dispose();
  } finally {
    g["requestAnimationFrame"] = savedRaf;
  }
});
