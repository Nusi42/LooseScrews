// The generic mode state, the lobby state and the match results (protocol 5,
// M3) as plain objects. ls-wasm hands them over as JSON (`Game.modeState()`,
// `Game.lobbyState()`, `Game.results()`, layouts in crates/ls-wasm/src/modes.rs);
// the parsers here tolerate missing or broken fields, so the HUD never
// throws on an older or newer core. Constants mirror ls_sim::mode.

import type { Vec3 } from "../math/vec3.ts";

/** `ls_sim::mode::objective_kind`. */
export const OBJECTIVE = {
  safeZone: 0,
  nextZone: 1,
  core: 2,
  extraction: 3,
  reactor: 4,
  /** The Architect: the exit gate (holder = the round's architect; state per `ARCHITECT_EXIT`, game/architect/model.ts). */
  exit: 5,
  /** The Architect: one stroke (state = stroke kind | phase << 4; holder = the architect). */
  stroke: 6,
} as const;

/** `ls_sim::mode::player_flags`. */
export const PLAYER_FLAG = {
  eliminated: 1 << 0,
  spectator: 1 << 1,
  carrier: 1 << 2,
  /** The Architect: this player is the round's architect (no ship). */
  architect: 1 << 3,
  /** The Architect: a runner through the exit (out until the next round). */
  finished: 1 << 4,
  /** Belt Up: held in place (start lights, a death hold at a gate, heat results). */
  held: 1 << 5,
} as const;

/** `ls_sim::mode::view_flags`. */
export const VIEW_FLAG = {
  instagib: 1 << 0,
  monochrome: 1 << 1,
  paintScores: 1 << 2,
  lives: 1 << 3,
  meter: 1 << 4,
} as const;

export interface ModePlayer {
  id: number;
  /** Team index, or the player's own side id in free-for-all. */
  side: number;
  score: number;
  /** Lives left, -1 = no limit. */
  lives: number;
  /** Mode meter (Last Color Standing colour, 0-100). */
  meter: number;
  flags: number;
}

export interface Objective {
  id: number;
  kind: number;
  /** Owning team, -1 = none. */
  team: number;
  /** Carrier / holder ship id, -1 = none. */
  holder: number;
  state: number;
  /** Timer tick (0 = none), e.g. the reactor's detonation. */
  timerEnd: number;
  radius: number;
  pos: Vec3;
}

/** Belt Up's heat phases (`ls_sim::modes::belt_up::heat_phase`). */
export type RacePhase = "lights" | "racing" | "results" | "done";

/** One racer of a Belt Up heat (`ls_net::NetRacer`). */
export interface Racer {
  id: number;
  /** Gates passed this heat (8 = the lap is done: the exit is open). */
  gates: number;
  /** Next gate (0-7), -1 once the lap is done. */
  nextGate: number;
  /** Tick it reached the exit, 0 = not finished. */
  finishTick: number;
  /** Live place (final after the heat), 0 = none. */
  place: number;
  /** Held at a gate until this tick after a death, 0 = not held. */
  holdEndTick: number;
  /** Start slot this heat (0-7). */
  slot: number;
}

/** Belt Up's race (`ls_net::NetRaceView`; null in other modes). */
export interface RaceState {
  /** Current heat (0-based) of `heats`. */
  heat: number;
  heats: number;
  phase: RacePhase;
  /** Tick the phase ends (start lights, heat limit, results). */
  phaseEndTick: number;
  /** Racers of this heat, by id. */
  racers: Racer[];
}

/** Mood ids by index (ls_sim moods, ls-wasm MOOD_IDS). */
export const MOOD_IDS = ["calm", "bored", "grumpy", "giddy"] as const;
export type MoodId = (typeof MOOD_IDS)[number];

/**
 * The comedy director's state and the level's up (protocol 6 `MoodView`,
 * ls-wasm modes.rs `mood_json`).
 */
export interface MoodState {
  mood: MoodId;
  /** Running event id (`data/events/<id>.ron`), "" for none. */
  eventId: string;
  /** "announce" | "active" | "ending" | "" without an event. */
  phase: string;
  /** Player the event names, -1 = none. */
  target: number;
  activeTick: number;
  phaseEndTick: number;
  params: number[];
  /** The level's up (unit; +Y unless Bored level turned it). */
  up: Vec3;
}

