// Scoreboard (hold Tab) for every mode: one block per team (2-4 teams) or
// one ranked list in free-for-all, with the mode score, kills, deaths and
// lives where the mode counts them. Kills and deaths come from ls-wasm's
// tally, scores and lives from the replicated mode state. The model is a
// pure function for Node tests; the class only writes the DOM on change.
// (The end-of-match screen is ui/results.ts.)

import { modeInfo, PLAYER_FLAG, VIEW_FLAG, type ModeState } from "../game/modeState.ts";
import { sideCss, teamName } from "../game/sides.ts";
import { playerName, rowsByTeam, type ScoreRow } from "../game/stats.ts";

export interface PlayerLine {
  name: string;
  score: number;
  kills: number;
  deaths: number;
  /** Lives left, -1 = not shown. */
  lives: number;
  alive: boolean;
  /** Eliminated or spectating. */
  out: boolean;
  isYou: boolean;
  color: string;
}

export interface TeamBlock {
  team: number;
  name: string;
  color: string;
  /** Team score text ("12", "34.5%"); "" in free-for-all. */
  score: string;
  yours: boolean;
  leading: boolean;
  players: PlayerLine[];
}

export interface ScoreboardModel {
  title: string;
  scoreLabel: string;
  showLives: boolean;
  teams: TeamBlock[];
  /** Footnote on where the numbers come from. */
  note: string;
}

/** Scoreboard model: rows from `Game.scoreboard()`, scores and lives from the mode state. */
export function scoreboardModel(rows: readonly ScoreRow[], mode: ModeState, ownSide: number, online: boolean): ScoreboardModel {
  const info = modeInfo(mode.mode);
  const paint = (mode.flags & VIEW_FLAG.paintScores) !== 0;
  const showLives = (mode.flags & VIEW_FLAG.lives) !== 0;
  const byId = new Map(mode.players.map((p) => [p.id, p]));
  const line = (r: ScoreRow): PlayerLine => {
    const mp = byId.get(r.id);
    return {
      name: playerName(r),
      score: mp?.score ?? 0,
      kills: r.kills,
      deaths: r.deaths,
      lives: showLives && mp ? mp.lives : -1,
      alive: r.alive,
      out: mp !== undefined && (mp.flags & (PLAYER_FLAG.eliminated | PLAYER_FLAG.spectator)) !== 0,
      isYou: r.isYou,
      color: sideCss(r.team, mode.teams),
    };
  };
  const byScore = (a: PlayerLine, b: PlayerLine): number => b.score - a.score || b.kills - a.kills || a.deaths - b.deaths;
  let teams: TeamBlock[];
  if (mode.teams > 0) {
    const best = Math.max(...mode.teamScores);
    const leaders = mode.teamScores.filter((s) => s === best).length;
    teams = rowsByTeam(rows, mode.teams).map(({ team, rows: list }) => {
      const s = mode.teamScores[team] ?? 0;
      return {
        team,
        name: teamName(team),
        color: sideCss(team, mode.teams),
        score: paint ? `${mode.scoreTotal > 0 ? ((100 * s) / mode.scoreTotal).toFixed(1) : "0.0"}%` : String(s),
        yours: team === ownSide,
        leading: mode.teamScores.length > 0 && s === best && leaders === 1,
        players: list.map(line).sort(byScore),
      };
    });
  } else {
    teams = [
      {
        team: -1,
        name: "Free-for-all",
        color: "",
        score: "",
        yours: false,
        leading: false,
        players: rows.map(line).sort(byScore),
      },
    ];
  }
  return {
    title: info.name,
    scoreLabel: paint ? "Score" : info.scoreLabel,
    showLives,
    teams,
    note: online ? "Kills and deaths since you joined" : "Practice",
  };
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, parent?: HTMLElement): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  parent?.appendChild(e);
  return e;
}

function renderTeams(parent: HTMLElement, model: ScoreboardModel): void {
  const frag = document.createDocumentFragment();
  const heads = ["Pilot", model.scoreLabel, "K", "D", ...(model.showLives ? ["Lives"] : [])];
  for (const t of model.teams) {
    const block = el("div", `sb-team${t.yours ? " yours" : ""}${t.leading ? " leading" : ""}`);
    if (t.color) block.style.setProperty("--side", t.color);
    const head = el("div", "sb-team-head", block);
    el("span", "sb-team-name", head).textContent = t.name + (t.yours ? " · your team" : "");
    if (t.score) el("span", "sb-team-score", head).textContent = t.score;
    const table = el("table", "sb-table", block);
    const hr = el("tr", "", el("thead", "", table));
    for (const h of heads) el("th", "", hr).textContent = h;
    const body = el("tbody", "", table);
    if (t.players.length === 0) {
      const td = el("td", "", el("tr", "sb-empty", body));
      td.colSpan = heads.length;
      td.textContent = "No pilots";
    }
    for (const p of t.players) {
      const tr = el("tr", `${p.isYou ? "you" : ""}${p.alive ? "" : " dead"}${p.out ? " out" : ""}`, body);
      tr.style.setProperty("--side", p.color);
      const name = el("td", "sb-name", tr);
      el("span", "sb-swatch", name);
      el("span", "", name).textContent = p.name;
      el("td", "sb-num", tr).textContent = String(p.score);
      el("td", "sb-num", tr).textContent = String(p.kills);
      el("td", "sb-num", tr).textContent = String(p.deaths);
      if (model.showLives) el("td", "sb-num", tr).textContent = p.out ? "out" : p.lives >= 0 ? String(p.lives) : "–";
    }
    frag.appendChild(block);
  }
  parent.replaceChildren(frag);
}

/** The Tab scoreboard. */
export class Scoreboard {
  readonly root: HTMLElement;
  private readonly title: HTMLElement;
  private readonly teams: HTMLElement;
  private readonly note: HTMLElement;
  private last = "";

  constructor(parent: HTMLElement) {
    this.root = el("div", "scoreboard hidden", parent);
    this.title = el("h2", "sb-title", this.root);
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
    this.title.textContent = model.title;
    renderTeams(this.teams, model);
    this.note.textContent = model.note;
  }

  dispose(): void {
    this.root.remove();
  }
}
