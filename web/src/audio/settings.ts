// Audio settings (M4 A6): master, effects, music and voice volumes (0-100)
// and mute, stored per browser in localStorage ("ls.audio"; every access in
// try/catch like ui/viewSettings.ts). The start screen's Options show the
// sliders (`audioSettingsHtml`, `bindAudioSettings`); a change applies at
// once to the running engine (`onAudioSettings`) and plays a short preview.

import type { Bus } from "./library.ts";

export interface AudioSettings {
  master: number;
  effects: number;
  music: number;
  voice: number;
  muted: boolean;
}

export const AUDIO_STORAGE_KEY = "ls.audio";

export const DEFAULT_AUDIO: Readonly<AudioSettings> = Object.freeze({ master: 80, effects: 80, music: 60, voice: 85, muted: false });

/** Minimal storage interface (localStorage, or a stub in tests). */
export interface KeyValueStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

const pageCopy = new Map<string, string>();
const pageStore: KeyValueStore = {
  getItem: (key) => pageCopy.get(key) ?? null,
  setItem: (key, value) => void pageCopy.set(key, value),
};

function defaultStore(): KeyValueStore {
  try {
    if (typeof localStorage !== "undefined" && localStorage) return localStorage;
  } catch {
    /* blocked site data */
  }
  return pageStore;
}

const vol = (v: unknown, d: number): number => (typeof v === "number" && Number.isFinite(v) ? Math.round(Math.min(100, Math.max(0, v))) : d);

/** Settings from any parsed value: unknown or broken fields take their defaults. */
export function readAudioSettings(v: unknown): AudioSettings {
  const o = typeof v === "object" && v !== null ? (v as Record<string, unknown>) : {};
  return {
    master: vol(o["master"], DEFAULT_AUDIO.master),
    effects: vol(o["effects"], DEFAULT_AUDIO.effects),
    music: vol(o["music"], DEFAULT_AUDIO.music),
    voice: vol(o["voice"], DEFAULT_AUDIO.voice),
    muted: typeof o["muted"] === "boolean" ? o["muted"] : DEFAULT_AUDIO.muted,
  };
}

export function loadAudioSettings(store: KeyValueStore | null = defaultStore()): AudioSettings {
  try {
    const raw = store?.getItem(AUDIO_STORAGE_KEY);
    return readAudioSettings(raw ? JSON.parse(raw) : null);
  } catch {
    return { ...DEFAULT_AUDIO };
  }
}

const listeners = new Set<(s: AudioSettings) => void>();

/** Calls `fn` with every saved change; returns the unsubscribe. */
export function onAudioSettings(fn: (s: AudioSettings) => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function saveAudioSettings(value: AudioSettings, store: KeyValueStore | null = defaultStore()): void {
  const s = readAudioSettings(value);
  try {
    store?.setItem(AUDIO_STORAGE_KEY, JSON.stringify(s));
  } catch {
    /* storage unavailable: the settings last for this page only */
  }
  for (const fn of listeners) fn(s);
}

/** Linear gain of a bus (or the master) from the 0-100 slider: a squared (perceptual) curve. */
export function busGain(s: AudioSettings, bus: Bus | "master"): number {
  if (bus === "master") return s.muted ? 0 : (s.master / 100) ** 2;
  return (s[bus] / 100) ** 2;
}

const SLIDERS: readonly { key: "master" | Bus; label: string }[] = [
  { key: "master", label: "Master volume" },
  { key: "effects", label: "Effects" },
  { key: "music", label: "Music" },
  { key: "voice", label: "Announcer" },
];

/** The volume controls (inside a form, like the view settings). */
export function audioSettingsHtml(): string {
  const rows = SLIDERS.map(
    (s) => `
        <label class="field range">
          <span>${s.label} <output name="audio-${s.key}-value"></output></span>
          <input name="audio-${s.key}" type="range" min="0" max="100" step="1" />
        </label>`,
  ).join("");
  return `
      <fieldset class="view-settings audio-settings">
        <legend>Sound</legend>${rows}
        <label class="check">
          <input name="audio-muted" type="checkbox" />
          <span>Mute all sound</span>
        </label>
      </fieldset>`;
}

/**
 * Fills the volume controls of `form` from storage and saves every change;
 * `preview(bus)` plays a short sample after a slider moves (the engine
 * starts on that gesture).
 */
export function bindAudioSettings(form: HTMLFormElement, preview?: (bus: Bus | "master") => void, store: KeyValueStore | null = defaultStore()): void {
  const current = loadAudioSettings(store);
  const inputs = new Map<"master" | Bus, HTMLInputElement>();
  for (const s of SLIDERS) {
    const input = form.elements.namedItem(`audio-${s.key}`) as HTMLInputElement | null;
    if (!input) return;
    inputs.set(s.key, input);
    input.value = String(current[s.key]);
  }
  const muted = form.elements.namedItem("audio-muted") as HTMLInputElement | null;
  if (!muted) return;
  muted.checked = current.muted;
  const show = (s: AudioSettings): void => {
    for (const k of inputs.keys()) {
      const out = form.elements.namedItem(`audio-${k}-value`) as HTMLOutputElement | null;
      if (out) out.textContent = s.muted && k === "master" ? "muted" : `${s[k]}`;
    }
  };
  const read = (): AudioSettings =>
    readAudioSettings({
      master: Number(inputs.get("master")?.value),
      effects: Number(inputs.get("effects")?.value),
      music: Number(inputs.get("music")?.value),
      voice: Number(inputs.get("voice")?.value),
      muted: muted.checked,
    });
  show(current);
  for (const [key, input] of inputs) {
    input.addEventListener("input", () => {
      const s = read();
      show(s);
      saveAudioSettings(s, store);
    });
    // Preview once the slider is let go (not on every step of a drag).
    input.addEventListener("change", () => preview?.(key));
  }
  muted.addEventListener("change", () => {
    const s = read();
    show(s);
    saveAudioSettings(s, store);
  });
}
