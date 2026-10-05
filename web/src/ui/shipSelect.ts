// Ship bay: ship type choice for the next respawn with each type's numbers
// and loadout preview (the rule set's loadout, secondaries limited to the
// ship's slots). Opens by itself while the ship is destroyed and with B;
// keys 1-4 pick a type (only the types the rule set allows) and B closes it
// again. Model in game/mechanics.ts. Each card has a model box; a
// `CardBackdrop` (attract/shipBay.ts PanelBay, first-impression pass) turns
// the hulls behind them.

import { SHIP_ROLES, shipCardLines, type ShipChoice } from "../game/mechanics.ts";

export interface ShipPanelModel {
  visible: boolean;
  title: string;
  hint: string;
  cards: {
    key: string;
    name: string;
    role: string;
    lines: string[];
    allowed: boolean;
    current: boolean;
    next: boolean;
  }[];
  /** Last selection error ("the rule set does not allow ..."), or "". */
  error: string;
}

/** Builds the panel model. `dead`: the ship is destroyed (respawn pending). */
export function shipPanelModel(choices: readonly ShipChoice[], open: boolean, dead: boolean, error = ""): ShipPanelModel {
  const visible = (open || dead) && choices.length > 0;
  const next = choices.find((c) => c.next);
  const current = choices.find((c) => c.current);
  const flying = next ?? current;
  return {
    visible,
    title: dead ? "Destroyed · choose your next ship" : "Ship bay · next respawn",
    hint: `${flying ? `Respawning as ${flying.name}. ` : ""}Press 1-4 to choose${dead ? "" : ", B to close"}.`,
    cards: choices.map((c, i) => ({
      key: String(i + 1),
      name: c.name,
      role: SHIP_ROLES[c.shipType] ?? "",
      lines: shipCardLines(c),
      allowed: c.allowed,
      current: c.current,
      next: c.next,
    })),
    error,
  };
}

/** A card's model box for the hull view behind the panel. */
export interface CardSlot {
  el: HTMLElement;
  shipType: number;
  allowed: boolean;
  /** The ship flown next (or now, when none is picked). */
  selected: boolean;
}

/** Something drawn behind the cards (the FI hull view); `null` = the panel is hidden. */
export interface CardBackdrop {
  update(cards: readonly CardSlot[] | null): void;
  dispose(): void;
}

export class ShipPanel {
  readonly root: HTMLElement;
  private rendered = "";
  /** Called with a ship type index when a card is clicked. */
  onPick: ((shipType: number) => void) | null = null;
  /** Hull view behind the cards (optional). */
  backdrop: CardBackdrop | null = null;

  constructor(parent: HTMLElement) {
    this.root = document.createElement("div");
    this.root.className = "ship-panel hidden";
    parent.appendChild(this.root);
  }

  update(m: ShipPanelModel): void {
    const repr = JSON.stringify(m);
    if (repr === this.rendered) return;
    this.rendered = repr;
    this.root.classList.toggle("hidden", !m.visible);
    if (!m.visible) {
      this.backdrop?.update(null);
      return;
    }
    const title = document.createElement("h2");
    title.textContent = m.title;
    const cards = document.createElement("div");
    cards.className = "ship-cards";
    const slots: CardSlot[] = [];
    const anyNext = m.cards.some((c) => c.next);
    m.cards.forEach((c, i) => {
      const card = document.createElement("button");
      card.type = "button";
      card.className = `ship-card${c.allowed ? "" : " banned"}${c.current ? " current" : ""}${c.next ? " next" : ""}`;
      card.disabled = !c.allowed;
      const model = document.createElement("span");
      model.className = "ship-card-model";
      card.appendChild(model);
      slots.push({ el: model, shipType: i, allowed: c.allowed, selected: anyNext ? c.next : c.current });
      const head = document.createElement("div");
      head.className = "ship-card-head";
      head.textContent = `${c.key} · ${c.name}${c.current ? " (flying)" : ""}${c.next ? " (next)" : ""}${c.allowed ? "" : " (not allowed)"}`;
      const role = document.createElement("div");
      role.className = "ship-card-role";
      role.textContent = c.role;
      card.append(head, role);
      for (const line of c.lines) {
        const l = document.createElement("div");
        l.className = "ship-card-line";
        l.textContent = line;
        card.appendChild(l);
      }
      card.addEventListener("click", () => this.onPick?.(i));
      cards.appendChild(card);
    });
    const hint = document.createElement("p");
    hint.className = "ship-panel-hint";
    hint.textContent = m.hint;
    const error = document.createElement("p");
    error.className = "ship-panel-error";
    error.textContent = m.error;
    // A hull view canvas (CardBackdrop) stays.
    this.root.replaceChildren(...this.root.querySelectorAll(":scope > canvas"), title, cards, hint, error);
    this.backdrop?.update(slots);
  }

  dispose(): void {
    this.backdrop?.dispose();
    this.backdrop = null;
    this.root.remove();
  }
}
