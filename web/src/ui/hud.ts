// HTML/CSS heads-up display. Call `update()` with any subset of values each
// frame; DOM writes only happen for values that actually changed.
//
// The top-centre scoreline (mode, timer, scores) is the mode HUD
// (ui/modeHud.ts); this is the ship: hull, energy and boost meters, the
// weapon panel, the centre status line and the kill feed.
//
// The weapon panel lists the carried primaries with their number keys
// (1-9, 0), the secondaries (X cycles) and the gadget (G), per the M2 key
// scheme; the selected ones are highlighted.

import { FitRing } from "./fitRing.ts";
import { emptyLoadout, type LoadoutState } from "../game/views.ts";
import { chargeLabel, gadgetDetail, paintColorLabel, PAINT_COLORS, type ChargeMeter, type GadgetState } from "../game/mechanics.ts";

export interface HudValues {
  hull: number;
  hullMax: number;
  energy: number;
  energyMax: number;
  boost: number;
  boostMax: number;
  scrap: number;
  /** Selected weapon name and its ammo text (e.g. "Rivet Gun" and "42", or "∞"). */
  weapon: string;
  ammo: string;
  /** Secondary weapon and count (e.g. "Bolt Rocket ×3"); empty hides it. */
  secondary: string;
  /** Team share of painted surface, 0..1, indexed by 0-based team id (Paint the Void; the mode HUD shows scores). */
  paint: readonly number[];
  /** Remaining match time in seconds (shown by the mode HUD). */
  timeLeft: number;
  horizonAssist: boolean;
  scavenging: boolean;
  /** Centre status line ("Waiting for the server", "Destroyed, respawning"); empty hides it. */
  status: string;
  /** Free-form debug line (fps, chunk counts); empty hides it. */
  debug: string;
  /** Carried weapons and gadget (weapon panel). */
  loadout: LoadoutState;
  /** Own gadget with cooldown (the G line); null uses the loadout's on/off. */
  gadget: GadgetState | null;
  /** Charge in progress (Fusion Charger, Polite Laser) and its weapon name; null hides the meter. */
  charge: { name: string; meter: ChargeMeter } | null;
  /** Paint Thrower colour id (1 red, 2 blue, 3 yellow) while it is the selected primary (from ls-wasm); 0 hides the V line. */
  paintColor: number;
  /** Seconds the weapons stay offline (Borrowed Time); 0 hides it. */
  weaponsOffline: number;
}

export const DEFAULT_HUD: Readonly<HudValues> = {
  hull: 100,
  hullMax: 100,
  energy: 100,
  energyMax: 100,
  boost: 100,
  boostMax: 100,
  scrap: 0,
  weapon: "Scrap Laser",
  ammo: "∞",
  secondary: "",
  paint: [0, 0],
  timeLeft: 300,
  horizonAssist: true,
  scavenging: false,
  status: "",
  debug: "",
  loadout: emptyLoadout(),
  gadget: null,
  charge: null,
  paintColor: 0,
  weaponsOffline: 0,
};

/** One line of the weapon panel. */
export interface LoadoutLine {
  /** Key hint: "1".."9", "0", "X", "G" or "" (primaries beyond ten). */
  key: string;
  name: string;
  /** Ammo text, level ("L2/6") for upgradable weapons, "on"/"off" for the gadget. */
  detail: string;
  selected: boolean;
  kind: "primary" | "secondary" | "gadget";
}

/** Weapon panel lines: primaries (key order), secondaries, then the gadget (with its cooldown when `gadget` is given). */
export function loadoutLines(l: LoadoutState, gadget: GadgetState | null = null): LoadoutLine[] {
  const detail = (ammo: string, level: number, maxLevel: number): string =>
    maxLevel > 1 ? `L${level}/${maxLevel} · ${ammo}` : ammo;
  const lines: LoadoutLine[] = l.primaries.map((w) => ({
    key: w.key > 0 ? String(w.key % 10) : "",
    name: w.name,
    detail: detail(w.ammo, w.level, w.maxLevel),
    selected: w.selected,
    kind: "primary",
  }));
  for (const w of l.secondaries) {
    lines.push({ key: "X", name: w.name, detail: detail(w.ammo, w.level, w.maxLevel), selected: w.selected, kind: "secondary" });
  }
  if (gadget && gadget.name) {
    lines.push({ key: "G", name: gadget.name, detail: gadgetDetail(gadget), selected: gadget.active, kind: "gadget" });
  } else if (l.gadget) {
    lines.push({ key: "G", name: l.gadget, detail: l.gadgetActive ? "on" : "off", selected: l.gadgetActive, kind: "gadget" });
  }
  return lines;
}

/** The charge meter: label, fill percentage and state class. */
export function chargeModel(c: HudValues["charge"]): { label: string; percent: number; state: string } | null {
  if (!c) return null;
  const m = c.meter;
  return {
    label: chargeLabel(c.name, m),
    percent: Math.round(m.fraction * 1000) / 10,
    state: m.overcharge ? "overcharge" : m.fraction >= 1 ? "full" : m.ready ? "ready" : "charging",
  };
}

