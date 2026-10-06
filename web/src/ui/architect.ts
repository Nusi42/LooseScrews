// The Architect's HUD (M4 C4, milestone-4-plan.md §3.5): for the round's
// architect a drafting-table dock (the budget bar with what the armed stroke
// costs, eight stroke cards on keys 1-8 with their states), the reticle
// readout under the crosshair (stroke, size, why it cannot go there), the
// stroke wheel (hold R), and with the pointer free the world's slider panel
// (the workshop's `SliderPanel`: dragging a slider picks its stroke); for
// the runners a plate naming the architect with their budget. Both get the
// round banners and short toasts. Models are pure functions for Node tests;
// the DOM class only writes on change.

import { STROKES, strokeCost, strokeCss, strokeForSlider, type ArchitectRules, type ArchitectView } from "../game/architect/model.ts";
import type { MoodState } from "../game/modeState.ts";
import { nameOf } from "./modeHud.ts";
import { SLIDER_ICONS, SliderPanel, type SliderKey, type SliderValues } from "./workshop.ts";

export type CardState = "ready" | "armed" | "poor" | "cooldown" | "used" | "busy" | "off" | "full";

export interface StrokeCard {
  index: number;
  /** Number key (1-8). */
  key: number;
  name: string;
  verb: string;
  blurb: string;
  cost: number;
  css: string;
  /** Icon of the slider it moves (ui/workshop.ts SLIDER_ICONS). */
  slider: SliderKey;
  /** "+", "−" or "" for the switches. */
  sign: string;
  state: CardState;
  /** Why it is not ready ("" when ready). */
  note: string;
}

export interface ArchitectBanner {
  /** "Round 2 of 3". */
  kicker: string;
  title: string;
  sub: string;
  tone: "architect" | "runner" | "win" | "loss" | "neutral";
}

export interface ArchitectToast {
  text: string;
  tone: "bad" | "good" | "info";
}

export interface ArchitectHudInput {
  view: ArchitectView;
  rules: ArchitectRules;
  own: number;
  nowTick: number;
  names: ReadonlyMap<number, string>;
  /** Armed stroke index, -1 = none. */
  armed: number;
  radius: number;
  /** Why the armed stroke cannot go where the reticle points (null: it can). */
  problem: string | null;
  /** Distance to the aim point (u), null without one. */
  aimDistance: number | null;
  cooldownUntil: number;
  verticalityUsed: boolean;
  mood: MoodState;
  overview: boolean;
  pointerLocked: boolean;
  radial: { open: boolean; selected: number };
  banner: ArchitectBanner | null;
  toast: ArchitectToast | null;
  /** A stroke went out and the server has not answered yet. */
  waiting: boolean;
  /** The world's sliders (the panel's resting values), null when unknown. */
  sliders: SliderValues | null;
}

export interface ArchitectHudModel {
  role: "architect" | "runner";
  roundText: string;
  architectName: string;
  budget: number;
  budgetMax: number;
  budgetPerSec: number;
  /** Budget bar fill 0..100. */
  budgetFill: number;
  /** Fill left after paying the armed stroke (0..100), -1 without one. */
  afterFill: number;
  /** The armed stroke is affordable. */
  affordable: boolean;
  cards: StrokeCard[];
  reticle: { name: string; css: string; size: string; status: string; ok: boolean } | null;
  radial: { open: boolean; selected: number };
  overview: boolean;
  hint: string;
  banner: ArchitectBanner | null;
  toast: ArchitectToast | null;
  showPanel: boolean;
  sliders: SliderValues | null;
  /** "3 runners · 1 escaped · 1 out of lives". */
  runnersLine: string;
  /** Strokes in flight ("2 of 4 strokes in flight"), "" for none. */
  inFlight: string;
  /** Strokes telegraphing and acting now (also `data-telegraphs` / `data-acting` on the root for the browser smoke). */
  telegraphs: number;
  acting: number;
}

