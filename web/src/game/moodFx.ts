// Mood presentation (M4 B5; docs/milestone-4-plan.md §3.1 "Moods"): the
// level has one of four moods (calm, bored, grumpy, giddy) that drifts
// every 60-90 s (the comedy director, replicated in the mood view). The mood
// has no gameplay effect; it shows as
// - a slow grade tint over the world's grade (eased over MOOD_EASE_SECS:
//   calm cool and clear, bored washed out and flat, grumpy dark, warm and
//   hard, giddy warm pink and saturated),
// - the dust motes' colour (calm pale blue, bored grey and dim, grumpy
//   ember, giddy pastel rainbow),
// - the HUD mood icon (ui/worldHud.ts), with the mood's name for a moment
//   after a change,
// - an ambient drone per mood through the audio engine (A6), crossfaded.

import type { ColorGrade } from "../renderer/post.ts";
import type { Rgb } from "../renderer/renderer.ts";
import { MOOD_IDS, type MoodId } from "./modeState.ts";

/** Seconds a mood change takes to settle (grade, motes, drone). */
export const MOOD_EASE_SECS = 6;

export interface MoodLook {
  label: string;
  /** One line under the icon on a change. */
  line: string;
  /** Grade: slope multiplier (tint and brightness), offset added, saturation and contrast factor. */
  slope: Rgb;
  offset: Rgb;
  saturation: number;
  /** Power (gamma) multiplier: above 1 deepens the darks. */
  power: number;
  /** Dust mote tint (linear, per unit brightness), brightness factor and rainbow share. */
  mote: Rgb;
  moteBrightness: number;
  rainbow: number;
  /** Ambient drone (audio/sounds/world.ts) and its gain. */
  drone: string;
  droneGain: number;
  /** HUD accent (CSS colour). */
  accent: string;
}

export const MOOD_LOOKS: Readonly<Record<MoodId, MoodLook>> = {
  calm: {
    label: "Calm",
    line: "The level is calm",
    slope: [0.97, 1.0, 1.04],
    offset: [0, 0.002, 0.004],
    saturation: 1.0,
    power: 1.0,
    mote: [0.72, 0.82, 0.95],
    moteBrightness: 1,
    rainbow: 0,
    drone: "mood.calm",
    droneGain: 0.55,
    accent: "#8fd3ff",
  },
  bored: {
    label: "Bored",
    line: "The level is bored",
    slope: [0.98, 0.97, 0.94],
    offset: [0.012, 0.012, 0.012],
    saturation: 0.72,
    power: 0.92,
    mote: [0.62, 0.62, 0.6],
    moteBrightness: 0.6,
    rainbow: 0,
    drone: "mood.bored",
    droneGain: 0.5,
    accent: "#b9b4a6",
  },
  grumpy: {
    label: "Grumpy",
    line: "The level is grumpy",
    slope: [1.05, 0.9, 0.82],
    offset: [0.004, 0, 0],
    saturation: 1.05,
    power: 1.12,
    mote: [1.0, 0.5, 0.22],
    moteBrightness: 1.15,
    rainbow: 0,
    drone: "mood.grumpy",
    droneGain: 0.6,
    accent: "#ff7a45",
  },
  giddy: {
    label: "Giddy",
    line: "The level is giddy",
    slope: [1.06, 0.98, 1.05],
    offset: [0.006, 0.0, 0.005],
    saturation: 1.18,
    power: 0.96,
    mote: [1.0, 0.8, 0.95],
    moteBrightness: 1.35,
    rainbow: 0.9,
    drone: "mood.giddy",
    droneGain: 0.5,
    accent: "#ff8ad8",
  },
};

/** Mood weights (sum 1), eased toward the current mood. */
export type MoodWeights = Record<MoodId, number>;

export function moodWeights(mood: MoodId): MoodWeights {
  return { calm: mood === "calm" ? 1 : 0, bored: mood === "bored" ? 1 : 0, grumpy: mood === "grumpy" ? 1 : 0, giddy: mood === "giddy" ? 1 : 0 };
}

/** Moves `w` toward `mood` over MOOD_EASE_SECS (exponential, renormalised). */
export function easeMood(w: MoodWeights, mood: MoodId, dt: number): MoodWeights {
  const k = 1 - Math.exp(-Math.max(0, dt) * (3 / MOOD_EASE_SECS));
  const out = { ...w };
  let sum = 0;
  for (const id of MOOD_IDS) {
    out[id] += ((id === mood ? 1 : 0) - out[id]) * k;
    sum += out[id];
  }
  for (const id of MOOD_IDS) out[id] /= sum || 1;
  return out;
}

