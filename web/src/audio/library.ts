// Sound library (M4 A6): every sound the client can play is a `SoundDef`, a
// synthesis parameter set (dsp.ts) plus how it is mixed (bus, gain,
// priority, distance, reverb send, instance limits). Definitions live in
// sounds/*.ts, one file per group; later packages (event visuals, race
// sounds, mood drones) add their own files there and list them in
// sounds/index.ts, or call `registerSounds` at runtime. Every registered
// sound shows in `?audio=demo` (demo.ts).

import type { SynthDef } from "./dsp.ts";

/** Mixer buses (settings.ts has a volume per bus; `ui` sounds go to effects without position). */
export type Bus = "effects" | "music" | "voice";

export interface SoundDef {
  /** Unique id, `group.name` (e.g. `fire.bolt`). */
  id: string;
  /** Short name in the demo page. */
  label: string;
  /** Demo page group heading. */
  group: string;
  /** One line: what plays it. */
  description: string;
  bus: Bus;
  synth: SynthDef;
  /** Rendered variants (different noise and jitter), picked at random per play [3; loops 1]. */
  variants?: number;
  /** Mix gain (linear) [1]. */
  gain?: number;
  /** Voice-stealing priority 0..10 (higher survives) [5]. */
  priority?: number;
  /** At most this many instances at once (the oldest is replaced) [8]. */
  maxInstances?: number;
  /** Minimum time between two starts (s); a closer start is skipped [0.02]. */
  minGap?: number;
  /** Distance (u) at which a positional sound plays at full gain [8]. */
  refDistance?: number;
  /** Reverb send 0..1 [0.25]. */
  space?: number;
  /** Random playback-rate spread per play (fraction) [0.03]. */
  pitchJitter?: number;
  /** Loops until stopped (a `LoopHandle`). */
  loop?: boolean;
}

const registry = new Map<string, SoundDef>();

/** Adds (or replaces) sound definitions. */
export function registerSounds(defs: readonly SoundDef[]): void {
  for (const d of defs) registry.set(d.id, d);
}

export function soundDef(id: string): SoundDef | undefined {
  return registry.get(id);
}

/** Every registered sound in registration order. */
export function allSounds(): SoundDef[] {
  return [...registry.values()];
}

/** Defaults applied to a definition's optional mix fields. */
export function mixOf(d: SoundDef): Required<Pick<SoundDef, "variants" | "gain" | "priority" | "maxInstances" | "minGap" | "refDistance" | "space" | "pitchJitter">> {
  return {
    variants: d.variants ?? (d.loop ? 1 : 3),
    gain: d.gain ?? 1,
    priority: d.priority ?? 5,
    maxInstances: d.maxInstances ?? 8,
    minGap: d.minGap ?? 0.02,
    refDistance: d.refDistance ?? 8,
    space: d.space ?? 0.25,
    pitchJitter: d.pitchJitter ?? 0.03,
  };
}
