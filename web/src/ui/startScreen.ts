// Start screen: share-code entry, horizon-assist preference and a Play button.

import { checkShareCode, SHARE_CODE_PRINTED_LEN } from "./shareCode.ts";

export interface StartOptions {
  /** Normalised share code, or null to accept the server's world. */
  shareCode: string | null;
  horizonAssist: boolean;
}

const STORAGE_KEY = "ls.start";

interface Stored {
  shareCode?: string;
  horizonAssist?: boolean;
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

/**
 * Shows the start screen inside `parent` and resolves when the player presses
 * Play. The screen removes itself afterwards.
 */
export function showStartScreen(parent: HTMLElement): Promise<StartOptions> {
  const stored = loadStored();
  const root = document.createElement("div");
  root.className = "screen start-screen";
  root.innerHTML = `
    <form class="start-card" novalidate>
      <h1>Loose Screws</h1>
      <p class="tagline">6DOF scrap-ship brawls in worlds you can blow apart.</p>
      <label class="field">
        <span>Share code</span>
        <input name="code" type="text" autocomplete="off" spellcheck="false"
               maxlength="${SHARE_CODE_PRINTED_LEN + 6}" placeholder="LSxx-xxxx-xxxx-… (empty = server's world)" />
      </label>
      <p class="field-error" role="alert"></p>
      <label class="check">
        <input name="assist" type="checkbox" />
        <span>Horizon assist (slowly levels your roll)</span>
      </label>
      <button type="submit" class="play">Play</button>
      <p class="controls-hint">
        Mouse look · W/S thrust · A/D strafe · Space/Ctrl (or C) up/down · Q/E roll ·
        Shift boost · mouse buttons fire · R scavenge · H horizon assist
      </p>
    </form>`;
  parent.appendChild(root);

  const form = root.querySelector("form") as HTMLFormElement;
  const codeInput = form.elements.namedItem("code") as HTMLInputElement;
  const assistInput = form.elements.namedItem("assist") as HTMLInputElement;
  const error = root.querySelector(".field-error") as HTMLElement;
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
      let shareCode: string | null = null;
      if (codeInput.value.trim() !== "") {
        const check = checkShareCode(codeInput.value);
        if (!check.ok) {
          error.textContent = check.error;
          codeInput.classList.add("invalid");
          codeInput.focus();
          return;
        }
        shareCode = check.code;
      }
      const options: StartOptions = { shareCode, horizonAssist: assistInput.checked };
      saveStored({ shareCode: shareCode ?? "", horizonAssist: options.horizonAssist });
      root.remove();
      resolve(options);
    });
  });
}
