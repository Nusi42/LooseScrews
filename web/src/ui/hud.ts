// HTML/CSS heads-up display. Call `update()` with any subset of values each
// frame; DOM writes only happen for values that actually changed.

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
  /** Team share of painted surface, 0..1, indexed by 0-based team id. */
  paint: readonly number[];
  /** Remaining match time in seconds. */
  timeLeft: number;
  horizonAssist: boolean;
  scavenging: boolean;
  /** Centre status line ("Waiting for the server", "Destroyed, respawning"); empty hides it. */
  status: string;
  /** Free-form debug line (fps, chunk counts); empty hides it. */
  debug: string;
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
};

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
  private values: HudValues = { ...DEFAULT_HUD, paint: [...DEFAULT_HUD.paint] };
  /** Last rendered representation per DOM slot, to skip redundant writes. */
  private readonly rendered = new Map<string, string>();
  private readonly meters: Record<MeterName, Meter>;
  private readonly scrap: HTMLElement;
  private readonly weapon: HTMLElement;
  private readonly ammo: HTMLElement;
  private readonly secondary: HTMLElement;
  private readonly status: HTMLElement;
  private readonly feedEl: HTMLElement;
  private feed: FeedEntry[] = [];
  private readonly timer: HTMLElement;
  private readonly paintFills: HTMLElement[] = [];
  private readonly paintLabels: HTMLElement[] = [];
  private readonly assist: HTMLElement;
  private readonly scavenge: HTMLElement;
  private readonly debug: HTMLElement;
  private readonly prompt: HTMLElement;

  constructor(parent: HTMLElement, teamCount = 2) {
    this.root = el("div", "hud hidden", parent);
    el("div", "hud-crosshair", this.root);

    const top = el("div", "hud-top", this.root);
    this.timer = el("div", "hud-timer", top);
    const paint = el("div", "hud-paint", top);
    for (let t = 0; t < teamCount; t++) {
      const row = el("div", `hud-paint-row team-${t}`, paint);
      const bar = el("div", "hud-paint-bar", row);
      this.paintFills[t] = el("div", "hud-paint-fill", bar);
      this.paintLabels[t] = el("span", "hud-paint-label", row);
    }

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
    const scrapRow = el("div", "hud-scrap", right);
    el("span", "hud-meter-name", scrapRow).textContent = "Scrap";
    this.scrap = el("span", "hud-scrap-value", scrapRow);

    const flags = el("div", "hud-flags", this.root);
    this.assist = el("span", "hud-flag", flags);
    this.scavenge = el("span", "hud-flag scavenge", flags);
    this.scavenge.textContent = "Scavenging";

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
    this.write("status", v.status, () => {
      this.status.textContent = v.status;
      this.status.classList.toggle("hidden", v.status === "");
    });
    const timer = formatTimer(v.timeLeft);
    this.write("timer", timer, () => {
      this.timer.textContent = timer;
    });

    for (let t = 0; t < this.paintFills.length; t++) {
      const text = `${Math.max(0, Math.min(100, (v.paint[t] ?? 0) * 100)).toFixed(1)}%`;
      const fill = this.paintFills[t];
      const label = this.paintLabels[t];
      this.write(`paint.${t}`, text, () => {
        if (fill) fill.style.width = text;
        if (label) label.textContent = text;
      });
    }

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
