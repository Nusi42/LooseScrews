// Rule set editor screen: edit every match rule of the design doc (weapon
// availability, starting loadout, pickup weights and respawn times, friendly fire,
// ship types, tuning multipliers, gadgets, scrap piles, pigment pickups, the Absurdity
// cap and comedy event bans), see the problems the server
// would report, and get an `LR` rule set code, an invite link (world + rule set
// codes) and the host's `ls-server` command line.
//
// Opened with `?editor=rules` (main.ts; the start screen links to it). The logic
// lives in code.ts / validate.ts / model.ts (Node-tested); this file is only DOM.

import { CATALOG } from "./catalog.ts";
import { availabilityOf, decodeRules, eventBanned, gadgetAvailabilityOf, MAX_NAME, RuleCodeError, setEventBanned } from "./code.ts";
import {
  defaultRules,
  FRIENDLY_FIRE_STEPS,
  inviteLink,
  loadoutCount,
  rulesSummary,
  serverCommand,
  setGadgetAvailability,
  setGadgetPickup,
  setLoadoutGadget,
  setLoadoutPrimary,
  setLoadoutSecondary,
  setPickup,
  setShipTypeAllowed,
  setTuning,
  setUpgradePickup,
  setWeaponAvailability,
  tuningOf,
} from "./model.ts";
import { AVAILABILITIES, type Availability, type Catalog, type RuleSetData } from "./types.ts";
import { rulesToCode, type RuleProblem } from "./validate.ts";

/** Query parameter that opens the editor instead of the game (`?editor=rules`). */
export const EDITOR_PARAM = "editor";
const STORAGE_KEY = "ls.rules.code";

const AVAILABILITY_LABELS: Record<Availability, string> = {
  LoadoutAndPickup: "Loadout + pickup",
  PickupOnly: "Pickup only",
  LoadoutOnly: "Loadout only",
  Banned: "Banned",
};

export interface RuleEditorOptions {
  /** Base URL for invite links (the page URL). */
  baseUrl: string;
  /** Rule set code to start from (else the last edited one, else the default). */
  rules?: string | null;
  /** World share code for the invite link. */
  world?: string | null;
  catalog?: Catalog;
  /** Called by the Close button (default: back to the start screen). */
  onClose?: () => void;
}

/** Whether the page URL asks for the editor. */
export function wantsRuleEditor(loc: { search: string }): boolean {
  return new URLSearchParams(loc.search).get(EDITOR_PARAM) === "rules";
}

/** The page URL with the editor opened (other parameters kept). */
export function ruleEditorUrl(href: string): string {
  const u = new URL(href);
  u.searchParams.set(EDITOR_PARAM, "rules");
  return u.toString();
}

/** A small "Rule set editor" link for the start screen; remove it when the game starts. */
export function addRuleEditorLink(parent: HTMLElement, href: string): HTMLElement {
  injectStyle();
  const a = document.createElement("a");
  a.className = "rules-editor-link";
  a.href = ruleEditorUrl(href);
  a.textContent = "Rule set editor";
  parent.appendChild(a);
  return a;
}

function loadStored(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function saveStored(code: string): void {
  try {
    localStorage.setItem(STORAGE_KEY, code);
  } catch {
    /* storage unavailable: the code is just not remembered */
  }
}

type Attrs = Record<string, string | number | boolean>;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, attrs: Attrs = {}, ...children: (Node | string)[]): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") e.className = String(v);
    else if (typeof v === "boolean") {
      if (v) e.setAttribute(k, "");
    } else e.setAttribute(k, String(v));
  }
  e.append(...children);
  return e;
}

function numberInput(value: number, attrs: Attrs, onChange: (v: number) => void): HTMLInputElement {
  const input = el("input", { type: "number", value: Number.isFinite(value) ? value : "", ...attrs });
  input.addEventListener("change", () => onChange(input.value.trim() === "" ? Number.NaN : Number(input.value)));
  return input;
}

function select<T extends string | number>(value: T, options: readonly [T, string][], onChange: (v: T) => void): HTMLSelectElement {
  const s = el("select");
  for (const [v, label] of options) {
    const o = el("option", { value: String(v) }, label);
    o.selected = v === value;
    s.appendChild(o);
  }
  s.addEventListener("change", () => {
    const hit = options.find(([v]) => String(v) === s.value);
    if (hit) onChange(hit[0]);
  });
  return s;
}

