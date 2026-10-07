// Belt Up's race HUD (M4 C7, docs/milestone-4-plan.md §3.8 "HUD"):
// - the start lights (five red lamps coming on, all out: "BELT UP!");
// - the position card: place of the field (P3/6), heat and points, the race
//   clock, the gaps to the racer ahead and behind in seconds (their course
//   difference over the own average speed, game/race/course.ts);
// - the distance to the exit along the course and a marker on the next gate
//   (or the EXIT once the lap is done), clamped to the screen edge with an
//   arrow when it is off screen;
// - the ring minimap: the belt, the 8 gates (next bright, passed in the own
//   colour), every racer as a dot in its colour, the exit;
// - the wrong-way warning;
// - the radiation meter: dose rate, a Geiger trefoil, "hull lasts 7 s" (in
//   Instagib a "dose left" bar instead of the hidden hull);
// - the finish banner (place and time) and the heat results table.
// The model is a pure function (Node tests); the class writes the DOM only
// when a part of the model changed.

import * as mat4 from "../math/mat4.ts";
import type { Vec3 } from "../math/vec3.ts";
import {
  averageSpeed,
  azimuth,
  axisDistance,
  courseLeft,
  gapSecs,
  gateAzimuth,
  LAP_GATES,
  nextGate,
  nextTarget,
  TICK_HZ,
  WEDGES,
  type Belt,
  type Standing,
} from "../game/race/course.ts";
import { lapNumber, START_LAMPS, type StartLights } from "../game/race/raceFx.ts";
import type { RaceState } from "../game/modeState.ts";
import { sideCss } from "../game/sides.ts";

/** Default points per place (`belt_up.ron` points_1..8). */
export const HEAT_POINTS: readonly number[] = [10, 8, 6, 5, 4, 3, 2, 1];

export interface RaceHudInput {
  belt: Belt | null;
  race: RaceState | null;
  /** Own player id. */
  own: number;
  names: ReadonlyMap<number, string>;
  /** Side (colour) per racer id. */
  sides: ReadonlyMap<number, number>;
  /** Match points per racer id (the mode score). */
  points: ReadonlyMap<number, number>;
  /** Positions of the racers' ships (own included) by id. */
  positions: ReadonlyMap<number, Vec3>;
  ownPos: Vec3 | null;
  ownVel: Vec3 | null;
  nowTick: number;
  /** Tick the heat's race started (0 unknown). */
  raceStart: number;
  /** Own dose rate (hull/s). */
  dose: number;
  /** Own hull. */
  hull: number;
  /** Own full hull (Instagib's dose-left bar). */
  hullMax?: number;
  instagib: boolean;
  lights: StartLights;
  wrongWay: boolean;
  viewProj: mat4.Mat4;
  width: number;
  height: number;
  time: number;
  spectating: boolean;
}

export interface GapModel {
  name: string;
  /** "+1.4 s" (ahead of you) or "-0.8 s" (behind). */
  gap: string;
  color: string;
}

export interface MarkerModel {
  x: number;
  y: number;
  onScreen: boolean;
  /** Arrow angle (rad, screen, 0 = right, clockwise down) when off screen. */
  angle: number;
  label: string;
  dist: string;
  exit: boolean;
}

export interface MiniDot {
  x: number;
  y: number;
  color: string;
  own: boolean;
  done: boolean;
}

export interface MiniGate {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  state: "next" | "later" | "passed";
}

export interface ResultRow {
  place: number;
  name: string;
  time: string;
  /** Points this heat ("+10"). */
  gain: string;
  total: number;
  own: boolean;
  color: string;
}

export interface RaceHudModel {
  visible: boolean;
  /** Place and field size; null before the race has places. */
  place: { place: number; of: number } | null;
  heat: string;
  points: number;
  clock: string;
  ahead: GapModel | null;
  behind: GapModel | null;
  distance: string;
  next: string;
  marker: MarkerModel | null;
  wrongWay: boolean;
  rad: { level: number; rate: string; lasts: string; danger: boolean; instagib: boolean } | null;
  lights: { show: boolean; lit: number; go: boolean };
  finish: { place: number; time: string } | null;
  results: { title: string; rows: ResultRow[] } | null;
  mini: { dots: MiniDot[]; gates: MiniGate[]; ownColor: string } | null;
}