/** The state of a stroke's card regardless of where it is aimed. */
export function cardState(i: ArchitectHudInput, index: number): { state: CardState; note: string } {
  const cost = strokeCost(i.rules, index);
  const def = STROKES[index];
  if (!def) return { state: "off", note: "" };
  const v = i.view;
  if (def.levelWide && i.rules.absurdity <= 0) return { state: "off", note: "Events are off" };
  if (index === 7 && i.verticalityUsed) return { state: "used", note: "Used this round" };
  if (def.levelWide && (i.mood.eventId !== "" || v.strokes.some((s) => s.def.levelWide && s.phase === "telegraph"))) return { state: "busy", note: "Event running" };
  if (v.budget < cost) return { state: "poor", note: `${cost - v.budget} short` };
  if (v.strokes.length >= i.rules.maxStrokes) return { state: "full", note: "Too many at once" };
  if (i.nowTick < i.cooldownUntil) return { state: "cooldown", note: `${((i.cooldownUntil - i.nowTick) / 60).toFixed(1)} s` };
  return { state: index === i.armed ? "armed" : "ready", note: "" };
}

/** Builds the HUD model (`role` from whether the local player is the round's architect). */
export function architectHudModel(i: ArchitectHudInput): ArchitectHudModel {
  const v = i.view;
  const amArchitect = v.architect === i.own && i.own >= 0;
  const budgetMax = Math.max(1, i.rules.budgetMax);
  const armedDef = amArchitect && i.armed >= 0 ? STROKES[i.armed] : undefined;
  const armedCost = armedDef ? strokeCost(i.rules, armedDef.index) : 0;
  const cards: StrokeCard[] = amArchitect
    ? STROKES.map((s) => {
        const c = cardState(i, s.index);
        return {
          index: s.index,
          key: s.index + 1,
          name: s.name,
          verb: s.verb,
          blurb: s.blurb,
          cost: strokeCost(i.rules, s.index),
          css: strokeCss(s),
          slider: s.slider,
          sign: s.sign > 0 ? "+" : s.sign < 0 ? "−" : "",
          state: c.state === "ready" && s.index === i.armed ? "armed" : c.state,
          note: c.note,
        };
      })
    : [];
  let reticle: ArchitectHudModel["reticle"] = null;
  if (armedDef && !v.roundOver) {
    const status = i.waiting ? "Drawing…" : i.problem ?? (i.pointerLocked ? "Click to place" : "Click the view to aim");
    reticle = {
      name: armedDef.name,
      css: strokeCss(armedDef),
      size: `${Math.round(i.radius)} u${i.aimDistance !== null ? ` · ${Math.round(i.aimDistance)} u away` : ""}`,
      status,
      ok: i.problem === null,
    };
  }
  const runners = v.runners;
  const escaped = runners.filter((r) => r.finished).length;
  const out = runners.filter((r) => r.out && !r.finished).length;
  const runnersLine = `${runners.length} ${runners.length === 1 ? "runner" : "runners"}${escaped ? ` · ${escaped} escaped` : ""}${out ? ` · ${out} out of lives` : ""}`;
  const hint = amArchitect
    ? i.radial.open
      ? "Move the mouse to a stroke · let go of R to pick it"
      : i.overview
        ? "WASD pan · Space/C move the cut · mouse turn · 1-8 or hold R: stroke · wheel: size · click: place · right click: put down · V: fly"
        : "1-8 or hold R: stroke · wheel: size · click: place · right click: put down · V: map view · WASD Space/C fly, Shift fast"
    : "";
  return {
    role: amArchitect ? "architect" : "runner",
    roundText: `Round ${v.round + 1} of ${v.rounds}`,
    architectName: v.architect >= 0 ? (amArchitect ? "You" : nameOf(i.names, v.architect)) : "",
    budget: v.budget,
    budgetMax,
    budgetPerSec: i.rules.budgetPerSec,
    budgetFill: Math.max(0, Math.min(100, (100 * v.budget) / budgetMax)),
    afterFill: armedDef ? Math.max(0, Math.min(100, (100 * (v.budget - armedCost)) / budgetMax)) : -1,
    affordable: !armedDef || v.budget >= armedCost,
    cards,
    reticle,
    radial: amArchitect ? i.radial : { open: false, selected: -1 },
    overview: amArchitect && i.overview,
    hint,
    banner: i.banner,
    toast: i.toast,
    showPanel: amArchitect && !i.pointerLocked && !v.roundOver,
    sliders: i.sliders,
    runnersLine,
    inFlight: v.strokes.length > 0 ? `${v.strokes.length} of ${i.rules.maxStrokes} strokes in flight` : "",
    telegraphs: v.strokes.filter((s) => s.phase === "telegraph").length,
    acting: v.strokes.filter((s) => s.phase === "active").length,
  };
}

