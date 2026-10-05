// Overlay data (debug stats, scoreboard, match result), their view models,
// the rejoin helper and the session plumbing behind them.

import assert from "node:assert/strict";
import { test } from "node:test";
import { DebugOverlay, formatDebugLines, FpsMeter, type DebugInfo } from "../ui/debugOverlay.ts";
import { scoreboardModel } from "../ui/scoreboard.ts";
import { emptyModeState, VIEW_FLAG } from "./modeState.ts";
import type { ChunkMeshHandle, GameHandle, HudStateHandle, LoadoutHandle } from "../wasm/api.ts";
import { stubLoadout } from "../wasm/stub.ts";
import { rejoin, rejoinDelay, RejoinCancelled } from "./rejoin.ts";
import { transportLabel } from "../net/connect.ts";
import { GameSession, type PacketLink } from "./session.ts";
import {
  parseDebugStats,
  parseMatchInfo,
  parseScoreboard,
  playerName,
  rowsByTeam,
  teamName,
} from "./stats.ts";

const NAMES = { entityKinds: [], shapes: [] };

test("stats: flat arrays decode; short or broken arrays are safe", () => {
  const d = parseDebugStats([42.5, 100, 3, 1, 0.25, 9, 8, 1, 0, 1]);
  assert.deepEqual(d, {
    pingMs: 42.5,
    accepted: 100,
    corrected: 3,
    snapped: 1,
    lastError: 0.25,
    shotsPredicted: 9,
    shotsConfirmed: 8,
    shotsDropped: 1,
    shotsUnpredicted: 0,
    synced: true,
  });
  assert.equal(parseDebugStats([-1, 0, 0, 0, 0, 0, 0, 0, 0, 0]).pingMs, null, "no pong yet");
  assert.equal(parseDebugStats([]).pingMs, null);
  assert.equal(parseDebugStats([Number.NaN, 0, 0, 0, 0, 0, 0, 0, 0, 0]).pingMs, 0);

  const rows = parseScoreboard([3, 1, 1, 4, 2, 0, 0, 0, 0, 1, 3, 1, 9], ["Ada"]);
  assert.equal(rows.length, 2, "a truncated trailing row is ignored");
  assert.deepEqual(rows[0], { id: 3, name: "Ada", team: 1, alive: true, kills: 4, deaths: 2, isYou: false });
  assert.deepEqual(rows[1], { id: 0, name: "", team: 0, alive: false, kills: 1, deaths: 3, isYou: true });

  const m = parseMatchInfo([1, 0, 30, 10, 200, 2.5]);
  assert.deepEqual(m, { over: true, winner: 0, teamCells: [30, 10], surfaceCells: 200, secsSinceEnd: 2.5 });
  assert.equal(parseMatchInfo([1, -1, 0, 0, 0, 0]).winner, null, "draw");
  assert.equal(parseMatchInfo([]).over, false);
});

test("stats: names and team grouping", () => {
  assert.equal(playerName({ id: 4, isYou: false }), "Player 5");
  assert.equal(playerName({ id: 4, isYou: true }), "You");
  assert.equal(playerName({ id: 4, isYou: false, name: "Ada" }), "Ada", "names from the server");
  assert.equal(playerName({ id: 4, isYou: true, name: "Ada" }), "Ada (you)");
  assert.equal(playerName({ id: 4, isYou: true, name: "You" }), "You", "practice names the player You");
  assert.equal(teamName(0), "Orange");
  assert.equal(teamName(1), "Blue");
  assert.equal(teamName(2), "Lime");
  assert.equal(teamName(3), "Magenta");
  const grouped = rowsByTeam(parseScoreboard([1, 1, 1, 0, 0, 0, 2, 0, 1, 0, 0, 1]));
  assert.deepEqual(
    grouped.map((g) => [g.team, g.rows.map((r) => r.id)]),
    [
      [0, [2]],
      [1, [1]],
    ],
  );
  assert.deepEqual(rowsByTeam([]).map((g) => g.rows.length), [0, 0], "both teams always listed");
  assert.deepEqual(rowsByTeam([], 4).map((g) => g.team), [0, 1, 2, 3], "every team of a 4-team match");
});