function checkbox(checked: boolean, onChange: (v: boolean) => void, attrs: Attrs = {}): HTMLInputElement {
  const c = el("input", { type: "checkbox", ...attrs });
  c.checked = checked;
  c.addEventListener("change", () => onChange(c.checked));
  return c;
}

async function copy(text: string, button: HTMLButtonElement): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    button.textContent = "Copied";
  } catch {
    button.textContent = "Copy failed";
  }
  setTimeout(() => (button.textContent = "Copy"), 1500);
}

/** Shows the editor inside `parent`; `remove` takes it down. */
export function showRuleEditor(parent: HTMLElement, opts: RuleEditorOptions): { remove(): void } {
  injectStyle();
  const catalog = opts.catalog ?? CATALOG;
  let rules: RuleSetData = defaultRules(catalog);
  let loadError = "";
  const start = opts.rules ?? loadStored();
  if (start) {
    try {
      rules = decodeRules(start, catalog);
    } catch (err) {
      if (!(err instanceof RuleCodeError)) throw err;
      loadError = `Could not load ${start}: ${err.message}.`;
    }
  }
  let world = opts.world ?? "";

  const root = el("div", { class: "screen rules-editor" });
  const card = el("div", { class: "rules-card" });
  root.appendChild(card);
  parent.appendChild(root);

  const close = (): void => {
    if (opts.onClose) opts.onClose();
    else {
      const u = new URL(opts.baseUrl);
      u.searchParams.delete(EDITOR_PARAM);
      location.assign(u.toString());
    }
  };

  /** Applies an edit and redraws. */
  const edit = (f: (r: RuleSetData) => void): void => {
    f(rules);
    loadError = "";
    render();
  };

  const render = (): void => {
    const result = rulesToCode(rules, catalog);
    const problemFields = new Set(result.problems.map((p) => p.field));
    card.replaceChildren(
      header(),
      loadRow(),
      generalSection(problemFields),
      weaponsSection(problemFields),
      upgradesSection(),
      gadgetsSection(problemFields),
      scrapSection(problemFields),
      pigmentSection(problemFields),
      eventsSection(problemFields),
      outputSection(result.ok ? result.code : null, result.problems),
    );
    if (result.ok) saveStored(result.code);
  };

  const header = (): HTMLElement => {
    const closeBtn = el("button", { type: "button", class: "secondary" }, "Close");
    closeBtn.addEventListener("click", close);
    const reset = el("button", { type: "button", class: "secondary" }, "Reset to default");
    reset.addEventListener("click", () => edit((r) => Object.assign(r, defaultRules(catalog))));
    return el("div", { class: "rules-header" }, el("h1", {}, "Rule set editor"), el("div", { class: "rules-actions" }, reset, closeBtn));
  };

  const loadRow = (): HTMLElement => {
    const input = el("input", { type: "text", placeholder: "Paste a rule set code (LR...)", spellcheck: "false" });
    const btn = el("button", { type: "button" }, "Load");
    const load = (): void => {
      try {
        rules = decodeRules(input.value, catalog);
        loadError = "";
      } catch (err) {
        if (!(err instanceof RuleCodeError)) throw err;
        loadError = `Could not load the code: ${err.message}.`;
      }
      render();
    };
    btn.addEventListener("click", load);
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") load();
    });
    return el("div", { class: "rules-row" }, input, btn, el("p", { class: "field-error", role: "alert" }, loadError));
  };

  const generalSection = (bad: Set<string>): HTMLElement => {
    const name = el("input", { type: "text", value: rules.name, maxlength: MAX_NAME, spellcheck: "false" });
    name.addEventListener("change", () => edit((r) => (r.name = name.value.trim())));
    const ffSteps = FRIENDLY_FIRE_STEPS.includes(rules.friendly_fire_percent)
      ? FRIENDLY_FIRE_STEPS
      : [...FRIENDLY_FIRE_STEPS, rules.friendly_fire_percent].sort((a, b) => a - b);
    const ff = select(
      rules.friendly_fire_percent,
      ffSteps.map((v) => [v, v === 0 ? "Off" : `${v}%`] as [number, string]),
      (v) => edit((r) => (r.friendly_fire_percent = v)),
    );
    const ships = el(
      "div",
      { class: `rules-ships${bad.has("ships") ? " invalid" : ""}` },
      ...catalog.shipTypes.map((t) =>
        el(
          "label",
          { class: "check" },
          checkbox(rules.allowed_ship_types.includes(t), (on) => edit((r) => setShipTypeAllowed(r, catalog, t, on))),
          el("span", {}, t),
        ),
      ),
    );
    const field = (label: string, input: HTMLElement): HTMLElement => el("label", { class: "rules-field" }, el("span", {}, label), input);
    return el(
      "section",
      { class: bad.has("general") ? "invalid" : "" },
      el("h2", {}, "Match"),
      el(
        "div",
        { class: "rules-grid" },
        field("Name", name),
        field("Friendly fire", ff),
        field(
          "Match length (s)",
          numberInput(rules.mode.match_secs, { min: 1, step: 1 }, (v) => edit((r) => (r.mode.match_secs = v))),
        ),
        field(
          "Respawn (s)",
          numberInput(rules.mode.respawn_secs, { min: 0, step: 0.1 }, (v) => edit((r) => (r.mode.respawn_secs = v))),
        ),
        field(
          "Absurdity cap (0-100)",
          numberInput(rules.absurdity, { min: 0, max: 100, step: 1, title: "Comedy events run at the world's Absurdity, at most this; 0 = none" }, (v) =>
            edit((r) => (r.absurdity = v)),
          ),
        ),
      ),
      el("h3", {}, "Allowed ship types"),
      ships,
    );
  };

  const availabilityOptions = AVAILABILITIES.map((a) => [a, AVAILABILITY_LABELS[a]] as [Availability, string]);

  const weaponsSection = (bad: Set<string>): HTMLElement => {
    const head = el(
      "tr",
      {},
      ...["Weapon", "Availability", "Loadout", "Pickup weight", "Respawn (s)", "Ammo", "Max on map", "Damage x", "Fire rate x", "Ammo x"].map(
        (h) => el("th", {}, h),
      ),
    );
    const rows = catalog.weapons.map((w) => {
      const a = availabilityOf(rules, w.id);
      const pickup = rules.pickups[w.id];
      const t = tuningOf(rules, w.id);
      let loadout: HTMLElement;
      if (w.slot === "primary") {
        loadout = checkbox(rules.starting_loadout.primaries.includes(w.id), (on) => edit((r) => setLoadoutPrimary(r, w.id, on)), {
          disabled: a === "Banned" || a === "PickupOnly",
          title: "Start with it",
        });
      } else {
        loadout = numberInput(
          loadoutCount(rules, w.id),
          { min: 0, max: 255, step: 1, class: "narrow", disabled: a === "Banned" || a === "PickupOnly", title: "Starting count" },
          (v) => edit((r) => setLoadoutSecondary(r, w.id, v)),
        );
      }
      const pickupCell = (v: number | undefined, attrs: Attrs, f: (v: number) => void): HTMLElement =>
        el("td", {}, pickup ? numberInput(v ?? 0, { class: "narrow", ...attrs }, f) : el("span", { class: "muted" }, "-"));
      const tune = (v: number, f: (v: number) => void): HTMLElement =>
        el("td", {}, numberInput(v, { min: 0, step: 0.05, class: "narrow", disabled: a === "Banned" }, f));
      return el(
        "tr",
        { class: `${a === "Banned" ? "banned" : ""}${bad.has(`weapon:${w.id}`) ? " invalid" : ""}` },
        el("td", { title: `${w.family} ${w.slot}` }, w.name),
        el("td", {}, select(a, availabilityOptions, (v) => edit((r) => setWeaponAvailability(r, catalog, w.id, v)))),
        el("td", {}, loadout),
        pickupCell(pickup?.weight, { min: 0, step: 0.25 }, (v) => edit((r) => setPickup(r, w.id, { weight: v }))),
        pickupCell(pickup?.respawn_secs, { min: 0, step: 1 }, (v) => edit((r) => setPickup(r, w.id, { respawn_secs: v }))),
        w.count
          ? pickupCell(pickup?.ammo, { min: 0, max: 63, step: 1 }, (v) => edit((r) => setPickup(r, w.id, { ammo: v })))
          : el("td", {}, el("span", { class: "muted" }, "-")),
        pickupCell(pickup?.max_on_map, { min: 0, max: 15, step: 1, title: "0 = no limit" }, (v) =>
          edit((r) => setPickup(r, w.id, { max_on_map: v })),
        ),
        tune(t.damage, (v) => edit((r) => setTuning(r, w.id, { damage: v }))),
        tune(t.fire_rate, (v) => edit((r) => setTuning(r, w.id, { fire_rate: v }))),
        w.count ? tune(t.ammo, (v) => edit((r) => setTuning(r, w.id, { ammo: v }))) : el("td", {}, el("span", { class: "muted" }, "-")),
      );
    });
    return el(
      "section",
      {},
      el("h2", {}, "Weapons"),
      el("p", { class: "muted" }, "Pickup weight: how often it spawns relative to the others (quarters). Respawn in whole seconds. Multipliers in steps of 0.05."),
      el("div", { class: "rules-table" }, el("table", {}, el("thead", {}, head), el("tbody", {}, ...rows))),
    );
  };

  const upgradesSection = (): HTMLElement => {
    const rows = catalog.weapons
      .filter((w) => w.maxLevel > 1)
      .map((w) => {
        const up = rules.upgrade_pickups[w.id];
        const usable = availabilityOf(rules, w.id) === "LoadoutAndPickup" || availabilityOf(rules, w.id) === "PickupOnly";
        return el(
          "tr",
          {},
          el("td", {}, `${w.name} (levels 1-${w.maxLevel})`),
          el(
            "td",
            {},
            checkbox(up !== undefined, (on) => edit((r) => setUpgradePickup(r, w.id, on ? {} : null)), { disabled: !usable && up === undefined }),
          ),
          el("td", {}, up ? numberInput(up.weight, { min: 0, step: 0.25, class: "narrow" }, (v) => edit((r) => setUpgradePickup(r, w.id, { weight: v }))) : "-"),
          el(
            "td",
            {},
            up ? numberInput(up.respawn_secs, { min: 0, step: 1, class: "narrow" }, (v) => edit((r) => setUpgradePickup(r, w.id, { respawn_secs: v }))) : "-",
          ),
        );
      });
    return el(
      "section",
      {},
      el("h2", {}, "Upgrade pickups"),
      el(
        "div",
        { class: "rules-table" },
        el("table", {}, el("thead", {}, el("tr", {}, ...["Weapon", "Spawns", "Weight", "Respawn (s)"].map((h) => el("th", {}, h)))), el("tbody", {}, ...rows)),
      ),
    );
  };

  const gadgetsSection = (bad: Set<string>): HTMLElement => {
    const rows = catalog.gadgets.map((g) => {
      const a = gadgetAvailabilityOf(rules, g.id);
      const p = rules.gadget_pickups[g.id];
      const loadout = el("input", { type: "radio", name: "rules-gadget", disabled: a === "Banned" || a === "PickupOnly" });
      loadout.checked = rules.starting_loadout.gadget === g.id;
      loadout.addEventListener("change", () => edit((r) => setLoadoutGadget(r, g.id)));
      return el(
        "tr",
        { class: `${a === "Banned" ? "banned" : ""}${bad.has(`gadget:${g.id}`) ? " invalid" : ""}` },
        el("td", {}, g.name),
        el("td", {}, select(a, availabilityOptions, (v) => edit((r) => setGadgetAvailability(r, catalog, g.id, v)))),
        el("td", {}, loadout),
        el("td", {}, p ? numberInput(p.weight, { min: 0, step: 0.25, class: "narrow" }, (v) => edit((r) => setGadgetPickup(r, g.id, { weight: v }))) : "-"),
        el("td", {}, p ? numberInput(p.respawn_secs, { min: 0, step: 1, class: "narrow" }, (v) => edit((r) => setGadgetPickup(r, g.id, { respawn_secs: v }))) : "-"),
      );
    });
    const none = el("input", { type: "radio", name: "rules-gadget" });
    none.checked = rules.starting_loadout.gadget === null;
    none.addEventListener("change", () => edit((r) => setLoadoutGadget(r, null)));
    return el(
      "section",
      { class: bad.has("loadout") ? "invalid" : "" },
      el("h2", {}, "Gadgets"),
      el(
        "div",
        { class: "rules-table" },
        el(
          "table",
          {},
          el("thead", {}, el("tr", {}, ...["Gadget", "Availability", "Loadout", "Pickup weight", "Respawn (s)"].map((h) => el("th", {}, h)))),
          el("tbody", {}, ...rows, el("tr", {}, el("td", {}, "No gadget"), el("td"), el("td", {}, none), el("td"), el("td"))),
        ),
      ),
    );
  };

  const scrapSection = (bad: Set<string>): HTMLElement => {
    const s = rules.scrap_pickup;
    const on = checkbox(s !== null, (v) => edit((r) => (r.scrap_pickup = v ? { ...(catalog.defaults.PaintTheVoid.scrap_pickup ?? { weight: 4, respawn_secs: 15, amount: 30 }) } : null)));
    const fields = s
      ? [
          el("label", { class: "rules-field" }, el("span", {}, "Weight"), numberInput(s.weight, { min: 0, step: 0.25 }, (v) => edit((r) => r.scrap_pickup && (r.scrap_pickup.weight = v)))),
          el("label", { class: "rules-field" }, el("span", {}, "Respawn (s)"), numberInput(s.respawn_secs, { min: 0, step: 1 }, (v) => edit((r) => r.scrap_pickup && (r.scrap_pickup.respawn_secs = v)))),
          el("label", { class: "rules-field" }, el("span", {}, "Scrap per pile"), numberInput(s.amount, { min: 1, max: 255, step: 1 }, (v) => edit((r) => r.scrap_pickup && (r.scrap_pickup.amount = v)))),
        ]
      : [];
    return el(
      "section",
      { class: bad.has("scrap") ? "invalid" : "" },
      el("h2", {}, "Scrap piles"),
      el("div", { class: "rules-grid" }, el("label", { class: "check" }, on, el("span", {}, "Scrap piles spawn at pickup points")), ...fields),
    );
  };

  const pigmentSection = (bad: Set<string>): HTMLElement => {
    const p = rules.pigment_pickup;
    const fallback = catalog.defaults[rules.mode.kind].pigment_pickup ?? { respawn_secs: 30, players_per_pickup: 4, min_on_map: 2 };
    const on = checkbox(p !== null, (v) => edit((r) => (r.pigment_pickup = v ? { ...fallback } : null)));
    const fields = p
      ? [
          el("label", { class: "rules-field" }, el("span", {}, "Respawn (s)"), numberInput(p.respawn_secs, { min: 0, max: 1023, step: 1 }, (v) => edit((r) => r.pigment_pickup && (r.pigment_pickup.respawn_secs = v)))),
          el("label", { class: "rules-field" }, el("span", {}, "One per players"), numberInput(p.players_per_pickup, { min: 1, max: 15, step: 1 }, (v) => edit((r) => r.pigment_pickup && (r.pigment_pickup.players_per_pickup = v)))),
          el("label", { class: "rules-field" }, el("span", {}, "At least"), numberInput(p.min_on_map, { min: 0, max: 15, step: 1 }, (v) => edit((r) => r.pigment_pickup && (r.pigment_pickup.min_on_map = v)))),
        ]
      : [];
    return el(
      "section",
      { class: bad.has("pigment") ? "invalid" : "" },
      el("h2", {}, "Pigment pickups"),
      el("div", { class: "rules-grid" }, el("label", { class: "check" }, on, el("span", {}, "Pigment at pickup points on monochrome levels")), ...fields),
    );
  };

  const eventsSection = (bad: Set<string>): HTMLElement => {
    const boxes = catalog.events.map((e) =>
      el(
        "label",
        { class: eventBanned(rules, e.id) ? "check banned" : "check" },
        checkbox(!eventBanned(rules, e.id), (on) => edit((r) => setEventBanned(r, e.id, !on))),
        el("span", {}, e.name),
      ),
    );
    return el(
      "section",
      { class: bad.has("events") ? "invalid" : "" },
      el("h2", {}, "Comedy events"),
      el(
        "p",
        { class: "muted" },
        rules.absurdity === 0
          ? "Absurdity cap 0: no comedy events at all."
          : `The level rolls these at its own Absurdity, at most ${rules.absurdity}. Untick to ban one.`,
      ),
      el("div", { class: rules.absurdity === 0 ? "rules-events off" : "rules-events" }, ...boxes),
    );
  };

  const outputSection = (code: string | null, problems: RuleProblem[]): HTMLElement => {
    const worldInput = el("input", { type: "text", value: world, placeholder: "World share code (optional, LS...)", spellcheck: "false" });
    worldInput.addEventListener("change", () => {
      world = worldInput.value.trim();
      render();
    });
    const out = (label: string, value: string): HTMLElement => {
      const input = el("input", { type: "text", value, readonly: true, spellcheck: "false" });
      input.addEventListener("focus", () => input.select());
      const btn = el("button", { type: "button" }, "Copy");
      btn.addEventListener("click", () => void copy(value, btn));
      return el("label", { class: "rules-field rules-out" }, el("span", {}, label), el("div", { class: "rules-row" }, input, btn));
    };
    const invite = { world: world || null, rules: code };
    return el(
      "section",
      { class: "rules-output" },
      el("h2", {}, "Share"),
      problems.length
        ? el("ul", { class: "rules-problems", role: "alert" }, ...problems.map((p) => el("li", {}, p.message)))
        : el("p", { class: "muted" }, rulesSummary(rules, catalog)),
      ...(code
        ? [
            out("Rule set code", code),
            el("label", { class: "rules-field" }, el("span", {}, "World"), worldInput),
            out("Invite link", inviteLink(opts.baseUrl, invite)),
            out("Host command", serverCommand(invite)),
          ]
        : [el("p", { class: "field-error" }, "Fix the problems above to get a code.")]),
    );
  };

  render();
  return { remove: () => root.remove() };
}

