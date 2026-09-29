// Start screen: pilot name, share code, horizon-assist preference and two ways
// to play: online (the server's world) or practice offline (a local world from
// the share code, or a random one when the field is empty).

import { checkShareCode, SHARE_CODE_PRINTED_LEN, type ShareCodeCheck } from "./shareCode.ts";

export type PlayMode = "online" | "practice";

export interface StartOptions {
  mode: PlayMode;
  /** Printed share code, or null: online = the server's world, practice = random. */
  shareCode: string | null;
  horizonAssist: boolean;
  name: string;
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
}

function loadStored(): Stored {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as Stored) : {};
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

/**
 * Shows the start screen inside `parent` and resolves when the player picks a
 * mode. The screen removes itself afterwards.
 */
export function showStartScreen(parent: HTMLElement, validate?: CodeValidator): Promise<StartOptions> {
  const stored = loadStored();
  const root = document.createElement("div");
  root.className = "screen start-screen";
  root.innerHTML = `
    <form class="start-card" novalidate>
      <h1>Loose Screws</h1>
      <p class="tagline">6DOF scrap-ship brawls in worlds you can blow apart.</p>
      <label class="field">
        <span>Pilot name</span>
        <input name="name" type="text" autocomplete="nickname" spellcheck="false" maxlength="${MAX_NAME_BYTES}" />
      </label>
      <p class="field-error"></p>
      <label class="field">
        <span>Share code</span>
        <input name="code" type="text" autocomplete="off" spellcheck="false"
               maxlength="${SHARE_CODE_PRINTED_LEN + 6}" placeholder="LSxx-xxxx-xxxx-… (empty = server's / random world)" />
      </label>
      <p class="field-error code-error" role="alert"></p>
      <label class="check">
        <input name="assist" type="checkbox" />
        <span>Horizon assist (slowly levels your roll)</span>
      </label>
      <div class="play-row">
        <button type="submit" class="play" value="online">Play online</button>
        <button type="submit" class="play secondary" value="practice">Practice offline</button>
      </div>
      <p class="controls-hint">
        Online plays the world the server runs; a share code you enter is used for practice.
        Mouse look · W/S thrust · A/D strafe · Space/Ctrl (or C) up/down · Q/E roll ·
        Shift boost · mouse buttons fire · 1-4 weapons · F overdrive · R scavenge · H horizon assist ·
        Tab scoreboard · F3 debug overlay
      </p>
    </form>`;
  parent.appendChild(root);

  const form = root.querySelector("form") as HTMLFormElement;
  const nameInput = form.elements.namedItem("name") as HTMLInputElement;
  const codeInput = form.elements.namedItem("code") as HTMLInputElement;
  const assistInput = form.elements.namedItem("assist") as HTMLInputElement;
  const error = root.querySelector(".code-error") as HTMLElement;
  nameInput.value = stored.name ?? DEFAULT_NAME;
  codeInput.value = stored.shareCode ?? "";
  assistInput.checked = stored.horizonAssist ?? true;
  codeInput.focus();

  codeInput.addEventListener("input", () => {
    error.textContent = "";
    codeInput.classList.remove("invalid");
  });

  return new Promise((resolve) => {
    form.addEventListener("submit", (e) => {
      e.preventDefault();
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
      };
      saveStored({ shareCode: check.code ?? "", horizonAssist: options.horizonAssist, name: options.name });
      root.remove();
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
