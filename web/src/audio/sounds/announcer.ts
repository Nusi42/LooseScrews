// The announcer's stings (ui/announcer.ts plays one before a line): a
// fanfare for event announcements, a gong for first blood, a chiptune run for
// multi-kills, and the caption's blip.

import type { Layer } from "../dsp.ts";
import type { SoundDef } from "../library.ts";

const A = "Announcer";

/** A brass-like note: two detuned saws through a low-pass that opens with the attack. */
function brass(at: number, freq: number, dur: number, gain = 0.5): Layer[] {
  const env = { a: 0.025, d: 0.35, s: 0.65, r: 0.07 };
  return [
    { wave: "saw", at, dur, freq, vibrato: { rate: 5.5, depth: 0.08 }, filter: { type: "lp", freq: [700, 3600], time: 0.06, q: 1.4 }, env, gain },
    { wave: "saw", at, dur, freq: freq * 1.006, filter: { type: "lp", freq: [600, 3000], time: 0.07, q: 1.2 }, env, gain: gain * 0.8 },
    { wave: "square", at, dur, freq: freq / 2, duty: 0.4, filter: { type: "lp", freq: 900 }, env, gain: gain * 0.25 },
  ];
}

export const ANNOUNCER_SOUNDS: readonly SoundDef[] = [
  {
    id: "sting.event",
    label: "Event fanfare",
    group: A,
    description: "Before an event is announced: a cheeky \"ta-da-DAAA\"",
    bus: "voice",
    gain: 0.55,
    priority: 10,
    maxInstances: 1,
    space: 0.3,
    pitchJitter: 0,
    variants: 1,
    synth: {
      layers: [
        ...brass(0, 392, 0.12),
        ...brass(0.15, 392, 0.12),
        ...brass(0.3, 523.25, 0.62, 0.6),
        ...brass(0.3, 659.25, 0.62, 0.35),
        { wave: "noise", at: 0.3, dur: 0.25, filter: { type: "bp", freq: 2500, q: 0.8 }, env: { a: 0.001, d: 0.07 }, gain: 0.45 },
        { wave: "sine", at: 0.3, dur: 0.3, freq: [140, 60], env: { a: 0.001, d: 0.08 }, gain: 0.6 },
      ],
    },
  },
  {
    id: "sting.firstBlood",
    label: "First blood",
    group: A,
    description: "The match's first kill: a low brass stab over a gong",
    bus: "voice",
    gain: 0.55,
    priority: 10,
    maxInstances: 1,
    space: 0.45,
    pitchJitter: 0,
    variants: 1,
    synth: {
      layers: [
        ...brass(0, 196, 0.5, 0.55),
        ...brass(0, 233.08, 0.5, 0.4),
        { wave: "modal", dur: 2.2, freq: 98, modal: { ratios: [1, 1.52, 2.03, 2.71, 3.39, 4.2], decays: [2.2, 1.7, 1.3, 1.0, 0.7, 0.5] }, gain: 0.6 },
        { wave: "noise", dur: 0.3, filter: { type: "lp", freq: [4000, 400] }, env: { a: 0.001, d: 0.08 }, gain: 0.3 },
      ],
    },
  },
  {
    id: "sting.multiKill",
    label: "Multi-kill",
    group: A,
    description: "Two or more kills in a row: a quick chiptune run up",
    bus: "voice",
    gain: 0.4,
    priority: 10,
    maxInstances: 1,
    space: 0.2,
    pitchJitter: 0,
    variants: 1,
    synth: {
      layers: [
        { wave: "square", duty: 0.25, dur: 0.07, freq: 523.25, filter: { type: "lp", freq: 6000 }, env: { a: 0.002, h: 0.04, r: 0.02 }, repeat: { count: 4, every: 0.06, pitchStep: 4 }, gain: 0.5 },
        { wave: "square", duty: 0.5, at: 0.24, dur: 0.35, freq: 1046.5, vibrato: { rate: 9, depth: 0.25 }, filter: { type: "lp", freq: 5000 }, env: { a: 0.002, d: 0.2, s: 0.3, r: 0.05 }, gain: 0.45 },
        { wave: "triangle", dur: 0.6, freq: 130.81, env: { a: 0.002, d: 0.25 }, gain: 0.5 },
      ],
    },
  },
  {
    id: "caption.blip",
    label: "Caption blip",
    group: A,
    description: "An announcer caption slides in",
    bus: "voice",
    gain: 0.3,
    priority: 9,
    maxInstances: 2,
    space: 0.1,
    pitchJitter: 0,
    variants: 1,
    synth: {
      layers: [
        { wave: "triangle", dur: 0.05, freq: 1760, env: { a: 0.001, d: 0.02 }, gain: 0.5 },
        { wave: "sine", at: 0.04, dur: 0.09, freq: 2637, env: { a: 0.001, d: 0.035 }, gain: 0.35 },
      ],
    },
  },
];
