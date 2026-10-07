// The comedy events' HUD (M4 C1): the event banner and the per-event HUD
// hints of game/events/.
// - Banner: during the 3 s announcement a card under the mode bar pops in
//   with the event's icon, kicker, name, the player it aims at and a
//   countdown bar; while the event runs it shrinks to a pill with the icon,
//   the name and a ring of the time left; it slides away at the end.
// - Down arrow (Bored level): an arrow on a ring around the crosshair
//   pointing at the level's new down while it turns.
// - Retro title card (Wrong genre): "WORLD 1-1" when the genre switches,
//   "1UP" when it switches back, in a blocky 8-bit style.
// - Sleepy letters (Bored level's announcement): Z's drifting up.
// - Comic words ("HIC!") popping on the screen.
// `#ui[data-event]` and `[data-event-stage]` expose the event to the smoke.

import { eventInfo } from "../game/events/info.ts";
import type { EventHudHints } from "../game/events/types.ts";

export interface EventBannerModel {
  id: string;
  stage: "announce" | "active";
  /** Seconds left of the stage. */
  secsLeft: number;
  /** Length of the stage (s). */
  total: number;
  /** Name of the player the event aims at ("" = none). */
  target: string;
}

export interface EventHudModel {
  banner: EventBannerModel | null;
  hints: EventHudHints;
  /** The side view hides the crosshair and the fit ring. */
  sideView: boolean;
}

const SVG_NS = "http://www.w3.org/2000/svg";

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, parent?: HTMLElement): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = cls;
  parent?.appendChild(e);
  return e;
}

/** Ring circumference of the pill's timer (r = 15). */
const RING = 2 * Math.PI * 15;

export class EventHud {
  readonly root: HTMLDivElement;
  private readonly ui: HTMLElement;
  private readonly card: HTMLDivElement;
  private readonly icon: HTMLDivElement;
  private readonly kicker: HTMLDivElement;
  private readonly name: HTMLDivElement;
  private readonly target: HTMLDivElement;
  private readonly bar: HTMLDivElement;
  private readonly ring: SVGCircleElement;
  private readonly down: HTMLDivElement;
  private readonly retro: HTMLDivElement;
  private readonly retroTitle: HTMLDivElement;
  private readonly retroSub: HTMLDivElement;
  private readonly zzz: HTMLDivElement;
  private readonly pops: HTMLDivElement[] = [];
  private shownId = "";
  private shownStage = "";

  constructor(ui: HTMLElement) {
    this.ui = ui;
    this.root = el("div", "event-hud");
    this.card = el("div", "event-card", this.root);
    const ringSvg = document.createElementNS(SVG_NS, "svg");
    ringSvg.setAttribute("viewBox", "0 0 36 36");
    ringSvg.classList.add("event-ring");
    const track = document.createElementNS(SVG_NS, "circle");
    track.setAttribute("cx", "18");
    track.setAttribute("cy", "18");
    track.setAttribute("r", "15");
    track.setAttribute("class", "event-ring-track");
    this.ring = document.createElementNS(SVG_NS, "circle");
    this.ring.setAttribute("cx", "18");
    this.ring.setAttribute("cy", "18");
    this.ring.setAttribute("r", "15");
    this.ring.setAttribute("class", "event-ring-fill");
    this.ring.setAttribute("stroke-dasharray", RING.toFixed(2));
    ringSvg.append(track, this.ring);
    this.icon = el("div", "event-icon", this.card);
    this.card.appendChild(ringSvg);
    const text = el("div", "event-text", this.card);
    this.kicker = el("div", "event-kicker", text);
    this.name = el("div", "event-name", text);
    this.target = el("div", "event-target", text);
    this.bar = el("div", "event-bar", el("div", "event-bar-track", this.card));
    this.down = el("div", "event-down", this.root);
    this.down.innerHTML = `<svg viewBox="0 0 120 120"><circle cx="60" cy="60" r="50" class="event-down-ring"/><g class="event-down-arrow"><path d="M60 118l-13-20h8V84h10v14h8z"/></g></svg><span>down</span>`;
    this.retro = el("div", "event-retro", this.root);
    this.retroTitle = el("div", "event-retro-title", this.retro);
    this.retroSub = el("div", "event-retro-sub", this.retro);
    this.zzz = el("div", "event-zzz", this.root);
    for (let i = 0; i < 7; i++) {
      const z = el("span", "", this.zzz);
      z.textContent = i % 2 === 0 ? "Z" : "z";
      z.style.setProperty("--i", String(i));
    }
    for (let i = 0; i < 4; i++) {
      const p = el("div", "event-pop", this.root);
      p.style.opacity = "0";
      this.pops.push(p);
    }
    ui.appendChild(this.root);
  }

