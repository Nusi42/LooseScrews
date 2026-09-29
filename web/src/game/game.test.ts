import assert from "node:assert/strict";
import { test } from "node:test";
import { emptyInput, type InputState } from "../input/input.ts";
import * as q from "../math/quat.ts";
import type { MeshData } from "../renderer/mesh.ts";
import type { ChunkMeshHandle, GameHandle, HudStateHandle, LoadoutHandle } from "../wasm/api.ts";
import { stubLoadout } from "../wasm/stub.ts";
import { boundsCenter, ShipCamera } from "./camera.ts";
import { ChunkLoader, takeMesh } from "./chunks.ts";
import { BUTTONS, ControlState } from "./controls.ts";
import { buildScene, TEAM_HULL } from "./scene.ts";
import { GameSession, hudValues, type PacketLink } from "./session.ts";
import { parsePose, parseProjectiles, parseShips } from "./views.ts";

const NAMES = { entityKinds: ["debris", "weaponPickup"], shapes: ["bolt", "paint"] };

function input(over: Partial<InputState>): InputState {
  return { ...emptyInput(), pointerLocked: true, ...over };
}

test("controls: axes, look signs, buttons and weapon-selection intents", () => {
  const c = new ControlState(true);
  const out = Array.from(
    c.update(input({ thrustForward: 1, thrustRight: -1, thrustUp: 2, roll: 1, lookX: 100, lookY: -50, boost: true, firePrimary: true })),
  );
  assert.equal(out.length, 11);
  assert.deepEqual(out.slice(0, 4), [1, -1, 1, 1]);
  assert.ok(out[4]! > 0, "mouse right = yaw right (positive)");
  assert.ok(out[5]! > 0, "mouse up = pitch up (positive)");
  assert.equal(out[6], BUTTONS.boost | BUTTONS.firePrimary | BUTTONS.horizonAssist);
  assert.deepEqual(out.slice(7), [0, 0, 0, 0], "no weapon intent");

  // Number keys, wheel steps and X are passed through for ls-wasm to resolve.
  let o = c.update(input({ weaponKey: 10, cyclePrimary: -3, cycleSecondary: true, gadget: true }));
  assert.deepEqual(Array.from(o.slice(7)), [0, 10, -1, 1]);
  assert.equal((o[6] ?? 0) & BUTTONS.gadget, BUTTONS.gadget, "G sets the gadget button");
  o = c.update(input({}));
  assert.deepEqual(Array.from(o.slice(7)), [0, 0, 0, 0], "intents are one-shot");

  // H toggles horizon assist off; overdrive and scavenge map to their bits.
  o = c.update(input({ toggleHorizonAssist: true, overdrive: true, scavenge: true, fireSecondary: true }));
  assert.equal(c.horizonAssist, false);
  assert.equal(o[6], BUTTONS.overdrive | BUTTONS.scavenge | BUTTONS.fireSecondary);

  c.settings.invertY = true;
  o = c.update(input({ lookY: -50 }));
  assert.ok(o[5]! < 0, "inverted Y");
  o = c.update(input({ thrustForward: Number.NaN }));
  assert.equal(o[0], 0);
});

test("views: flat arrays decode into poses, ships and projectiles", () => {
  assert.equal(parsePose(undefined), null);
  assert.equal(parsePose(new Float32Array(3)), null);
  const pose = parsePose(new Float32Array([1, 2, 3, 0, 0, 0, 2, 4, 5, 6, 1]));
  assert.ok(pose);
  assert.deepEqual(pose.position, { x: 1, y: 2, z: 3 });
  assert.deepEqual(pose.orientation, { x: 0, y: 0, z: 0, w: 1 }, "normalised");
  assert.deepEqual(pose.velocity, { x: 4, y: 5, z: 6 });
  assert.equal(pose.alive, true);

  const ships = parseShips(
    new Float32Array([
      3, 1, 1, 0, 10, 20, 30, 0, 0, 0, 1, 2, 0.65, 1,
      4, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 1, 0.5, 0,
    ]),
  );
  assert.equal(ships.length, 2);
  assert.deepEqual(ships[0], {
    id: 3,
    team: 1,
    alive: true,
    boosting: false,
    position: { x: 10, y: 20, z: 30 },
    orientation: { x: 0, y: 0, z: 0, w: 1 },
    shipType: 2,
    radius: ships[0]!.radius,
    gadgetActive: true,
  });
  assert.ok(Math.abs(ships[0]!.radius - 0.65) < 1e-6);
  assert.equal(ships[1]!.alive, false);
  const scene = buildScene(ships, [], [], [], () => undefined, 0);
  const hulls = scene.opaque.ship ?? [];
  assert.equal(hulls.length, 1, "dead ships are not drawn");
  assert.deepEqual(hulls[0]!.color, TEAM_HULL[1]);
  assert.ok(Math.abs(hulls[0]!.scale - 1.3) < 1e-6, "Hauler hull drawn at its hitbox size");
  assert.equal(scene.translucent.sphere?.length, 1, "active gadget shimmer");

  const shots = parseProjectiles(
    new Float32Array([1, 3, 0, 0, 0, 0, 10, 0, 9, 0, 3, 7, 0, 0, 0, 0, 0, -5, 8, 0]),
  );
  assert.equal(shots.length, 2);
  assert.equal(shots[0]!.kind, 1);
  assert.equal(shots[0]!.visual, 9);
  assert.equal(shots[1]!.visual, 8);
  // A truncated trailing record is ignored.
  assert.equal(parseShips(new Float32Array(20)).length, 1);
});

