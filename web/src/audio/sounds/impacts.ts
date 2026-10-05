// Impacts and explosions: what a shot does when it hits rock, a hull or a
// shield (the impact kind of game/vfx.ts VFX_SHAPES), and the blasts of
// ordnance, dying ships and shockwaves.

import type { SoundDef } from "../library.ts";

const I = "Impacts";
const X = "Explosions";

export const IMPACT_SOUNDS: readonly SoundDef[] = [
  {
    id: "impact.sparks",
    label: "Sparks on rock",
    group: I,
    description: "Bolts, pellets and spikes hitting rock: a sizzling spray",
    bus: "effects",
    gain: 0.4,
    priority: 3,
    maxInstances: 6,
    minGap: 0.03,
    space: 0.3,
    synth: {
      layers: [
        { wave: "noise", dur: 0.2, filter: { type: "hp", freq: 2600 }, env: { a: 0.0005, d: 0.045 }, gain: 0.7 },
        { wave: "crackle", density: 1000, dur: 0.28, filter: { type: "bp", freq: 4200, q: 0.9 }, env: { a: 0.001, d: 0.08 }, gain: 0.7 },
        { wave: "sine", dur: 0.06, freq: [2300, 900], env: { a: 0.0005, d: 0.015 }, gain: 0.2 },
        { wave: "noise", dur: 0.08, filter: { type: "lp", freq: 700 }, env: { a: 0.0005, d: 0.02 }, gain: 0.5 },
      ],
    },
  },
  {
    id: "impact.plasma",
    label: "Plasma splash",
    group: I,
    description: "Orbs and arcs hitting rock: a wet, fizzing wub",
    bus: "effects",
    gain: 0.45,
    priority: 3,
    maxInstances: 6,
    minGap: 0.03,
    space: 0.3,
    synth: {
      layers: [
        { wave: "noise", dur: 0.32, filter: { type: "lp", freq: [5200, 600], q: 3 }, env: { a: 0.001, d: 0.09 }, gain: 0.8 },
        { wave: "sine", dur: 0.22, freq: [320, 75], env: { a: 0.001, d: 0.06 }, gain: 0.6 },
        { wave: "crackle", density: 600, dur: 0.3, filter: { type: "hp", freq: 3000 }, env: { a: 0.01, d: 0.1 }, gain: 0.3 },
      ],
    },
  },
  {
    id: "impact.thud",
    label: "Thud",
    group: I,
    description: "Harpoon hits and other blunt impacts",
    bus: "effects",
    gain: 0.55,
    priority: 3,
    space: 0.3,
    synth: {
      layers: [
        { wave: "sine", dur: 0.26, freq: [125, 48], env: { a: 0.0008, d: 0.06 }, drive: 1.2, gain: 1 },
        { wave: "noise", dur: 0.12, filter: { type: "lp", freq: 650 }, env: { a: 0.0008, d: 0.03 }, gain: 0.6 },
      ],
    },
  },
  {
    id: "impact.splat",
    label: "Splat",
    group: I,
    description: "Paint and the sneeze blob landing: a squelch with bubbles",
    bus: "effects",
    gain: 0.45,
    priority: 3,
    maxInstances: 5,
    space: 0.25,
    synth: {
      layers: [
        { wave: "noise", dur: 0.3, filter: { type: "bp", freq: [1900, 280], q: 3 }, env: { a: 0.001, d: 0.08 }, gain: 0.9 },
        { wave: "sine", at: 0.05, dur: 0.06, freq: [380, 1150], env: { a: 0.002, d: 0.02 }, repeat: { count: 3, every: 0.055, jitter: 0.02, fade: 0.7, pitchStep: 3 }, gain: 0.3 },
      ],
    },
  },
  {
    id: "impact.hull",
    label: "Hull hit",
    group: I,
    description: "A shot hitting another ship: a metallic clang",
    bus: "effects",
    gain: 0.55,
    priority: 5,
    maxInstances: 6,
    minGap: 0.03,
    space: 0.2,
    synth: {
      layers: [
        { wave: "modal", dur: 0.8, freq: 340, modal: { ratios: [1, 1.58, 2.33, 3.17, 4.31, 5.9], decays: [0.7, 0.55, 0.42, 0.3, 0.2, 0.12] }, gain: 0.6 },
        { wave: "noise", dur: 0.05, filter: { type: "hp", freq: 1600 }, env: { a: 0.0003, d: 0.01 }, gain: 0.75 },
        { wave: "sine", dur: 0.1, freq: [165, 70], env: { a: 0.0005, d: 0.03 }, gain: 0.55 },
      ],
    },
  },
  {
    id: "impact.hullOwn",
    label: "Own hull hit",
    group: I,
    description: "Your hull takes a hit: a heavy clank close around you",
    bus: "effects",
    gain: 0.7,
    priority: 8,
    maxInstances: 3,
    minGap: 0.05,
    space: 0.12,
    synth: {
      layers: [
        { wave: "modal", dur: 0.7, freq: 205, modal: { ratios: [1, 1.47, 2.09, 2.83, 3.71], decays: [0.6, 0.45, 0.35, 0.25, 0.16] }, gain: 0.5 },
        { wave: "sine", dur: 0.28, freq: [95, 38], env: { a: 0.0005, d: 0.07 }, drive: 2, gain: 1 },
        { wave: "noise", dur: 0.2, filter: { type: "lp", freq: 2600 }, env: { a: 0.0005, d: 0.04 }, gain: 0.5 },
        { wave: "crackle", density: 500, dur: 0.35, filter: { type: "bp", freq: 3800, q: 1 }, env: { a: 0.01, d: 0.1 }, gain: 0.25 },
      ],
    },
  },
  {
    id: "impact.shield",
    label: "Shield hit",
    group: I,
    description: "A shot glancing off a raised Shield: a shimmering bwong",
    bus: "effects",
    gain: 0.5,
    priority: 5,
    maxInstances: 5,
    minGap: 0.03,
    space: 0.3,
    synth: {
      layers: [
        { wave: "sine", dur: 0.6, freq: [540, 380], fm: { ratio: 3.5, index: [6, 0.4] }, env: { a: 0.002, d: 0.2 }, gain: 0.7 },
        { wave: "triangle", dur: 0.5, freq: [1080, 760], vibrato: { rate: 18, depth: 0.6 }, env: { a: 0.002, d: 0.15 }, gain: 0.3 },
        { wave: "noise", dur: 0.15, filter: { type: "bp", freq: 3000, q: 3 }, env: { a: 0.001, d: 0.04 }, gain: 0.25 },
      ],
    },
  },
  {
    id: "explosion.small",
    label: "Explosion",
    group: X,
    description: "Rockets, missiles, mines and grenades going off",
    bus: "effects",
    gain: 0.85,
    priority: 7,
    maxInstances: 6,
    minGap: 0.03,
    refDistance: 12,
    space: 0.45,
    synth: {
      layers: [
        { wave: "noise", dur: 1.4, filter: { type: "lp", freq: [6500, 230], curve: 0.55, q: 0.9 }, env: { a: 0.0008, d: 0.24 }, gain: 1 },
        { wave: "sine", dur: 0.8, freq: [88, 31], sweepCurve: 0.6, env: { a: 0.002, d: 0.22 }, drive: 1.5, gain: 1.1 },
        { wave: "brown", at: 0.04, dur: 2.0, filter: { type: "lp", freq: 320 }, env: { a: 0.05, d: 0.6 }, gain: 0.6 },
        { wave: "crackle", density: 280, at: 0.05, dur: 1.2, filter: { type: "bp", freq: 2400, q: 0.8 }, env: { a: 0.02, d: 0.4 }, gain: 0.35 },
      ],
    },
  },
  {
    id: "explosion.big",
    label: "Big explosion",
    group: X,
    description: "Heavy Rocket and other blasts of 3.5 u and more, Hot Reactor shockwaves",
    bus: "effects",
    gain: 1,
    priority: 8,
    maxInstances: 4,
    minGap: 0.05,
    refDistance: 20,
    space: 0.6,
    synth: {
      layers: [
        { wave: "noise", dur: 2.6, filter: { type: "lp", freq: [5200, 120], curve: 0.5, q: 0.8 }, env: { a: 0.001, d: 0.5 }, gain: 1 },
        { wave: "sine", dur: 1.5, freq: [62, 24], sweepCurve: 0.6, env: { a: 0.003, d: 0.45 }, drive: 2, gain: 1.2 },
        { wave: "brown", at: 0.05, dur: 3.6, filter: { type: "lp", freq: 260 }, env: { a: 0.08, d: 1.2 }, gain: 0.75 },
        { wave: "crackle", density: 220, at: 0.08, dur: 3.0, filter: { type: "bp", freq: 1800, q: 0.7 }, env: { a: 0.05, d: 0.9 }, gain: 0.4 },
      ],
    },
  },
  {
    id: "explosion.ship",
    label: "Ship destroyed",
    group: X,
    description: "A ship blows apart: blast, electrical fizz and hull plates clattering away",
    bus: "effects",
    gain: 0.95,
    priority: 9,
    maxInstances: 4,
    refDistance: 14,
    space: 0.5,
    synth: {
      layers: [
        { wave: "noise", dur: 1.6, filter: { type: "lp", freq: [7000, 260], curve: 0.55 }, env: { a: 0.0008, d: 0.3 }, gain: 1 },
        { wave: "sine", dur: 0.9, freq: [80, 28], env: { a: 0.002, d: 0.26 }, drive: 1.8, gain: 1.1 },
        { wave: "crackle", density: 2200, dur: 0.7, filter: { type: "bp", freq: 5200, q: 1 }, env: { a: 0.005, d: 0.2 }, gain: 0.45 },
        { wave: "modal", at: 0.12, dur: 0.32, freq: 920, jitter: 0.25, modal: { ratios: [1, 2.42, 4.07], decays: [0.26, 0.16, 0.1] }, repeat: { count: 7, every: 0.09, jitter: 0.07, fade: 0.78, pitchStep: -1 }, gain: 0.32 },
        { wave: "brown", at: 0.05, dur: 2.2, filter: { type: "lp", freq: 300 }, env: { a: 0.05, d: 0.7 }, gain: 0.55 },
      ],
    },
  },
];