/** Race time "m:ss.t". */
export function formatRaceTime(secs: number): string {
  const s = Math.max(0, secs);
  const m = Math.floor(s / 60);
  const rest = s - m * 60;
  return `${m}:${rest < 10 ? "0" : ""}${rest.toFixed(1)}`;
}

/** Gap text: "+1.4 s" when the other is ahead, "-0.8 s" behind ("+0.0 s" rounds). */
export function formatGap(secs: number): string {
  const v = Math.abs(secs) < 0.05 ? 0 : secs;
  return `${v >= 0 ? "+" : "−"}${Math.abs(v).toFixed(1)} s`;
}

/** Ordinal "P3". */
export function placeLabel(place: number): string {
  return place > 0 ? `P${place}` : "P–";
}

/** Minimap size (px of its SVG view box) and the middle ring's radius. */
export const MINI_SIZE = 170;
export const MINI_RING = 60;

/** Minimap point of a world position (north = -z up, prograde counter-clockwise). */
export function miniPoint(b: Belt, p: Vec3): { x: number; y: number } {
  const a = azimuth(b, p);
  const mid = 0.5 * (b.innerRadius + b.outerRadius);
  const rr = Math.min(1.35, axisDistance(b, p) / mid) * MINI_RING;
  return { x: MINI_SIZE / 2 + rr * Math.cos(a), y: MINI_SIZE / 2 - rr * Math.sin(a) };
}

/** Marker of a world point: screen position, or the edge with an arrow angle when off screen or behind. */
export function screenMarker(p: Vec3, viewProj: mat4.Mat4, width: number, height: number, margin = 48): { x: number; y: number; onScreen: boolean; angle: number } {
  const m = viewProj;
  const cx = m[0]! * p.x + m[4]! * p.y + m[8]! * p.z + m[12]!;
  const cy = m[1]! * p.x + m[5]! * p.y + m[9]! * p.z + m[13]!;
  const cw = m[3]! * p.x + m[7]! * p.y + m[11]! * p.z + m[15]!;
  // Dividing by |w| keeps the side of a point behind the camera (left stays left).
  const nx = cx / Math.abs(cw || 1e-6);
  let ny = cy / Math.abs(cw || 1e-6);
  const behind = cw <= 0;
  // Straight behind: point down (turn around).
  if (behind && Math.abs(nx) < 0.05 && Math.abs(ny) < 0.05) ny = -1;
  let x = (nx * 0.5 + 0.5) * width;
  let y = (0.5 - ny * 0.5) * height;
  const on = !behind && x >= margin && x <= width - margin && y >= margin && y <= height - margin;
  const angle = Math.atan2(y - height / 2, x - width / 2);
  if (!on) {
    // Push out to the edge along the direction from the centre.
    const dx = Math.cos(angle);
    const dy = Math.sin(angle);
    const hw = width / 2 - margin;
    const hh = height / 2 - margin;
    const k = Math.min(Math.abs(dx) > 1e-6 ? hw / Math.abs(dx) : Infinity, Math.abs(dy) > 1e-6 ? hh / Math.abs(dy) : Infinity);
    x = width / 2 + dx * k;
    y = height / 2 + dy * k;
  }
  return { x, y, onScreen: on, angle };
}