test("scoreboard model: teams with paint scores, free-for-all ranked by mode score with lives", () => {
  const rows = parseScoreboard([5, 0, 1, 2, 1, 0, 2, 1, 0, 3, 0, 1], ["Ada", ""]);
  const paint = { ...emptyModeState(), mode: "paint_the_void", teams: 2, flags: VIEW_FLAG.paintScores, teamScores: [30, 70], scoreTotal: 400 };
  const board = scoreboardModel(rows, paint, 1, true);
  assert.equal(board.title, "Paint the Void");
  assert.equal(board.teams.length, 2);
  assert.equal(board.teams[1]!.yours, true);
  assert.equal(board.teams[1]!.leading, true);
  assert.equal(board.teams[0]!.score, "7.5%");
  assert.equal(board.teams[1]!.players[0]!.name, "You");
  assert.equal(board.teams[1]!.players[0]!.kills, 3);
  assert.equal(board.showLives, false);
  assert.match(board.note, /since you joined/);

  // Free-for-all (Hot Reactor-like): one ranked list, lives column, eliminated marked.
  const ffaRows = parseScoreboard([4, 4, 1, 1, 0, 0, 7, 7, 0, 5, 3, 1, 9, 9, 1, 0, 0, 0], ["Bea", "Me", "Cy"]);
  const ffa = {
    ...emptyModeState(),
    mode: "hot_reactor",
    teams: 0,
    flags: VIEW_FLAG.lives,
    players: [
      { id: 4, side: 4, score: 3, lives: 2, meter: 0, flags: 0 },
      { id: 7, side: 7, score: 1, lives: 0, meter: 0, flags: 1 },
      { id: 9, side: 9, score: 5, lives: 3, meter: 0, flags: 0 },
    ],
  };
  const fb = scoreboardModel(ffaRows, ffa, 7, true);
  assert.equal(fb.teams.length, 1);
  assert.equal(fb.showLives, true);
  assert.deepEqual(fb.teams[0]!.players.map((p) => [p.name, p.score, p.lives, p.out]), [
    ["Cy", 5, 3, false],
    ["Bea", 3, 2, false],
    ["Me (you)", 1, 0, true],
  ]);
  assert.notEqual(fb.teams[0]!.players[0]!.color, fb.teams[0]!.players[1]!.color, "every side its own colour");
});

test("debug overlay text and fps meter", () => {
  const info: DebugInfo = {
    fps: 143.6,
    worstFrameMs: 21.04,
    mode: "online",
    transport: "WebSocket",
    droppedSends: 0,
    stats: parseDebugStats([48.4, 100, 2, 1, 0.31, 9, 8, 1, 0, 1]),
    chunks: { meshedTotal: 120, pending: 4, shown: 90, drawn: 60, triangles: 123456 },
    shareCode: "LS04-TEST",
    speed: 12.34,
    gpuError: null,
  };
  const lines = formatDebugLines(info);
  assert.equal(lines[0], "144 fps · worst frame 21.0 ms");
  assert.equal(lines[1], "ping 48 ms · WebSocket");
  assert.equal(lines[2], "prediction: 2 corrections · 1 snaps · 100 ok · last error 0.31 u");
  assert.equal(lines[3], "shots: 9 predicted · 8 confirmed · 1 dropped");
  assert.equal(lines[4], "chunks: 120 meshed · 4 pending · 60/90 drawn · 123k tris");
  assert.equal(lines[5], "12.3 u/s · LS04-TEST");
  const unsynced = formatDebugLines({
    ...info,
    droppedSends: 7,
    stats: { ...info.stats, pingMs: null, synced: false },
    gpuError: "oops",
  });
  assert.equal(unsynced[1], "ping — · WebSocket · syncing · 7 sends skipped");
  assert.equal(unsynced.at(-1), "GPU error: oops");
  const practice = formatDebugLines({ ...info, mode: "practice", transport: null });
  assert.equal(practice[1], "practice (offline)");
  assert.equal(practice.length, 4);

  const fps = new FpsMeter(1000);
  assert.equal(fps.fps, 0);
  for (let t = 0; t <= 2000; t += 10) fps.tick(t);
  assert.ok(Math.abs(fps.fps - 100) < 1, `${fps.fps}`);
  fps.tick(2050);
  assert.equal(fps.worstFrameMs, 50);
  // The window slides: an old stall is forgotten.
  for (let t = 2060; t <= 3200; t += 10) fps.tick(t);
  assert.equal(fps.worstFrameMs, 10);
  assert.equal(typeof DebugOverlay, "function");
});

test("rejoin retries with backoff, keeps the last error and can be cancelled", async () => {
  assert.equal(rejoinDelay(0), 500);
  assert.equal(rejoinDelay(1), 1000);
  assert.equal(rejoinDelay(9), 4000);

  const slept: number[] = [];
  const sleep = async (ms: number): Promise<void> => {
    slept.push(ms);
  };
  let calls = 0;
  const attempts: number[] = [];
  const value = await rejoin(
    async () => {
      calls++;
      if (calls < 3) throw new Error(`refused ${calls}`);
      return "joined";
    },
    { sleep, onAttempt: (n) => attempts.push(n) },
  );
  assert.equal(value, "joined");
  assert.deepEqual(slept, [500, 1000, 2000]);
  assert.deepEqual(attempts, [0, 1, 2]);

  await assert.rejects(
    rejoin(
      async () => {
        throw new Error("server gone");
      },
      { sleep, attempts: 2 },
    ),
    /server gone/,
  );
  let stop = false;
  await assert.rejects(
    rejoin(
      async () => {
        stop = true;
        throw new Error("no");
      },
      { sleep, cancelled: () => stop },
    ),
    RejoinCancelled,
  );
});

