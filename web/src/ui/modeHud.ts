// Generic mode HUD (M3), driven by the replicated mode state of any mode:
// the top-centre scoreline (mode, timer, team scores or the free-for-all
// leaders, the score target), the mode stage (The Squeeze's phases), the
// own lives and colour meter, alerts (outside the safe zone, carrying the
// core, holding the reactor, spectating) and objective markers projected
// into the view (clamped to the screen edge with an arrow when off screen).
// Models are pure functions for Node tests; the class only writes the DOM
// when its model changed.

import * as mat4 from "../math/mat4.ts";
import type { Vec3 } from "../math/vec3.ts";
import {
  isOut,
  modeInfo,
  OBJECTIVE,
  ownModePlayer,
  PLAYER_FLAG,
  secsUntil,
  VIEW_FLAG,
  type ModeState,
  type Objective,
} from "../game/modeState.ts";
import type { FlowStage } from "../game/matchFlow.ts";
import { sideCss, teamName } from "../game/sides.ts";
import { formatTimer } from "./hud.ts";

export interface ScoreChip {
  side: number;
  name: string;
  color: string;
  /** Score text ("12", "34.5%"). */
  score: string;
  /** Bar fill 0..100 (towards the target, or the paint share). */
  fill: number;
  yours: boolean;
  leading: boolean;
}

export type AlertLevel = "info" | "warning" | "danger";

export interface ModeHudModel {
  /** Mode name ("Team Brawl"), or "Warm-up". */
  title: string;
  /** m:ss, "" hides the timer. */
  timer: string;
  timerUrgent: boolean;
  /** "First to 15", "3 deliveries win", ""… */
  target: string;
  /** Stage line ("Phase 2 · closing 0:12"), "" hides it. */
  stage: string;
  /** Team scores (team modes) or the top three (free-for-all). */
  chips: ScoreChip[];
  /** Own standing in free-for-all ("You · 4th of 8 · 7"), "" in team modes. */
  ownLine: string;
  /** Own lives (pips), -1 hides them. */
  lives: number;
  /** Own colour meter 0..100, -1 hides it. */
  meter: number;
  alerts: Array<{ text: string; level: AlertLevel }>;
  /** Instagib: the HUD hides hull and energy. */
  instagib: boolean;
}

export interface ModeHudInput {
  mode: ModeState;
  stage: FlowStage;
  /** Own player id and side. */
  own: number;
  ownSide: number;
  /** Server tick estimate (Game.nowTick()). */
  nowTick: number;
  /** Seconds left on the match timer (the HUD values' timeLeft). */
  timeLeft: number;
  /** Player names by id. */
  names: ReadonlyMap<number, string>;
  /** Own ship position (null when there is none). */
  ownPos: Vec3 | null;
  spectating: boolean;
  /** Followed ship while spectating (null: free camera). */
  following: number | null;
}

/** Ordinal ("1st", "2nd", "3rd", "4th", "11th"). */
export function ordinal(n: number): string {
  const s = n % 100;
  if (s >= 11 && s <= 13) return `${n}th`;
  const last = n % 10;
  return `${n}${last === 1 ? "st" : last === 2 ? "nd" : last === 3 ? "rd" : "th"}`;
}

/** Name of a player id (names table, else "Pilot n"). */
export function nameOf(names: ReadonlyMap<number, string>, id: number): string {
  return names.get(id) || `Pilot ${id + 1}`;
}

/** Paint share of a team score in percent (paint modes). */
function paintShare(m: ModeState, score: number): number {
  return m.scoreTotal > 0 ? (100 * score) / m.scoreTotal : 0;
}

function dist(a: Vec3, b: Vec3): number {
  return Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z);
}

