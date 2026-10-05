// View settings (visual-quality-plan.md VQ1): horizontal field of view, the
// cockpit hull frame and the fit ring. Stored per browser in localStorage
// (every access in try/catch: private windows and blocked storage just
// forget them). The start screen shows the controls (`viewSettingsHtml`,
// `bindViewSettings`); the game reads them with `loadViewSettings`.

import { clampFov, DEFAULT_FOV_DEG, FOV_MAX_DEG, FOV_MIN_DEG } from "../game/camera.ts";

export interface ViewSettings {
  /** Horizontal field of view in degrees at 16:9 (FOV_MIN_DEG..FOV_MAX_DEG). */
  fov: number;
  /** Draw the own ship's canopy frame and nose (size cue). */
  hullFrame: boolean;
  /** Show the fit ring around the crosshair. */
  fitRing: boolean;
}

export const VIEW_STORAGE_KEY = "ls.view";

export const DEFAULT_VIEW: Readonly<ViewSettings> = Object.freeze({ fov: DEFAULT_FOV_DEG, hullFrame: true, fitRing: true });

/** Minimal storage interface (localStorage, or a stub in tests). */
export interface KeyValueStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** Where the settings live without localStorage (blocked storage): this page only. */
const pageCopy = new Map<string, string>();
const pageStore: KeyValueStore = {
  getItem: (key) => pageCopy.get(key) ?? null,
  setItem: (key, value) => void pageCopy.set(key, value),
};

/**
 * localStorage, or the page's own copy when storage is blocked (private
 * windows, disabled site data): the start screen's choices then still reach
 * the game on this page instead of falling back to the defaults.
 */
function defaultStore(): KeyValueStore {
  try {
    if (typeof localStorage !== "undefined" && localStorage) return localStorage;
  } catch {
    /* access throws when site data is blocked */
  }
  return pageStore;
}

/** Settings from any parsed value: unknown or broken fields take their defaults. */
export function readViewSettings(v: unknown): ViewSettings {
  const o = typeof v === "object" && v !== null ? (v as Record<string, unknown>) : {};
  return {
    fov: o["fov"] === undefined ? DEFAULT_VIEW.fov : clampFov(o["fov"]),
    hullFrame: typeof o["hullFrame"] === "boolean" ? o["hullFrame"] : DEFAULT_VIEW.hullFrame,
    fitRing: typeof o["fitRing"] === "boolean" ? o["fitRing"] : DEFAULT_VIEW.fitRing,
  };
}

export function loadViewSettings(store: KeyValueStore | null = defaultStore()): ViewSettings {
  try {
    const raw = store?.getItem(VIEW_STORAGE_KEY);
    return readViewSettings(raw ? JSON.parse(raw) : null);
  } catch {
    return { ...DEFAULT_VIEW };
  }
}

export function saveViewSettings(value: ViewSettings, store: KeyValueStore | null = defaultStore()): void {
  try {
    store?.setItem(VIEW_STORAGE_KEY, JSON.stringify(readViewSettings(value)));
  } catch {
    /* storage unavailable: the settings last for this page only */
  }
}

/** The start screen's view controls (inside its form). */
export function viewSettingsHtml(): string {
  return `
      <fieldset class="view-settings">
        <legend>View</legend>
        <label class="field range">
          <span>Field of view <output name="fovValue"></output></span>
          <input name="fov" type="range" min="${FOV_MIN_DEG}" max="${FOV_MAX_DEG}" step="1" />
        </label>
        <label class="check">
          <input name="hullFrame" type="checkbox" />
          <span>Cockpit frame (shows your ship's size)</span>
        </label>
        <label class="check">
          <input name="fitRing" type="checkbox" />
          <span>Fit ring (your hull's size at the rock ahead)</span>
        </label>
      </fieldset>`;
}

/** Fills the view controls of `form` from storage and saves every change. */
export function bindViewSettings(form: HTMLFormElement, store: KeyValueStore | null = defaultStore()): void {
  const fov = form.elements.namedItem("fov") as HTMLInputElement | null;
  const out = form.elements.namedItem("fovValue") as HTMLOutputElement | null;
  const frame = form.elements.namedItem("hullFrame") as HTMLInputElement | null;
  const ring = form.elements.namedItem("fitRing") as HTMLInputElement | null;
  if (!fov || !frame || !ring) return;
  const current = loadViewSettings(store);
  fov.value = String(current.fov);
  frame.checked = current.hullFrame;
  ring.checked = current.fitRing;
  const sync = (): void => {
    const next = readViewSettings({ fov: Number(fov.value), hullFrame: frame.checked, fitRing: ring.checked });
    if (out) out.textContent = `${next.fov}°`;
    saveViewSettings(next, store);
  };
  if (out) out.textContent = `${current.fov}°`;
  for (const input of [fov, frame, ring]) input.addEventListener("input", sync);
  frame.addEventListener("change", sync);
  ring.addEventListener("change", sync);
}