/** Builds the race HUD model. */
export function raceHudModel(i: RaceHudInput): RaceHudModel {
  const b = i.belt;
  const race = i.race;
  const hidden: RaceHudModel = {
    visible: false,
    place: null,
    heat: "",
    points: 0,
    clock: "",
    ahead: null,
    behind: null,
    distance: "",
    next: "",
    marker: null,
    wrongWay: false,
    rad: null,
    lights: { show: false, lit: 0, go: false },
    finish: null,
    results: null,
    mini: null,
  };
  if (!b || !race) return hidden;
  const colorOf = (id: number): string => sideCss(i.sides.get(id) ?? id, 0);
  const nameOf = (id: number): string => (id === i.own ? "You" : i.names.get(id) || `Pilot ${id + 1}`);
  const own = race.racers.find((r) => r.id === i.own) ?? null;
  const n = race.racers.length;
  const standings: Standing[] = race.racers.map((r) => {
    const pos = i.positions.get(r.id) ?? (r.id === i.own ? i.ownPos : null);
    return { id: r.id, place: r.place, left: r.finishTick > 0 ? 0 : pos ? courseLeft(b, r, pos) : Infinity, finishTick: r.finishTick };
  });
  const elapsed = i.raceStart > 0 ? (i.nowTick - i.raceStart) / TICK_HZ : 0;
  const racing = race.phase === "racing";
  const model: RaceHudModel = { ...hidden, visible: true };
  model.heat = `HEAT ${race.heat + 1}/${race.heats}`;
  model.points = i.points.get(i.own) ?? 0;
  model.lights = { show: i.lights.show, lit: i.lights.lit, go: i.lights.go };
  if (racing && i.raceStart > 0) {
    const t = own && own.finishTick > 0 ? (own.finishTick - i.raceStart) / TICK_HZ : elapsed;
    model.clock = formatRaceTime(t);
  } else if (race.phase === "lights") model.clock = formatRaceTime(0);

  if (own) {
    model.place = own.place > 0 ? { place: own.place, of: n } : null;
    const me = standings.find((s) => s.id === i.own);
    if (me && racing) {
      const avg = averageSpeed(b, me.left === Infinity ? 0 : me.left, i.raceStart, own.finishTick > 0 ? own.finishTick : i.nowTick);
      const ahead = standings.find((s) => s.place === own.place - 1);
      const behind = standings.find((s) => s.place === own.place + 1);
      const gap = (o: Standing): GapModel | null => {
        if (!Number.isFinite(o.left) || !Number.isFinite(me.left)) return null;
        return { name: nameOf(o.id), gap: formatGap(gapSecs(me, o, avg, i.nowTick)), color: colorOf(o.id) };
      };
      model.ahead = ahead ? gap(ahead) : null;
      model.behind = behind ? gap(behind) : null;
    }
    if (own.finishTick === 0 && i.ownPos && (racing || race.phase === "lights")) {
      const left = courseLeft(b, own, i.ownPos);
      model.distance = `${Math.round(left)} u to the EXIT`;
      const g = nextGate(own);
      const target = nextTarget(b, own);
      const sm = screenMarker(target, i.viewProj, i.width, i.height);
      const d = Math.hypot(target.x - i.ownPos.x, target.y - i.ownPos.y, target.z - i.ownPos.z);
      model.next = g < 0 ? "EXIT" : `GATE ${own.gates + 1}/${LAP_GATES}`;
      model.marker = { ...sm, label: model.next, dist: `${Math.round(Math.max(0, g < 0 ? d - b.exitRadius : d))} u`, exit: g < 0 };
      model.wrongWay = racing && i.wrongWay;
    }
    if (own.finishTick > 0 && racing) {
      model.finish = { place: own.place, time: formatRaceTime(i.raceStart > 0 ? (own.finishTick - i.raceStart) / TICK_HZ : 0) };
    }
  }

  // Radiation meter.
  if (i.dose > 0.05 && !i.spectating) {
    const level = Math.min(1, i.dose / 16);
    const lasts = i.dose > 0 ? i.hull / i.dose : Infinity;
    const max = i.hullMax ?? 100;
    model.rad = {
      level,
      rate: `${i.dose.toFixed(1)} hull/s`,
      lasts: i.instagib ? `dose left ${Math.round(Math.max(0, Math.min(100, (100 * i.hull) / max)))}%` : Number.isFinite(lasts) ? `hull lasts ${Math.max(0, Math.round(lasts))} s` : "",
      danger: lasts < 8,
      instagib: i.instagib,
    };
  }

  // Heat results.
  if (race.phase === "results" || race.phase === "done") {
    const rows: ResultRow[] = [...race.racers]
      .sort((x, y) => (x.place || 99) - (y.place || 99) || x.id - y.id)
      .map((r) => ({
        place: r.place,
        name: nameOf(r.id),
        time: r.finishTick > 0 && i.raceStart > 0 ? formatRaceTime((r.finishTick - i.raceStart) / TICK_HZ) : r.finishTick > 0 ? "finished" : "DNF",
        gain: r.place > 0 ? `+${HEAT_POINTS[r.place - 1] ?? 0}` : "+0",
        total: i.points.get(r.id) ?? 0,
        own: r.id === i.own,
        color: colorOf(r.id),
      }));
    model.results = { title: `HEAT ${race.heat + 1} OF ${race.heats} · RESULTS`, rows };
  }

  // Minimap.
  const dots: MiniDot[] = [];
  for (const r of race.racers) {
    const p = r.id === i.own ? (i.ownPos ?? i.positions.get(r.id)) : i.positions.get(r.id);
    if (!p) continue;
    const m = miniPoint(b, p);
    dots.push({ ...m, color: colorOf(r.id), own: r.id === i.own, done: r.finishTick > 0 });
  }
  // The own dot on top.
  dots.sort((x, y) => Number(x.own) - Number(y.own));
  const gates: MiniGate[] = [];
  for (let k = 0; k < WEDGES; k++) {
    const a = gateAzimuth(k);
    const r0 = (b.gateInnerRadius / (0.5 * (b.innerRadius + b.outerRadius))) * MINI_RING;
    const r1 = (b.gateOuterRadius / (0.5 * (b.innerRadius + b.outerRadius))) * MINI_RING;
    let state: MiniGate["state"] = "later";
    if (own) {
      const ln = lapNumber(k, own.slot);
      state = ln <= own.gates ? "passed" : ln === own.gates + 1 ? "next" : "later";
    }
    const c = MINI_SIZE / 2;
    gates.push({ x1: c + r0 * Math.cos(a), y1: c - r0 * Math.sin(a), x2: c + r1 * Math.cos(a), y2: c - r1 * Math.sin(a), state });
  }
  model.mini = { dots, gates, ownColor: colorOf(i.own) };
  return model;
}

