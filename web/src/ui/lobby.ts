// Lobby overlay (M3, plan §3.1): between matches the players fly the next
// level with weapons off (warm-up) while this card shows the next mode and
// world, who is in the room and ready, the team picks and the countdown.
// Enter toggles ready, T cycles the team pick (auto, then each team), O
// toggles spectating the next match; with the mouse released the buttons
// work too. The model is a pure function for Node tests; the class only
// writes the DOM when the model changed.

import type { LobbyState } from "../game/modeState.ts";
import { modeInfo } from "../game/modeState.ts";
import { sideCss, teamName } from "../game/sides.ts";
import { formatTimer } from "./hud.ts";

export interface LobbyRow {
  id: number;
  name: string;
  you: boolean;
  ready: boolean;
  spectator: boolean;
  /** Team pick label ("Auto", "Orange", ...); "" in free-for-all. */
  team: string;
  /** CSS colour of the picked team ("" for auto / free-for-all). */
  color: string;
}

export interface TeamOption {
  value: number;
  label: string;
  color: string;
  selected: boolean;
}

export interface LobbyModel {
  /** "Warm-up" or "Match starting". */
  kicker: string;
  mode: string;
  blurb: string;
  /** "Free-for-all", "2 teams", ... */
  format: string;
  ruleset: string;
  world: string;
  room: string;
  /** Big countdown ("0:08"), "" when there is none. */
  countdown: string;
  countdownLabel: string;
  /** What the room waits for. */
  status: string;
  players: LobbyRow[];
  teamOptions: TeamOption[];
  ownReady: boolean;
  ownSpectator: boolean;
  /** Key hints. */
  keys: string;
}

/** The team pick after `current` (auto = -1, then 0..teams-1, then auto again). */
export function nextTeamPick(current: number, teams: number): number {
  if (teams <= 0) return -1;
  const next = current + 1;
  return next >= teams ? -1 : next;
}

/** Builds the lobby card's model. */
export function lobbyModel(
  lobby: LobbyState,
  own: number,
  names: ReadonlyMap<number, string>,
  secsLeft: number,
): LobbyModel {
  const info = modeInfo(lobby.nextModeId);
  const players: LobbyRow[] = lobby.players.map((p) => ({
    id: p.id,
    name: names.get(p.id) || `Pilot ${p.id + 1}`,
    you: p.id === own,
    ready: p.ready,
    spectator: p.spectator,
    team: lobby.teams > 0 ? (p.team >= 0 ? teamName(p.team) : "Auto") : "",
    color: lobby.teams > 0 && p.team >= 0 ? sideCss(p.team, lobby.teams) : "",
  }));
  // Own row first, then by id.
  players.sort((a, b) => Number(b.you) - Number(a.you) || a.id - b.id);
  const me = lobby.players.find((p) => p.id === own);
  const playing = lobby.players.filter((p) => !p.spectator);
  const ready = playing.filter((p) => p.ready).length;
  const countdown = lobby.phase === "countdown";
  let status: string;
  const missing = lobby.minPlayers - playing.length;
  if (countdown) status = "Get ready: the match starts when the countdown ends";
  else if (missing > 0) status = `Waiting for ${missing} more pilot${missing === 1 ? "" : "s"} (${lobby.minPlayers} needed)`;
  else if (ready < playing.length) status = `${ready} of ${playing.length} ready · starts when everyone is ready${secsLeft > 0 ? " or the timer runs out" : ""}`;
  else status = "Everyone is ready";
  const format = lobby.teams > 0 ? `${lobby.teams} teams` : "Free-for-all";
  const ownTeam = me?.team ?? -1;
  const teamOptions: TeamOption[] =
    lobby.teams > 0
      ? [-1, ...Array.from({ length: lobby.teams }, (_, t) => t)].map((t) => ({
          value: t,
          label: t < 0 ? "Auto" : teamName(t),
          color: t < 0 ? "" : sideCss(t, lobby.teams),
          selected: t === ownTeam,
        }))
      : [];
  const keys = ["Enter ready", ...(lobby.teams > 0 ? ["T team"] : []), "O spectate", "Esc frees the mouse"].join(" · ");
  return {
    kicker: countdown ? "Match starting" : "Warm-up",
    mode: info.name,
    blurb: info.blurb,
    format,
    ruleset: lobby.nextRuleset,
    world: lobby.nextShareCode,
    room: lobby.roomCode,
    countdown: secsLeft > 0 ? formatTimer(secsLeft) : "",
    countdownLabel: secsLeft > 0 ? (countdown ? "Starts in" : "Countdown in") : "",
    status,
    players,
    teamOptions,
    ownReady: me?.ready === true,
    ownSpectator: me?.spectator === true,
    keys,
  };
}

export interface LobbyActions {
  ready(ready: boolean): void;
  team(team: number): void;
  spectate(spectate: boolean): void;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, parent?: HTMLElement): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  parent?.appendChild(e);
  return e;
}

