// The subset of the ls-wasm API the client uses, as structural interfaces.
// `load.ts` checks the generated bindings (./pkg, built by `npm run build:wasm`)
// against these at compile time; the game glue depends only on the interfaces,
// so Node unit tests can drive it with fakes.
//
// Flat array layouts (see crates/ls-wasm/src/lib.rs):
//   localPose():   [px, py, pz, qx, qy, qz, qw, vx, vy, vz, alive]
//   remoteShips(): SHIP_STRIDE floats each  [id, team, alive, boosting, px, py, pz, qx, qy, qz, qw,
//                                            shipType, radius, gadgetActive]
//   projectiles(): PROJECTILE_STRIDE floats [kind, owner, px, py, pz, vx, vy, vz, visual, state]
//   entities():    ENTITY_STRIDE floats     [id, kind, visual, owner, team, px, py, pz, vx, vy, vz,
//                                            qx, qy, qz, qw, state0, state1, state2, state3, radius (0 = unknown)]
//   visuals():     VISUAL_STRIDE floats per visual id [shapeIndex (into visualShapeNames()), r, g, b (0..1), scale]
//   playerInfo():  PLAYER_STRIDE numbers each [id, team, shipType, gadget (255 = none)]; names: playerNames()
//   frame() controls: CONTROLS_LEN floats   [forward, right, up, rollRight, yawRight, pitchUp, buttons, weaponSelect,
//                                            weaponKey (1-10), cyclePrimary (±1), cycleSecondary (+1)]
//   debugStats():  DEBUG_STATS_LEN numbers  [pingMs (-1 = unknown), accepted, corrected, snapped, lastError,
//                                            shotsPredicted, shotsConfirmed, shotsDropped, shotsUnpredicted, synced]
//   scoreboard():  SCORE_STRIDE numbers each [id, team, alive, kills, deaths, isYou]; names: scoreboardNames()
//   matchInfo():   MATCH_INFO_LEN numbers   [over, winner (-1 = draw/none), team0Cells, team1Cells, surfaceCells, secsSinceEnd]
//   loadout():     names[i], ammo[i], maxLevels[i] and LOADOUT_INFO_STRIDE bytes info[4i..4i+4] =
//                  [slot (0 primary, 1 secondary), selected, key (1-10, 0 none), level], primaries first
//   takeCues():    CUE_STRIDE numbers each  [kind (ls_sim::CueKind), source, target (255 = none), px, py, pz, value, tick];
//                  kind 250 is the client-only hit cue (game/vfx.ts CUE_HIT: attacker, victim, impact point, damage)
//   charge():      CHARGE_LEN numbers or empty [weapon, fraction, heldSecs, fullSecs, minSecs, autoRelease, overcharge, 1]
//   shipChoices(): JSON array of {shipType, name, allowed, current, next, hull, topSpeedPct, damageMult,
//                  secondarySlots, radius, turnRate, canPassScrap, primaries, secondaries: [[name, count]], gadget}

export const POSE_LEN = 11;
export const SHIP_STRIDE = 14;
export const PROJECTILE_STRIDE = 10;
export const ENTITY_STRIDE = 20;
export const VISUAL_STRIDE = 5;
export const PLAYER_STRIDE = 4;
export const CONTROLS_LEN = 11;
export const DEBUG_STATS_LEN = 10;
export const SCORE_STRIDE = 6;
export const MATCH_INFO_LEN = 6;
export const LOADOUT_INFO_STRIDE = 4;
export const CUE_STRIDE = 8;
export const CHARGE_LEN = 8;