const SVG = "http://www.w3.org/2000/svg";

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, parent: HTMLElement): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = cls;
  parent.appendChild(e);
  return e;
}

function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string>, parent?: Element): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  parent?.appendChild(e);
  return e;
}

/** The radiation trefoil (SVG path, centred at 0,0, radius 10). */
const TREFOIL =
  "M0 -2.2 A2.2 2.2 0 1 1 0 2.2 A2.2 2.2 0 1 1 0 -2.2 Z " +
  [0, 120, 240]
    .map((deg) => {
      const a0 = ((deg - 90 - 30) * Math.PI) / 180;
      const a1 = ((deg - 90 + 30) * Math.PI) / 180;
      const p = (r: number, a: number): string => `${(r * Math.cos(a)).toFixed(2)} ${(r * Math.sin(a)).toFixed(2)}`;
      return `M${p(3.4, a0)} L${p(10, a0)} A10 10 0 0 1 ${p(10, a1)} L${p(3.4, a1)} A3.4 3.4 0 0 0 ${p(3.4, a0)} Z`;
    })
    .join(" ");

/** The race HUD's DOM. */
export class RaceHud {
  readonly root: HTMLElement;
  private readonly card: HTMLElement;
  private readonly placeEl: HTMLElement;
  private readonly ofEl: HTMLElement;
  private readonly heatEl: HTMLElement;
  private readonly clockEl: HTMLElement;
  private readonly aheadEl: HTMLElement;
  private readonly behindEl: HTMLElement;
  private readonly distEl: HTMLElement;
  private readonly lightsEl: HTMLElement;
  private readonly lamps: HTMLElement[] = [];
  private readonly goEl: HTMLElement;
  private readonly wrongEl: HTMLElement;
  private readonly radEl: HTMLElement;
  private readonly radBar: HTMLElement;
  private readonly radRate: HTMLElement;
  private readonly radLasts: HTMLElement;
  private readonly finishEl: HTMLElement;
  private readonly resultsEl: HTMLElement;
  private readonly markerEl: HTMLElement;
  private readonly markerLabel: HTMLElement;
  private readonly markerArrow: HTMLElement;
  private readonly mini: SVGSVGElement;
  private readonly miniGates: SVGGElement;
  private readonly miniDots: SVGGElement;
  private shown: Record<string, string> = {};

