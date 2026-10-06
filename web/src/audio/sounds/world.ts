// World sounds of M4 B5 (game/worldFx.ts, game/moodFx.ts): the four mood
// drones (ambient loops on the music bus, crossfaded as the level's mood
// drifts), the gravity well's hum, the cave-in's warning and crash, and the
// pigment splash.

import type { SoundDef } from "../library.ts";

const M = "Moods";
const W = "World";

/** Loop length of the drones (s). */
const DRONE = 9.6;

export const WORLD_SOUNDS: readonly SoundDef[] = [
  {
    id: "mood.calm",
    label: "Calm (drone)",
    group: M,
    description: "The level is calm: a slow, warm open fifth with air moving through the caves",
    bus: "music",
    loop: true,
    gain: 0.32,
    priority: 3,
    space: 0.6,
    synth: {
      loopFade: 1.2,
      layers: [
        { wave: "sine", dur: DRONE, freq: 110, vibrato: { rate: 0.13, depth: 0.08 }, am: { rate: 0.21, depth: 0.25 }, env: { a: 0.01 }, gain: 0.7 },
        { wave: "triangle", dur: DRONE, freq: 165, filter: { type: "lp", freq: 700 }, am: { rate: 0.17, depth: 0.4 }, env: { a: 0.01 }, gain: 0.32 },
        { wave: "sine", dur: DRONE, freq: 220.6, am: { rate: 0.09, depth: 0.6 }, env: { a: 0.01 }, gain: 0.18 },
        { wave: "pink", dur: DRONE, filter: { type: "bp", freq: 520, q: 0.8 }, am: { rate: 0.11, depth: 0.7 }, env: { a: 0.01 }, gain: 0.22 },
      ],
    },
  },
  {
    id: "mood.bored",
    label: "Bored (drone)",
    group: M,
    description: "The level is bored: one flat, sagging organ note and a slow sigh of air",
    bus: "music",
    loop: true,
    gain: 0.3,
    priority: 3,
    space: 0.5,
    synth: {
      loopFade: 1.2,
      layers: [
        { wave: "saw", dur: DRONE, freq: 98, vibrato: { rate: 0.07, depth: 0.25 }, filter: { type: "lp", freq: 380, q: 0.6 }, env: { a: 0.01 }, gain: 0.5 },
        { wave: "square", dur: DRONE, freq: 97.4, duty: 0.3, filter: { type: "lp", freq: 300 }, env: { a: 0.01 }, gain: 0.22 },
        { wave: "brown", dur: DRONE, filter: { type: "lp", freq: 260 }, am: { rate: 0.3125, depth: 0.6 }, env: { a: 0.01 }, gain: 0.4 },
      ],
    },
  },
  {
    id: "mood.grumpy",
    label: "Grumpy (drone)",
    group: M,
    description: "The level is grumpy: a low growl with a dissonant rub and the rock grumbling",
    bus: "music",
    loop: true,
    gain: 0.36,
    priority: 3,
    space: 0.55,
    synth: {
      loopFade: 1.2,
      layers: [
        { wave: "saw", dur: DRONE, freq: 55, filter: { type: "lp", freq: 240, q: 2.2 }, am: { rate: 0.6, depth: 0.35, random: true }, drive: 1.5, env: { a: 0.01 }, gain: 0.7 },
        { wave: "saw", dur: DRONE, freq: 58.3, filter: { type: "lp", freq: 200, q: 1.5 }, env: { a: 0.01 }, gain: 0.45 },
        { wave: "brown", dur: DRONE, filter: { type: "lp", freq: 120, q: 1.2 }, am: { rate: 1.3, depth: 0.7, random: true }, env: { a: 0.01 }, gain: 0.5 },
        { wave: "crackle", density: 9, dur: DRONE, filter: { type: "bp", freq: 600, q: 1.2 }, env: { a: 0.01 }, gain: 0.25 },
      ],
    },
  },
  {
    id: "mood.giddy",
    label: "Giddy (drone)",
    group: M,
    description: "The level is giddy: a bright major shimmer, bubbling and wobbling",
    bus: "music",
    loop: true,
    gain: 0.28,
    priority: 3,
    space: 0.65,
    synth: {
      loopFade: 1.2,
      layers: [
        { wave: "triangle", dur: DRONE, freq: 261.6, vibrato: { rate: 5.2, depth: 0.12 }, am: { rate: 0.4, depth: 0.3 }, env: { a: 0.01 }, gain: 0.4 },
        { wave: "triangle", dur: DRONE, freq: 329.6, vibrato: { rate: 4.7, depth: 0.12 }, am: { rate: 0.31, depth: 0.45 }, env: { a: 0.01 }, gain: 0.3 },
        { wave: "sine", dur: DRONE, freq: 392, am: { rate: 0.53, depth: 0.55 }, env: { a: 0.01 }, gain: 0.26 },
        { wave: "sine", dur: 0.12, freq: [700, 1400], env: { a: 0.002, d: 0.04 }, repeat: { count: 40, every: 0.24, jitter: 0.08, fade: 1, pitchStep: 0 }, gain: 0.12 },
      ],
    },
  },
  {
    id: "well.hum",
    label: "Gravity well hum (loop)",
    group: W,
    description: "Near a gravity well: a deep pulsing hum that bends in pitch",
    bus: "effects",
    loop: true,
    gain: 0.55,
    priority: 4,
    refDistance: 7,
    space: 0.4,
    synth: {
      loopFade: 0.8,
      layers: [
        { wave: "sine", dur: 4.8, freq: 48, vibrato: { rate: 0.42, depth: 1.2 }, am: { rate: 0.84, depth: 0.5 }, env: { a: 0.01 }, gain: 1 },
        { wave: "sine", dur: 4.8, freq: 96.5, fm: { ratio: 0.5, index: 1.4 }, am: { rate: 0.42, depth: 0.7 }, env: { a: 0.01 }, gain: 0.35 },
        { wave: "pink", dur: 4.8, filter: { type: "bp", freq: 330, q: 3 }, am: { rate: 0.84, depth: 0.8 }, env: { a: 0.01 }, gain: 0.2 },
      ],
    },
  },
  {
    id: "cavein.warning",
    label: "Cave-in warning",
    group: W,
    description: "The ceiling is about to come down: groaning rock, cracking, pebbles trickling for 1.5 s",
    bus: "effects",
    gain: 0.9,
    priority: 8,
    maxInstances: 3,
    refDistance: 18,
    space: 0.5,
    synth: {
      layers: [
        { wave: "brown", dur: 1.7, filter: { type: "lp", freq: [90, 220] }, env: { a: 0.3, s: 1, r: 0.2 }, gain: 1 },
        { wave: "saw", dur: 1.7, freq: [52, 40], vibrato: { rate: 0.8, depth: 1.5 }, filter: { type: "bp", freq: 320, q: 4 }, am: { rate: 5, depth: 0.85, random: true }, env: { a: 0.25 }, gain: 0.55 },
        { wave: "crackle", density: 30, dur: 1.7, filter: { type: "bp", freq: 1600, q: 0.9 }, env: { a: 0.6 }, gain: 0.7 },
        { wave: "noise", dur: 0.07, at: 0.35, filter: { type: "bp", freq: 2400, q: 2 }, env: { a: 0.001, d: 0.02 }, repeat: { count: 5, every: 0.24, jitter: 0.08, fade: 1.15 }, gain: 0.6 },
      ],
    },
  },
  {
    id: "cavein.crash",
    label: "Cave-in crash",
    group: W,
    description: "The ceiling comes down: a crash, a thump and rubble settling",
    bus: "effects",
    gain: 1,
    priority: 9,
    maxInstances: 2,
    refDistance: 24,
    space: 0.65,
    synth: {
      layers: [
        { wave: "sine", dur: 1.2, freq: [80, 28], env: { a: 0.003, d: 0.35 }, drive: 2, gain: 1 },
        { wave: "noise", dur: 2.2, filter: { type: "lp", freq: [4200, 160], curve: 0.6 }, env: { a: 0.002, d: 0.6 }, gain: 0.95 },
        { wave: "crackle", density: 90, at: 0.1, dur: 2.6, filter: { type: "bp", freq: 1100, q: 0.7 }, env: { a: 0.05, d: 0.9 }, gain: 0.6 },
        { wave: "modal", dur: 1.0, freq: 70, modal: { ratios: [1, 1.9, 3.1], decays: [0.5, 0.3, 0.2] }, gain: 0.35 },
      ],
    },
  },
  {
    id: "pigment.splash",
    label: "Pigment",
    group: W,
    description: "A ship takes pigment: a wet splash and a bright rising chime",
    bus: "effects",
    gain: 0.7,
    priority: 6,
    refDistance: 10,
    space: 0.35,
    synth: {
      layers: [
        { wave: "noise", dur: 0.25, filter: { type: "bp", freq: [900, 2600], q: 1.4 }, env: { a: 0.004, d: 0.07 }, gain: 0.7 },
        { wave: "sine", dur: 0.16, freq: [320, 900], env: { a: 0.002, d: 0.05 }, gain: 0.5 },
        { wave: "triangle", dur: 0.6, at: 0.05, freq: 784, env: { a: 0.004, d: 0.18 }, repeat: { count: 3, every: 0.07, fade: 0.8, pitchStep: 4 }, gain: 0.45 },
      ],
    },
  },
];
