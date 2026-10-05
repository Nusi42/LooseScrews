// Results screen (M3): after every match, per mode. Team modes show the
// team scores as bars (paint share in Paint the Void); free-for-all modes a
// podium of the top three. Below, every pilot with the mode score, kills
// and deaths, and at the bottom what comes next (the lobby's next mode and
// its countdown). Online the table is the server's `MatchResults`; in
// practice it is built from the local mode state (practiceResults). Models
// are pure for Node tests; the class only writes the DOM on change.

import { modeInfo, VIEW_FLAG, type LobbyState, type MatchResults, type ModeState } from "../game/modeState.ts";
import { sideCss, teamName } from "../game/sides.ts";
import type { ScoreRow } from "../game/stats.ts";

export interface ResultTeam {
  team: number;
  name: string;
  color: string;
  score: string;
  /** Bar width 0..100 relative to the best team (paint: the share). */
  fill: number;
  winner: boolean;
  yours: boolean;
}

export interface ResultLine {
  place: number;
  name: string;
  /** Team name in team modes, "" in free-for-all. */
  team: string;
  color: string;
  score: number;
  kills: number;
  deaths: number;
  you: boolean;
  winner: boolean;
}

export interface ResultsModel {
  /** "Team Brawl · Match over". */
  kicker: string;
  title: string;
  subtitle: string;
  outcome: "win" | "loss" | "draw" | "neutral";
  /** Colour of the winning side ("" on a draw). */
  winnerColor: string;
  teams: ResultTeam[];
  /** Top three in free-for-all (empty in team modes). */
  podium: ResultLine[];
  scoreLabel: string;
  rows: ResultLine[];
  footer: string;
}

export interface ResultsInput {
  results: MatchResults;
  /** Own player id and side. */
  own: number;
  ownSide: number;
  online: boolean;
  /** Mode state flags and paintable surface (paint share), from the finished match. */
  modeFlags: number;
  scoreTotal: number;
  /** Lobby (next mode) and the seconds until the next phase. */
  lobby: LobbyState | null;
  secsLeft: number;
  /** Names for rows without one (practice). */
  names?: ReadonlyMap<number, string>;
}

/** Results of a finished practice match from the local mode state and kill tally. */
export function practiceResults(mode: ModeState, rows: readonly ScoreRow[], names: ReadonlyMap<number, string>): MatchResults {
  const out = mode.players.map((p) => {
    const r = rows.find((x) => x.id === p.id);
    return {
      id: p.id,
      side: p.side,
      kills: r?.kills ?? 0,
      deaths: r?.deaths ?? 0,
      score: p.score,
      name: names.get(p.id) || r?.name || "",
    };
  });
  return { tick: 0, kind: mode.kind, mode: mode.mode, teams: mode.teams, winner: mode.winner, teamScores: [...mode.teamScores], rows: out };
}

function pct(score: number, total: number): number {
  return total > 0 ? (100 * score) / total : 0;
}

/** Builds the results screen model. */
export function resultsModel(i: ResultsInput): ResultsModel {
  const r = i.results;
  const info = modeInfo(r.mode);
  const paint = (i.modeFlags & VIEW_FLAG.paintScores) !== 0;
  const teamsMode = r.teams > 0;
  const draw = r.winner < 0;
  const nameOf = (id: number, name: string): string => name || i.names?.get(id) || `Pilot ${id + 1}`;
  const winnerRow = !teamsMode && !draw ? r.rows.find((x) => x.side === r.winner) : undefined;
  const ownWins = !draw && (teamsMode ? r.winner === i.ownSide : winnerRow?.id === i.own);
  const ownPlays = r.rows.some((x) => x.id === i.own);

  let title: string;
  let outcome: ResultsModel["outcome"];
  if (draw) {
    title = "Draw";
    outcome = "draw";
  } else if (ownWins) {
    title = "Victory";
    outcome = "win";
  } else if (ownPlays) {
    title = "Defeat";
    outcome = "loss";
  } else {
    title = teamsMode ? `${teamName(r.winner)} wins` : `${nameOf(winnerRow?.id ?? r.winner, winnerRow?.name ?? "")} wins`;
    outcome = "neutral";
  }

  let subtitle: string;
  if (teamsMode) {
    const scores = r.teamScores.map((s) => (paint ? `${pct(s, i.scoreTotal).toFixed(1)}%` : String(s)));
    if (draw) subtitle = `The teams finished level · ${scores.join(" – ")}`;
    else subtitle = `${teamName(r.winner)} ${paint ? "painted the most" : "wins"} · ${scores.join(" – ")}`;
  } else if (draw) {
    subtitle = "Nobody came out on top";
  } else {
    const who = winnerRow ? nameOf(winnerRow.id, winnerRow.name) : "The winner";
    subtitle = winnerRow?.id === i.own ? `You win with ${winnerRow.score} · ${info.name}` : `${who} wins with ${winnerRow?.score ?? 0}`;
  }

  const best = Math.max(1, ...r.teamScores);
  const teams: ResultTeam[] = teamsMode
    ? r.teamScores.map((s, t) => ({
        team: t,
        name: teamName(t),
        color: sideCss(t, r.teams),
        score: paint ? `${pct(s, i.scoreTotal).toFixed(1)}%` : String(s),
        fill: Math.max(0, Math.min(100, paint ? pct(s, i.scoreTotal) : (100 * Math.max(0, s)) / best)),
        winner: t === r.winner,
        yours: t === i.ownSide,
      }))
    : [];

  const rows: ResultLine[] = r.rows.map((x, n) => ({
    place: n + 1,
    name: nameOf(x.id, x.name),
    team: teamsMode ? teamName(x.side) : "",
    color: sideCss(x.side, r.teams),
    score: x.score,
    kills: x.kills,
    deaths: x.deaths,
    you: x.id === i.own,
    winner: !draw && x.side === r.winner,
  }));

  let footer: string;
  if (!i.online) footer = "Practice over · reload for a new round";
  else {
    const next = i.lobby && i.lobby.nextModeId ? modeInfo(i.lobby.nextModeId).name : "";
    const secs = Math.ceil(i.secsLeft);
    if (next && secs > 0) footer = `Next: ${next} in ${secs} s`;
    else if (next) footer = `Next: ${next}`;
    else footer = secs > 0 ? `Next match in ${secs} s` : "Starting the next match…";
  }

  return {
    kicker: `${info.name} · Match over`,
    title,
    subtitle,
    outcome,
    winnerColor: draw ? "" : sideCss(r.winner, r.teams),
    teams,
    podium: teamsMode ? [] : rows.slice(0, 3),
    scoreLabel: info.scoreLabel,
    rows,
    footer,
  };
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, parent?: HTMLElement): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  parent?.appendChild(e);
  return e;
}