  constructor(parent: HTMLElement) {
    this.root = el("div", "race-hud", parent);
    // Start lights.
    this.lightsEl = el("div", "race-lights hidden", this.root);
    const row = el("div", "race-lamps", this.lightsEl);
    for (let k = 0; k < START_LAMPS; k++) this.lamps.push(el("div", "race-lamp", row));
    this.goEl = el("div", "race-go", this.lightsEl);
    this.goEl.textContent = "BELT UP!";
    // Position card.
    this.card = el("div", "race-card hidden", this.root);
    const head = el("div", "race-place-row", this.card);
    this.placeEl = el("span", "race-place", head);
    this.ofEl = el("span", "race-of", head);
    this.heatEl = el("div", "race-heat", this.card);
    this.clockEl = el("div", "race-clock", this.card);
    this.aheadEl = el("div", "race-gap ahead", this.card);
    this.behindEl = el("div", "race-gap behind", this.card);
    this.distEl = el("div", "race-dist", this.card);
    // Minimap.
    const miniBox = el("div", "race-mini", this.root);
    this.mini = svg("svg", { viewBox: `0 0 ${MINI_SIZE} ${MINI_SIZE}`, class: "race-mini-svg" });
    miniBox.appendChild(this.mini);
    const c = MINI_SIZE / 2;
    svg("circle", { cx: `${c}`, cy: `${c}`, r: `${MINI_RING}`, class: "race-mini-belt" }, this.mini);
    svg("circle", { cx: `${c}`, cy: `${c}`, r: `${MINI_RING * 0.31}`, class: "race-mini-cocoon" }, this.mini);
    svg("circle", { cx: `${c}`, cy: `${c}`, r: "4.5", class: "race-mini-exit" }, this.mini);
    this.miniGates = svg("g", {}, this.mini);
    this.miniDots = svg("g", {}, this.mini);
    // Next gate / exit marker.
    this.markerEl = el("div", "race-marker hidden", this.root);
    this.markerArrow = el("div", "race-marker-arrow", this.markerEl);
    this.markerLabel = el("div", "race-marker-label", this.markerEl);
    // Wrong way, finish, results.
    this.wrongEl = el("div", "race-wrong hidden", this.root);
    this.wrongEl.textContent = "WRONG WAY";
    this.finishEl = el("div", "race-finish hidden", this.root);
    this.resultsEl = el("div", "race-results hidden", this.root);
    // Radiation meter.
    this.radEl = el("div", "race-rad hidden", this.root);
    const icon = svg("svg", { viewBox: "-11 -11 22 22", class: "race-rad-icon" });
    svg("path", { d: TREFOIL, "fill-rule": "evenodd" }, icon);
    this.radEl.appendChild(icon);
    const radBody = el("div", "race-rad-body", this.radEl);
    const track = el("div", "race-rad-track", radBody);
    this.radBar = el("div", "race-rad-bar", track);
    const text = el("div", "race-rad-text", radBody);
    this.radRate = el("span", "race-rad-rate", text);
    this.radLasts = el("span", "race-rad-lasts", text);
  }

  private changed(key: string, value: unknown): boolean {
    const s = JSON.stringify(value);
    if (this.shown[key] === s) return false;
    this.shown[key] = s;
    return true;
  }

