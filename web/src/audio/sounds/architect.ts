// The Architect's sounds (M4 C4, game/architect/): the architect's pencil
// (placing a stroke, a refused one, picking from the stroke wheel), every
// stroke's telegraph (a rising "here it comes" whistle heard by everyone
// near it), the strokes acting (rock growing, the well thumping in, the ink
// splat, the event's party pop), a runner through the exit, and the round's
// end.

import type { SoundDef } from "../library.ts";

const A = "The Architect";

export const ARCHITECT_SOUNDS: readonly SoundDef[] = [
  {
    id: "architect.place",
    label: "Stroke placed",
    group: A,
    description: "The architect commits a stroke: a confident pencil scribble and a drafting-table thunk",
    bus: "effects",
    gain: 0.7,
    priority: 6,
    synth: {
      layers: [
        { wave: "noise", dur: 0.05, filter: { type: "bp", freq: [3200, 5200], q: 3 }, env: { a: 0.004, d: 0.02 }, repeat: { count: 4, every: 0.055, jitter: 0.015, fade: 0.9 }, gain: 0.55 },
        { wave: "modal", dur: 0.4, at: 0.2, freq: 180, modal: { ratios: [1, 2.3, 3.9], decays: [0.18, 0.08, 0.05], gains: [1, 0.4, 0.2] }, gain: 0.9 },
        { wave: "sine", dur: 0.18, at: 0.2, freq: [880, 1320], env: { a: 0.004, d: 0.06 }, gain: 0.25 },
      ],
    },
  },
  {
    id: "architect.denied",
    label: "Stroke refused",
    group: A,
    description: "The stroke cannot go there: a polite two-note 'nope' from the drafting table",
    bus: "effects",
    gain: 0.6,
    priority: 6,
    minGap: 0.2,
    synth: {
      layers: [
        { wave: "square", dur: 0.12, freq: 330, duty: 0.35, filter: { type: "lp", freq: 1800 }, env: { a: 0.004, d: 0.06 }, gain: 0.5 },
        { wave: "square", dur: 0.2, at: 0.13, freq: 247, duty: 0.35, filter: { type: "lp", freq: 1400 }, env: { a: 0.004, d: 0.09 }, gain: 0.55 },
      ],
    },
  },
  {
    id: "architect.pick",
    label: "Stroke picked",
    group: A,
    description: "A stroke picked from the wheel or the number keys: a soft wooden click",
    bus: "effects",
    gain: 0.45,
    priority: 4,
    minGap: 0.04,
    synth: {
      layers: [
        { wave: "modal", dur: 0.12, freq: 1100, modal: { ratios: [1, 2.7], decays: [0.04, 0.02] }, gain: 1 },
        { wave: "noise", dur: 0.02, filter: { type: "hp", freq: 4000 }, env: { a: 0.001, d: 0.006 }, gain: 0.3 },
      ],
    },
  },
  {
    id: "architect.telegraph",
    label: "Stroke incoming (telegraph)",
    group: A,
    description: "A stroke glows somewhere: a rising slide-whistle over a ticking clock for the 2 s before it acts",
    bus: "effects",
    gain: 0.85,
    priority: 7,
    maxInstances: 4,
    refDistance: 22,
    space: 0.45,
    synth: {
      layers: [
        { wave: "sine", dur: 2.0, freq: [320, 1250], sweepCurve: 1.6, vibrato: { rate: 6, depth: 0.35 }, env: { a: 0.05, s: 1, r: 0.08 }, gain: 0.45 },
        { wave: "triangle", dur: 2.0, freq: [160, 625], sweepCurve: 1.6, env: { a: 0.05, s: 1, r: 0.08 }, gain: 0.25 },
        { wave: "modal", dur: 0.08, freq: 2400, modal: { ratios: [1, 1.5], decays: [0.03, 0.02] }, repeat: { count: 8, every: 0.25, fade: 1.08 }, gain: 0.35 },
      ],
    },
  },
  {
    id: "architect.rock",
    label: "Stroke: rock moves",
    group: A,
    description: "Enclosure, Density or Instability acts: a deep stone groan and a crunchy thump as the rock regrows",
    bus: "effects",
    gain: 1,
    priority: 7,
    maxInstances: 4,
    refDistance: 24,
    space: 0.55,
    synth: {
      layers: [
        { wave: "brown", dur: 0.9, filter: { type: "lp", freq: [400, 90] }, env: { a: 0.01, d: 0.35 }, drive: 1.5, gain: 1 },
        { wave: "saw", dur: 0.7, freq: [70, 38], filter: { type: "lp", freq: 300 }, env: { a: 0.02, d: 0.25 }, drive: 2, gain: 0.6 },
        { wave: "crackle", density: 60, dur: 0.6, filter: { type: "bp", freq: 1400, q: 0.8 }, env: { a: 0.01, d: 0.2 }, gain: 0.5 },
      ],
    },
  },
  {
    id: "architect.well",
    label: "Stroke: gravity well",
    group: A,
    description: "Gravity+ acts: a sucking whoomp that drops in pitch, like the level inhaling",
    bus: "effects",
    gain: 0.95,
    priority: 7,
    refDistance: 22,
    space: 0.5,
    synth: {
      layers: [
        { wave: "sine", dur: 0.8, freq: [220, 38], sweepCurve: 0.7, env: { a: 0.01, d: 0.3 }, gain: 1 },
        { wave: "pink", dur: 0.7, filter: { type: "bp", freq: [2600, 180], q: 3 }, env: { a: 0.25, d: 0.15 }, gain: 0.5 },
      ],
    },
  },
  {
    id: "architect.ink",
    label: "Stroke: ink blackout",
    group: A,
    description: "Palette acts: a wet squid-ink splat and a gloopy bubble",
    bus: "effects",
    gain: 0.9,
    priority: 7,
    refDistance: 20,
    space: 0.4,
    synth: {
      layers: [
        { wave: "noise", dur: 0.35, filter: { type: "lp", freq: [3000, 400] }, env: { a: 0.002, d: 0.08 }, gain: 0.8 },
        { wave: "sine", dur: 0.25, at: 0.05, freq: [180, 520], env: { a: 0.01, d: 0.08 }, gain: 0.6 },
        { wave: "sine", dur: 0.12, at: 0.32, freq: [300, 700], env: { a: 0.005, d: 0.04 }, repeat: { count: 3, every: 0.12, fade: 0.7, pitchStep: 2 }, gain: 0.4 },
      ],
    },
  },
  {
    id: "architect.event",
    label: "Stroke: comedy event",
    group: A,
    description: "Absurdity+ or Verticality acts: a party popper and a kazoo flourish",
    bus: "effects",
    gain: 0.9,
    priority: 7,
    space: 0.4,
    synth: {
      layers: [
        { wave: "noise", dur: 0.12, filter: { type: "hp", freq: 1500 }, env: { a: 0.001, d: 0.03 }, gain: 0.9 },
        { wave: "saw", dur: 0.16, at: 0.1, freq: 392, vibrato: { rate: 11, depth: 0.4 }, filter: { type: "bp", freq: 1200, q: 1.6 }, env: { a: 0.01, s: 1, r: 0.03 }, gain: 0.45 },
        { wave: "saw", dur: 0.16, at: 0.26, freq: 494, vibrato: { rate: 11, depth: 0.4 }, filter: { type: "bp", freq: 1300, q: 1.6 }, env: { a: 0.01, s: 1, r: 0.03 }, gain: 0.45 },
        { wave: "saw", dur: 0.42, at: 0.42, freq: [587, 620], vibrato: { rate: 9, depth: 0.6 }, filter: { type: "bp", freq: 1400, q: 1.6 }, env: { a: 0.01, d: 0.3 }, gain: 0.5 },
      ],
    },
  },
  {
    id: "architect.escape",
    label: "Runner escaped",
    group: A,
    description: "A runner gets through the exit: a bright 'ding-dong' like a shop door",
    bus: "effects",
    gain: 0.8,
    priority: 8,
    space: 0.5,
    synth: {
      layers: [
        { wave: "modal", dur: 0.9, freq: 784, modal: { ratios: [1, 2.76, 5.4], decays: [0.7, 0.3, 0.12], gains: [1, 0.35, 0.15] }, gain: 0.8 },
        { wave: "modal", dur: 1.1, at: 0.28, freq: 622, modal: { ratios: [1, 2.76, 5.4], decays: [0.8, 0.3, 0.12], gains: [1, 0.35, 0.15] }, gain: 0.8 },
      ],
    },
  },
  {
    id: "architect.round",
    label: "Round over",
    group: A,
    description: "A round ends: a short brass-ish fanfare (the results banner shows who won it)",
    bus: "effects",
    gain: 0.8,
    priority: 8,
    space: 0.5,
    synth: {
      layers: [
        { wave: "saw", dur: 0.18, freq: 392, filter: { type: "lp", freq: 2200 }, env: { a: 0.01, s: 1, r: 0.04 }, gain: 0.4 },
        { wave: "saw", dur: 0.18, at: 0.18, freq: 523, filter: { type: "lp", freq: 2400 }, env: { a: 0.01, s: 1, r: 0.04 }, gain: 0.4 },
        { wave: "saw", dur: 0.6, at: 0.36, freq: 659, vibrato: { rate: 5, depth: 0.15 }, filter: { type: "lp", freq: 2600 }, env: { a: 0.01, d: 0.4 }, gain: 0.45 },
        { wave: "saw", dur: 0.6, at: 0.36, freq: 523, filter: { type: "lp", freq: 2000 }, env: { a: 0.01, d: 0.4 }, gain: 0.3 },
      ],
    },
  },
];