export interface ChunkMeshHandle {
  readonly key: string;
  positions(): Float32Array;
  normals(): Float32Array;
  indices(): Uint32Array;
  teams(): Uint8Array;
  /** Surface state bits per vertex (hot 1, charged 2, glow 4, scorch 8), or empty when the terrain has none. */
  surface?(): Uint8Array;
  /** Rock material per vertex (0 weak, 1 rock, 2 hard), or empty. */
  materials?(): Uint8Array;
  /** Display blend weights of weak and hard rock, two per vertex (0..255), or empty. */
  materialWeights?(): Uint8Array;
  /** Paint effect colour per vertex (0 plain, 1 red, 2 blue, 3 yellow), or empty. */
  paintColors?(): Uint8Array;
  /** Baked ambient occlusion per vertex (255 = open, 0 = fully occluded), or empty. */
  ao?(): Uint8Array;
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

export interface LoadoutHandle {
  readonly names: string[];
  readonly ammo: string[];
  readonly info: Uint8Array;
  readonly maxLevels: Uint8Array;
  readonly gadget: string;
  readonly gadgetActive: boolean;
  free(): void;
}

export interface GadgetHandle {
  readonly id: number;
  readonly key: string;
  readonly name: string;
  readonly active: boolean;
  readonly toggle: boolean;
  readonly cooldownLeft: number;
  readonly cooldownSecs: number;
  readonly energyCost: number;
  readonly drainPerSec: number;
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
  /** Clock (ms) that times pings; set once per frame before handling packets. */
  setClockMs(ms: number): void;
  frame(dt: number, controls: Float32Array): unknown[];
  localPose(): Float32Array | undefined;
  remoteShips(): Float32Array;
  projectiles(): Float32Array;
  entities(): Float32Array;
  /** The match's visual table (constant for a match). */
  visuals(): Float32Array;
  playerInfo(): Float64Array;
  playerNames(): string[];
  loadout(): LoadoutHandle;
  hud(): HudStateHandle;
  takeFeed(): string[];
  debugStats(): Float64Array;
  scoreboard(): Float64Array;
  scoreboardNames(): string[];
  matchInfo(): Float64Array;
  meshNext(x: number, y: number, z: number): ChunkMeshHandle | undefined;
  takeUnloads(x: number, y: number, z: number): string[];
  pendingChunks(x: number, y: number, z: number): number;
  setViewRadius(r: number): void;
  resetMeshes(): void;
  // Mechanics UI (M2 wave C); optional so older fakes keep working.
  /** Game cues since the last call (CUE_STRIDE numbers each). */
  takeCues?(): Float64Array;
  /** Current simulation tick (the cues' clock). */
  nowTick?(): number;
  /** Charge in progress (CHARGE_LEN numbers) or empty. */
  charge?(): Float64Array;
  /** The own Leech Arc beam drawn locally online: [from x, y, z, to x, y, z, target (255 = none)] or empty (VQP). */
  localBeam?(): Float32Array;
  /** Visual id (index into visuals()) of a weapon's projectiles (VQP). */
  projectileVisual?(weapon: number): number;
  /** The own gadget. */
  gadget?(): GadgetHandle;
  /** Ship types with loadout preview (JSON). */
  shipChoices?(): string;
  /** Picks the ship type for the next respawn; online returns the SelectShip packet to send reliably. */
  selectShip?(shipType: number): Uint8Array | undefined;
  /** Own Paint Thrower colour (1 red, 2 blue, 3 yellow) while it is the selected primary, else 0 (V sends `BUTTONS.weaponMode`). */
  readonly paintColor?: number;
  /** Seconds until the own weapons are back online after a Borrowed Time rewind (0 = online). */
  readonly weaponsOfflineSecs?: number;
  // Modes and lobby (M3, protocol 5); optional so older fakes keep working.
  /** Mode state JSON (game/modeState.ts `parseModeState`). */
  modeState?(): string;
  /** Lobby state JSON, `undefined` before the server sent one (and offline). */
  lobbyState?(): string | undefined;
  /** Match results JSON, `undefined` until the match ended (online only). */
  results?(): string | undefined;
  /** True once after each match transition (a new Welcome on the open connection). */
  takeMatchTransition?(): boolean;
  /** Match transitions handled so far. */
  readonly transitions?: number;
  /** Room code of the match (empty: the public room or practice). */
  readonly roomCode?: string;
  /** `Ready` packet (send reliably; `undefined` offline). */
  readyPacket?(ready: boolean): Uint8Array | undefined;
  /** `PickTeam` packet (`team` < 0 = auto-balance). */
  pickTeamPacket?(team: number): Uint8Array | undefined;
  /** `Spectate` packet (spectate from the next match on, or play again). */
  spectatePacket?(spectate: boolean): Uint8Array | undefined;
  // VQ1 scale cues (ls-wasm probe.rs); optional so older fakes keep working.
  /** Hitbox radius of the own ship type (u). */
  readonly shipRadius?: number;
  /** Distance to the first rock (or the level bounds) along a ray, -1 for none within `max`. */
  raycast?(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, max: number): number;
  /** First contact of a swept sphere: [travel, contact xyz, normal xyz], empty when clear within `max`. */
  sweepSphere?(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, radius: number, max: number): Float32Array;
  /** Practice levels built lazily (`Game.practicePreview`): generates the missing chunk nearest the point; returns how many are left. */
  generateNearest?(x: number, y: number, z: number): number;
  /** Practice only (visual gallery test shapes): carve (`add` false) or add a sphere of rock; false online. */
  practiceEdit?(add: boolean, x: number, y: number, z: number, radius: number): boolean;
  /** Practice only (gallery, M3 C1): The Squeeze's shell on the terrain; a radius below 0 removes it. */
  practiceShell?(x: number, y: number, z: number, radius: number): boolean;
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
  /** Hello asking for a ship type (ls-sim ShipType index). */
  helloPacketAs?(name: string, preferredTeam: number, shipType: number): Uint8Array;
  /** Gadget string ids in id order (players' `gadget`). */
  gadgetIds?(): string[];
  /** Mode ids in ModeKind index order (`kind` of the mode state). */
  modeIds?(): string[];
  /** `Disconnect` reason of a server that starts a new match (rejoin, not an error). */
  matchRestartReason(): string;
  weaponIds(): string[];
  weaponNames(): string[];
  /** Visual shape names in shape-index order (first number of each `visuals()` row). */
  visualShapeNames(): string[];
  /** Entity kind names in wire-byte order (`kind` of `entities()`). */
  entityKindNames(): string[];
  goldenHashes(code: string): string[];
  tickHz(): number;
  Game: {
    fromWelcome(packet: Uint8Array): GameHandle;
    practice(code: string, dummies: number): GameHandle;
    practiceAs?(code: string, dummies: number, shipType: number): GameHandle;
    /** A practice level for looking at: no dummies, chunks generated lazily (the FI backdrop). */
    practicePreview?(code: string): GameHandle;
  };
}