/** Builds the mode HUD model. */
export function modeHudModel(i: ModeHudInput): ModeHudModel {
  const m = i.mode;
  const info = modeInfo(m.mode);
  const paint = (m.flags & VIEW_FLAG.paintScores) !== 0;
  const warmup = i.stage === "warmup" || i.stage === "countdown";
  const own = ownModePlayer(m, i.own);
  const alerts: ModeHudModel["alerts"] = [];

  const chips: ScoreChip[] = [];
  let ownLine = "";
  if (!warmup && m.teams > 0) {
    const best = Math.max(...m.teamScores, 0);
    const leaders = m.teamScores.filter((s) => s === best).length;
    m.teamScores.forEach((score, team) => {
      const fill = paint ? paintShare(m, score) : m.scoreTarget > 0 ? (100 * Math.max(0, score)) / m.scoreTarget : 0;
      chips.push({
        side: team,
        name: teamName(team),
        color: sideCss(team, m.teams),
        score: paint ? `${paintShare(m, score).toFixed(1)}%` : String(score),
        fill: Math.max(0, Math.min(100, fill)),
        yours: team === i.ownSide,
        leading: score === best && leaders === 1 && best > 0,
      });
    });
  } else if (!warmup) {
    const ranked = m.players.filter((p) => (p.flags & PLAYER_FLAG.spectator) === 0);
    const top = ranked[0]?.score ?? 0;
    for (const p of ranked.slice(0, 3)) {
      chips.push({
        side: p.side,
        name: nameOf(i.names, p.id),
        color: sideCss(p.side, 0),
        score: String(p.score),
        fill: m.scoreTarget > 0 ? Math.max(0, Math.min(100, (100 * p.score) / m.scoreTarget)) : 0,
        yours: p.id === i.own,
        leading: p.score === top && ranked.filter((x) => x.score === top).length === 1,
      });
    }
    const rank = ranked.findIndex((p) => p.id === i.own);
    if (rank >= 0 && own) ownLine = `You · ${ordinal(rank + 1)} of ${ranked.length} · ${own.score}`;
  }

  let target = "";
  if (!warmup && m.scoreTarget > 0) target = `First to ${m.scoreTarget}`;

  let stage = "";
  if (!warmup && info.stageLabel && m.stage > 0) {
    const left = secsUntil(m.stageEndTick, i.nowTick);
    stage = `${info.stageLabel} ${m.stage}${left > 0 ? ` · ${formatTimer(left)}` : ""}`;
  }

  // Objectives that concern the own ship.
  if (!warmup && m.running && !i.spectating) {
    const zone = m.objectives.find((o) => o.kind === OBJECTIVE.safeZone);
    if (zone && i.ownPos && zone.radius > 0 && dist(i.ownPos, zone.pos) > zone.radius) {
      alerts.push({ text: "Outside the safe zone: get back in!", level: "danger" });
    }
    for (const o of m.objectives) {
      if (o.holder !== i.own) continue;
      if (o.kind === OBJECTIVE.core) {
        alerts.push({ text: "You carry the core: no boost · fly it to an extraction zone", level: "warning" });
      } else if (o.kind === OBJECTIVE.reactor) {
        const fuse = secsUntil(o.timerEnd, i.nowTick);
        alerts.push({
          text: `You hold the reactor! Hit someone to pass it${fuse > 0 ? ` · ${fuse.toFixed(1)} s` : ""}`,
          level: "danger",
        });
      }
    }
  }
  if (i.spectating && !warmup) {
    const out = own && (own.flags & PLAYER_FLAG.eliminated) !== 0;
    const who = i.following !== null ? `Following ${nameOf(i.names, i.following)}` : "Free camera";
    alerts.push({
      text: `${out ? "You are out · " : "Spectating · "}${who} · left click: next ship · right click: free camera`,
      level: "info",
    });
  }

  const showLives = !warmup && (m.flags & VIEW_FLAG.lives) !== 0 && own !== undefined && own.lives >= 0;
  const showMeter = !warmup && (m.flags & VIEW_FLAG.meter) !== 0 && own !== undefined && !isOut(own);
  const timerSecs = Math.max(0, i.timeLeft);
  return {
    title: warmup ? "Warm-up" : info.name,
    timer: warmup || !m.running ? "" : formatTimer(timerSecs),
    timerUrgent: !warmup && m.running && timerSecs > 0 && timerSecs <= 30,
    target,
    stage,
    chips,
    ownLine,
    lives: showLives ? own.lives : -1,
    meter: showMeter ? Math.max(0, Math.min(100, own.meter)) : -1,
    alerts,
    instagib: (m.flags & VIEW_FLAG.instagib) !== 0,
  };
}

// ---- Objective markers ----------------------------------------------------

export interface Marker {
  id: number;
  kind: number;
  label: string;
  /** Distance text ("42 u"). */
  detail: string;
  color: string;
  /** CSS pixels on the canvas. */
  x: number;
  y: number;
  /** Clamped to the screen edge (the arrow points towards the objective). */
  edge: boolean;
  /** Arrow direction (radians, screen space, 0 = right, clockwise) when on the edge. */
  angle: number;
}

/** Screen fraction (of the half-extent) markers keep from the edge. */
const EDGE_X = 0.92;
const EDGE_Y = 0.86;