/** The lobby card, docked at the left so the warm-up view stays free. */
export class LobbyOverlay {
  readonly root: HTMLElement;
  private readonly kicker: HTMLElement;
  private readonly mode: HTMLElement;
  private readonly blurb: HTMLElement;
  private readonly chips: HTMLElement;
  private readonly countdown: HTMLElement;
  private readonly countdownValue: HTMLElement;
  private readonly countdownLabel: HTMLElement;
  private readonly status: HTMLElement;
  private readonly list: HTMLElement;
  private readonly teams: HTMLElement;
  private readonly readyBtn: HTMLButtonElement;
  private readonly spectateBtn: HTMLButtonElement;
  private readonly keys: HTMLElement;
  private model: LobbyModel | null = null;
  private last = "";

  constructor(parent: HTMLElement, actions: LobbyActions) {
    this.root = el("div", "lobby hidden", parent);
    const card = el("div", "lobby-card", this.root);
    const head = el("div", "lobby-head", card);
    const titles = el("div", "lobby-titles", head);
    this.kicker = el("div", "lobby-kicker", titles);
    this.mode = el("h2", "lobby-mode", titles);
    this.countdown = el("div", "lobby-countdown", head);
    this.countdownValue = el("div", "lobby-countdown-value", this.countdown);
    this.countdownLabel = el("div", "lobby-countdown-label", this.countdown);
    this.blurb = el("p", "lobby-blurb", card);
    this.chips = el("div", "lobby-chips", card);
    this.status = el("div", "lobby-status", card);
    this.list = el("ul", "lobby-players", card);
    this.teams = el("div", "lobby-teams", card);
    const actionsRow = el("div", "lobby-actions", card);
    this.readyBtn = el("button", "lobby-btn ready", actionsRow);
    this.readyBtn.type = "button";
    this.readyBtn.addEventListener("click", () => actions.ready(!(this.model?.ownReady ?? false)));
    this.spectateBtn = el("button", "lobby-btn ghost", actionsRow);
    this.spectateBtn.type = "button";
    this.spectateBtn.addEventListener("click", () => actions.spectate(!(this.model?.ownSpectator ?? false)));
    this.teams.addEventListener("click", (ev) => {
      const target = ev.target instanceof HTMLElement ? ev.target.closest<HTMLElement>("[data-team]") : null;
      const v = Number(target?.dataset["team"]);
      if (target && Number.isInteger(v)) actions.team(v);
    });
    this.keys = el("div", "lobby-keys", card);
  }

  get current(): LobbyModel | null {
    return this.model;
  }

  show(visible: boolean): void {
    this.root.classList.toggle("hidden", !visible);
  }

  get visible(): boolean {
    return !this.root.classList.contains("hidden");
  }

  update(model: LobbyModel): void {
    this.model = model;
    const key = JSON.stringify(model);
    if (key === this.last) return;
    this.last = key;
    this.root.classList.toggle("countdown", model.kicker === "Match starting");
    this.kicker.textContent = model.kicker;
    this.mode.textContent = model.mode;
    this.blurb.textContent = model.blurb;
    this.blurb.classList.toggle("hidden", model.blurb === "");
    this.countdown.classList.toggle("hidden", model.countdown === "");
    this.countdownValue.textContent = model.countdown;
    this.countdownLabel.textContent = model.countdownLabel;
    const chip = (label: string, value: string): HTMLElement => {
      const c = el("span", "lobby-chip");
      el("span", "lobby-chip-label", c).textContent = label;
      el("span", "lobby-chip-value", c).textContent = value;
      return c;
    };
    const chips = [chip("Format", model.format)];
    if (model.world) chips.push(chip("World", model.world));
    if (model.ruleset) chips.push(chip("Rules", model.ruleset));
    if (model.room) chips.push(chip("Room", model.room));
    this.chips.replaceChildren(...chips);
    this.status.textContent = model.status;
    this.list.replaceChildren(
      ...model.players.map((p) => {
        const li = el("li", `lobby-player${p.you ? " you" : ""}${p.ready ? " ready" : ""}${p.spectator ? " spectator" : ""}`);
        el("span", "lobby-check", li).textContent = p.spectator ? "◌" : p.ready ? "✓" : "";
        el("span", "lobby-name", li).textContent = p.you ? `${p.name} (you)` : p.name;
        const team = el("span", "lobby-team", li);
        team.textContent = p.spectator ? "Spectating" : p.team;
        if (p.color) team.style.setProperty("--side", p.color);
        team.classList.toggle("picked", p.color !== "");
        return li;
      }),
    );
    this.teams.replaceChildren(
      ...model.teamOptions.map((t) => {
        const b = el("button", `lobby-team-btn${t.selected ? " selected" : ""}`);
        b.type = "button";
        b.dataset["team"] = String(t.value);
        if (t.color) b.style.setProperty("--side", t.color);
        b.textContent = t.label;
        return b;
      }),
    );
    this.teams.classList.toggle("hidden", model.teamOptions.length === 0);
    this.readyBtn.textContent = model.ownReady ? "Ready ✓" : "Ready up";
    this.readyBtn.classList.toggle("on", model.ownReady);
    this.readyBtn.disabled = model.ownSpectator;
    this.spectateBtn.textContent = model.ownSpectator ? "Play next match" : "Spectate";
    this.keys.textContent = model.keys;
  }

  dispose(): void {
    this.root.remove();
  }
}