export function emptyMood(): MoodState {
  return { mood: "calm", eventId: "", phase: "", target: -1, activeTick: 0, phaseEndTick: 0, params: [], up: { x: 0, y: 1, z: 0 } };
}

/** Parses the `mood` object of the mode state (missing or broken: calm, no event, up +Y). */
export function parseMood(v: unknown): MoodState {
  const m = obj(v);
  const id = MOOD_IDS.find((x) => x === m["moodId"]) ?? MOOD_IDS[num(m["mood"])] ?? "calm";
  const up = list(m["up"]).map((x) => num(x));
  const len = Math.hypot(up[0] ?? 0, up[1] ?? 0, up[2] ?? 0);
  return {
    mood: id,
    eventId: str(m["eventId"]),
    phase: str(m["phaseId"]),
    target: num(m["target"], -1),
    activeTick: num(m["activeTick"]),
    phaseEndTick: num(m["phaseEndTick"]),
    params: list(m["params"]).map((x) => num(x)),
    up: len > 0.5 ? { x: (up[0] ?? 0) / len, y: (up[1] ?? 0) / len, z: (up[2] ?? 0) / len } : { x: 0, y: 1, z: 0 },
  };
}

export interface ModeState {
  kind: number;
  /** Mode id (`team_brawl`, ...); "" when unknown. */
  mode: string;
  /** 0 = free-for-all, else 2-4. */
  teams: number;
  running: boolean;
  /** Winning side, -1 = none / draw. */
  winner: number;
  endTick: number;
  stage: number;
  stageEndTick: number;
  scoreTotal: number;
  scoreTarget: number;
  flags: number;
  teamScores: number[];
  /** Best first. */
  players: ModePlayer[];
  objectives: Objective[];
  /** Belt Up's race, null in other modes. */
  race: RaceState | null;
  /** Mood and comedy event (protocol 6; calm and no event from older cores). */
  mood: MoodState;
}

export type LobbyPhase = "lobby" | "countdown" | "match" | "results";

export interface LobbyPlayer {
  id: number;
  /** Picked team, -1 = auto. */
  team: number;
  ready: boolean;
  spectator: boolean;
}

export interface LobbyState {
  phase: LobbyPhase;
  /** Ticks (60 Hz) until the phase ends, 0 = open-ended. */
  phaseTicksLeft: number;
  roomCode: string;
  nextMode: number;
  nextModeId: string;
  nextRuleset: string;
  nextShareCode: string;
  minPlayers: number;
  maxPlayers: number;
  teams: number;
  players: LobbyPlayer[];
}

export interface ResultRow {
  id: number;
  side: number;
  kills: number;
  deaths: number;
  score: number;
  name: string;
}

/** A title a comedy event gave a player (M4 B3: Hat rain's Hat Baron). */
export interface ResultAward {
  /** Event id (`data/events/<id>.ron`), e.g. "hat_rain". */
  event: string;
  /** Player id. */
  player: number;
  /** Event-defined value (hats worn). */
  value: number;
}

export interface MatchResults {
  tick: number;
  kind: number;
  mode: string;
  teams: number;
  /** Winning side, -1 = draw. */
  winner: number;
  teamScores: number[];
  /** Best first. */
  rows: ResultRow[];
  /** Titles from comedy events, in award order (empty in older servers' results). */
  awards: ResultAward[];
}

/** Display data per mode id. */
export interface ModeInfo {
  name: string;
  /** One line for the lobby card. */
  blurb: string;
  /** What `score` counts, for table headers. */
  scoreLabel: string;
  /** What a stage is called (The Squeeze: "Phase"); "" = stages are not shown. */
  stageLabel: string;
}