function fakeMesh(key: string, triangles: number, log: string[]): ChunkMeshHandle {
  return {
    key,
    positions: () => new Float32Array(9 * Math.max(1, triangles)),
    normals: () => new Float32Array(9 * Math.max(1, triangles)),
    indices: () => new Uint32Array(3 * triangles),
    teams: () => new Uint8Array(3 * Math.max(1, triangles)),
    surface: () => new Uint8Array(0),
    aabb: () => new Float32Array(6),
    free: () => log.push(`free ${key}`),
  };
}

class FakeChunks {
  queue: Array<[string, number]> = [];
  unloads: string[] = [];
  resets = 0;
  log: string[] = [];
  meshNext(): ChunkMeshHandle | undefined {
    const next = this.queue.shift();
    return next ? fakeMesh(next[0], next[1], this.log) : undefined;
  }
  takeUnloads(): string[] {
    return this.unloads.splice(0);
  }
  pendingChunks(): number {
    return this.queue.length;
  }
  resetMeshes(): void {
    this.resets++;
  }
}

test("chunk loader: time budget, empty meshes, unloads and reset", () => {
  let clock = 0;
  const sink: Array<[string, MeshData | null]> = [];
  const loader = new ChunkLoader((k, m) => sink.push([k, m]), () => clock);
  const game = new FakeChunks();
  game.queue = [
    ["0,0,0", 2],
    ["1,0,0", 0],
    ["2,0,0", 1],
    ["3,0,0", 1],
  ];
  // Each meshNext "takes" 3 ms: a 4 ms budget meshes two chunks this frame.
  const origNext = game.meshNext.bind(game);
  game.meshNext = () => {
    clock += 3;
    return origNext();
  };
  let s = loader.update(game, { x: 0, y: 0, z: 0 }, { millis: 4, maxChunks: 10 });
  assert.equal(s.meshed, 2);
  assert.equal(s.pending, 2);
  assert.equal(s.shown, 1, "the empty mesh is not shown");
  assert.deepEqual(
    sink.map(([k, m]) => [k, m ? m.indices.length : null]),
    [["0,0,0", 6]],
  );
  assert.deepEqual(game.log, ["free 0,0,0", "free 1,0,0"], "wasm meshes are freed");

  // At least one chunk per frame even with a zero budget; max count respected.
  s = loader.update(game, { x: 0, y: 0, z: 0 }, { millis: 0, maxChunks: 10 });
  assert.equal(s.meshed, 1);
  // A remesh that became empty removes the chunk; unloads remove too.
  game.queue = [["0,0,0", 0]];
  game.unloads = ["2,0,0"];
  s = loader.update(game, { x: 0, y: 0, z: 0 }, { millis: 100, maxChunks: 10 });
  assert.equal(s.removed, 1);
  assert.equal(s.shown, 0);
  assert.deepEqual(sink.slice(-2), [
    ["2,0,0", null],
    ["0,0,0", null],
  ]);
  loader.reset(game);
  assert.equal(game.resets, 1);
});

test("takeMesh maps teams to MeshData.team and frees on error", () => {
  const log: string[] = [];
  const m = takeMesh(fakeMesh("k", 1, log));
  assert.equal(m.key, "k");
  assert.equal(m.mesh.team.length, 3);
  const broken = { ...fakeMesh("x", 1, log), positions: () => { throw new Error("boom"); } };
  assert.throws(() => takeMesh(broken));
  assert.deepEqual(log, ["free k", "free x"]);
});