function hud(): HudStateHandle {
  return {
    hull: 1, hullMax: 1, energy: 1, energyMax: 1, boost: 1, boostMax: 1, scrap: 0, scrapMax: 1,
    weapon: "", ammo: "", secondary: "", paint0: 0, paint1: 0, timeLeft: 1, team: 0, alive: true,
    status: "", free: () => {},
  };
}

class Game implements GameHandle {
  readonly isOnline = true;
  readonly shareCode = "LS04";
  readonly player = 1;
  readonly team = 1;
  readonly editCount = 0;
  sends = 3;
  bounds(): Float32Array { return new Float32Array(6); }
  clockMs: number[] = [];
  handlePacket(): string | undefined { return undefined; }
  setClockMs(ms: number): void { this.clockMs.push(ms); }
  frame(): unknown[] { return Array.from({ length: this.sends }, (_, i) => new Uint8Array([i])); }
  localPose(): Float32Array | undefined { return undefined; }
  remoteShips(): Float32Array { return new Float32Array(0); }
  projectiles(): Float32Array { return new Float32Array(0); }
  hud(): HudStateHandle { return hud(); }
  takeFeed(): string[] { return []; }
  debugStats(): Float64Array { return new Float64Array([30, 5, 1, 0, 0.1, 2, 2, 0, 0, 1]); }
  scoreboard(): Float64Array { return new Float64Array([0, 0, 1, 1, 0, 0, 1, 1, 1, 0, 1, 1]); }
  matchInfo(): Float64Array { return new Float64Array([1, 0, 50, 20, 100, 1]); }
  meshNext(): ChunkMeshHandle | undefined { return undefined; }
  takeUnloads(): string[] { return []; }
  pendingChunks(): number { return 0; }
  setViewRadius(): void {}
  entities(): Float32Array { return new Float32Array(0); }
  visuals(): Float32Array { return new Float32Array(0); }
  playerInfo(): Float64Array { return new Float64Array(0); }
  playerNames(): string[] { return []; }
  loadout(): LoadoutHandle { return stubLoadout(); }
  scoreboardNames(): string[] { return []; }
  resetMeshes(): void {}
  free(): void {}
}

/** A link with a send buffer that backs up, like WebSocketTransport. */
class BufferedLink implements PacketLink {
  readonly kind = "webtransport";
  isOpen = true;
  closeReason: string | null = null;
  buffered = 0;
  limit = 2;
  sent: number[] = [];
  droppedPackets = 0;
  send(p: Uint8Array, droppable = false): void {
    if (droppable && this.buffered >= this.limit) {
      this.droppedPackets++;
      return;
    }
    this.buffered++;
    this.sent.push(p[0] ?? -1);
  }
  drain(): Uint8Array[] { return []; }
  close(reason?: string): void { this.isOpen = false; this.closeReason = reason ?? "closed"; }
}

test("session: overlay data, transport name and send backpressure", () => {
  const game = new Game();
  const link = new BufferedLink();
  const s = new GameSession(game, link, NAMES);
  s.clock = () => 1234.5;
  assert.equal(s.transportName, "WebTransport");
  assert.equal(s.debugStats().pingMs, 30);
  assert.equal(s.scoreboard().length, 2);
  assert.equal(s.scoreboard()[1]!.isYou, true);
  assert.equal(s.matchInfo().winner, 0);

  // Inputs and pings are droppable: once the buffer is backed up the rest of
  // the frame is skipped instead of queued, and the count is reported.
  s.frame(1 / 60, {
    thrustForward: 0, thrustRight: 0, thrustUp: 0, roll: 0, lookX: 0, lookY: 0, boost: false,
    firePrimary: false, fireSecondary: false, scavenge: false, overdrive: false,
    toggleHorizonAssist: false, weaponKey: 0, cyclePrimary: 0, cycleSecondary: false, gadget: false, cyclePaintColor: false, toggleShipSelect: false, pointerLocked: true, scoreboard: false, toggleDebug: false, toggleReady: false, cycleTeam: false, toggleSpectate: false,
  });
  assert.deepEqual(link.sent, [0, 1]);
  assert.equal(s.droppedSends, 1);
  assert.deepEqual(game.clockMs, [1234.5], "the ping clock is set before packets are handled");
  link.buffered = 0;
  s.idleFrame(0.25);
  assert.deepEqual(link.sent, [0, 1, 0, 1]);
  assert.equal(s.droppedSends, 2);

  // Disposed sessions answer with empty data instead of touching freed wasm.
  s.dispose();
  assert.deepEqual(s.scoreboard(), []);
  assert.equal(s.matchInfo().over, false);
  assert.equal(s.debugStats().pingMs, null);

  const practice = new GameSession(new Game(), null, NAMES);
  assert.equal(practice.transportName, null);
  assert.equal(practice.droppedSends, null);
});

test("session transport name comes from the link's kind", () => {
  const asLink = (o: object): PacketLink => o as unknown as PacketLink;
  assert.equal(transportLabel(asLink({ kind: "websocket" })), "WebSocket");
  assert.equal(transportLabel(asLink({ kind: "webtransport" })), "WebTransport");
  assert.equal(transportLabel(asLink({})), "offline");
});
