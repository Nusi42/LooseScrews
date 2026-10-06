// HUD pieces of M4 B5 (game/worldFx.ts, game/moodFx.ts):
// - the pull indicator: a violet arc on a ring around the crosshair pointing
//   where a gravity well pulls the own ship, thicker and brighter the harder
//   it pulls, with "PULL n%" (of the ship's thrust) under the crosshair; a
//   pull along the view shows as a dot (ahead) or a broken ring (behind);
// - the mood badge: a small face for the level's mood (calm, bored, grumpy,
//   giddy) at the top left, with the mood's line for a moment after it
//   changes.
// DOM writes only happen when the shown values change.

import type { MoodIconModel } from "../game/moodFx.ts";
import type { PullModel } from "../game/worldFx.ts";

const SVG = "http://www.w3.org/2000/svg";

function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string>, parent?: Element): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  parent?.appendChild(e);
  return e;
}

/** Ring radius of the pull indicator (px of its 200 px box). */
export const PULL_RING = 64;

/** SVG arc path on the pull ring centred on screen angle `ang` (rad, 0 = right, counter-clockwise up), `span` rad wide. */
export function pullArc(ang: number, span: number, r = PULL_RING, c = 100): string {
  const a0 = ang - span / 2;
  const a1 = ang + span / 2;
  const p = (a: number): string => `${(c + Math.cos(a) * r).toFixed(1)} ${(c - Math.sin(a) * r).toFixed(1)}`;
  return `M ${p(a0)} A ${r} ${r} 0 0 0 ${p(a1)}`;
}

/** What the indicator draws for a pull (pure, for tests). */
export function pullView(p: PullModel | null): { visible: boolean; arc: string | null; chevron: string | null; ahead: boolean; width: number; opacity: number; label: string } {
  if (!p) return { visible: false, arc: null, chevron: null, ahead: false, width: 0, opacity: 0, label: "" };
  const width = 3 + 7 * p.strength;
  const opacity = 0.35 + 0.65 * Math.min(1, p.strength * 1.4);
  const label = `PULL ${Math.round(p.ofThrust * 100)}%`;
  if (!p.screen) return { visible: true, arc: null, chevron: null, ahead: p.ahead, width, opacity, label };
  const ang = Math.atan2(p.screen.y, p.screen.x);
  const span = 0.35 + 0.9 * p.strength;
  // A chevron just outside the ring, pointing outward along the pull.
  const tip = PULL_RING + 14 + 4 * p.strength;
  const base = PULL_RING + 6;
  const pt = (r: number, a: number): string => `${(100 + Math.cos(a) * r).toFixed(1)},${(100 - Math.sin(a) * r).toFixed(1)}`;
  const chevron = `${pt(base, ang - 0.12)} ${pt(tip, ang)} ${pt(base, ang + 0.12)}`;
  return { visible: true, arc: pullArc(ang, span), chevron, ahead: p.ahead, width, opacity, label };
}

export class PullIndicator {
  readonly root: HTMLElement;
  private readonly arc: SVGPathElement;
  private readonly chevron: SVGPolylineElement;
  private readonly dot: SVGCircleElement;
  private readonly behind: SVGCircleElement;
  private readonly label: HTMLElement;
  private shown = "";

  constructor(parent: HTMLElement) {
    this.root = document.createElement("div");
    this.root.className = "pull-indicator hidden";
    const box = svg("svg", { viewBox: "0 0 200 200", class: "pull-svg" }, this.root);
    svg("circle", { cx: "100", cy: "100", r: String(PULL_RING), class: "pull-track" }, box);
    this.arc = svg("path", { class: "pull-arc", d: "" }, box);
    this.chevron = svg("polyline", { class: "pull-chevron", points: "" }, box);
    this.dot = svg("circle", { cx: "100", cy: "100", r: "5", class: "pull-dot" }, box);
    this.behind = svg("circle", { cx: "100", cy: "100", r: String(PULL_RING - 10), class: "pull-behind" }, box);
    this.label = document.createElement("div");
    this.label.className = "pull-label";
    this.root.appendChild(this.label);
    parent.appendChild(this.root);
  }

