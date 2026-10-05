// Start screen: pilot name, ship, share code and two ways to play: online (the
// server's world) or practice offline (a local world from the share code, or
// a random one when the field is empty); options (horizon assist and the View
// fieldset of ui/viewSettings.ts: FOV, cockpit frame, fit ring) and the
// controls fold away. First-impression pass (FI): the wordmark, a left-docked
// glass panel over the live flythrough (attract/backdrop.ts, wired in
// main.ts), the ship picker with the turning hulls (attract/shipBay.ts,
// through `StartScreenHooks.ships`), fade in and out.

import { checkShareCode, SHARE_CODE_PRINTED_LEN, type ShareCodeCheck } from "./shareCode.ts";
import { bindViewSettings, viewSettingsHtml } from "./viewSettings.ts";
import { audioSettingsHtml, bindAudioSettings } from "../audio/settings.ts";
import { previewBus } from "../audio/index.ts";

export type PlayMode = "online" | "practice";

export interface StartOptions {
  mode: PlayMode;
  /** Printed share code, or null: online = the server's world, practice = random. */
  shareCode: string | null;
  horizonAssist: boolean;
  name: string;
  /** Ship type to fly first (ls-sim ShipType index; the rule set may refuse it). */
  shipType: number;
}

/** Full validation (checksum, generator version) from ls-wasm; the format check runs first. */
export type CodeValidator = (code: string) => ShareCodeCheck;

const STORAGE_KEY = "ls.start";
export const DEFAULT_NAME = "Scrapling";
/** ls-net `MAX_NAME_LEN` (bytes; UTF-8). */
export const MAX_NAME_BYTES = 32;

interface Stored {
  shareCode?: string;
  horizonAssist?: boolean;
  name?: string;
  shipType?: number;
}

/** Ship types offered on the start screen, by ls-sim ShipType index. */
export const START_SHIPS: readonly { name: string; role: string }[] = [
  { name: "Skitter", role: "fast and fragile, 1 secondary" },
  { name: "Tinker", role: "all-rounder" },
  { name: "Hauler", role: "tough and slow, most secondaries" },
  { name: "Tug", role: "support, passes scrap" },
];
/** Tinker. */
export const DEFAULT_SHIP = 1;

/** A stored ship type index, or the Tinker when missing or out of range. */
export function readShipType(v: unknown): number {
  return typeof v === "number" && Number.isInteger(v) && v >= 0 && v < START_SHIPS.length ? v : DEFAULT_SHIP;
}

/**
 * The stored start preferences from their JSON (null, unparsable or not an
 * object: none). Fields of the wrong type are dropped, so a damaged entry
 * (another build, a hand edit, `null`) never breaks the start screen.
 */
export function readStored(raw: string | null): Stored {
  let v: unknown;
  try {
    v = raw ? JSON.parse(raw) : null;
  } catch {
    return {};
  }
  if (typeof v !== "object" || v === null || Array.isArray(v)) return {};
  const o = v as Record<string, unknown>;
  const out: Stored = {};
  if (typeof o["shareCode"] === "string") out.shareCode = o["shareCode"];
  if (typeof o["horizonAssist"] === "boolean") out.horizonAssist = o["horizonAssist"];
  if (typeof o["name"] === "string") out.name = o["name"];
  if (typeof o["shipType"] === "number") out.shipType = o["shipType"];
  return out;
}

function loadStored(): Stored {
  try {
    return readStored(localStorage.getItem(STORAGE_KEY));
  } catch {
    return {};
  }
}

function saveStored(value: Stored): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
  } catch {
    /* storage unavailable (private mode etc.): preferences just are not remembered */
  }
}

/** Trims a pilot name to what the server accepts (1-32 UTF-8 bytes, no control characters). */
export function cleanName(raw: string): string {
  let name = raw.replace(/\p{Cc}/gu, "").trim();
  const enc = new TextEncoder();
  while (enc.encode(name).length > MAX_NAME_BYTES) name = name.slice(0, -1);
  return name.trim() || DEFAULT_NAME;
}

/** Validates the share-code field: empty is allowed (null), otherwise format then `validate`. */
export function readShareCode(raw: string, validate?: CodeValidator): ShareCodeCheck | { ok: true; code: null } {
  if (raw.trim() === "") return { ok: true, code: null };
  const format = checkShareCode(raw);
  if (!format.ok || !validate) return format;
  return validate(format.code);
}

/** Short roles under the ship names on the start screen's picker. */
const PICK_ROLES = ["Fast · fragile", "All-rounder", "Tough · slow", "Support · tows"];