/** Projects `p` to the canvas, clamping off-screen and behind-camera points to the edge. */
export function projectMarker(
  viewProj: mat4.Mat4,
  p: Vec3,
  width: number,
  height: number,
): { x: number; y: number; edge: boolean; angle: number } {
  const [cx, cy, , w] = mat4.transformPoint(viewProj, p);
  let nx: number, ny: number;
  let edge = false;
  if (w > 1e-3) {
    nx = cx / w;
    ny = cy / w;
    if (Math.abs(nx) > EDGE_X || Math.abs(ny) > EDGE_Y) edge = true;
  } else {
    // Behind the camera: the clip x/y keep the side the point lies on.
    nx = cx;
    ny = cy;
    edge = true;
    if (Math.abs(nx) < 1e-6 && Math.abs(ny) < 1e-6) ny = -1;
  }
  if (edge) {
    const s = Math.max(Math.abs(nx) / EDGE_X, Math.abs(ny) / EDGE_Y, 1e-9);
    nx /= s;
    ny /= s;
  }
  const x = (nx * 0.5 + 0.5) * width;
  const y = (1 - (ny * 0.5 + 0.5)) * height;
  return { x, y, edge, angle: Math.atan2(-ny, nx) };
}

function markerStyle(o: Objective, m: ModeState, names: ReadonlyMap<number, string>, nowTick: number): { label: string; color: string } | null {
  const held = o.holder >= 0 ? ` · ${nameOf(names, o.holder)}` : "";
  switch (o.kind) {
    case OBJECTIVE.safeZone:
      return { label: "Safe zone", color: "rgb(120, 230, 255)" };
    case OBJECTIVE.nextZone:
      return { label: "Next zone", color: "rgb(255, 214, 90)" };
    case OBJECTIVE.core:
      return { label: `Core${held}`, color: "rgb(255, 205, 80)" };
    case OBJECTIVE.extraction:
      return { label: "Extraction", color: o.team >= 0 ? sideCss(o.team, m.teams) : "rgb(150, 255, 170)" };
    case OBJECTIVE.reactor: {
      const fuse = secsUntil(o.timerEnd, nowTick);
      return { label: `Reactor${fuse > 0 ? ` ${fuse.toFixed(1)} s` : ""}${held}`, color: "rgb(255, 90, 70)" };
    }
    default:
      return null;
  }
}

/**
 * Markers for the mode's objectives: everything except what the own ship
 * carries. Zones show their centre.
 */
export function objectiveMarkers(
  m: ModeState,
  own: number,
  names: ReadonlyMap<number, string>,
  nowTick: number,
  viewProj: mat4.Mat4,
  camera: Vec3,
  width: number,
  height: number,
): Marker[] {
  if (!m.running) return [];
  const out: Marker[] = [];
  for (const o of m.objectives) {
    if (o.holder === own && own >= 0) continue;
    const style = markerStyle(o, m, names, nowTick);
    if (!style) continue;
    const s = projectMarker(viewProj, o.pos, width, height);
    const d = dist(camera, o.pos);
    const inside = (o.kind === OBJECTIVE.safeZone || o.kind === OBJECTIVE.nextZone) && o.radius > 0 && d <= o.radius;
    out.push({
      id: o.id,
      kind: o.kind,
      label: style.label,
      detail: inside ? "inside" : `${Math.round(d)} u`,
      color: style.color,
      ...s,
    });
  }
  return out;
}

// ---- DOM ------------------------------------------------------------------

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, parent?: HTMLElement): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  parent?.appendChild(e);
  return e;
}

/** The mode HUD's DOM: scoreline, stage, lives/meter, alerts and markers. */
export class ModeHud {
  readonly root: HTMLElement;
  private readonly title: HTMLElement;
  private readonly timer: HTMLElement;
  private readonly target: HTMLElement;
  private readonly chips: HTMLElement;
  private readonly ownLine: HTMLElement;
  private readonly stage: HTMLElement;
  private readonly status: HTMLElement;
  private readonly alerts: HTMLElement;
  private readonly hint: HTMLElement;
  private readonly markers: HTMLElement;
  private readonly markerPool: HTMLElement[] = [];
  private last = "";

  constructor(parent: HTMLElement) {
    this.root = el("div", "mode-hud", parent);
    this.markers = el("div", "mode-markers", this.root);
    const bar = el("div", "mode-bar", this.root);
    const head = el("div", "mode-head", bar);
    this.title = el("span", "mode-title", head);
    this.timer = el("span", "mode-timer", head);
    this.chips = el("div", "mode-chips", bar);
    const foot = el("div", "mode-foot", bar);
    this.target = el("span", "mode-target", foot);
    this.ownLine = el("span", "mode-own", foot);
    this.stage = el("div", "mode-stage", this.root);
    this.status = el("div", "mode-status", this.root);
    this.alerts = el("div", "mode-alerts", this.root);
    this.hint = el("div", "mode-hint hidden", this.root);
  }

