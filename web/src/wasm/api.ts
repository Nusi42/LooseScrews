// The subset of the ls-wasm API the client uses, as structural interfaces.
// `load.ts` checks the generated bindings (./pkg, built by `npm run build:wasm`)
// against these at compile time; the game glue depends only on the interfaces,
// so Node unit tests can drive it with fakes.
//
// Flat array layouts (see crates/ls-wasm/src/lib.rs):
//   localPose():   [px, py, pz, qx, qy, qz, qw, vx, vy, vz, alive]
//   remoteShips(): SHIP_STRIDE floats each  [id, team, alive, boosting, px, py, pz, qx, qy, qz, qw]
//   projectiles(): PROJECTILE_STRIDE floats [kind, owner, px, py, pz, vx, vy, vz]
//   frame() controls: CONTROLS_LEN floats   [forward, right, up, rollRight, yawRight, pitchUp, buttons, weaponSelect]

export const POSE_LEN = 11;
export const SHIP_STRIDE = 11;
export const PROJECTILE_STRIDE = 8;
export const CONTROLS_LEN = 8;

export interface ChunkMeshHandle {
  readonly key: string;
  positions(): Float32Array;
  normals(): Float32Array;
  indices(): Uint32Array;
  teams(): Uint8Array;
  aabb(): Float32Array;
  free(): void;
}

export interface HudStateHandle {
  readonly hull: number;
  readonly hullMax: number;
  readonly energy: number;
  readonly energyMax: number;
  readonly boost: number;
  readonly boostMax: number;
  readonly scrap: number;
  readonly scrapMax: number;
  readonly weapon: string;
  readonly ammo: string;
  readonly secondary: string;
  readonly paint0: number;
  readonly paint1: number;
  readonly timeLeft: number;
  readonly team: number;
  readonly alive: boolean;
  readonly status: string;
  free(): void;
}

export interface GameHandle {
  readonly isOnline: boolean;
  readonly shareCode: string;
  readonly player: number;
  readonly team: number;
  readonly editCount: number;
  bounds(): Float32Array;
  handlePacket(packet: Uint8Array): string | undefined;
  frame(dt: number, controls: Float32Array): unknown[];
  localPose(): Float32Array | undefined;
  remoteShips(): Float32Array;
  projectiles(): Float32Array;
  hud(): HudStateHandle;
  takeFeed(): string[];
  meshNext(x: number, y: number, z: number): ChunkMeshHandle | undefined;
  takeUnloads(x: number, y: number, z: number): string[];
  pendingChunks(x: number, y: number, z: number): number;
  setViewRadius(r: number): void;
  resetMeshes(): void;
  free(): void;
}

export interface ShareCodeInfoHandle {
  readonly ok: boolean;
  readonly code: string;
  readonly error: string;
  free(): void;
}

export interface WasmApi {
  checkShareCode(text: string): ShareCodeInfoHandle;
  randomShareCode(entropyLo: number, entropyHi: number, enclosure: number, density: number): string;
  helloPacket(name: string, preferredTeam: number): Uint8Array;
  /** `Disconnect` reason of a server that starts a new match (rejoin, not an error). */
  matchRestartReason(): string;
  weaponIds(): string[];
  weaponNames(): string[];
  goldenHashes(code: string): string[];
  tickHz(): number;
  Game: {
    fromWelcome(packet: Uint8Array): GameHandle;
    practice(code: string, dummies: number): GameHandle;
  };
}
