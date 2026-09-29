// Full-screen effect layer over the game canvas: the Flash Pod whiteout,
// warning banners ("excuse me", Tectonic crack warning, overcharge), the
// Eyeball view frame with its hint, and the own cloak's screen-edge
// shimmer. The timing lives in game/mechanics.ts; this only writes the DOM
// when something changed.

import type { Banner } from "../game/mechanics.ts";

export interface EffectsView {
  /** 0..1 whiteout. */
  whiteout: number;
  banners: readonly Banner[];
  /** Riding the Eyeball camera. */
  eyeball: boolean;
  /** The own ship is cloaked. */
  cloaked: boolean;
}

/** Banner text with its countdown ("Excuse me… 1.2 s"). */
export function bannerText(b: Banner): string {
  return b.countdown === null ? b.text : `${b.text} · ${b.countdown.toFixed(1)} s`;
}

/** The hint shown while guiding an Eyeball. */
export const EYEBALL_HINT = "Eyeball view · mouse steers the missile · your ship drifts without thrust · the view returns when it hits";

export class EffectsOverlay {
  readonly root: HTMLElement;
  private readonly white: HTMLElement;
  private readonly banners: HTMLElement;
  private readonly eyeball: HTMLElement;
  private readonly cloak: HTMLElement;
  private rendered = new Map<string, string>();

  constructor(parent: HTMLElement) {
    this.root = document.createElement("div");
    this.root.className = "effects";
    const div = (cls: string): HTMLElement => {
      const e = document.createElement("div");
      e.className = cls;
      this.root.appendChild(e);
      return e;
    };
    this.cloak = div("fx-cloak hidden");
    this.eyeball = div("fx-eyeball hidden");
    const hint = document.createElement("div");
    hint.className = "fx-eyeball-hint";
    hint.textContent = EYEBALL_HINT;
    this.eyeball.appendChild(hint);
    this.banners = div("fx-banners");
    this.white = div("fx-whiteout");
    this.white.style.opacity = "0";
    parent.appendChild(this.root);
  }

  private write(key: string, repr: string, apply: () => void): void {
    if (this.rendered.get(key) === repr) return;
    this.rendered.set(key, repr);
    apply();
  }

  update(v: EffectsView): void {
    const white = v.whiteout.toFixed(2);
    this.write("white", white, () => {
      this.white.style.opacity = white;
    });
    this.write("eyeball", String(v.eyeball), () => this.eyeball.classList.toggle("hidden", !v.eyeball));
    this.write("cloak", String(v.cloaked), () => this.cloak.classList.toggle("hidden", !v.cloaked));
    // Rebuild the banner elements only when the set of banners changes; a
    // running countdown just rewrites its text node.
    let shape = "";
    for (const b of v.banners) shape += `${b.tone}|${b.text}
`;
    this.write("banners", shape, () => {
      this.banners.replaceChildren(
        ...v.banners.map((b) => {
          const e = document.createElement("div");
          e.className = `fx-banner ${b.tone}`;
          return e;
        }),
      );
    });
    v.banners.forEach((b, i) => {
      const e = this.banners.children[i];
      const text = bannerText(b);
      if (e && e.textContent !== text) e.textContent = text;
    });
  }

  dispose(): void {
    this.root.remove();
  }
}