  update(m: RaceHudModel): void {
    this.root.classList.toggle("hidden", !m.visible);
    if (!m.visible) return;
    if (this.changed("lights", m.lights)) {
      this.lightsEl.classList.toggle("hidden", !m.lights.show);
      this.lightsEl.classList.toggle("go", m.lights.go);
      this.lamps.forEach((l, k) => l.classList.toggle("on", m.lights.go || k < m.lights.lit));
    }
    if (this.changed("card", [m.place, m.heat, m.points, m.clock, m.ahead, m.behind, m.distance])) {
      this.card.classList.toggle("hidden", m.place === null && m.heat === "");
      this.placeEl.textContent = m.place ? `P${m.place.place}` : "P–";
      this.ofEl.textContent = m.place ? `/${m.place.of}` : "";
      this.heatEl.textContent = `${m.heat} · ${m.points} PTS`;
      this.clockEl.textContent = m.clock;
      const gap = (e: HTMLElement, g: GapModel | null, arrow: string): void => {
        e.classList.toggle("hidden", g === null);
        if (!g) return;
        e.replaceChildren();
        const a = document.createElement("span");
        a.className = "race-gap-arrow";
        a.textContent = arrow;
        const dot = document.createElement("span");
        dot.className = "race-gap-dot";
        dot.style.background = g.color;
        const name = document.createElement("span");
        name.className = "race-gap-name";
        name.textContent = g.name;
        const t = document.createElement("span");
        t.className = "race-gap-time";
        t.textContent = g.gap;
        e.append(a, dot, name, t);
      };
      gap(this.aheadEl, m.ahead, "▲");
      gap(this.behindEl, m.behind, "▼");
      this.distEl.textContent = m.distance;
      this.distEl.classList.toggle("hidden", m.distance === "");
    }
    if (this.changed("marker", m.marker && { ...m.marker, x: Math.round(m.marker.x), y: Math.round(m.marker.y), angle: Math.round(m.marker.angle * 50) })) {
      const mk = m.marker;
      this.markerEl.classList.toggle("hidden", mk === null);
      if (mk) {
        this.markerEl.style.transform = `translate(${mk.x.toFixed(0)}px, ${mk.y.toFixed(0)}px)`;
        this.markerEl.classList.toggle("off", !mk.onScreen);
        this.markerEl.classList.toggle("exit", mk.exit);
        this.markerArrow.style.transform = `rotate(${mk.angle.toFixed(3)}rad)`;
        this.markerLabel.textContent = `${mk.label} · ${mk.dist}`;
      }
    }
    if (this.changed("wrong", m.wrongWay)) this.wrongEl.classList.toggle("hidden", !m.wrongWay);
    if (this.changed("rad", m.rad && { ...m.rad, level: Math.round(m.rad.level * 100) })) {
      this.radEl.classList.toggle("hidden", m.rad === null);
      if (m.rad) {
        this.radEl.style.setProperty("--rad", m.rad.level.toFixed(3));
        this.radEl.classList.toggle("danger", m.rad.danger);
        this.radBar.style.width = `${Math.round(Math.max(4, m.rad.level * 100))}%`;
        this.radRate.textContent = `RADIATION ${m.rad.rate}`;
        this.radLasts.textContent = m.rad.lasts;
      }
    }
    if (this.changed("finish", m.finish)) {
      this.finishEl.classList.toggle("hidden", m.finish === null);
      if (m.finish) {
        this.finishEl.replaceChildren();
        const big = document.createElement("div");
        big.className = "race-finish-big";
        big.textContent = `FINISHED P${m.finish.place}`;
        const t = document.createElement("div");
        t.className = "race-finish-time";
        t.textContent = m.finish.time;
        this.finishEl.append(big, t);
      }
    }
    if (this.changed("results", m.results)) {
      this.resultsEl.classList.toggle("hidden", m.results === null);
      // Phones: the heat table covers the place card (raceHud.css hides the card under it).
      this.root.classList.toggle("with-results", m.results !== null);
      if (m.results) {
        this.resultsEl.replaceChildren();
        const h = document.createElement("div");
        h.className = "race-results-title";
        h.textContent = m.results.title;
        this.resultsEl.appendChild(h);
        for (const r of m.results.rows) {
          const row = document.createElement("div");
          row.className = `race-results-row${r.own ? " own" : ""}`;
          const cells: Array<[string, string]> = [
            ["place", r.place > 0 ? `P${r.place}` : "–"],
            ["name", r.name],
            ["time", r.time],
            ["gain", r.gain],
            ["total", `${r.total}`],
          ];
          for (const [cls, text] of cells) {
            const s = document.createElement("span");
            s.className = `race-results-${cls}`;
            s.textContent = text;
            if (cls === "name") s.style.setProperty("--side", r.color);
            row.appendChild(s);
          }
          this.resultsEl.appendChild(row);
        }
      }
    }
    if (m.mini && this.changed("mini", { ...m.mini, dots: m.mini.dots.map((d) => [Math.round(d.x), Math.round(d.y), d.color, d.own, d.done]) })) {
      this.miniGates.replaceChildren();
      for (const g of m.mini.gates) {
        const line = svg("line", { x1: g.x1.toFixed(1), y1: g.y1.toFixed(1), x2: g.x2.toFixed(1), y2: g.y2.toFixed(1), class: `race-mini-gate ${g.state}` }, this.miniGates);
        if (g.state === "passed") line.style.stroke = m.mini.ownColor;
      }
      this.miniDots.replaceChildren();
      for (const d of m.mini.dots) {
        const c = svg("circle", { cx: d.x.toFixed(1), cy: d.y.toFixed(1), r: d.own ? "5.5" : "4", class: `race-mini-dot${d.own ? " own" : ""}${d.done ? " done" : ""}` }, this.miniDots);
        c.style.fill = d.color;
      }
    }
  }

  dispose(): void {
    this.root.remove();
  }
}
