// Session lifecycle: match restarts, the hidden-tab keep-alive frame,
// droppable sends and idempotent disposal.

import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChunkMeshHandle, GameHandle, HudStateHandle } from "../wasm/api.ts";
import { GameSession, type PacketLink } from "./session.ts";

const WEAPONS = ["scrap_laser", "rivet_gun", "bolt_rocket", "paint_thrower"];
const RESTART = "match over; a new match is starting";

function hud(): HudStateHandle {
  return {
    hull: 1, hullMax: 1, energy: 1, energyMax: 1, boost: 1, boostMax: 1, scrap: 0, scrapMax: 1,
    weapon: "Scrap Laser", ammo: "", secondary: "", paint0: 0, paint1: 0, timeLeft: 1, team: 0,
    alive: true, status: "", free: () => {},
  };
}

class Game implements GameHandle {
  readonly isOnline = true;
  readonly shareCode = "LS04";
  readonly player = 0;
  readonly team = 0;
  readonly editCount = 0;
  /** Packet value -> Disconnect reason returned for it. */
  reasons = new Map<number, string>();
  controls: number[][] = [];
  freed = 0;
  bounds(): Float32Array { return new Float32Array([0, 0, 0, 1, 1, 1]); }
  handlePacket(packet: Uint8Array): string | undefined { return this.reasons.get(packet[0] ?? -1); }
  frame(_dt: number, controls: Float32Array): unknown[] {
    this.controls.push(Array.from(controls));
    return [new Uint8Array([1])];
  }
  localPose(): Float32Array | undefined { return undefined; }
  remoteShips(): Float32Array { return new Float32Array(0); }
  projectiles(): Float32Array { return new Float32Array(0); }
  hud(): HudStateHandle { return hud(); }
  takeFeed(): string[] { return []; }
  meshNext(): ChunkMeshHandle | undefined { return undefined; }
  takeUnloads(): string[] { return []; }
  pendingChunks(): number { return 0; }
  setViewRadius(): void {}
  resetMeshes(): void {}
  free(): void { this.freed += 1; }
}

class Link implements PacketLink {
  isOpen = true;
  closeReason: string | null = null;
  inbox: Uint8Array[] = [];
  sent: { bytes: number[]; droppable: boolean }[] = [];
  closes = 0;
  send(p: Uint8Array, droppable = false): void { this.sent.push({ bytes: Array.from(p), droppable }); }
  drain(): Uint8Array[] { return this.inbox.splice(0); }
  close(reason?: string): void { this.isOpen = false; this.closeReason = reason ?? "closed"; this.closes += 1; }
}

test("a match restart is reported as such, not as an error", () => {
  const game = new Game();
  game.reasons.set(7, RESTART);
  game.reasons.set(8, "kicked");
  const link = new Link();
  const s = new GameSession(game, link, WEAPONS, true, RESTART);
  link.inbox.push(new Uint8Array([7]));
  const r = s.frame(1 / 60, {
    thrustForward: 0, thrustRight: 0, thrustUp: 0, roll: 0, lookX: 0, lookY: 0, boost: false,
    firePrimary: false, fireSecondary: false, scavenge: false, overdrive: false,
    toggleHorizonAssist: false, weaponKey: 0, pointerLocked: true,
  });
  assert.equal(r.matchRestart, true);
  assert.match(r.disconnected ?? "", /new match/);

  const other = new Game();
  other.reasons.set(8, "kicked");
  const l2 = new Link();
  const s2 = new GameSession(other, l2, WEAPONS, true, RESTART);
  l2.inbox.push(new Uint8Array([8]));
  const r2 = s2.idleFrame(0.25);
  assert.ok(r2);
  assert.equal(r2.matchRestart, false);
  assert.match(r2.disconnected ?? "", /kicked/);
});

test("the hidden-tab frame sends idle input as droppable packets", () => {
  const game = new Game();
  const link = new Link();
  const s = new GameSession(game, link, WEAPONS, true, RESTART);
  assert.equal(s.idleFrame(0.25), null, "still connected");
  assert.equal(game.controls.length, 1);
  // Idle controls: no thrust, look, buttons or weapon change (horizon assist
  // is a setting and may stay set in the button bits).
  const c = game.controls[0] ?? [];
  assert.ok(c.slice(0, 6).every((v) => v === 0), `${c}`);
  assert.equal(c[7], 0);
  assert.deepEqual(link.sent, [{ bytes: [1], droppable: true }]);

  // Practice sessions and ended sessions have nothing to keep alive.
  const practice = new GameSession(new Game(), null, WEAPONS);
  assert.equal(practice.idleFrame(0.25), null);
});

test("dispose closes the link and frees the game once", () => {
  const game = new Game();
  const link = new Link();
  const s = new GameSession(game, link, WEAPONS, true, RESTART);
  s.dispose();
  s.dispose();
  assert.equal(game.freed, 1);
  assert.equal(link.closes, 1);
  assert.ok(s.isDisposed);
  assert.equal(s.idleFrame(0.25), null, "a disposed session does nothing");
  assert.equal(game.controls.length, 0);
});
