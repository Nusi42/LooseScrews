// World workshop (M4 A8, milestone-4-plan.md §3.7 "World workshop"): the
// start screen's "Make a world" page. Seven sliders with one-line meanings,
// a seed with a dice button, the level archetype (Classic or Asteroid belt,
// §3.8), the share code with "Save" (copy), "Fly it" (practice on the world)
// and "Host it" (an invite link with `code=`; on a belt world it proposes
// the Belt Up rules). The live preview is the start screen's flythrough
// (attract/intro.ts `StartIntro.preview`), regenerated PREVIEW_DEBOUNCE_MS
// after the last change.
//
// `SliderPanel` is the slider block on its own: The Architect's UI (M4 C4)
// reuses it. The model half (slider table, meanings, drafts, codes, links)
// is pure and covered by workshop.test.ts; the DOM half has no state of its
// own beyond the draft.
//
// Hooks for tools/browser-smoke (phase "workshop"): `.workshop` carries
// `data-code` (the current code); the preview sets `body[data-preview-code]`
// when the world of that code flies (attract/intro.ts).

import { CATALOG } from "../rules/catalog.ts";
import { defaultRules, inviteLink as editorLink } from "../rules/model.ts";
import { encodeRules } from "../rules/code.ts";
import type { Catalog, ModeKindName } from "../rules/types.ts";
import { worldKind } from "../attract/worlds.ts";
import { inviteLink } from "./party.ts";

// ---------------------------------------------------------------------------
// Sliders and archetypes

export type SliderKey = "enclosure" | "density" | "verticality" | "gravityChaos" | "palette" | "absurdity" | "instability";

/** Level archetypes by share-code archetype id (ls-worldgen `Archetype`, append only). */
export const ARCHETYPES = ["classic", "belt"] as const;
export type Archetype = (typeof ARCHETYPES)[number];

export const ARCHETYPE_LABELS: Record<Archetype, string> = { classic: "Classic", belt: "Asteroid belt" };

export interface SliderDef {
  key: SliderKey;
  name: string;
  /** What the slider does, one line (classic levels). */
  hint: string;
  /** The same on a belt level (§3.8 "Sliders on a belt"). */
  beltHint: string;
}

/** The seven sliders in share-code order (ls-worldgen `Sliders::to_array`). */
export const SLIDERS: readonly SliderDef[] = [
  { key: "enclosure", name: "Enclosure", hint: "Open sky to caverns to mine shafts", beltHint: "Size of the asteroids" },
  { key: "density", name: "Density", hint: "How much rock fills the space", beltHint: "Asteroids and drifting boulders" },
  { key: "verticality", name: "Verticality", hint: "Flat and wide to tall and narrow", beltHint: "Thickness of the belt" },
  { key: "gravityChaos", name: "Gravity Chaos", hint: "Gravity wells that pull you in", beltHint: "Shepherd moons along the ring" },
  { key: "palette", name: "Palette", hint: "Full colour to black-and-white ink", beltHint: "Full colour to black-and-white ink" },
  { key: "absurdity", name: "Absurdity", hint: "How often the level plays a joke", beltHint: "How often the level plays a joke" },
  { key: "instability", name: "Instability", hint: "How easily tunnels cave in", beltHint: "How easily the rock gives way" },
];

export type SliderValues = Record<SliderKey, number>;

/** ls-worldgen `Sliders::default()`. */
export const DEFAULT_SLIDERS: Readonly<SliderValues> = Object.freeze({
  enclosure: 50,
  density: 40,
  verticality: 50,
  gravityChaos: 0,
  palette: 0,
  absurdity: 20,
  instability: 30,
});

/**
 * Where a fresh workshop world starts: caverns that open to the sky with
 * plenty of rock (like the start backdrop's worlds; a closed cavern makes a
 * dark first preview), the other five at the generator's defaults.
 */
export const WORKSHOP_START_SLIDERS: Readonly<SliderValues> = Object.freeze({ ...DEFAULT_SLIDERS, enclosure: 38, density: 60 });

/** Slider values in share-code order. */
export function sliderArray(v: SliderValues): number[] {
  return SLIDERS.map((s) => v[s.key]);
}

/** Slider values from share-code order (missing or bad entries: the defaults). */
export function slidersFrom(a: ArrayLike<number>): SliderValues {
  const out = { ...DEFAULT_SLIDERS };
  SLIDERS.forEach((s, i) => {
    const n = a[i];
    if (typeof n === "number" && Number.isFinite(n)) out[s.key] = clampSlider(n);
  });
  return out;
}