  update(p: PullModel | null): void {
    const v = pullView(p);
    const key = v.visible ? `${v.arc}|${v.chevron}|${v.ahead}|${v.width.toFixed(1)}|${v.opacity.toFixed(2)}|${v.label}` : "";
    if (key === this.shown) return;
    this.shown = key;
    this.root.classList.toggle("hidden", !v.visible);
    if (!v.visible) return;
    this.root.style.setProperty("--pull-width", v.width.toFixed(1));
    this.root.style.setProperty("--pull-opacity", v.opacity.toFixed(2));
    this.arc.setAttribute("d", v.arc ?? "");
    this.chevron.setAttribute("points", v.chevron ?? "");
    const along = v.arc === null;
    this.dot.style.display = along && v.ahead ? "" : "none";
    this.behind.style.display = along && !v.ahead ? "" : "none";
    this.label.textContent = v.label;
  }

  dispose(): void {
    this.root.remove();
  }
}

/** Face of a mood as SVG markup inside a 40 x 40 box (eyes, brows, mouth). */
export function moodFace(mood: string): string {
  switch (mood) {
    case "bored":
      return '<circle cx="20" cy="20" r="17" class="mood-head"/><path d="M10 16 h7 M23 16 h7" class="mood-lid"/><circle cx="13.5" cy="18" r="1.6" class="mood-eye"/><circle cx="26.5" cy="18" r="1.6" class="mood-eye"/><path d="M13 28 h14" class="mood-mouth"/>';
    case "grumpy":
      return '<circle cx="20" cy="20" r="17" class="mood-head"/><path d="M9 12 l8 4 M31 12 l-8 4" class="mood-lid"/><circle cx="14" cy="18.5" r="2" class="mood-eye"/><circle cx="26" cy="18.5" r="2" class="mood-eye"/><path d="M12 30 q8 -7 16 0" class="mood-mouth"/>';
    case "giddy":
      return '<circle cx="20" cy="20" r="17" class="mood-head"/><path d="M10 17 q3.5 -5 7 0 M23 17 q3.5 -5 7 0" class="mood-lid"/><path d="M11 23 q9 11 18 0 z" class="mood-grin"/><path d="M33 6 l1.2 3 3 1.2 -3 1.2 -1.2 3 -1.2 -3 -3 -1.2 3 -1.2z" class="mood-spark"/>';
    default:
      return '<circle cx="20" cy="20" r="17" class="mood-head"/><path d="M10 18 q3.5 3 7 0 M23 18 q3.5 3 7 0" class="mood-lid"/><path d="M14 26 q6 4 12 0" class="mood-mouth"/>';
  }
}

export class MoodBadge {
  readonly root: HTMLElement;
  private readonly face: HTMLElement;
  private readonly name: HTMLElement;
  private readonly line: HTMLElement;
  private shown = "";

  constructor(parent: HTMLElement) {
    this.root = document.createElement("div");
    this.root.className = "mood-badge hidden";
    this.face = document.createElement("div");
    this.face.className = "mood-face";
    this.name = document.createElement("div");
    this.name.className = "mood-name";
    this.line = document.createElement("div");
    this.line.className = "mood-line";
    this.root.append(this.face, this.name, this.line);
    parent.appendChild(this.root);
  }

  update(m: MoodIconModel | null): void {
    const key = m ? `${m.mood}|${m.announce ?? ""}` : "";
    if (key === this.shown) return;
    this.shown = key;
    this.root.classList.toggle("hidden", !m);
    if (!m) return;
    this.root.dataset["mood"] = m.mood;
    this.root.style.setProperty("--mood-accent", m.accent);
    this.face.innerHTML = `<svg viewBox="0 0 40 40">${moodFace(m.mood)}</svg>`;
    this.name.textContent = m.label;
    this.line.textContent = m.announce ?? "";
    this.root.classList.toggle("announce", m.announce !== null);
  }

  dispose(): void {
    this.root.remove();
  }
}