/** The radial wheel's sector (0-7, clockwise from the top) for a mouse offset, -1 near the centre. */
export function radialSector(dx: number, dy: number, deadZone = 24): number {
  if (Math.hypot(dx, dy) < deadZone) return -1;
  // Screen y grows downward; 0 = up, clockwise.
  const a = Math.atan2(dx, -dy);
  const turn = (a / (Math.PI * 2) + 1 + 1 / 16) % 1;
  return Math.floor(turn * 8) % 8;
}

// ---------------------------------------------------------------------------
// DOM

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, parent?: HTMLElement): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  parent?.appendChild(e);
  return e;
}

const PENCIL = `<svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 20l1.2-4.6L16.4 4.2a2.3 2.3 0 0 1 3.4 3.4L8.6 18.8Z"/><path d="M14.6 6l3.4 3.4"/><path d="M4 20l4.6-1.2"/></svg>`;

export interface ArchitectHudHandlers {
  /** A stroke card was clicked (pointer free). */
  arm(index: number): void;
}

/** The Architect's HUD DOM (both roles). */
export class ArchitectHud {
  readonly root: HTMLElement;
  private readonly plate: HTMLElement;
  private readonly dock: HTMLElement;
  private readonly budget: HTMLElement;
  private readonly cards: HTMLElement;
  private readonly reticle: HTMLElement;
  private readonly radial: HTMLElement;
  private readonly banner: HTMLElement;
  private readonly toast: HTMLElement;
  private readonly hint: HTMLElement;
  private readonly panelBox: HTMLElement;
  private panel: SliderPanel | null = null;
  private resting: SliderValues | null = null;
  private readonly handlers: ArchitectHudHandlers;
  private last = "";
  private lastRadial = "";

  constructor(parent: HTMLElement, handlers: ArchitectHudHandlers) {
    this.handlers = handlers;
    this.root = el("div", "arch-hud hidden", parent);
    this.plate = el("div", "arch-plate", this.root);
    this.reticle = el("div", "arch-reticle hidden", this.root);
    this.radial = el("div", "arch-radial hidden", this.root);
    this.banner = el("div", "arch-banner hidden", this.root);
    this.toast = el("div", "arch-toast hidden", this.root);
    this.panelBox = el("div", "arch-panel hidden", this.root);
    this.dock = el("div", "arch-dock hidden", this.root);
    this.budget = el("div", "arch-budget", this.dock);
    this.cards = el("div", "arch-cards", this.dock);
    this.hint = el("div", "arch-hint", this.dock);
    this.buildRadial();
  }