export function clampSlider(n: number): number {
  return Math.max(0, Math.min(100, Math.round(n)));
}

const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;
const fmt = (n: number): string => (Math.round(n * 2) / 2).toString();

/** Level box of a Verticality value (milestone-4-plan.md §3.3): chunks of 32 u. */
export function verticalityBox(v: number): [number, number, number] {
  if (v < 20) return [10, 2, 10];
  if (v < 40) return [9, 3, 9];
  if (v < 60) return [8, 4, 8];
  if (v < 80) return [7, 5, 7];
  return [6, 7, 6];
}

/** Gravity wells of a classic level (§3.3): none at 0, else 1 + v / 15, at most 7. */
export function wellCount(v: number): number {
  return v <= 0 ? 0 : Math.min(7, 1 + Math.floor(v / 15));
}

function pullWord(v: number): string {
  return v < 35 ? "gentle" : v < 70 ? "strong" : "fierce";
}

/** The world kind shown for a draft (captions, the code box). */
export function draftKind(d: Pick<WorldDraft, "archetype" | "sliders">): string {
  return d.archetype === "belt" ? "Asteroid belt" : worldKind(d.sliders.enclosure);
}

/**
 * What a slider value means, one short line (the workshop shows it next to
 * the value). Belt meanings follow §3.8; Palette, Absurdity and Instability
 * mean the same on every level.
 */
export function sliderMeaning(key: SliderKey, value: number, archetype: Archetype = "classic"): string {
  const v = clampSlider(value);
  const t = v / 100;
  if (archetype === "belt") {
    switch (key) {
      case "enclosure":
        return `Asteroids ${fmt(lerp(1.5, 4, t))}-${fmt(lerp(3, 10, t))} u${v >= 60 ? ", the biggest hollow" : ""}`;
      case "density":
        return `${Math.round(lerp(15, 60, t))} asteroids, ${Math.round(lerp(3, 10, t))} boulders a sector`;
      case "verticality":
        return `Belt ${Math.round(2 * lerp(8, 18, t))} u thick`;
      case "gravityChaos":
        return v < 20 ? "No shepherd moons" : `8 shepherd moons, ${pullWord(v)} pull`;
      default:
        break;
    }
  }
  switch (key) {
    case "enclosure":
      return worldKind(v);
    case "density":
      return v < 20 ? "Wide open, few obstacles" : v < 45 ? "Some pillars and boulders" : v < 70 ? "Pillars and floating rock" : "Crowded with rock";
    case "verticality": {
      const [x, y, z] = verticalityBox(v);
      const shape = ["Flat and wide", "Low", "Balanced", "Tall", "Towering"][Math.min(4, Math.floor(v / 20))];
      return `${shape}, ${x * 32} × ${y * 32} × ${z * 32} u`;
    }
    case "gravityChaos": {
      const n = wellCount(v);
      return n === 0 ? "No gravity wells" : `${n} gravity well${n === 1 ? "" : "s"}, ${pullWord(v)} pull`;
    }
    case "palette":
      return v < 10 ? "Full colour" : v < 50 ? "Muted colours" : v < 80 ? "Ink outlines, toon light" : "Black-and-white ink, pigment";
    case "absurdity":
      return v === 0 ? "No comedy events" : v <= 25 ? "A rare joke" : v <= 55 ? "A few jokes a match" : v < 85 ? "Frequent jokes" : "Pure chaos";
    case "instability":
      return v === 0 ? "Solid rock, no cave-ins" : v < 30 ? "Sturdy, rare cave-ins" : v < 65 ? "Brittle, fights bring tunnels down" : "Crumbling, tunnels cave in often";
  }
}

// ---------------------------------------------------------------------------
// Drafts and codes

/** The world being made: what a share code holds. */
export interface WorldDraft {
  /** Seed as a decimal u64. */
  seed: string;
  sliders: SliderValues;
  archetype: Archetype;
}

/** Largest seed (u64). */
const SEED_MAX = (1n << 64n) - 1n;

