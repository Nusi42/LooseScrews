// HUD of the visitor events (M4 C2, game/events/visitors.ts): the Gossip
// wall's through-wall markers (every revealed ship with its name and
// distance) and its speech bubble at the face; the Customer survey's prompt
// for the surveyed leader; the Pizza delivery's hold ring for the
// recipient; and short lines (toasts) when a visitor does something
// ("Taxed! 10 % of your energy.", "Rita rated Bo's match" and the stars).
// The layout is a pure function of the model and a projection, so the
// gallery and tests can drive it.

import type { Vec3 } from "../math/vec3.ts";
import type { Toast } from "../game/events/visitorKit.ts";
import type { GossipMarker } from "../game/events/gossipWall.ts";

/** Seconds a toast stays up (it fades over the last 0.4 s). */
export const TOAST_SECS = 3.6;
/** At most this many toasts at once (the oldest goes). */
export const MAX_TOASTS = 3;
/** Markers keep this far (px) from the screen edge when their ship is off screen. */
export const EDGE = 28;

export interface VisitorHudModel {
  markers: readonly GossipMarker[];
  bubble: { text: string; anchor: Vec3 } | null;
  /** The own pilot is being surveyed. */
  survey: boolean;
  /** The own pizza's hold progress 0..1, null without one. */
  pizza: number | null;
  /** New toasts this frame. */
  toasts: readonly Toast[];
}

export function emptyVisitorHud(): VisitorHudModel {
  return { markers: [], bubble: null, survey: false, pizza: null, toasts: [] };
}

/** Clip-space projection of a world point: x, y in -w..w (y up) and w (> 0 in front). */
export type Project = (p: Vec3) => { x: number; y: number; w: number };

/** Where a marker goes on a `width` x `height` screen: on its ship, or pinned to the edge toward it (with the arrow's angle). */
export function markerSpot(clip: { x: number; y: number; w: number }, width: number, height: number): { x: number; y: number; edge: boolean; angle: number } {
  let nx = clip.x / Math.max(1e-4, Math.abs(clip.w));
  let ny = clip.y / Math.max(1e-4, Math.abs(clip.w));
  const behind = clip.w <= 1e-3;
  if (behind) {
    nx = -nx;
    ny = -ny;
  }
  const inside = !behind && Math.abs(nx) <= 1 && Math.abs(ny) <= 1;
  if (inside) return { x: (nx * 0.5 + 0.5) * width, y: (0.5 - ny * 0.5) * height, edge: false, angle: 0 };
  // Off screen: along the direction from the centre to the edge band.
  const hx = width / 2 - EDGE, hy = height / 2 - EDGE;
  const dx = nx * (width / 2), dy = -ny * (height / 2);
  const len = Math.hypot(dx, dy) || 1;
  const k = Math.min(hx / Math.max(1e-4, Math.abs(dx / len)), hy / Math.max(1e-4, Math.abs(dy / len)));
  return { x: width / 2 + (dx / len) * k, y: height / 2 + (dy / len) * k, edge: true, angle: Math.atan2(dy, dx) };
}

/** Label margin from the screen edge (px). */
export const LABEL_MARGIN = 4;

/**
 * Horizontal shift (px) of a label `labelWidth` wide centred on `x` that keeps
 * it on a `width` wide screen (the eye and arrow stay on the spot; a name
 * pinned to the left or right edge would otherwise be cut off).
 */
export function labelShift(x: number, labelWidth: number, width: number, margin = LABEL_MARGIN): number {
  const half = labelWidth / 2;
  const lo = margin + half - x;
  const hi = width - margin - half - x;
  if (lo > hi) return (lo + hi) / 2; // wider than the screen: centred
  return Math.max(lo, Math.min(hi, 0));
}

interface LiveToast {
  toast: Toast;
  born: number;
  el: HTMLElement;
}

const EYE_ICON = `<svg viewBox="0 0 24 16" aria-hidden="true"><path d="M1 8 C5 1 19 1 23 8 C19 15 5 15 1 8 Z" fill="#fff6f3" stroke="#3a2230" stroke-width="1.6"/><circle cx="12" cy="8" r="4.2" fill="#3b8fd6"/><circle cx="12" cy="8" r="2" fill="#140c12"/><circle cx="13.4" cy="6.6" r="0.9" fill="#fff"/></svg>`;

const FACE_ICON = `<svg viewBox="0 0 40 40" aria-hidden="true"><ellipse cx="20" cy="21" rx="15" ry="17" fill="#9a8a78" stroke="#3a2f28" stroke-width="1.6"/><path d="M9 13 Q14 9 18 13 M22 12 Q27 8 31 12" stroke="#3a2f28" stroke-width="2" fill="none" stroke-linecap="round"/><ellipse cx="14" cy="17" rx="3.4" ry="2.2" fill="#fff"/><ellipse cx="26" cy="17" rx="3.4" ry="2.2" fill="#fff"/><circle cx="15.5" cy="17.3" r="1.4" fill="#2a5a8a"/><circle cx="27.5" cy="17.3" r="1.4" fill="#2a5a8a"/><path d="M12 15.2 H17 M23 15.2 H29" stroke="#7a6a5a" stroke-width="2"/><path d="M19 19 Q20 25 18 26" stroke="#3a2f28" stroke-width="1.4" fill="none"/><path d="M13 30 Q20 27 27 30 Q20 35 13 30 Z" fill="#c8506a" stroke="#3a2f28" stroke-width="1.2"/></svg>`;

