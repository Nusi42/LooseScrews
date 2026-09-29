// A neutral GameHandle (empty state, no packets) that Node tests extend and
// override, so each fake only spells out what its test needs.

import type { ChunkMeshHandle, GameHandle, HudStateHandle, LoadoutHandle } from "./api.ts";

export function stubHud(over: Partial<HudStateHandle> = {}): HudStateHandle {
  return {
    hull: 1, hullMax: 1, energy: 1, energyMax: 1, boost: 1, boostMax: 1, scrap: 0, scrapMax: 1,
    weapon: "", ammo: "", secondary: "", paint0: 0, paint1: 0, timeLeft: 1, team: 0, alive: true,
    status: "", free: () => {}, ...over,
  };
}

export function stubLoadout(over: Partial<LoadoutHandle> = {}): LoadoutHandle {
  return {
    names: [], ammo: [], info: new Uint8Array(0), maxLevels: new Uint8Array(0), gadget: "", gadgetActive: false,
    free: () => {}, ...over,
  };
}

export class GameStub implements GameHandle {
  isOnline = true;
  shareCode = "LS04";
  player = 0;
  team = 0;
  editCount = 0;
  bounds(): Float32Array { return new Float32Array(6); }
  handlePacket(_packet: Uint8Array): string | undefined { return undefined; }
  setClockMs(_ms: number): void {}
  frame(_dt: number, _controls: Float32Array): unknown[] { return []; }
  localPose(): Float32Array | undefined { return undefined; }
  remoteShips(): Float32Array { return new Float32Array(0); }
  projectiles(): Float32Array { return new Float32Array(0); }
  entities(): Float32Array { return new Float32Array(0); }
  visuals(): Float32Array { return new Float32Array(0); }
  playerInfo(): Float64Array { return new Float64Array(0); }
  playerNames(): string[] { return []; }
  loadout(): LoadoutHandle { return stubLoadout(); }
  hud(): HudStateHandle { return stubHud(); }
  takeFeed(): string[] { return []; }
  debugStats(): Float64Array { return new Float64Array(10); }
  scoreboard(): Float64Array { return new Float64Array(0); }
  scoreboardNames(): string[] { return []; }
  matchInfo(): Float64Array { return new Float64Array(6); }
  meshNext(_x: number, _y: number, _z: number): ChunkMeshHandle | undefined { return undefined; }
  takeUnloads(_x: number, _y: number, _z: number): string[] { return []; }
  pendingChunks(_x: number, _y: number, _z: number): number { return 0; }
  setViewRadius(_r: number): void {}
  resetMeshes(): void {}
  free(): void {}
}