function fakeHud(over: Partial<HudStateHandle> = {}): HudStateHandle {
  return {
    hull: 80, hullMax: 100, energy: 50, energyMax: 100, boost: 20, boostMax: 100, scrap: 5, scrapMax: 100,
    weapon: "Scrap Laser", ammo: "12 shots", secondary: "Bolt Rocket ×3", paint0: 0.25, paint1: 0.5,
    timeLeft: 123, team: 0, alive: true, status: "", free: () => {}, ...over,
  };
}

class FakeGame implements GameHandle {
  isOnline = true;
  shareCode = "LS04-0000-0000-000A-K4M6-8001-8F1P";
  player = 2;
  team = 1;
  editCount = 0;
  handled: number[] = [];
  frames: Array<[number, number[]]> = [];
  disconnectOn = -1;
  throwOn = -1;
  bounds(): Float32Array { return new Float32Array([0, 0, 0, 256, 128, 256]); }
  handlePacket(p: Uint8Array): string | undefined {
    if (p[0] === this.throwOn) throw new Error("terrain edit gap");
    this.handled.push(p[0] ?? -1);
    return p[0] === this.disconnectOn ? "match closed" : undefined;
  }
  frame(dt: number, controls: Float32Array): unknown[] {
    this.frames.push([dt, Array.from(controls)]);
    return [new Uint8Array([4, 1]), new Uint8Array([7])];
  }
  localPose(): Float32Array | undefined { return new Float32Array([1, 2, 3, 0, 0, 0, 1, 0, 0, 0, 1]); }
  remoteShips(): Float32Array { return new Float32Array([5, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0.5, 0]); }
  projectiles(): Float32Array { return new Float32Array(0); }
  hud(): HudStateHandle { return fakeHud(); }
  setClockMs(): void {}
  takeFeed(): string[] { return ["You destroyed Player 6"]; }
  debugStats(): Float64Array { return new Float64Array([42, 10, 2, 1, 0.5, 6, 5, 1, 0, 1]); }
  scoreboard(): Float64Array { return new Float64Array([2, 1, 1, 3, 1, 1, 5, 0, 1, 0, 2, 0]); }
  matchInfo(): Float64Array { return new Float64Array([0, -1, 25, 50, 100, 0]); }
  meshNext(): ChunkMeshHandle | undefined { return undefined; }
  takeUnloads(): string[] { return []; }
  pendingChunks(): number { return 0; }
  setViewRadius(): void {}
  entities(): Float32Array {
    return new Float32Array([7, 1, 1, 255, 255, 1, 2, 3, 0, 0, 0, 0, 0, 0, 1, 4, 10, 0, 0, 0]);
  }
  visuals(): Float32Array { return new Float32Array([0, 1, 0, 0, 1, 1, 0.5, 0.5, 0.5, 2]); }
  playerInfo(): Float64Array { return new Float64Array([2, 1, 1, 255, 5, 0, 3, 0]); }
  playerNames(): string[] { return ["Ada", "Bob"]; }
  loadout(): LoadoutHandle {
    return stubLoadout({ names: ["Scrap Laser", "Bolt Rocket"], ammo: ["∞", "3"], info: new Uint8Array([0, 1, 1, 2, 1, 1, 0, 1]), maxLevels: new Uint8Array([6, 1]), gadget: "Shield" });
  }
  scoreboardNames(): string[] { return ["Ada", "Bob"]; }
  resetMeshes(): void {}
  free(): void {}
}

class FakeLink implements PacketLink {
  isOpen = true;
  closeReason: string | null = null;
  inbox: Uint8Array[] = [];
  sent: number[][] = [];
  send(p: Uint8Array): void { this.sent.push(Array.from(p)); }
  drain(): Uint8Array[] { return this.inbox.splice(0); }
  close(reason?: string): void { this.isOpen = false; this.closeReason = reason ?? "closed"; }
}

