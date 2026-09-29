// Scoreboard (hold Tab) and the end-of-match screen (team paint scores,
// winner, intermission countdown and the scoreboard). The models are pure
// functions so they can be tested in Node; the classes only write the DOM,
// and only when the model changed.

import {
  intermissionLeft,
  paintPercent,
  playerName,
  rowsByTeam,
  teamName,
  type MatchInfo,
  type ScoreRow,
} from "../game/stats.ts";

export interface TeamLine {
  team: number;
  name: string;
  /** Paint share in percent, one decimal. */
  paint: string;
  winner: boolean;
  yours: boolean;
}

export interface PlayerLine {
  name: string;
  kills: number;
  deaths: number;
  alive: boolean;
  isYou: boolean;
}

export interface ScoreboardModel {
  teams: Array<TeamLine & { players: PlayerLine[] }>;
  /** Footnote on where the numbers come from. */
  note: string;
}

export interface EndScreenModel {
  title: string;
  subtitle: string;
  teams: TeamLine[];
  /** Countdown / what happens next. */
  footer: string;
}

function teamLine(info: MatchInfo, team: number, ownTeam: number): TeamLine {
  return {
    team,
    name: teamName(team),
    paint: `${paintPercent(info, team).toFixed(1)}%`,
    winner: info.over && info.winner === team,
    yours: team === ownTeam,
  };
}

export function scoreboardModel(rows: readonly ScoreRow[], info: MatchInfo, ownTeam: number, online: boolean): ScoreboardModel {
  return {
    teams: rowsByTeam(rows).map(({ team, rows: list }) => ({
      ...teamLine(info, team, ownTeam),
      players: list.map((r) => ({ name: playerName(r), kills: r.kills, deaths: r.deaths, alive: r.alive, isYou: r.isYou })),
    })),
    note: online ? "Kills and deaths since you joined · paint is per team" : "Practice · paint is per team",
  };
}

export function endScreenModel(info: MatchInfo, ownTeam: number, online: boolean, intermissionSecs?: number): EndScreenModel {
  const title =
    info.winner === null ? "Draw" : info.winner === ownTeam ? `${teamName(info.winner)} wins!` : `${teamName(info.winner)} wins`;
  const subtitle = info.winner === null ? "Both teams painted the same share" : info.winner === ownTeam ? "Your team painted the most" : "Your team lost this one";
  let footer: string;
  if (!online) {
    footer = "Practice over · reload for a new round";
  } else {
    const left = intermissionLeft(info, intermissionSecs);
    footer = left > 0 ? `Next match in ${left} s` : "Starting the next match…";
  }
  return { title, subtitle, teams: [0, 1].map((t) => teamLine(info, t, ownTeam)), footer };
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, parent?: HTMLElement): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  parent?.appendChild(e);
  return e;
}

function renderTeams(parent: HTMLElement, model: ScoreboardModel): void {
  const frag = document.createDocumentFragment();
  for (const t of model.teams) {
    const block = el("div", `sb-team team-${t.team}${t.yours ? " yours" : ""}`);
    const head = el("div", "sb-team-head", block);
    el("span", "sb-team-name", head).textContent = t.name + (t.yours ? " (your team)" : "");
    el("span", "sb-team-paint", head).textContent = `${t.paint} painted`;
    const table = el("table", "sb-table", block);
    const hr = el("tr", "", el("thead", "", table));
    for (const h of ["Pilot", "Kills", "Deaths"]) el("th", "", hr).textContent = h;
    const body = el("tbody", "", table);
    if (t.players.length === 0) {
      const tr = el("tr", "sb-empty", body);
      const td = el("td", "", tr);
      td.colSpan = 3;
      td.textContent = "No pilots";
    }
    for (const p of t.players) {
      const tr = el("tr", `${p.isYou ? "you" : ""}${p.alive ? "" : " dead"}`, body);
      el("td", "sb-name", tr).textContent = p.name;
      el("td", "sb-num", tr).textContent = String(p.kills);
      el("td", "sb-num", tr).textContent = String(p.deaths);
    }
    frag.appendChild(block);
  }
  parent.replaceChildren(frag);
}

/** The Tab scoreboard. */
export class Scoreboard {
  readonly root: HTMLElement;
  private readonly teams: HTMLElement;
  private readonly note: HTMLElement;
  private last = "";

  constructor(parent: HTMLElement) {
    this.root = el("div", "scoreboard hidden", parent);
    el("h2", "sb-title", this.root).textContent = "Scoreboard";
    this.teams = el("div", "sb-teams", this.root);
    this.note = el("div", "sb-note", this.root);
  }

  show(visible: boolean): void {
    this.root.classList.toggle("hidden", !visible);
  }

  get visible(): boolean {
    return !this.root.classList.contains("hidden");
  }

  update(model: ScoreboardModel): void {
    const key = JSON.stringify(model);
    if (key === this.last) return;
    this.last = key;
    renderTeams(this.teams, model);
    this.note.textContent = model.note;
  }

  dispose(): void {
    this.root.remove();
  }
}

/** Full-screen end-of-match overlay with the scoreboard below the result. */
export class EndScreen {
  readonly root: HTMLElement;
  private readonly title: HTMLElement;
  private readonly subtitle: HTMLElement;
  private readonly bars: HTMLElement;
  private readonly board: HTMLElement;
  private readonly footer: HTMLElement;
  private last = "";
  private lastBoard = "";

  constructor(parent: HTMLElement) {
    this.root = el("div", "end-screen hidden", parent);
    const card = el("div", "end-card", this.root);
    el("div", "end-kicker", card).textContent = "Match over";
    this.title = el("h1", "end-title", card);
    this.subtitle = el("p", "end-subtitle", card);
    this.bars = el("div", "end-bars", card);
    this.board = el("div", "sb-teams end-board", card);
    this.footer = el("p", "end-footer", card);
  }

  show(visible: boolean): void {
    this.root.classList.toggle("hidden", !visible);
  }

  get visible(): boolean {
    return !this.root.classList.contains("hidden");
  }

  update(model: EndScreenModel, board: ScoreboardModel): void {
    const key = JSON.stringify(model);
    if (key !== this.last) {
      this.last = key;
      this.title.textContent = model.title;
      this.subtitle.textContent = model.subtitle;
      this.footer.textContent = model.footer;
      this.bars.replaceChildren(
        ...model.teams.map((t) => {
          const row = el("div", `end-bar-row team-${t.team}${t.winner ? " winner" : ""}${t.yours ? " yours" : ""}`);
          el("span", "end-bar-name", row).textContent = t.name;
          const bar = el("div", "end-bar", row);
          el("div", "end-bar-fill", bar).style.width = t.paint;
          el("span", "end-bar-value", row).textContent = t.paint;
          return row;
        }),
      );
    }
    const boardKey = JSON.stringify(board);
    if (boardKey !== this.lastBoard) {
      this.lastBoard = boardKey;
      renderTeams(this.board, board);
    }
  }

  dispose(): void {
    this.root.remove();
  }
}