/** How long a kill-feed line stays visible (ms). */
export const FEED_MS = 6000;
/** Kill-feed lines shown at once. */
export const FEED_LINES = 5;

export interface FeedEntry {
  text: string;
  expires: number;
}

/** Adds lines to a feed list, dropping expired and overflowing entries (oldest first). */
export function pushFeed(feed: readonly FeedEntry[], lines: readonly string[], now: number): FeedEntry[] {
  const next = feed.filter((e) => e.expires > now);
  for (const text of lines) next.push({ text, expires: now + FEED_MS });
  return next.slice(-FEED_LINES);
}

/** Formats seconds as m:ss (clamped at 0, rounded up so 0:00 means time is over). */
export function formatTimer(seconds: number): string {
  const s = Math.max(0, Math.ceil(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Percentage 0..100 for a meter, clamped; 0 when max is not positive. */
export function meterPercent(value: number, max: number): number {
  if (!(max > 0)) return 0;
  return Math.max(0, Math.min(100, (value / max) * 100));
}

interface Meter {
  bar: HTMLElement;
  fill: HTMLElement;
  label: HTMLElement;
}

type MeterName = "hull" | "energy" | "boost";

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  parent?: HTMLElement,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  parent?.appendChild(e);
  return e;
}

export class Hud {
  readonly root: HTMLElement;
  /** The fit ring around the crosshair (VQ1, ui/fitRing.ts); main.ts feeds it. */
  readonly fitRing: FitRing;
  private values: HudValues = { ...DEFAULT_HUD, paint: [...DEFAULT_HUD.paint] };
  /** Last rendered representation per DOM slot, to skip redundant writes. */
  private readonly rendered = new Map<string, string>();
  private readonly meters: Record<MeterName, Meter>;
  private readonly scrap: HTMLElement;
  private readonly weapon: HTMLElement;
  private readonly ammo: HTMLElement;
  private readonly secondary: HTMLElement;
  private readonly loadoutEl: HTMLElement;
  private readonly status: HTMLElement;
  private readonly feedEl: HTMLElement;
  private feed: FeedEntry[] = [];
  private readonly assist: HTMLElement;
  private readonly scavenge: HTMLElement;
  private readonly debug: HTMLElement;
  private readonly prompt: HTMLElement;
  private readonly chargeEl: HTMLElement;
  private readonly chargeFill: HTMLElement;
  private readonly chargeLabelEl: HTMLElement;
  private readonly paintEl: HTMLElement;
  private readonly offlineEl: HTMLElement;

  constructor(parent: HTMLElement) {
    this.values.loadout = emptyLoadout();
    this.root = el("div", "hud hidden", parent);
    el("div", "hud-crosshair", this.root);
    this.fitRing = new FitRing(this.root);

    const left = el("div", "hud-panel hud-left", this.root);
    const meter = (name: string, cls: MeterName): Meter => {
      const row = el("div", `hud-meter ${cls}`, left);
      el("span", "hud-meter-name", row).textContent = name;
      const bar = el("div", "hud-meter-bar", row);
      const fill = el("div", "hud-meter-fill", bar);
      const label = el("span", "hud-meter-value", row);
      return { bar, fill, label };
    };
    this.meters = {
      hull: meter("Hull", "hull"),
      energy: meter("Energy", "energy"),
      boost: meter("Boost", "boost"),
    };

    const right = el("div", "hud-panel hud-right", this.root);
    this.weapon = el("div", "hud-weapon", right);
    this.ammo = el("div", "hud-ammo", right);
    this.secondary = el("div", "hud-secondary", right);
    this.loadoutEl = el("div", "hud-loadout", right);
    this.paintEl = el("div", "hud-paint-color hidden", right);
    const scrapRow = el("div", "hud-scrap", right);
    el("span", "hud-meter-name", scrapRow).textContent = "Scrap";
    this.scrap = el("span", "hud-scrap-value", scrapRow);

    const flags = el("div", "hud-flags", this.root);
    this.assist = el("span", "hud-flag", flags);
    this.scavenge = el("span", "hud-flag scavenge", flags);
    this.scavenge.textContent = "Scavenging";

    this.chargeEl = el("div", "hud-charge hidden", this.root);
    const chargeBar = el("div", "hud-charge-bar", this.chargeEl);
    this.chargeFill = el("div", "hud-charge-fill", chargeBar);
    this.chargeLabelEl = el("div", "hud-charge-label", this.chargeEl);
    this.offlineEl = el("div", "hud-offline hidden", this.root);
    this.status = el("div", "hud-status", this.root);
    this.feedEl = el("div", "hud-feed", this.root);
    this.debug = el("div", "hud-debug", this.root);
    this.prompt = el("div", "hud-prompt", this.root);
    this.prompt.textContent = "Click to fly · Esc releases the mouse";

    this.renderAll();
  }

  show(visible: boolean): void {
    this.root.classList.toggle("hidden", !visible);
  }

  /** Instagib: hull and energy are hidden (any hit kills; energy is not spent). */
  setInstagib(on: boolean): void {
    this.root.classList.toggle("instagib", on);
  }

  /** Spectating: the ship panels and crosshair are hidden. */
  setSpectating(on: boolean): void {
    this.root.classList.toggle("spectating", on);
  }

  /** Shows the "click to fly" prompt (while the pointer is not locked). */
  showPrompt(visible: boolean): void {
    this.prompt.classList.toggle("hidden", !visible);
  }

  /** Adds kill-feed lines and expires old ones; call every frame with the current time (ms). */
  updateFeed(lines: readonly string[], now: number): void {
    const before = this.feed;
    this.feed = pushFeed(this.feed, lines, now);
    const same = before.length === this.feed.length && before.every((e, i) => e === this.feed[i]);
    if (same) return;
    this.feedEl.replaceChildren(
      ...this.feed.map((e) => {
        const line = document.createElement("div");
        line.textContent = e.text;
        return line;
      }),
    );
  }

  get current(): Readonly<HudValues> {
    return this.values;
  }

  update(changes: Partial<HudValues>): void {
    this.values = { ...this.values, ...changes };
    this.renderAll();
  }

  private write(key: string, repr: string, apply: () => void): void {
    if (this.rendered.get(key) === repr) return;
    this.rendered.set(key, repr);
    apply();
  }

  private renderAll(): void {
    const v = this.values;
    const meters: [MeterName, number, number][] = [
      ["hull", v.hull, v.hullMax],
      ["energy", v.energy, v.energyMax],
      ["boost", v.boost, v.boostMax],
    ];
    for (const [name, value, max] of meters) {
      const m = this.meters[name];
      const pct = meterPercent(value, max).toFixed(1);
      this.write(`${name}.fill`, pct, () => {
        m.fill.style.width = `${pct}%`;
      });
      const text = String(Math.round(Math.max(0, value)));
      this.write(`${name}.label`, text, () => {
        m.label.textContent = text;
      });
    }
    const hullLow = meterPercent(v.hull, v.hullMax) < 25;
    this.write("hull.low", String(hullLow), () => {
      this.meters.hull.bar.classList.toggle("low", hullLow);
    });

    const scrap = String(Math.floor(v.scrap));
    this.write("scrap", scrap, () => {
      this.scrap.textContent = scrap;
    });
    this.write("weapon", v.weapon, () => {
      this.weapon.textContent = v.weapon;
    });
    this.write("ammo", v.ammo, () => {
      this.ammo.textContent = v.ammo;
    });
    this.write("secondary", v.secondary, () => {
      this.secondary.textContent = v.secondary;
      this.secondary.classList.toggle("hidden", v.secondary === "");
    });
    const lines = loadoutLines(v.loadout, v.gadget);
    this.write("loadout", JSON.stringify(lines), () => {
      this.loadoutEl.replaceChildren(
        ...lines.map((line) => {
          const row = el("div", `hud-slot ${line.kind}${line.selected ? " selected" : ""}`);
          el("span", "hud-slot-key", row).textContent = line.key;
          el("span", "hud-slot-name", row).textContent = line.name;
          el("span", "hud-slot-detail", row).textContent = line.detail;
          return row;
        }),
      );
      this.loadoutEl.classList.toggle("hidden", lines.length === 0);
    });
    const charge = chargeModel(v.charge);
    this.write("charge", JSON.stringify(charge), () => {
      this.chargeEl.classList.toggle("hidden", charge === null);
      if (!charge) return;
      this.chargeFill.style.width = `${charge.percent}%`;
      this.chargeLabelEl.textContent = charge.label;
      this.chargeEl.dataset["state"] = charge.state;
    });
    const paint = v.paintColor > 0 && v.paintColor <= 3 ? v.paintColor : 0;
    this.write("paintColor", String(paint), () => {
      this.paintEl.classList.toggle("hidden", paint === 0);
      const c = PAINT_COLORS[paint] ?? [1, 1, 1];
      this.paintEl.textContent = `V paint: ${paintColorLabel(paint)}`;
      this.paintEl.style.setProperty("--paint", `rgb(${c.map((x) => Math.round(x * 255)).join(",")})`);
    });
    const offline = v.weaponsOffline > 0 ? `Weapons offline ${v.weaponsOffline.toFixed(1)} s` : "";
    this.write("offline", offline, () => {
      this.offlineEl.textContent = offline;
      this.offlineEl.classList.toggle("hidden", offline === "");
    });
    this.write("status", v.status, () => {
      this.status.textContent = v.status;
      this.status.classList.toggle("hidden", v.status === "");
    });

    const assist = `Horizon assist: ${v.horizonAssist ? "on" : "off"} (H)`;
    this.write("assist", assist, () => {
      this.assist.textContent = assist;
    });
    this.write("scavenging", String(v.scavenging), () => {
      this.scavenge.classList.toggle("hidden", !v.scavenging);
    });
    this.write("debug", v.debug, () => {
      this.debug.textContent = v.debug;
      this.debug.classList.toggle("hidden", v.debug === "");
    });
  }
}