function blend3(w: MoodWeights, pick: (l: MoodLook) => Rgb): [number, number, number] {
  const o: [number, number, number] = [0, 0, 0];
  for (const id of MOOD_IDS) {
    const c = pick(MOOD_LOOKS[id]);
    o[0] += c[0] * w[id];
    o[1] += c[1] * w[id];
    o[2] += c[2] * w[id];
  }
  return o;
}

function blend1(w: MoodWeights, pick: (l: MoodLook) => number): number {
  let o = 0;
  for (const id of MOOD_IDS) o += pick(MOOD_LOOKS[id]) * w[id];
  return o;
}

/** The world's grade with the mood's tint on top (`strength` 0 = untouched). */
export function moodGrade(base: ColorGrade, w: MoodWeights, strength = 1): ColorGrade {
  const s = blend3(w, (l) => l.slope);
  const o = blend3(w, (l) => l.offset);
  const sat = blend1(w, (l) => l.saturation);
  const pow = blend1(w, (l) => l.power);
  const m = (a: number, b: number): number => a + (b - a) * strength;
  return {
    ...base,
    slope: [base.slope[0] * m(1, s[0]), base.slope[1] * m(1, s[1]), base.slope[2] * m(1, s[2])],
    offset: [base.offset[0] + o[0] * strength, base.offset[1] + o[1] * strength, base.offset[2] + o[2] * strength],
    power: [base.power[0] * m(1, pow), base.power[1] * m(1, pow), base.power[2] * m(1, pow)],
    saturation: base.saturation * m(1, sat),
  };
}

/** Dust mote colour of the mood mix: linear tint (brightness folded in) and rainbow share. */
export function moodMotes(w: MoodWeights): { color: [number, number, number]; rainbow: number } {
  const c = blend3(w, (l) => l.mote);
  const b = blend1(w, (l) => l.moteBrightness);
  return { color: [c[0] * b, c[1] * b, c[2] * b], rainbow: blend1(w, (l) => l.rainbow) };
}

/** What the HUD shows: the mood, and its line for a few seconds after a change. */
export interface MoodIconModel {
  mood: MoodId;
  label: string;
  accent: string;
  /** Show the line (the mood just changed). */
  announce: string | null;
}

/** Seconds the mood line stays after a change. */
export const MOOD_LINE_SECS = 4.5;

/** The audio engine's looping part (audio/engine.ts). */
export interface DroneEngine {
  loop(id: string):{ readonly playing: boolean; setGain(g: number, ramp?: number): void; stop(fade?: number): void } | null;
}

/** Per-match mood presentation state. */
export class MoodFx {
  weights: MoodWeights = moodWeights("calm");
  private mood: MoodId | null = null;
  private changedAt = -1e9;
  private drones = new Map<string, { readonly playing: boolean; setGain(g: number, ramp?: number): void; stop(fade?: number): void }>();
  /** The audio engine for the drones (null: silent, e.g. the gallery). */
  engine: DroneEngine | null = null;

  reset(): void {
    this.weights = moodWeights("calm");
    this.mood = null;
    this.changedAt = -1e9;
    this.stopDrones();
  }

  /** Steps the mix toward `mood`; `quiet` (results screen, lobby) fades the drone out. */
  update(mood: MoodId, time: number, dt: number, quiet = false): void {
    if (this.mood === null) {
      // The first mood of a match shows at once (no drift from calm).
      this.weights = moodWeights(mood);
      this.mood = mood;
      this.changedAt = time;
    } else if (mood !== this.mood) {
      this.mood = mood;
      this.changedAt = time;
    }
    this.weights = easeMood(this.weights, mood, dt);
    this.updateDrones(quiet);
  }

  /** Jumps to `mood` fully settled (the gallery). */
  settle(mood: MoodId, time = 0): void {
    this.mood = mood;
    this.weights = moodWeights(mood);
    this.changedAt = time;
  }

  icon(time: number): MoodIconModel | null {
    if (!this.mood) return null;
    const l = MOOD_LOOKS[this.mood];
    return { mood: this.mood, label: l.label, accent: l.accent, announce: time - this.changedAt < MOOD_LINE_SECS ? l.line : null };
  }

  private updateDrones(quiet: boolean): void {
    const e = this.engine;
    if (!e) return;
    for (const id of MOOD_IDS) {
      const look = MOOD_LOOKS[id];
      const g = quiet ? 0 : this.weights[id] * look.droneGain;
      let h = this.drones.get(look.drone);
      if (g < 0.01) {
        if (h) {
          h.stop(1.5);
          this.drones.delete(look.drone);
        }
        continue;
      }
      if (!h || !h.playing) {
        // Started at its own gain (the handle scales that base), then eased to the mix.
        h = e.loop(look.drone) ?? undefined;
        if (!h) continue;
        this.drones.set(look.drone, h);
      }
      h.setGain(g, 0.5);
    }
  }

  stopDrones(): void {
    for (const h of this.drones.values()) h.stop(1);
    this.drones.clear();
  }
}