/** Keys listed under "Controls" (key, action). */
export const CONTROL_KEYS: ReadonlyArray<readonly [string, string]> = [
  ["Mouse", "Look"],
  ["W / S", "Thrust"],
  ["A / D", "Strafe"],
  ["Space / Ctrl", "Up / down"],
  ["Q / E", "Roll"],
  ["Shift", "Boost"],
  ["Mouse buttons", "Fire"],
  ["1-9, 0, wheel", "Primaries"],
  ["X", "Secondaries"],
  ["G", "Gadget"],
  ["V", "Paint colour"],
  ["B", "Ship bay"],
  ["F", "Overdrive"],
  ["R", "Scavenge"],
  ["H", "Horizon assist"],
  ["Tab", "Scoreboard"],
  ["F3", "Debug overlay"],
];

/** The wordmark: a loose screw head and the name (CSS gives the chrome and the sheen). */
export const LOGO_HTML = `
  <h1 class="logo" aria-label="Loose Screws">
    <svg class="logo-screw" viewBox="0 0 64 64" aria-hidden="true">
      <defs>
        <radialGradient id="ls-screw-face" cx="38%" cy="32%" r="75%">
          <stop offset="0" stop-color="#f6f8fb"/>
          <stop offset="0.45" stop-color="#aeb6c3"/>
          <stop offset="1" stop-color="#4c5361"/>
        </radialGradient>
        <linearGradient id="ls-screw-rim" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color="#ffd08a"/>
          <stop offset="1" stop-color="#e2541c"/>
        </linearGradient>
      </defs>
      <circle cx="32" cy="32" r="29" fill="url(#ls-screw-rim)"/>
      <circle cx="32" cy="32" r="24" fill="url(#ls-screw-face)"/>
      <path d="M28 14h8v14h14v8H36v14h-8V36H14v-8h14z" fill="#262b35"/>
      <path d="M29.5 15.5h5v14h14v5" fill="none" stroke="#7d8696" stroke-width="1.2" opacity="0.6"/>
    </svg>
    <span class="logo-word"><span>Loose</span><span>Screws</span></span>
  </h1>`;

/** Hooks for what the start screen hosts but does not own (main.ts wires the GPU parts). */
export interface StartScreenHooks {
  /**
   * The ship picker is in the DOM: `host` is the box the hull view goes
   * behind, `cards` one element per ship type. Return the view's controls
   * (or null without one); `select` follows the picked ship.
   */
  ships?(host: HTMLElement, cards: readonly HTMLElement[]): { select(shipType: number): void; dispose(): void } | null;
}

/** Sets the line about the world behind the start screen ("" hides it). */
export function setStartWorld(parent: ParentNode, text: string): void {
  const el = parent.querySelector<HTMLElement>(".start-world");
  if (!el) return;
  el.textContent = text;
  el.classList.toggle("shown", text !== "");
}

/** Fade-out of the start screen (ms; style.css `.start-screen.leaving`). */
export const START_FADE_MS = 350;

/**
 * Shows the start screen inside `parent` and resolves when the player picks a
 * mode. The screen fades out and removes itself afterwards.
 */