test("session: packets in, controls to the game, packets out, state back", () => {
  const game = new FakeGame();
  const link = new FakeLink();
  const s = new GameSession(game, link, NAMES, true);
  link.inbox.push(new Uint8Array([5]), new Uint8Array([6]));
  const r = s.frame(1 / 60, input({ thrustForward: 1 }));
  assert.deepEqual(game.handled, [5, 6]);
  assert.equal(game.frames.length, 1);
  assert.equal(game.frames[0]![1][0], 1);
  assert.deepEqual(link.sent, [[4, 1], [7]]);
  assert.equal(r.disconnected, null);
  assert.deepEqual(r.pose?.position, { x: 1, y: 2, z: 3 });
  assert.equal(r.ships.length, 1);
  assert.deepEqual(r.hud.paint, [0.25, 0.5]);
  assert.equal(r.hud.horizonAssist, true);
  assert.deepEqual(r.feed, ["You destroyed Player 6"]);
  // M2: entities, players, visuals and the weapon panel come with every frame.
  assert.equal(r.entities.length, 1);
  assert.equal(r.entities[0]!.kind, "weaponPickup");
  assert.deepEqual(r.entities[0]!.state, [4, 10, 0, 0]);
  assert.deepEqual(r.players.map((p) => p.name), ["Ada", "Bob"]);
  assert.deepEqual(s.visuals.map((v) => v.shape), ["bolt", "paint"]);
  assert.equal(s.visuals[1]!.scale, 2);
  const loadout = r.hud.loadout!;
  assert.deepEqual(loadout.primaries.map((w) => [w.name, w.key, w.level, w.selected]), [["Scrap Laser", 1, 2, true]]);
  assert.equal(loadout.secondaries[0]!.ammo, "3");
  assert.equal(loadout.gadget, "Shield");
  assert.deepEqual(s.scoreboard().map((row) => row.name), ["Ada", "Bob"]);
  const teamOf = s.teamOf(r.ships);
  assert.equal(teamOf(2), 1, "own ship");
  assert.equal(teamOf(5), 0);
  assert.equal(teamOf(9), undefined);
});

test("session: server disconnects, desyncs and dropped connections end the session once", () => {
  const game = new FakeGame();
  game.disconnectOn = 9;
  const link = new FakeLink();
  const s = new GameSession(game, link, NAMES);
  link.inbox.push(new Uint8Array([9]), new Uint8Array([10]));
  const r = s.frame(1 / 60, input({}));
  assert.match(r.disconnected ?? "", /match closed/);
  assert.deepEqual(game.handled, [9], "packets after the disconnect are not handled");
  assert.equal(link.isOpen, false);
  assert.ok(s.hasEnded);
  const again = s.frame(1 / 60, input({}));
  assert.equal(again.disconnected, null, "reported once");
  assert.equal(link.sent.length, 0, "nothing sent after the end");

  const g2 = new FakeGame();
  g2.throwOn = 1;
  const l2 = new FakeLink();
  const s2 = new GameSession(g2, l2, NAMES);
  l2.inbox.push(new Uint8Array([1]));
  assert.match(s2.frame(0.016, input({})).disconnected ?? "", /Lost sync.*edit gap/);

  const l3 = new FakeLink();
  const s3 = new GameSession(new FakeGame(), l3, NAMES);
  l3.isOpen = false;
  l3.closeReason = "server went away";
  assert.match(s3.frame(0.016, input({})).disconnected ?? "", /server went away/);
});

test("practice sessions have no link and never disconnect", () => {
  const game = new FakeGame();
  game.isOnline = false;
  const s = new GameSession(game, null, NAMES);
  assert.equal(s.isOnline, false);
  const r = s.frame(1 / 60, input({}));
  assert.equal(r.disconnected, null);
  assert.equal(game.frames.length, 1);
});

test("hudValues copies and frees the wasm object; empty weapon shows a dash", () => {
  let freed = false;
  const v = hudValues(fakeHud({ weapon: "", free: () => { freed = true; } }));
  assert.ok(freed);
  assert.equal(v.weapon, "—");
  assert.equal(v.secondary, "Bolt Rocket ×3");
  assert.equal(v.timeLeft, 123);
});

test("camera follows the pose and holds the last view while dead", () => {
  const cam = new ShipCamera(boundsCenter([0, 0, 0, 256, 128, 256]));
  assert.deepEqual(cam.current.position, { x: 128, y: 64, z: 128 });
  const alive = { position: { x: 1, y: 2, z: 3 }, orientation: q.identity(), velocity: { x: 0, y: 0, z: 0 }, alive: true };
  assert.deepEqual(cam.update(alive).position, { x: 1, y: 2, z: 3 });
  const dead = { ...alive, position: { x: 50, y: 50, z: 50 }, alive: false };
  assert.deepEqual(cam.update(dead).position, { x: 1, y: 2, z: 3 });
  assert.deepEqual(cam.update(null).position, { x: 1, y: 2, z: 3 });
});