let styled = false;

function injectStyle(): void {
  if (styled || typeof document === "undefined") return;
  styled = true;
  const style = document.createElement("style");
  style.textContent = `
.rules-editor { overflow-y: auto; align-items: flex-start; }
.rules-card { background: var(--panel); border-radius: 10px; padding: 20px 24px; margin: 24px auto; max-width: 1100px; width: calc(100% - 32px); box-sizing: border-box; }
.rules-card h1 { margin: 0; font-size: 1.6rem; }
.rules-card h2 { font-size: 1.1rem; margin: 18px 0 8px; color: var(--accent); }
.rules-card h3 { font-size: 0.95rem; margin: 12px 0 6px; color: var(--muted); }
.rules-card section.invalid > h2 { color: var(--error); }
.rules-header { display: flex; justify-content: space-between; align-items: center; gap: 12px; flex-wrap: wrap; }
.rules-actions, .rules-row { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
.rules-row input[type=text] { flex: 1; min-width: 200px; }
.rules-grid { display: flex; gap: 12px 20px; flex-wrap: wrap; align-items: flex-end; }
.rules-field { display: flex; flex-direction: column; gap: 4px; font-size: 0.85rem; }
.rules-field span { color: var(--muted); }
.rules-out { margin-top: 10px; }
.rules-card input, .rules-card select { background: rgba(255,255,255,0.06); color: var(--text); border: 1px solid rgba(255,255,255,0.18); border-radius: 5px; padding: 5px 7px; font: inherit; }
.rules-card input.narrow { width: 5.5em; }
.rules-card input:disabled, .rules-card select:disabled { opacity: 0.35; }
.rules-card button { background: var(--accent); color: #1a1206; border: 0; border-radius: 5px; padding: 6px 12px; font: inherit; cursor: pointer; }
.rules-card button.secondary { background: rgba(255,255,255,0.12); color: var(--text); }
.rules-table { overflow-x: auto; }
.rules-table table { border-collapse: collapse; font-size: 0.85rem; width: 100%; }
.rules-table th { text-align: left; color: var(--muted); font-weight: normal; padding: 4px 6px; white-space: nowrap; }
.rules-table td { padding: 3px 6px; white-space: nowrap; }
.rules-table tr.banned td:first-child { text-decoration: line-through; color: var(--muted); }
.rules-table tr.invalid td:first-child, .rules-ships.invalid { color: var(--error); }
.rules-ships { display: flex; gap: 16px; flex-wrap: wrap; }
.rules-events { display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 2px 16px; }
.rules-events label.check { margin: 0; padding: 4px 0; }
.rules-events label.banned span { text-decoration: line-through; color: var(--muted); }
.rules-events.off { opacity: 0.45; }
@media (max-width: 600px) { .rules-events { grid-template-columns: 1fr 1fr; } .rules-events label.check { font-size: 13px; } }
.rules-problems { color: var(--error); margin: 6px 0; padding-left: 20px; }
.rules-card .muted { color: var(--muted); }
.rules-output { border-top: 1px solid rgba(255,255,255,0.12); margin-top: 18px; }
.rules-editor-link { position: fixed; right: 16px; bottom: 16px; color: var(--accent); background: var(--panel); padding: 6px 12px; border-radius: 6px; z-index: 20; }
`;
  document.head.appendChild(style);
}
