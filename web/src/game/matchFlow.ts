// What the client shows around a match (M3 lobby, plan §3.1): the lobby
// overlay during warm-up and countdown, the results screen after a match,
// and the spectator camera while the own player is out. Pure functions of
// the session state (no DOM), so Node tests cover every phase.
//
// The server sends `LobbyState` only when something changes; the countdown
// in it (`phaseTicksLeft`) is counted down locally from the moment the state
// arrived (LobbyClock).

import { isOut, ownModePlayer, type LobbyState, type MatchResults, type ModeState } from "./modeState.ts";

/** Lobby ticks per second (sim rate). */
export const LOBBY_HZ = 60;

export type FlowStage = "warmup" | "countdown" | "match" | "results";

export interface FlowInput {
  online: boolean;
  lobby: LobbyState | null;
  mode: ModeState;
  results: MatchResults | null;
  /** Own player / ship id. */
  own: number;
}

export interface FlowView {
  stage: FlowStage;
  /** Lobby overlay (warm-up, countdown). */
  showLobby: boolean;
  /** Results screen (the match ended). */
  showResults: boolean;
  /** No own ship: the spectator camera flies. */
  spectating: boolean;
}

/** Decides which overlays show and whether the own player spectates. */
export function matchFlow(i: FlowInput): FlowView {
  const phase = i.lobby?.phase ?? "match";
  const ended = !i.mode.running;
  let stage: FlowStage;
  if (phase === "lobby") stage = "warmup";
  else if (phase === "countdown") stage = "countdown";
  else if (phase === "results" || ended) stage = "results";
  else stage = "match";
  // Results: the server's table online; in practice the local mode state.
  const showResults = stage === "results" && (i.results !== null || !i.online);
  const own = ownModePlayer(i.mode, i.own);
  const lobbySpectator = i.lobby?.players.find((p) => p.id === i.own)?.spectator === true;
  const spectating = isOut(own) || (stage === "match" && lobbySpectator && own === undefined);
  return { stage, showLobby: stage === "warmup" || stage === "countdown", showResults, spectating };
}

/**
 * Counts a lobby phase's remaining time down between `LobbyState` updates:
 * every new state (any field changed) restarts the count from its
 * `phaseTicksLeft`.
 */
export class LobbyClock {
  private key = "";
  private baseMs = 0;
  private baseTicks = 0;

  /** Seconds left in the lobby phase at `nowMs`; 0 = open-ended or over. */
  secsLeft(lobby: LobbyState | null, nowMs: number): number {
    if (!lobby) {
      this.key = "";
      return 0;
    }
    const key = JSON.stringify(lobby);
    if (key !== this.key) {
      this.key = key;
      this.baseMs = nowMs;
      this.baseTicks = lobby.phaseTicksLeft;
    }
    if (!(this.baseTicks > 0)) return 0;
    return Math.max(0, this.baseTicks / LOBBY_HZ - (nowMs - this.baseMs) / 1000);
  }

  /** Forget the last state (a new match). */
  reset(): void {
    this.key = "";
  }
}