export function showStartScreen(parent: HTMLElement, validate?: CodeValidator, hooks: StartScreenHooks = {}): Promise<StartOptions> {
  performance.mark("ls:start-screen");
  const stored = loadStored();
  const root = document.createElement("div");
  root.className = "screen start-screen";
  const ships = START_SHIPS.map(
    (s, i) => `
          <label class="ship-pick-card" data-ship="${i}">
            <input type="radio" name="ship" value="${i}" />
            <span class="ship-pick-model"></span>
            <span class="ship-pick-name">${s.name}</span>
            <span class="ship-pick-role">${PICK_ROLES[i] ?? s.role}</span>
          </label>`,
  ).join("");
  const keys = CONTROL_KEYS.map(([k, a]) => `<span class="key">${k}</span><span>${a}</span>`).join("");
  root.innerHTML = `
    <div class="start-shade" aria-hidden="true"></div>
    <div class="start-layout">
      <header class="start-brand">
        ${LOGO_HTML}
        <p class="tagline">6DOF scrap-ship brawls in worlds you can blow apart.</p>
      </header>
      <form class="start-card" novalidate>
        <label class="field">
          <span>Pilot name</span>
          <input name="name" type="text" autocomplete="nickname" spellcheck="false" maxlength="${MAX_NAME_BYTES}" />
        </label>
        <fieldset class="ship-pick">
          <legend>Ship</legend>
          <div class="ship-pick-cards">${ships}
          </div>
        </fieldset>
        <label class="field">
          <span>Share code <em>optional</em></span>
          <input name="code" type="text" autocomplete="off" spellcheck="false"
                 maxlength="${SHARE_CODE_PRINTED_LEN + 6}" placeholder="Empty: the server's world / a random one" />
        </label>
        <p class="field-error code-error" role="alert"></p>
        <div class="play-row">
          <button type="submit" class="play" value="online"><span>Play online</span></button>
          <button type="submit" class="play secondary" value="practice"><span>Practice offline</span></button>
        </div>
        <p class="play-note">Online plays the world the server runs; a share code you enter is used for practice.</p>
        <details class="start-more">
          <summary>Options</summary>
          <label class="check">
            <input name="assist" type="checkbox" />
            <span>Horizon assist (slowly levels your roll)</span>
          </label>
          ${viewSettingsHtml()}
          ${audioSettingsHtml()}
        </details>
        <details class="start-more">
          <summary>Controls</summary>
          <div class="controls-grid">${keys}</div>
        </details>
      </form>
    </div>
    <footer class="start-footer"><span class="start-world"></span></footer>`;
  parent.appendChild(root);
  // Fade in on the next frame (the CSS transitions start from the hidden state).
  requestAnimationFrame(() => root.classList.add("shown"));

  const form = root.querySelector("form") as HTMLFormElement;
  const nameInput = form.elements.namedItem("name") as HTMLInputElement;
  const codeInput = form.elements.namedItem("code") as HTMLInputElement;
  const assistInput = form.elements.namedItem("assist") as HTMLInputElement;
  const shipInput = form.elements.namedItem("ship") as RadioNodeList;
  bindViewSettings(form);
  bindAudioSettings(form, previewBus);
  const cards = [...root.querySelectorAll<HTMLElement>(".ship-pick-card")];
  const shipHost = root.querySelector(".ship-pick-cards") as HTMLElement;
  const bay = hooks.ships?.(shipHost, cards) ?? null;
  const pickShip = (i: number): void => {
    shipInput.value = String(i);
    cards.forEach((c, k) => c.classList.toggle("selected", k === i));
    bay?.select(i);
  };
  pickShip(readShipType(stored.shipType));
  form.addEventListener("change", (e) => {
    const t = e.target;
    if (t instanceof HTMLInputElement && t.name === "ship") pickShip(readShipType(Number(t.value)));
  });
  const error = root.querySelector(".code-error") as HTMLElement;
  nameInput.value = stored.name ?? DEFAULT_NAME;
  codeInput.value = stored.shareCode ?? "";
  assistInput.checked = stored.horizonAssist ?? true;
  codeInput.focus({ preventScroll: true });

  codeInput.addEventListener("input", () => {
    error.textContent = "";
    codeInput.classList.remove("invalid");
  });

  return new Promise((resolve) => {
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      if (root.classList.contains("leaving")) return;
      const submitter = (e as SubmitEvent).submitter;
      const mode: PlayMode = submitter instanceof HTMLButtonElement && submitter.value === "practice" ? "practice" : "online";
      const check = readShareCode(codeInput.value, validate);
      if (!check.ok) {
        error.textContent = check.error;
        codeInput.classList.add("invalid");
        codeInput.focus();
        return;
      }
      const options: StartOptions = {
        mode,
        shareCode: check.code,
        horizonAssist: assistInput.checked,
        name: cleanName(nameInput.value),
        shipType: readShipType(Number(shipInput.value)),
      };
      saveStored({
        shareCode: check.code ?? "",
        horizonAssist: options.horizonAssist,
        name: options.name,
        shipType: options.shipType,
      });
      bay?.dispose();
      // Fade out, then go (the next screen draws over the fading one).
      root.classList.add("leaving");
      root.classList.remove("shown");
      setTimeout(() => root.remove(), START_FADE_MS);
      resolve(options);
    });
  });
}

/** Full-screen "please wait" card; returns a function that updates the text, and `remove`. */
export function showLoading(parent: HTMLElement, text: string): { set(text: string): void; remove(): void } {
  const root = document.createElement("div");
  root.className = "screen loading-screen";
  const card = document.createElement("div");
  card.className = "loading-card";
  card.textContent = text;
  root.appendChild(card);
  parent.appendChild(root);
  return {
    set: (t) => {
      card.textContent = t;
    },
    remove: () => root.remove(),
  };
}
