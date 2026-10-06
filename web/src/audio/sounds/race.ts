// Belt Up's race sounds (M4 C7, game/race/raceFx.ts): the start lights'
// beeps and the go tone, a chime per gate (pitched up as the lap goes on),
// the lap-done sting, the wrong-way buzzer, the finish fanfare, the Geiger
// counter's clicks (played at a rate that grows with the dose) and the
// exit portal's hum (a positional loop at the exit).

import type { SoundDef } from "../library.ts";

const R = "Race";

export const RACE_SOUNDS: readonly SoundDef[] = [
  {
    id: "race.lightOn",
    label: "Start light",
    group: R,
    description: "One of the five red start lights comes on: a short low beep",
    bus: "effects",
    gain: 0.6,
    priority: 9,
    maxInstances: 3,
    pitchJitter: 0,
    space: 0.15,
    synth: {
      layers: [
        { wave: "square", duty: 0.5, dur: 0.24, freq: 440, filter: { type: "lp", freq: 2400 }, env: { a: 0.003, h: 0.16, d: 0.03 }, gain: 0.6 },
        { wave: "sine", dur: 0.24, freq: 880, env: { a: 0.003, h: 0.16, d: 0.03 }, gain: 0.25 },
      ],
    },
  },
  {
    id: "race.go",
    label: "Go!",
    group: R,
    description: "The lights go out: a long high tone with a rising whoosh",
    bus: "effects",
    gain: 0.75,
    priority: 9,
    maxInstances: 2,
    pitchJitter: 0,
    space: 0.3,
    synth: {
      layers: [
        { wave: "square", duty: 0.5, dur: 0.7, freq: 880, filter: { type: "lp", freq: 3600 }, env: { a: 0.003, h: 0.45, d: 0.12 }, gain: 0.5 },
        { wave: "sine", dur: 0.7, freq: 1760, env: { a: 0.003, h: 0.45, d: 0.12 }, gain: 0.25 },
        { wave: "noise", dur: 0.9, filter: { type: "bp", freq: [400, 3200], q: 1.2 }, env: { a: 0.35, d: 0.2 }, gain: 0.35 },
      ],
    },
  },
  {
    id: "race.gate",
    label: "Gate passed",
    group: R,
    description: "Through a gate: a bright two-note chime (each gate a little higher)",
    bus: "effects",
    gain: 0.6,
    priority: 7,
    maxInstances: 3,
    pitchJitter: 0,
    space: 0.35,
    synth: {
      layers: [
        { wave: "triangle", dur: 0.5, freq: 988, env: { a: 0.002, d: 0.14 }, gain: 0.55 },
        { wave: "triangle", dur: 0.6, at: 0.08, freq: 1319, env: { a: 0.002, d: 0.2 }, gain: 0.55 },
        { wave: "sine", dur: 0.6, at: 0.08, freq: 2638, env: { a: 0.002, d: 0.12 }, gain: 0.15 },
      ],
    },
  },
  {
    id: "race.lapDone",
    label: "Lap done",
    group: R,
    description: "The eighth gate: the lap is done and the EXIT opens, a rising arpeggio",
    bus: "effects",
    gain: 0.7,
    priority: 8,
    maxInstances: 2,
    pitchJitter: 0,
    space: 0.4,
    synth: {
      layers: [
        { wave: "square", duty: 0.25, dur: 0.9, freq: 523, filter: { type: "lp", freq: 3000 }, env: { a: 0.002, d: 0.08 }, repeat: { count: 4, every: 0.09, fade: 1, pitchStep: 4 }, gain: 0.4 },
        { wave: "triangle", dur: 0.9, at: 0.36, freq: 1047, env: { a: 0.003, d: 0.35 }, gain: 0.45 },
      ],
    },
  },
  {
    id: "race.wrongWay",
    label: "Wrong way",
    group: R,
    description: "Flying the course backwards: a nagging two-tone buzzer",
    bus: "effects",
    gain: 0.55,
    priority: 7,
    maxInstances: 1,
    minGap: 0.5,
    pitchJitter: 0,
    space: 0.1,
    synth: {
      layers: [
        { wave: "saw", dur: 0.22, freq: 196, filter: { type: "lp", freq: 1200 }, env: { a: 0.003, h: 0.16, d: 0.02 }, gain: 0.5 },
        { wave: "saw", dur: 0.22, at: 0.26, freq: 156, filter: { type: "lp", freq: 1100 }, env: { a: 0.003, h: 0.16, d: 0.02 }, gain: 0.5 },
      ],
    },
  },
  {
    id: "race.finish",
    label: "Finish",
    group: R,
    description: "Through the EXIT: a little fanfare and a crowd-ish cheer of noise",
    bus: "effects",
    gain: 0.75,
    priority: 9,
    maxInstances: 1,
    pitchJitter: 0,
    space: 0.45,
    synth: {
      layers: [
        { wave: "square", duty: 0.3, dur: 0.16, freq: 523, filter: { type: "lp", freq: 3200 }, env: { a: 0.003, d: 0.06 }, gain: 0.45 },
        { wave: "square", duty: 0.3, dur: 0.16, at: 0.14, freq: 659, filter: { type: "lp", freq: 3200 }, env: { a: 0.003, d: 0.06 }, gain: 0.45 },
        { wave: "square", duty: 0.3, dur: 0.16, at: 0.28, freq: 784, filter: { type: "lp", freq: 3200 }, env: { a: 0.003, d: 0.06 }, gain: 0.45 },
        { wave: "square", duty: 0.3, dur: 0.9, at: 0.42, freq: 1047, vibrato: { rate: 6, depth: 0.25 }, filter: { type: "lp", freq: 3600 }, env: { a: 0.003, h: 0.3, d: 0.25 }, gain: 0.5 },
        { wave: "pink", dur: 1.4, at: 0.3, filter: { type: "bp", freq: 1400, q: 0.7 }, am: { rate: 9, depth: 0.5, random: true }, env: { a: 0.2, d: 0.5 }, gain: 0.3 },
      ],
    },
  },
  {
    id: "race.geiger",
    label: "Geiger click",
    group: R,
    description: "Radiation: one Geiger counter click (the race plays them faster the hotter it gets)",
    bus: "effects",
    gain: 0.45,
    priority: 6,
    maxInstances: 6,
    minGap: 0.012,
    pitchJitter: 0.15,
    space: 0.05,
    synth: {
      layers: [
        { wave: "noise", dur: 0.012, filter: { type: "hp", freq: 2600 }, env: { a: 0.0002, d: 0.0025 }, gain: 1 },
        { wave: "square", dur: 0.01, freq: 3100, env: { a: 0.0002, d: 0.002 }, gain: 0.3 },
      ],
    },
  },
  {
    id: "race.portal",
    label: "EXIT portal (loop)",
    group: R,
    description: "The swirling exit portal hums where it is: a low whirl with a shimmer",
    bus: "effects",
    loop: true,
    gain: 0.5,
    priority: 4,
    refDistance: 22,
    space: 0.6,
    synth: {
      loopFade: 0.8,
      layers: [
        { wave: "saw", dur: 4.8, freq: 73.4, filter: { type: "lp", freq: 420, q: 1.6 }, am: { rate: 1.25, depth: 0.45 }, env: { a: 0.01 }, gain: 0.5 },
        { wave: "sine", dur: 4.8, freq: 146.8, vibrato: { rate: 0.4, depth: 0.3 }, env: { a: 0.01 }, gain: 0.35 },
        { wave: "noise", dur: 4.8, filter: { type: "bp", freq: 2200, q: 3 }, am: { rate: 2.5, depth: 0.7 }, env: { a: 0.01 }, gain: 0.12 },
      ],
    },
  },
];