export const MODE_INFO: Readonly<Record<string, ModeInfo>> = {
  paint_the_void: {
    name: "Paint the Void",
    blurb: "Paint the most rock in your team's colour before the clock runs out.",
    scoreLabel: "Paint",
    stageLabel: "",
  },
  team_brawl: {
    name: "Team Brawl",
    blurb: "Destroy enemy ships. The first side to the kill target wins.",
    scoreLabel: "Score",
    stageLabel: "",
  },
  the_squeeze: {
    name: "The Squeeze",
    blurb: "One life. The walls close in phase by phase: stay inside the safe zone and be the last ship flying.",
    scoreLabel: "Score",
    stageLabel: "Phase",
  },
  heist: {
    name: "Heist",
    blurb: "Dig the core out of the deep vein and carry it to the surface. The carrier cannot boost.",
    scoreLabel: "Deliveries",
    stageLabel: "",
  },
  hot_reactor: {
    name: "Hot Reactor",
    blurb: "Pass the reactor on with a hit before it blows. Whoever holds it at detonation loses a life.",
    scoreLabel: "Score",
    stageLabel: "",
  },
  last_color_standing: {
    name: "Last Color Standing",
    blurb: "Every death drains your colour. The last pilot with any colour left wins.",
    scoreLabel: "Score",
    stageLabel: "",
  },
  the_architect: {
    name: "The Architect",
    blurb: "One player reshapes the level live; everyone else races for the exit. The architect role rotates every round.",
    scoreLabel: "Points",
    stageLabel: "Round",
  },
  belt_up: {
    name: "Belt Up",
    blurb: "Race one lap round the asteroid belt and dive to the EXIT. The belt shields you from radiation; open space is fast but burns hull.",
    scoreLabel: "Points",
    stageLabel: "Heat",
  },
};

/** Display data of a mode id (a generic entry for unknown ids). */
export function modeInfo(id: string): ModeInfo {
  return (
    MODE_INFO[id] ?? {
      name: id ? id.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase()) : "Match",
      blurb: "",
      scoreLabel: "Score",
      stageLabel: "",
    }
  );
}

function num(v: unknown, fallback = 0): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}

function list(v: unknown): unknown[] {
  return Array.isArray(v) ? v : [];
}

function obj(v: unknown): Record<string, unknown> {
  return v !== null && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
}