/** The seed text as a canonical decimal u64, or null (empty, not a whole number, too big). */
export function cleanSeed(raw: string): string | null {
  const t = raw.trim().replace(/[\s_,']/g, "");
  if (!/^\d{1,20}$/.test(t)) return null;
  const n = BigInt(t);
  return n <= SEED_MAX ? n.toString() : null;
}

/** A new dice seed (1 to 999 999: short enough to read out). */
export function diceSeed(random: () => number = Math.random): string {
  return String(1 + Math.floor(random() * 999_999));
}

export function defaultDraft(random: () => number = Math.random): WorldDraft {
  return { seed: diceSeed(random), sliders: { ...WORKSHOP_START_SLIDERS }, archetype: "classic" };
}

export function archetypeId(a: Archetype): number {
  return ARCHETYPES.indexOf(a);
}

export function archetypeOf(id: number | undefined): Archetype {
  return ARCHETYPES[id ?? 0] ?? "classic";
}

/** Encodes and decodes share codes through ls-wasm (the single source of the code layout). */
export interface WorldCodec {
  /** Archetypes this build generates (others show disabled). */
  readonly archetypes: readonly Archetype[];
  encode(d: WorldDraft): { ok: true; code: string } | { ok: false; error: string };
  decode(code: string): { ok: true; draft: WorldDraft; code: string } | { ok: false; error: string };
}

/** What `WasmApi` offers for the codec (structural, so tests can fake it). */
export interface CodecApi {
  checkShareCode(text: string): { ok: boolean; code: string; error: string; seed?: string; sliders?: ArrayLike<number>; archetype?: number; free(): void };
  makeShareCode?(seed: string, sliders: Uint8Array, archetype: number): string;
  levelArchetypes?(): string[];
}

export function wasmCodec(api: CodecApi): WorldCodec {
  const names = api.levelArchetypes?.() ?? ["classic"];
  const archetypes = ARCHETYPES.filter((a) => names.includes(a));
  return {
    archetypes,
    encode: (d) => {
      if (!api.makeShareCode) return { ok: false, error: "This build cannot make share codes." };
      if (!archetypes.includes(d.archetype)) return { ok: false, error: `${ARCHETYPE_LABELS[d.archetype]} worlds need a newer game build.` };
      try {
        return { ok: true, code: api.makeShareCode(d.seed, Uint8Array.from(sliderArray(d.sliders)), archetypeId(d.archetype)) };
      } catch (err) {
        return { ok: false, error: err instanceof Error ? err.message : String(err) };
      }
    },
    decode: (text) => {
      const info = api.checkShareCode(text);
      try {
        if (!info.ok) return { ok: false, error: info.error };
        if (info.seed === undefined || info.sliders === undefined) return { ok: false, error: "This build cannot read share codes." };
        return { ok: true, code: info.code, draft: { seed: info.seed, sliders: slidersFrom(info.sliders), archetype: archetypeOf(info.archetype) } };
      } finally {
        info.free();
      }
    },
  };
}

/** A stored draft from its JSON (anything damaged: null). */
export function readDraft(raw: string | null): WorldDraft | null {
  let v: unknown;
  try {
    v = raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
  if (typeof v !== "object" || v === null) return null;
  const o = v as Record<string, unknown>;
  const seed = typeof o["seed"] === "string" ? cleanSeed(o["seed"]) : null;
  const sliders = Array.isArray(o["sliders"]) && o["sliders"].length === SLIDERS.length ? (o["sliders"] as unknown[]) : null;
  if (seed === null || sliders === null || sliders.some((n) => typeof n !== "number")) return null;
  const archetype = ARCHETYPES.includes(o["archetype"] as Archetype) ? (o["archetype"] as Archetype) : "classic";
  return { seed, sliders: slidersFrom(sliders as number[]), archetype };
}

export function writeDraft(d: WorldDraft): string {
  return JSON.stringify({ seed: d.seed, sliders: sliderArray(d.sliders), archetype: d.archetype });
}

export const WORKSHOP_STORAGE_KEY = "ls.workshop";

function loadDraft(): WorldDraft | null {
  try {
    return readDraft(localStorage.getItem(WORKSHOP_STORAGE_KEY));
  } catch {
    return null;
  }
}

function saveDraft(d: WorldDraft): void {
  try {
    localStorage.setItem(WORKSHOP_STORAGE_KEY, writeDraft(d));
  } catch {
    /* storage unavailable: the draft is just not remembered */
  }
}

/** `?workshop` opens the world workshop at once; `?workshop=<code>` on that world. */
export function wantsWorkshop(loc: Pick<Location, "search">): boolean | string {
  const p = new URLSearchParams(loc.search);
  if (!p.has("workshop")) return false;
  const v = p.get("workshop")?.trim() ?? "";
  return v === "" ? true : v;
}

// ---------------------------------------------------------------------------
// Hosting

/** The Belt Up mode (ModeKind 7, M4 B6) when this build's rule catalog has it. */
export function beltUpMode(catalog: Catalog): ModeKindName | null {
  const m = catalog.modes.find((x) => (x.kind as string) === "BeltUp");
  return m ? m.kind : null;
}

/** The rule set code a belt world proposes (Belt Up's defaults), or null when the build has no Belt Up. */
export function beltUpRules(catalog: Catalog): string | null {
  const kind = beltUpMode(catalog);
  if (!kind || !catalog.defaults[kind]) return null;
  return encodeRules(defaultRules(catalog, kind), catalog);
}

export interface HostPlan {
  /** Invite link: opening it on this server starts a private match on the world. */
  link: string;
  /** Rule set code sent along (Belt Up on a belt world), or null for the server's rules. */
  rules: string | null;
  /** Command line for hosting on your own machine. */
  command: string;
  /** The rule set editor on this world (pick other rules). */
  editor: string;
  /** A line about the rules. */
  note: string;
}

/**
 * How to host a world: an invite link with `code=` on the page's server (a
 * private custom match there, ui/party.ts) and the ls-server command line. A
 * belt world proposes Belt Up (§3.8) when `proposeBeltUp` is on and the build
 * has it.
 */
export function hostPlan(pageHref: string, code: string, archetype: Archetype, proposeBeltUp = true, catalog: Catalog = CATALOG): HostPlan {
  const belt = archetype === "belt" ? beltUpRules(catalog) : null;
  const rules = proposeBeltUp ? belt : null;
  const clean = code.toUpperCase().replace(/[\s-]+/g, "");
  const command = ["ls-server", "--code", code, ...(rules ? ["--ruleset", rules] : [])].join(" ");
  let note: string;
  if (archetype === "belt") {
    note = belt
      ? rules
        ? "Belt Up: race one lap around the belt and dive to the EXIT."
        : "The server's rules. Belt worlds are built for Belt Up."
      : "This build has no Belt Up yet: the server's rules apply.";
  } else {
    note = "The server's rules. Pick others in the rule set editor.";
  }
  const editorUrl = editorLink(pageHref, { world: clean, rules });
  const e = new URL(editorUrl);
  e.searchParams.set("editor", "rules");
  return { link: inviteLink(pageHref, { world: clean, rules }), rules, command, editor: e.toString(), note };
}

// ---------------------------------------------------------------------------
// DOM: the slider panel (shared with The Architect)

const SVG = (body: string): string =>
  `<svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">${body}</svg>`;

/** Small line icons per slider and archetype. */
export const SLIDER_ICONS: Record<SliderKey, string> = {
  enclosure: SVG('<path d="M3 20V12a9 8 0 0 1 18 0v8"/><path d="M8 20v-6a4 4 0 0 1 8 0v6"/>'),
  density: SVG('<circle cx="7" cy="8" r="3"/><circle cx="16.5" cy="6.5" r="2"/><circle cx="15" cy="16" r="4"/><circle cx="6" cy="17.5" r="1.6"/>'),
  verticality: SVG('<path d="M12 3v18"/><path d="M8 7l4-4 4 4"/><path d="M8 17l4 4 4-4"/>'),
  gravityChaos: SVG('<circle cx="12" cy="12" r="2.2"/><path d="M12 4a8 8 0 0 1 8 8"/><path d="M12 20a8 8 0 0 1-8-8"/><path d="M17 12a5 5 0 0 1-5 5"/><path d="M7 12a5 5 0 0 1 5-5"/>'),
  palette: SVG('<circle cx="12" cy="12" r="8.5"/><path d="M12 3.5v17" /><path d="M12 6a6 6 0 0 1 0 12z" fill="currentColor" stroke="none"/>'),
  absurdity: SVG('<circle cx="12" cy="12" r="8.5"/><path d="M8.2 14.2c1.6 2.4 6 2.4 7.6 0"/><path d="M8.5 9.5h.01M15.5 9.5h.01" stroke-width="2.6"/>'),
  instability: SVG('<path d="M4 6h16"/><path d="M9 6l2 5-3 3 2 6"/><path d="M15 6l-1 4 3 3-1 4"/>'),
};

const ARCHETYPE_ICONS: Record<Archetype, string> = {
  classic: SVG('<path d="M3 19V12a9 8 0 0 1 18 0v7"/><path d="M3 19h18"/><path d="M9 19v-4a3 3 0 0 1 6 0v4"/>'),
  belt: SVG('<ellipse cx="12" cy="12" rx="9.6" ry="4.6" stroke-width="2.6" stroke-dasharray="0.1 3.4"/><circle cx="12" cy="12" r="2.1"/>'),
};

function esc(s: string): string {
  return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c] ?? c);
}

export interface SliderPanelOptions {
  values: SliderValues;
  archetype?: Archetype;
  /** Sliders shown (default all seven, share-code order). */
  keys?: readonly SliderKey[];
  /** Called on every movement of a slider (input events). */
  onInput?(key: SliderKey, value: number): void;
}

/** The seven sliders with their meanings (the workshop; reused by The Architect's stroke UI). */
export class SliderPanel {
  readonly root: HTMLElement;
  private readonly rows = new Map<SliderKey, { input: HTMLInputElement; value: HTMLElement; meaning: HTMLElement; hint: HTMLElement }>();
  private archetype: Archetype;
  private current: SliderValues;

  constructor(opts: SliderPanelOptions) {
    this.archetype = opts.archetype ?? "classic";
    this.current = { ...opts.values };
    const root = document.createElement("div");
    root.className = "slider-panel";
    const keys = opts.keys ?? SLIDERS.map((s) => s.key);
    for (const key of keys) {
      const def = SLIDERS.find((s) => s.key === key);
      if (!def) continue;
      const row = document.createElement("label");
      row.className = "slider-row";
      row.dataset["slider"] = key;
      row.innerHTML = `
        <span class="slider-icon">${SLIDER_ICONS[key]}</span>
        <span class="slider-head">
          <span class="slider-name">${esc(def.name)}</span>
          <span class="slider-meaning"></span>
          <output class="slider-value"></output>
        </span>
        <input type="range" min="0" max="100" step="1" name="${key}" />
        <span class="slider-hint"></span>`;
      const input = row.querySelector("input") as HTMLInputElement;
      const rowParts = {
        input,
        value: row.querySelector(".slider-value") as HTMLElement,
        meaning: row.querySelector(".slider-meaning") as HTMLElement,
        hint: row.querySelector(".slider-hint") as HTMLElement,
      };
      input.setAttribute("aria-label", def.name);
      input.addEventListener("input", () => {
        const v = clampSlider(Number(input.value));
        this.current[key] = v;
        this.paint(key);
        opts.onInput?.(key, v);
      });
      this.rows.set(key, rowParts);
      root.appendChild(row);
    }
    this.root = root;
    this.paintAll();
  }

  get values(): SliderValues {
    return { ...this.current };
  }

  set(values: SliderValues): void {
    this.current = { ...values };
    this.paintAll();
  }

  setArchetype(a: Archetype): void {
    this.archetype = a;
    this.paintAll();
  }

  private paintAll(): void {
    for (const key of this.rows.keys()) this.paint(key);
  }

  private paint(key: SliderKey): void {
    const row = this.rows.get(key);
    const def = SLIDERS.find((s) => s.key === key);
    if (!row || !def) return;
    const v = this.current[key];
    if (Number(row.input.value) !== v) row.input.value = String(v);
    row.input.style.setProperty("--fill", `${v}%`);
    row.value.textContent = String(v);
    row.meaning.textContent = sliderMeaning(key, v, this.archetype);
    row.hint.textContent = this.archetype === "belt" ? def.beltHint : def.hint;
  }
}

// ---------------------------------------------------------------------------
// DOM: the workshop page

/** Preview regeneration delay after the last change (ms, §3.7). */
export const PREVIEW_DEBOUNCE_MS = 300;

export type WorkshopExit = { kind: "fly"; code: string } | { kind: "back"; code: string | null };

export interface WorkshopDeps {
  codec: WorldCodec;
  /** Shows the world of `code` behind the page (debounced by the workshop). */
  preview?(code: string): void;
  /** Page URL the invite links are built on (default `location.href`). */
  pageHref?: string;
  /** Opening code (else the remembered draft, else a fresh one). */
  code?: string | null;
  random?: () => number;
  /** Copies text (default: the async clipboard, falling back to a selection). */
  copy?(text: string): Promise<boolean>;
  /** Remember the draft in localStorage (default true; the gallery passes false). */
  remember?: boolean;
}

export interface WorkshopHandle {
  readonly root: HTMLElement;
  /** Resolves when the player flies the world or goes back. */
  readonly done: Promise<WorkshopExit>;
  /** Preview status from the backdrop: generating (0..1 done) or ready. */
  setPreview(state: { code: string; progress: number; ready: boolean }): void;
  /** The draft now (tests, the gallery). */
  readonly draft: WorldDraft;
  /** Opens the host box (the gallery). */
  showHost(): void;
  dispose(): void;
}

async function defaultCopy(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/**
 * Shows the workshop in `parent` (the start screen's root). Nothing in it
 * blocks: every change re-encodes the code at once and schedules the preview.
 */
export function showWorkshop(parent: HTMLElement, deps: WorkshopDeps): WorkshopHandle {
  const random = deps.random ?? Math.random;
  const remember = deps.remember ?? true;
  const copy = deps.copy ?? defaultCopy;
  const pageHref = deps.pageHref ?? location.href;
  const codec = deps.codec;

  let draft: WorldDraft = defaultDraft(random);
  const opened = deps.code ? codec.decode(deps.code) : null;
  if (opened?.ok) draft = opened.draft;
  else if (remember) draft = loadDraft() ?? draft;
  if (!codec.archetypes.includes(draft.archetype)) draft = { ...draft, archetype: "classic" };

  const root = document.createElement("div");
  root.className = "workshop";
  const archetypeButtons = ARCHETYPES.map((a) => {
    const off = !codec.archetypes.includes(a);
    const title = off ? ` title="${esc(ARCHETYPE_LABELS[a])} worlds arrive with generator version 4"` : "";
    return `<button type="button" class="arch" role="radio" data-arch="${a}"${off ? " disabled" : ""}${title}>${ARCHETYPE_ICONS[a]}<span>${esc(ARCHETYPE_LABELS[a])}</span></button>`;
  }).join("");
  root.innerHTML = `
    <div class="workshop-layout">
      <header class="workshop-head">
        <button type="button" class="workshop-back" aria-label="Back to the start screen">${SVG('<path d="M15 5l-7 7 7 7"/>')}</button>
        <div>
          <h2 class="workshop-title">Make a world</h2>
          <p class="workshop-sub">Every slider is live: the world behind this panel rebuilds as you go.</p>
        </div>
      </header>
      <div class="preview-chip" aria-live="polite"><span class="preview-dot"></span><span class="preview-text">Preview</span></div>
      <section class="workshop-card">
        <div class="workshop-top">
          <div class="arch-switch" role="radiogroup" aria-label="Level type">${archetypeButtons}</div>
          <label class="seed-field">
            <span>Seed</span>
            <input name="seed" type="text" inputmode="numeric" autocomplete="off" spellcheck="false" maxlength="24" />
            <button type="button" class="dice" aria-label="Roll a new seed" title="Roll a new seed">${SVG('<rect x="4" y="4" width="16" height="16" rx="3.5"/><path d="M8.5 8.5h.01M15.5 8.5h.01M12 12h.01M8.5 15.5h.01M15.5 15.5h.01" stroke-width="2.8"/>')}</button>
          </label>
        </div>
        <div class="slider-slot"></div>
        <div class="code-box">
          <div class="code-meta"><span class="code-kind"></span><span class="code-status" aria-live="polite"></span></div>
          <div class="code-row">
            <code class="code-text"></code>
            <button type="button" class="save">${SVG('<path d="M8 4h9l3 3v13H4V4h4z"/><path d="M8 4v5h7V4"/><path d="M8 20v-6h8v6"/>')}<span>Save</span></button>
          </div>
          <p class="field-error code-error" role="alert"></p>
        </div>
        <div class="workshop-actions">
          <button type="button" class="play fly"><span>Fly it</span></button>
          <button type="button" class="play secondary host"><span>Host it</span></button>
        </div>
        <div class="host-box" hidden>
          <p class="host-note"></p>
          <label class="check belt-up-check" hidden><input type="checkbox" name="beltUp" checked /><span>Race it as Belt Up</span></label>
          <div class="host-row">
            <input class="host-link" type="text" readonly aria-label="Invite link" />
            <button type="button" class="host-copy">Copy</button>
          </div>
          <div class="host-links">
            <a class="host-open" href="#">Open the match</a>
            <a class="host-editor" href="#">Pick rules</a>
          </div>
          <p class="host-cmd-label">Your own server</p>
          <code class="host-cmd"></code>
        </div>
      </section>
    </div>`;
  parent.appendChild(root);
  requestAnimationFrame(() => root.classList.add("shown"));

  const q = <T extends HTMLElement>(sel: string): T => root.querySelector(sel) as T;
  const seedInput = q<HTMLInputElement>('input[name="seed"]');
  const codeText = q<HTMLElement>(".code-text");
  const codeKind = q<HTMLElement>(".code-kind");
  const codeStatus = q<HTMLElement>(".code-status");
  const codeError = q<HTMLElement>(".code-error");
  const hostBox = q<HTMLElement>(".host-box");
  const hostLink = q<HTMLInputElement>(".host-link");
  const hostNote = q<HTMLElement>(".host-note");
  const hostCmd = q<HTMLElement>(".host-cmd");
  const hostOpen = q<HTMLAnchorElement>(".host-open");
  const hostEditor = q<HTMLAnchorElement>(".host-editor");
  const beltCheck = q<HTMLElement>(".belt-up-check");
  const beltInput = q<HTMLInputElement>('input[name="beltUp"]');
  const chip = q<HTMLElement>(".preview-chip");
  const chipText = q<HTMLElement>(".preview-text");
  const panel = new SliderPanel({
    values: draft.sliders,
    archetype: draft.archetype,
    onInput: (key, v) => {
      draft = { ...draft, sliders: { ...draft.sliders, [key]: v } };
      changed();
    },
  });
  q<HTMLElement>(".slider-slot").appendChild(panel.root);

  let code: string | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let previewed: string | null = null;
  let disposed = false;
  let statusTimer: ReturnType<typeof setTimeout> | null = null;

  const flash = (text: string): void => {
    codeStatus.textContent = text;
    codeStatus.classList.add("on");
    if (statusTimer) clearTimeout(statusTimer);
    statusTimer = setTimeout(() => codeStatus.classList.remove("on"), 1600);
  };

  const paintArchetype = (): void => {
    root.dataset["archetype"] = draft.archetype;
    root.querySelectorAll<HTMLButtonElement>(".arch").forEach((b) => {
      const on = b.dataset["arch"] === draft.archetype;
      b.classList.toggle("selected", on);
      b.setAttribute("aria-checked", String(on));
    });
  };

  const paintHost = (): void => {
    if (!code) return;
    const plan = hostPlan(pageHref, code, draft.archetype, beltInput.checked);
    beltCheck.hidden = !(draft.archetype === "belt" && beltUpRules(CATALOG) !== null);
    hostNote.textContent = plan.note;
    hostLink.value = plan.link;
    hostOpen.href = plan.link;
    hostEditor.href = plan.editor;
    hostCmd.textContent = plan.command;
  };

  const schedulePreview = (): void => {
    if (!deps.preview || !code) return;
    if (timer) clearTimeout(timer);
    const want = code;
    timer = setTimeout(() => {
      timer = null;
      if (disposed || want === previewed) return;
      previewed = want;
      deps.preview?.(want);
    }, PREVIEW_DEBOUNCE_MS);
  };

  /** Re-encodes the draft, repaints and schedules the preview. */
  const changed = (persist = true): void => {
    const r = codec.encode(draft);
    if (r.ok) {
      code = r.code;
      codeText.textContent = r.code;
      root.dataset["code"] = r.code;
      codeError.textContent = "";
      root.classList.remove("invalid");
    } else {
      code = null;
      codeText.textContent = "—";
      delete root.dataset["code"];
      codeError.textContent = r.error;
      root.classList.add("invalid");
    }
    codeKind.textContent = draftKind(draft);
    if (persist && remember) saveDraft(draft);
    if (!hostBox.hidden) paintHost();
    schedulePreview();
  };

  const setArchetype = (a: Archetype): void => {
    if (a === draft.archetype || !codec.archetypes.includes(a)) return;
    draft = { ...draft, archetype: a };
    panel.setArchetype(a);
    paintArchetype();
    changed();
  };

  root.querySelectorAll<HTMLButtonElement>(".arch").forEach((b) =>
    b.addEventListener("click", () => setArchetype(b.dataset["arch"] as Archetype)),
  );
  root.querySelector(".arch-switch")?.addEventListener("keydown", (e) => {
    const k = (e as KeyboardEvent).key;
    if (k !== "ArrowLeft" && k !== "ArrowRight") return;
    const list = codec.archetypes;
    const i = list.indexOf(draft.archetype);
    const next = list[(i + (k === "ArrowRight" ? 1 : list.length - 1)) % list.length];
    if (next) {
      setArchetype(next);
      root.querySelector<HTMLButtonElement>(`.arch[data-arch="${next}"]`)?.focus();
    }
    e.preventDefault();
  });

  seedInput.value = draft.seed;
  seedInput.addEventListener("input", () => {
    const s = cleanSeed(seedInput.value);
    seedInput.classList.toggle("invalid", s === null);
    if (s === null) {
      codeError.textContent = "The seed is a whole number from 0 to 18446744073709551615.";
      return;
    }
    draft = { ...draft, seed: s };
    changed();
  });
  seedInput.addEventListener("blur", () => {
    seedInput.value = draft.seed;
    seedInput.classList.remove("invalid");
  });
  q<HTMLButtonElement>(".dice").addEventListener("click", (e) => {
    const b = e.currentTarget as HTMLElement;
    b.classList.remove("rolling");
    void b.offsetWidth; // restart the roll animation
    b.classList.add("rolling");
    draft = { ...draft, seed: diceSeed(random) };
    seedInput.value = draft.seed;
    seedInput.classList.remove("invalid");
    changed();
  });

  q<HTMLButtonElement>(".save").addEventListener("click", () => {
    if (!code) return;
    const c = code;
    void copy(c).then((ok) => flash(ok ? "Copied" : "Select and copy the code"));
    const sel = window.getSelection();
    if (sel) {
      const range = document.createRange();
      range.selectNodeContents(codeText);
      sel.removeAllRanges();
      sel.addRange(range);
    }
  });
  q<HTMLButtonElement>(".host-copy").addEventListener("click", () => {
    hostLink.select();
    void copy(hostLink.value).then((ok) => flash(ok ? "Invite link copied" : "Select and copy the link"));
  });
  const showHost = (): void => {
    hostBox.hidden = false;
    root.classList.add("hosting");
    paintHost();
    hostBox.scrollIntoView?.({ block: "nearest" });
  };
  q<HTMLButtonElement>(".host").addEventListener("click", () => {
    if (hostBox.hidden) showHost();
    else {
      hostBox.hidden = true;
      root.classList.remove("hosting");
    }
  });
  beltInput.addEventListener("change", paintHost);

  panel.setArchetype(draft.archetype);
  paintArchetype();
  changed(false);

  let finish: (exit: WorkshopExit) => void = () => {};
  const done = new Promise<WorkshopExit>((resolve) => {
    finish = (exit) => {
      if (disposed) return;
      handle.dispose();
      resolve(exit);
    };
  });
  q<HTMLButtonElement>(".fly").addEventListener("click", () => {
    if (code) finish({ kind: "fly", code });
  });
  q<HTMLButtonElement>(".workshop-back").addEventListener("click", () => finish({ kind: "back", code }));
  const onKey = (e: KeyboardEvent): void => {
    if (e.key === "Escape") finish({ kind: "back", code });
  };
  document.addEventListener("keydown", onKey);

  const handle: WorkshopHandle = {
    root,
    done,
    get draft() {
      return { ...draft, sliders: { ...draft.sliders } };
    },
    setPreview: (s) => {
      const current = s.code === code;
      chip.classList.toggle("busy", !s.ready);
      chip.classList.toggle("stale", !current);
      chipText.textContent = s.ready ? (current ? "Live preview" : "Updating…") : `Building the preview… ${Math.round(s.progress * 100)}%`;
    },
    showHost,
    dispose: () => {
      if (disposed) return;
      disposed = true;
      if (timer) clearTimeout(timer);
      if (statusTimer) clearTimeout(statusTimer);
      document.removeEventListener("keydown", onKey);
      root.remove();
    },
  };
  // Keyboard focus moves into the workshop (the start screen's button that opened it is hidden now).
  (root.querySelector<HTMLElement>(".arch.selected") ?? root.querySelector<HTMLElement>(".slider-panel input"))?.focus({ preventScroll: true });
  // The first preview without waiting for a change.
  if (deps.preview && code) {
    previewed = code;
    deps.preview(code);
  }
  return handle;
}