/** The visitor HUD's DOM. */
export class VisitorHud {
  readonly root: HTMLElement;
  private readonly markers: HTMLElement;
  private readonly bubble: HTMLElement;
  private readonly bubbleText: HTMLElement;
  private readonly survey: HTMLElement;
  private readonly pizza: HTMLElement;
  private readonly pizzaArc: SVGCircleElement;
  private readonly toastBox: HTMLElement;
  private readonly pool: HTMLElement[] = [];
  private toasts: LiveToast[] = [];
  private bubbleShown = "";

  constructor(parent: HTMLElement) {
    this.root = el("div", "visitor-hud", parent);
    this.markers = el("div", "visitor-markers", this.root);
    this.bubble = el("div", "gossip-bubble hidden", this.root);
    el("div", "gossip-bubble-face", this.bubble).innerHTML = FACE_ICON;
    this.bubbleText = el("div", "gossip-bubble-text", this.bubble);
    this.survey = el("div", "survey-prompt hidden", this.root);
    el("div", "survey-title", this.survey).textContent = "How was your match today?";
    const stars = el("div", "survey-stars", this.survey);
    for (let k = 0; k < 5; k++) el("span", "star", stars).textContent = "★";
    el("div", "survey-sub", this.survey).textContent = "Shoot a star. Anyone may answer.";
    this.pizza = el("div", "pizza-hold hidden", this.root);
    this.pizza.innerHTML = `<svg viewBox="0 0 100 100"><circle class="pizza-track" cx="50" cy="50" r="42"/><circle class="pizza-arc" cx="50" cy="50" r="42"/></svg><div class="pizza-label">Hold still for pizza</div>`;
    this.pizzaArc = this.pizza.querySelector(".pizza-arc") as SVGCircleElement;
    this.toastBox = el("div", "visitor-toasts", this.root);
  }

  update(m: VisitorHudModel, project: Project | null, width: number, height: number, now: number): void {
    // Through-wall markers.
    let used = 0;
    if (project) {
      for (const k of m.markers) {
        const spot = markerSpot(project(k.position), width, height);
        const node = this.pool[used] ?? this.newMarker();
        used++;
        node.classList.toggle("edge", spot.edge);
        node.style.transform = `translate(${spot.x.toFixed(1)}px, ${spot.y.toFixed(1)}px)`;
        node.style.setProperty("--arrow", `${spot.angle.toFixed(3)}rad`);
        const label = `${k.name} · ${Math.round(k.distance)} u`;
        const text = node.querySelector(".visitor-marker-name") as HTMLElement;
        if (text.textContent !== label) text.textContent = label;
        const shift = labelShift(spot.x, text.offsetWidth, width);
        text.style.transform = shift !== 0 ? `translateX(${shift.toFixed(1)}px)` : "";
      }
    }
    for (let k = 0; k < this.pool.length; k++) this.pool[k]!.classList.toggle("hidden", k >= used);
    // The gossip bubble beside the face (docked at the top when the face is off screen).
    if (m.bubble && project) {
      const spot = markerSpot(project(m.bubble.anchor), width, height);
      this.bubble.classList.remove("hidden");
      this.bubble.classList.toggle("docked", spot.edge);
      const x = spot.edge ? width / 2 : Math.max(180, Math.min(width - 180, spot.x));
      const y = spot.edge ? 110 : Math.max(100, Math.min(height - 40, spot.y));
      this.bubble.style.transform = `translate(${x.toFixed(1)}px, ${y.toFixed(1)}px)`;
      if (this.bubbleShown !== m.bubble.text) {
        this.bubbleShown = m.bubble.text;
        this.bubbleText.textContent = m.bubble.text;
        // Restart the pop-in.
        this.bubble.classList.remove("pop");
        void this.bubble.offsetWidth;
        this.bubble.classList.add("pop");
      }
    } else {
      this.bubble.classList.add("hidden");
      this.bubbleShown = "";
    }
    this.survey.classList.toggle("hidden", !m.survey);
    this.pizza.classList.toggle("hidden", m.pizza === null);
    if (m.pizza !== null) {
      const c = 2 * Math.PI * 42;
      this.pizzaArc.style.strokeDasharray = `${(c * Math.max(0, Math.min(1, m.pizza))).toFixed(1)} ${c.toFixed(1)}`;
    }
    // Toasts.
    for (const t of m.toasts) {
      const node = el("div", `visitor-toast${t.own ? " own" : ""}`, this.toastBox);
      node.dataset["event"] = t.event;
      node.textContent = t.text;
      if (t.stars !== undefined) {
        const box = el("span", "visitor-toast-stars", node);
        for (let k = 1; k <= 5; k++) el("span", k <= t.stars ? "star on" : "star", box).textContent = "★";
      }
      this.toasts.push({ toast: t, born: now, el: node });
    }
    while (this.toasts.length > MAX_TOASTS) this.toasts.shift()?.el.remove();
    this.toasts = this.toasts.filter((t) => {
      const age = now - t.born;
      if (age > TOAST_SECS) {
        t.el.remove();
        return false;
      }
      t.el.style.opacity = String(Math.min(1, (TOAST_SECS - age) / 0.4));
      return true;
    });
  }

  clear(): void {
    for (const t of this.toasts) t.el.remove();
    this.toasts = [];
    this.update(emptyVisitorHud(), null, 0, 0, 0);
  }

  dispose(): void {
    this.root.remove();
  }

  private newMarker(): HTMLElement {
    const node = el("div", "visitor-marker", this.markers);
    el("div", "visitor-marker-eye", node).innerHTML = EYE_ICON;
    el("div", "visitor-marker-name", node);
    el("div", "visitor-marker-arrow", node);
    this.pool.push(node);
    return node;
  }
}

function el(tag: string, cls: string, parent: HTMLElement): HTMLElement {
  const e = document.createElement(tag);
  e.className = cls;
  parent.appendChild(e);
  return e;
}