function parseJson(text: string | undefined | null): Record<string, unknown> | null {
  if (typeof text !== "string" || text === "") return null;
  try {
    const v: unknown = JSON.parse(text);
    return v !== null && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/** The mode state of a running match with no information (offline cores without the API). */
export function emptyModeState(): ModeState {
  return {
    kind: 0,
    mode: "",
    teams: 2,
    running: true,
    winner: -1,
    endTick: 0,
    stage: 0,
    stageEndTick: 0,
    scoreTotal: 0,
    scoreTarget: 0,
    flags: 0,
    teamScores: [],
    players: [],
    objectives: [],
    race: null,
    mood: emptyMood(),
  };
}

const RACE_PHASES: readonly RacePhase[] = ["lights", "racing", "results", "done"];

/** The `race` object of `Game.modeState()` (null when absent or broken). */
export function parseRace(v: unknown): RaceState | null {
  if (v === null || typeof v !== "object" || Array.isArray(v)) return null;
  const r = v as Record<string, unknown>;
  const phase = RACE_PHASES[num(r["phase"], -1)] ?? "racing";
  return {
    heat: num(r["heat"]),
    heats: num(r["heats"], 1),
    phase,
    phaseEndTick: num(r["phaseEndTick"]),
    racers: list(r["racers"]).map((x) => {
      const o = obj(x);
      return {
        id: num(o["id"]),
        gates: num(o["gates"]),
        nextGate: num(o["nextGate"], -1),
        finishTick: num(o["finishTick"]),
        place: num(o["place"]),
        holdEndTick: num(o["holdEndTick"]),
        slot: num(o["slot"]),
      };
    }),
  };
}

/** Parses `Game.modeState()`; broken input gives `emptyModeState()`. */
export function parseModeState(text: string | undefined | null): ModeState {
  const j = parseJson(text);
  if (!j) return emptyModeState();
  return {
    kind: num(j["kind"]),
    mode: str(j["mode"]),
    teams: num(j["teams"], 2),
    running: j["running"] !== false,
    winner: num(j["winner"], -1),
    endTick: num(j["endTick"]),
    stage: num(j["stage"]),
    stageEndTick: num(j["stageEndTick"]),
    scoreTotal: num(j["scoreTotal"]),
    scoreTarget: num(j["scoreTarget"]),
    flags: num(j["flags"]),
    teamScores: list(j["teamScores"]).map((v) => num(v)),
    players: list(j["players"]).map((v) => {
      const p = obj(v);
      return {
        id: num(p["id"]),
        side: num(p["side"]),
        score: num(p["score"]),
        lives: num(p["lives"], -1),
        meter: num(p["meter"]),
        flags: num(p["flags"]),
      };
    }),
    objectives: list(j["objectives"]).map((v) => {
      const o = obj(v);
      const pos = list(o["pos"]);
      return {
        id: num(o["id"]),
        kind: num(o["kind"]),
        team: num(o["team"], -1),
        holder: num(o["holder"], -1),
        state: num(o["state"]),
        timerEnd: num(o["timerEnd"]),
        radius: num(o["radius"]),
        pos: { x: num(pos[0]), y: num(pos[1]), z: num(pos[2]) },
      };
    }),
    race: parseRace(j["race"]),
    mood: parseMood(j["mood"]),
  };
}

const PHASES: readonly LobbyPhase[] = ["lobby", "countdown", "match", "results"];

/** Parses `Game.lobbyState()`; null when absent or broken. */
export function parseLobbyState(text: string | undefined | null): LobbyState | null {
  const j = parseJson(text);
  if (!j) return null;
  const phase = PHASES.find((p) => p === j["phase"]);
  if (!phase) return null;
  return {
    phase,
    phaseTicksLeft: num(j["phaseTicksLeft"]),
    roomCode: str(j["roomCode"]),
    nextMode: num(j["nextMode"]),
    nextModeId: str(j["nextModeId"]),
    nextRuleset: str(j["nextRuleset"]),
    nextShareCode: str(j["nextShareCode"]),
    minPlayers: num(j["minPlayers"]),
    maxPlayers: num(j["maxPlayers"]),
    teams: num(j["teams"]),
    players: list(j["players"]).map((v) => {
      const p = obj(v);
      return { id: num(p["id"]), team: num(p["team"], -1), ready: p["ready"] === true, spectator: p["spectator"] === true };
    }),
  };
}

/** Parses `Game.results()`; null when absent or broken. */
export function parseResults(text: string | undefined | null): MatchResults | null {
  const j = parseJson(text);
  if (!j) return null;
  return {
    tick: num(j["tick"]),
    kind: num(j["kind"]),
    mode: str(j["mode"]),
    teams: num(j["teams"]),
    winner: num(j["winner"], -1),
    teamScores: list(j["teamScores"]).map((v) => num(v)),
    rows: list(j["rows"]).map((v) => {
      const r = obj(v);
      return {
        id: num(r["id"]),
        side: num(r["side"]),
        kills: num(r["kills"]),
        deaths: num(r["deaths"]),
        score: num(r["score"]),
        name: str(r["name"]),
      };
    }),
    awards: list(j["awards"]).map((v) => {
      const a = obj(v);
      return { event: str(a["event"]), player: num(a["player"]), value: num(a["value"]) };
    }),
  };
}

/** The own row of the mode state, if listed. */
export function ownModePlayer(m: ModeState, own: number): ModePlayer | undefined {
  return m.players.find((p) => p.id === own);
}

/**
 * Out of play (eliminated, spectating, The Architect's architect without a
 * ship or a runner through the exit): no own ship to fly; the camera is free.
 */
export function isOut(p: ModePlayer | undefined): boolean {
  return p !== undefined && (p.flags & (PLAYER_FLAG.eliminated | PLAYER_FLAG.spectator | PLAYER_FLAG.architect | PLAYER_FLAG.finished)) !== 0;
}

/** Seconds from `nowTick` to `tick` at `hz` (0 when the tick is 0 or past). */
export function secsUntil(tick: number, nowTick: number, hz = 60): number {
  if (!(tick > 0)) return 0;
  return Math.max(0, (tick - nowTick) / hz);
}