  update(m: EventHudModel): void {
    const b = m.banner;
    if (b) {
      if (b.id !== this.shownId) {
        const info = eventInfo(b.id);
        this.icon.innerHTML = `<svg viewBox="0 0 48 48">${info.icon}</svg>`;
        this.kicker.textContent = info.kicker;
        this.name.textContent = info.name;
        this.card.style.setProperty("--event-accent", info.accent);
        this.shownId = b.id;
        // Restart the pop-in animation for a new event.
        this.card.classList.remove("announce", "active");
        void this.card.offsetWidth;
      }
      this.target.textContent = b.target ? `for ${b.target}` : "";
      this.target.hidden = !b.target;
      if (b.stage !== this.shownStage) {
        this.card.classList.toggle("announce", b.stage === "announce");
        this.card.classList.toggle("active", b.stage === "active");
        this.shownStage = b.stage;
      }
      const frac = b.total > 0 ? Math.min(1, Math.max(0, b.secsLeft / b.total)) : 0;
      this.bar.style.transform = `scaleX(${frac.toFixed(4)})`;
      this.ring.setAttribute("stroke-dashoffset", (RING * (1 - frac)).toFixed(2));
      this.card.classList.add("shown");
      this.ui.dataset["event"] = b.id;
      this.ui.dataset["eventStage"] = b.stage;
    } else {
      this.card.classList.remove("shown");
      this.shownStage = "";
      // The next event pops in again, even when it is the same one.
      this.shownId = "";
      delete this.ui.dataset["event"];
      delete this.ui.dataset["eventStage"];
    }
    const d = m.hints.down;
    this.down.style.opacity = d ? d.alpha.toFixed(3) : "0";
    if (d) this.down.style.setProperty("--down-angle", `${((d.angle * 180) / Math.PI).toFixed(2)}deg`);
    const r = m.hints.retro;
    this.retro.style.opacity = r ? r.alpha.toFixed(3) : "0";
    if (r) {
      this.retroTitle.textContent = r.title;
      this.retroSub.textContent = r.sub;
    }
    this.zzz.style.opacity = m.hints.zzz.toFixed(3);
    this.pops.forEach((p, i) => {
      const w = m.hints.pops[i];
      p.style.opacity = w ? w.alpha.toFixed(3) : "0";
      if (!w) return;
      if (p.textContent !== w.text) p.textContent = w.text;
      p.style.left = `${(w.x * 100).toFixed(2)}%`;
      p.style.top = `${(w.y * 100).toFixed(2)}%`;
      p.style.setProperty("--pop-color", w.color);
      p.style.transform = `translate(-50%, -50%) rotate(${w.tilt.toFixed(1)}deg) scale(${w.scale.toFixed(3)})`;
    });
    this.zzz.classList.toggle("drifting", m.hints.zzz > 0);
    this.ui.classList.toggle("event-side-view", m.sideView);
  }

  dispose(): void {
    this.root.remove();
    this.ui.classList.remove("event-side-view");
    delete this.ui.dataset["event"];
    delete this.ui.dataset["eventStage"];
  }
}