  private buildRadial(): void {
    const ring = el("div", "arch-radial-ring", this.radial);
    STROKES.forEach((s, n) => {
      const a = (n / 8) * Math.PI * 2;
      const item = el("div", "arch-radial-item", ring);
      item.style.setProperty("--x", `${(Math.sin(a) * 118).toFixed(1)}px`);
      item.style.setProperty("--y", `${(-Math.cos(a) * 118).toFixed(1)}px`);
      item.style.setProperty("--stroke", strokeCss(s));
      item.dataset["index"] = String(s.index);
      item.innerHTML = `<span class="arch-radial-icon">${SLIDER_ICONS[s.slider]}</span><span class="arch-radial-name"></span><span class="arch-radial-cost"></span>`;
      (item.querySelector(".arch-radial-name") as HTMLElement).textContent = s.name;
    });
    el("div", "arch-radial-hub", ring).innerHTML = PENCIL;
  }

  update(m: ArchitectHudModel): void {
    // The radial wheel follows the mouse every frame; the rest only on change.
    const rkey = JSON.stringify(m.radial);
    if (rkey !== this.lastRadial) {
      this.lastRadial = rkey;
      this.radial.classList.toggle("hidden", !m.radial.open);
      for (const item of Array.from(this.radial.querySelectorAll<HTMLElement>(".arch-radial-item"))) {
        item.classList.toggle("selected", Number(item.dataset["index"]) === m.radial.selected);
      }
    }
    const key = JSON.stringify({ ...m, radial: null });
    if (key === this.last) return;
    this.last = key;
    this.root.dataset["role"] = m.role;
    // Hooks for tools/browser-smoke (phase "architect").
    this.root.dataset["budget"] = String(m.budget);
    this.root.dataset["telegraphs"] = String(m.telegraphs);
    this.root.dataset["acting"] = String(m.acting);
    this.root.classList.toggle("overview", m.overview);

    // Plate: who builds, the round, the budget at a glance.
    this.plate.replaceChildren();
    const badge = el("div", "arch-badge", this.plate);
    el("span", "arch-badge-icon", badge).innerHTML = PENCIL;
    const who = el("div", "arch-badge-text", badge);
    el("div", "arch-badge-kicker", who).textContent = m.roundText;
    el("div", "arch-badge-title", who).textContent = m.role === "architect" ? "You are the architect" : m.architectName ? `Architect: ${m.architectName}` : "No architect this round";
    if (m.role === "runner" && m.architectName) {
      const mini = el("div", "arch-mini", this.plate);
      const bar = el("div", "arch-mini-bar", mini);
      el("div", "arch-mini-fill", bar).style.width = `${m.budgetFill.toFixed(1)}%`;
      el("span", "arch-mini-value", mini).textContent = `${m.budget} to spend`;
    }
    const lines = el("div", "arch-plate-lines", this.plate);
    el("div", "", lines).textContent = m.runnersLine;
    if (m.inFlight) el("div", "arch-inflight", lines).textContent = m.inFlight;

    // Dock (architect only).
    const architect = m.role === "architect";
    this.dock.classList.toggle("hidden", !architect);
    if (architect) {
      this.budget.replaceChildren();
      const head = el("div", "arch-budget-head", this.budget);
      el("span", "arch-budget-label", head).textContent = "Budget";
      el("span", "arch-budget-value", head).textContent = `${m.budget}`;
      el("span", "arch-budget-max", head).textContent = ` / ${m.budgetMax} · +${m.budgetPerSec}/s`;
      const bar = el("div", `arch-budget-bar${m.affordable ? "" : " short"}`, this.budget);
      el("div", "arch-budget-fill", bar).style.width = `${m.budgetFill.toFixed(1)}%`;
      if (m.afterFill >= 0) {
        const spend = el("div", "arch-budget-spend", bar);
        const lo = m.affordable ? m.afterFill : 0;
        spend.style.left = `${lo.toFixed(1)}%`;
        spend.style.width = `${Math.max(0, (m.affordable ? m.budgetFill : m.budgetFill) - lo).toFixed(1)}%`;
      }
      for (let t = 50; t < m.budgetMax; t += 50) el("span", "arch-budget-tick", bar).style.left = `${((100 * t) / m.budgetMax).toFixed(1)}%`;
      this.cards.replaceChildren(
        ...m.cards.map((c) => {
          const b = el("button", `arch-card ${c.state}`);
          b.type = "button";
          b.style.setProperty("--stroke", c.css);
          b.title = `${c.name}: ${c.verb}. ${c.blurb}.`;
          el("span", "arch-card-key", b).textContent = String(c.key);
          const icon = el("span", "arch-card-icon", b);
          icon.innerHTML = SLIDER_ICONS[c.slider];
          if (c.sign) el("span", "arch-card-sign", icon).textContent = c.sign;
          el("span", "arch-card-name", b).textContent = c.name;
          el("span", "arch-card-verb", b).textContent = c.note || c.verb;
          el("span", "arch-card-cost", b).textContent = String(c.cost);
          b.addEventListener("click", (e) => {
            e.stopPropagation();
            this.handlers.arm(c.index);
          });
          return b;
        }),
      );
      for (const item of Array.from(this.radial.querySelectorAll<HTMLElement>(".arch-radial-item"))) {
        const c = m.cards[Number(item.dataset["index"])];
        if (!c) continue;
        item.classList.toggle("dim", c.state !== "ready" && c.state !== "armed");
        const cost = item.querySelector(".arch-radial-cost");
        if (cost) cost.textContent = c.state === "ready" || c.state === "armed" ? String(c.cost) : c.note || String(c.cost);
      }
    }
    this.hint.textContent = m.hint;

    // Reticle readout.
    this.reticle.classList.toggle("hidden", m.reticle === null || m.radial.open);
    if (m.reticle) {
      this.reticle.style.setProperty("--stroke", m.reticle.css);
      this.reticle.classList.toggle("bad", !m.reticle.ok);
      this.reticle.replaceChildren();
      el("div", "arch-reticle-name", this.reticle).textContent = m.reticle.name;
      el("div", "arch-reticle-size", this.reticle).textContent = m.reticle.size;
      el("div", "arch-reticle-status", this.reticle).textContent = m.reticle.status;
    }

    // Banner and toast.
    this.banner.classList.toggle("hidden", m.banner === null);
    if (m.banner) {
      this.banner.dataset["tone"] = m.banner.tone;
      this.banner.replaceChildren();
      el("div", "arch-banner-kicker", this.banner).textContent = m.banner.kicker;
      el("div", "arch-banner-title", this.banner).textContent = m.banner.title;
      if (m.banner.sub) el("div", "arch-banner-sub", this.banner).textContent = m.banner.sub;
    }
    this.toast.classList.toggle("hidden", m.toast === null);
    if (m.toast) {
      this.toast.dataset["tone"] = m.toast.tone;
      this.toast.textContent = m.toast.text;
    }

    // The world's sliders while the pointer is free: drag one to pick its stroke.
    this.panelBox.classList.toggle("hidden", !m.showPanel || m.sliders === null);
    if (m.showPanel && m.sliders) this.ensurePanel(m.sliders);
  }

  private ensurePanel(values: SliderValues): void {
    this.resting = values;
    if (this.panel) {
      return;
    }
    el("div", "arch-panel-title", this.panelBox).textContent = "This world's sliders";
    el("div", "arch-panel-sub", this.panelBox).textContent = "Nudge a slider to pick its stroke, then click the view to aim it";
    const panel = new SliderPanel({
      values,
      onInput: (key, value) => {
        const base = this.resting?.[key] ?? value;
        const s = strokeForSlider(key, value - base);
        if (s && value !== base) this.handlers.arm(s.index);
      },
    });
    // The slider springs back: the world's numbers do not change, the stroke does it locally.
    panel.root.addEventListener("change", () => {
      if (this.resting) panel.set(this.resting);
    });
    this.panelBox.appendChild(panel.root);
    this.panel = panel;
  }

  show(visible: boolean): void {
    this.root.classList.toggle("hidden", !visible);
  }

  dispose(): void {
    this.root.remove();
  }
}