  update(model: ModeHudModel): void {
    const key = JSON.stringify(model);
    if (key === this.last) return;
    this.last = key;
    this.title.textContent = model.title;
    this.timer.textContent = model.timer;
    this.timer.classList.toggle("hidden", model.timer === "");
    this.timer.classList.toggle("urgent", model.timerUrgent);
    this.target.textContent = model.target;
    this.ownLine.textContent = model.ownLine;
    this.chips.replaceChildren(
      ...model.chips.map((c) => {
        const chip = el("div", `mode-chip${c.yours ? " yours" : ""}${c.leading ? " leading" : ""}`);
        chip.style.setProperty("--side", c.color);
        const top = el("div", "mode-chip-top", chip);
        el("span", "mode-chip-name", top).textContent = c.name;
        el("span", "mode-chip-score", top).textContent = c.score;
        const bar = el("div", "mode-chip-bar", chip);
        el("div", "mode-chip-fill", bar).style.width = `${c.fill.toFixed(1)}%`;
        return chip;
      }),
    );
    this.chips.classList.toggle("hidden", model.chips.length === 0);
    this.stage.textContent = model.stage;
    this.stage.classList.toggle("hidden", model.stage === "");

    const parts: HTMLElement[] = [];
    if (model.lives >= 0) {
      const lives = el("div", "mode-lives");
      el("span", "mode-label", lives).textContent = "Lives";
      const pips = el("span", "mode-pips", lives);
      const shown = Math.min(model.lives, 9);
      for (let n = 0; n < shown; n++) el("span", "mode-pip", pips);
      if (model.lives > shown) el("span", "mode-pip-more", pips).textContent = `+${model.lives - shown}`;
      if (model.lives === 0) el("span", "mode-pip-none", pips).textContent = "none";
      parts.push(lives);
    }
    if (model.meter >= 0) {
      const meter = el("div", "mode-meter");
      el("span", "mode-label", meter).textContent = "Colour";
      const bar = el("div", "mode-meter-bar", meter);
      el("div", "mode-meter-fill", bar).style.width = `${model.meter}%`;
      el("span", "mode-meter-value", meter).textContent = String(model.meter);
      parts.push(meter);
    }
    this.status.replaceChildren(...parts);
    this.status.classList.toggle("hidden", parts.length === 0);
    // Info lines (spectating) sit at the bottom; warnings in the view.
    const hint = model.alerts.filter((a) => a.level === "info").map((a) => a.text).join(" · ");
    this.hint.textContent = hint;
    this.hint.classList.toggle("hidden", hint === "");
    this.alerts.replaceChildren(
      ...model.alerts.filter((a) => a.level !== "info").map((a) => {
        const d = el("div", `mode-alert ${a.level}`);
        d.textContent = a.text;
        return d;
      }),
    );
  }

  /** Places the objective markers (call every frame; reuses elements). */
  updateMarkers(markers: readonly Marker[]): void {
    while (this.markerPool.length < markers.length) {
      const m = el("div", "mode-marker", this.markers);
      el("span", "mode-marker-arrow", m);
      el("span", "mode-marker-dot", m);
      const text = el("span", "mode-marker-text", m);
      el("span", "mode-marker-label", text);
      el("span", "mode-marker-detail", text);
      this.markerPool.push(m);
    }
    this.markerPool.forEach((node, i) => {
      const mk = markers[i];
      if (!mk) {
        node.style.display = "none";
        return;
      }
      node.style.display = "";
      node.classList.toggle("edge", mk.edge);
      node.style.setProperty("--side", mk.color);
      node.style.transform = `translate(${mk.x.toFixed(1)}px, ${mk.y.toFixed(1)}px)`;
      const arrow = node.children[0] as HTMLElement | undefined;
      if (arrow) arrow.style.transform = `translate(-50%, -50%) rotate(${mk.angle.toFixed(3)}rad)`;
      const text = node.children[2];
      const label = text?.children[0];
      const detail = text?.children[1];
      if (label && label.textContent !== mk.label) label.textContent = mk.label;
      if (detail && detail.textContent !== mk.detail) detail.textContent = mk.detail;
    });
  }

  show(visible: boolean): void {
    this.root.classList.toggle("hidden", !visible);
  }

  dispose(): void {
    this.root.remove();
  }
}