/** Full-screen results overlay. */
export class ResultsScreen {
  readonly root: HTMLElement;
  private readonly card: HTMLElement;
  private readonly kicker: HTMLElement;
  private readonly title: HTMLElement;
  private readonly subtitle: HTMLElement;
  private readonly teams: HTMLElement;
  private readonly podium: HTMLElement;
  private readonly table: HTMLTableElement;
  private readonly footer: HTMLElement;
  private last = "";

  constructor(parent: HTMLElement) {
    this.root = el("div", "results hidden", parent);
    this.card = el("div", "results-card", this.root);
    this.kicker = el("div", "results-kicker", this.card);
    this.title = el("h1", "results-title", this.card);
    this.subtitle = el("p", "results-subtitle", this.card);
    this.teams = el("div", "results-teams", this.card);
    this.podium = el("div", "results-podium", this.card);
    this.table = el("table", "results-table", this.card);
    this.footer = el("div", "results-footer", this.card);
  }

  show(visible: boolean): void {
    this.root.classList.toggle("hidden", !visible);
  }

  get visible(): boolean {
    return !this.root.classList.contains("hidden");
  }

  update(model: ResultsModel): void {
    const key = JSON.stringify(model);
    if (key === this.last) return;
    this.last = key;
    this.card.dataset["outcome"] = model.outcome;
    this.card.style.setProperty("--winner", model.winnerColor || "var(--muted)");
    this.kicker.textContent = model.kicker;
    this.title.textContent = model.title;
    this.subtitle.textContent = model.subtitle;
    this.teams.replaceChildren(
      ...model.teams.map((t) => {
        const row = el("div", `results-team${t.winner ? " winner" : ""}${t.yours ? " yours" : ""}`);
        row.style.setProperty("--side", t.color);
        el("span", "results-team-name", row).textContent = t.yours ? `${t.name} (you)` : t.name;
        const bar = el("div", "results-team-bar", row);
        el("div", "results-team-fill", bar).style.width = `${t.fill.toFixed(1)}%`;
        el("span", "results-team-score", row).textContent = t.score;
        return row;
      }),
    );
    this.teams.classList.toggle("hidden", model.teams.length === 0);
    // Podium order: 2nd, 1st, 3rd.
    const order = [model.podium[1], model.podium[0], model.podium[2]];
    this.podium.replaceChildren(
      ...order.flatMap((p) => {
        if (!p) return [];
        const step = el("div", `results-step place-${p.place}${p.you ? " you" : ""}`);
        step.style.setProperty("--side", p.color);
        el("div", "results-step-name", step).textContent = p.you ? `${p.name} (you)` : p.name;
        el("div", "results-step-score", step).textContent = String(p.score);
        el("div", "results-step-block", step).textContent = String(p.place);
        return [step];
      }),
    );
    this.podium.classList.toggle("hidden", model.podium.length === 0);
    const head = el("thead", "");
    const hr = el("tr", "", head);
    for (const h of ["#", "Pilot", model.scoreLabel, "Kills", "Deaths"]) el("th", "", hr).textContent = h;
    const body = el("tbody", "");
    for (const r of model.rows) {
      const tr = el("tr", `${r.you ? "you" : ""}${r.winner ? " winner" : ""}`, body);
      tr.style.setProperty("--side", r.color);
      el("td", "results-place", tr).textContent = String(r.place);
      const name = el("td", "results-name", tr);
      el("span", "results-swatch", name);
      el("span", "", name).textContent = r.you ? `${r.name} (you)` : r.name;
      if (r.team) el("span", "results-team-tag", name).textContent = r.team;
      el("td", "results-num", tr).textContent = String(r.score);
      el("td", "results-num", tr).textContent = String(r.kills);
      el("td", "results-num", tr).textContent = String(r.deaths);
    }
    if (model.rows.length === 0) {
      const td = el("td", "results-empty", el("tr", "", body));
      td.colSpan = 5;
      td.textContent = "No pilots";
    }
    this.table.replaceChildren(head, body);
    this.footer.textContent = model.footer;
  }

  dispose(): void {
    this.root.remove();
  }
}
