// Sounds of M4 C3's spectacle events (game/events/*.ts): the ducks' squeak,
// kazoo march and fanfare; Hat rain's slide whistle and pops; the cat's
// "mrrp", the rumble, crash, whoosh and the claws' scrape; Disco inferno's
// record scratch, glitter drop and four-on-the-floor loop; the Tired
// announcer's yawn and the kill-cam's tape rewind. All synthesized (dsp.ts).

import type { Layer } from "../dsp.ts";
import type { SoundDef } from "../library.ts";

const D = "Duck parade";
const H = "Hat rain";
const P = "Cat paw";
const S = "Disco inferno";
const T = "Tired announcer";

/** Equal-tempered frequency of a MIDI note. */
export const midi = (n: number): number => 440 * Math.pow(2, (n - 69) / 12);

/** A kazoo note: a buzzing saw through a nasal formant, a little flutter. */
function kazoo(at: number, note: number, dur: number, gain = 0.5): Layer[] {
  const f = midi(note);
  const env = { a: 0.018, h: dur * 0.6, d: 0.08, s: 0.7, r: 0.04 };
  return [
    { wave: "saw", at, dur, freq: f, vibrato: { rate: 6.2, depth: 0.12 }, am: { rate: 31, depth: 0.35, random: true }, filter: { type: "bp", freq: 1150, q: 2.2 }, filter2: { type: "peak", freq: 2600, q: 3, gain: 9 }, drive: 1.6, env, gain },
    { wave: "square", at, dur, freq: f * 1.003, duty: 0.3, filter: { type: "bp", freq: 700, q: 1.6 }, env, gain: gain * 0.4 },
  ];
}

/** A tuba-ish bass note. */
function tuba(at: number, note: number, dur: number, gain = 0.45): Layer[] {
  return [{ wave: "square", at, dur, freq: midi(note), duty: 0.45, filter: { type: "lp", freq: [420, 260], q: 1.2 }, env: { a: 0.02, d: 0.18, s: 0.5, r: 0.05 }, gain }];
}

/** A snare tap. */
function snare(at: number, gain = 0.35): Layer[] {
  return [
    { wave: "noise", at, dur: 0.14, filter: { type: "bp", freq: 2400, q: 0.8 }, env: { a: 0.001, d: 0.045 }, gain },
    { wave: "triangle", at, dur: 0.08, freq: [260, 170], env: { a: 0.001, d: 0.03 }, gain: gain * 0.6 },
  ];
}

/** The parade's march: 16 beats at 132 bpm, kazoo melody over tuba and snare. */
function kazooMarch(): Layer[] {
  const beat = 60 / 132;
  // [beat, midi note, beats]: a jaunty tune in C, two phrases.
  const tune: Array<[number, number, number]> = [
    [0, 72, 0.5], [0.5, 74, 0.5], [1, 76, 1], [2, 72, 0.5], [2.5, 76, 0.5], [3, 79, 1],
    [4, 77, 0.5], [4.5, 76, 0.5], [5, 74, 0.5], [5.5, 72, 0.5], [6, 74, 1.5],
    [8, 72, 0.5], [8.5, 74, 0.5], [9, 76, 1], [10, 79, 0.5], [10.5, 81, 0.5], [11, 79, 1],
    [12, 77, 0.5], [12.5, 74, 0.5], [13, 71, 0.5], [13.5, 74, 0.5], [14, 72, 1.6],
  ];
  const bass: Array<[number, number]> = [
    [0, 48], [1, 55], [2, 48], [3, 55], [4, 50], [5, 55], [6, 43], [7, 55],
    [8, 48], [9, 55], [10, 52], [11, 55], [12, 50], [13, 43], [14, 48], [15, 43],
  ];
  const out: Layer[] = [];
  for (const [b, n, len] of tune) out.push(...kazoo(b * beat, n, len * beat * 0.92, 0.5));
  for (const [b, n] of bass) out.push(...tuba(b * beat, n, beat * 0.55));
  for (let b = 0; b < 16; b++) out.push(...snare(b * beat + beat * 0.5, b % 2 === 1 ? 0.3 : 0.18));
  return out;
}

/** The disco loop: 4 bars at 120 bpm, kick on every beat, offbeat hats, claps, an octave bass and string stabs. */
function discoGroove(): Layer[] {
  const beat = 0.5;
  const out: Layer[] = [];
  const roots = [45, 45, 50, 52];
  for (let b = 0; b < 16; b++) {
    const t = b * beat;
    // Kick.
    out.push({ wave: "sine", at: t, dur: 0.32, freq: [150, 44], sweepTime: 0.12, env: { a: 0.001, d: 0.14 }, drive: 1.4, gain: 0.95 });
    out.push({ wave: "noise", at: t, dur: 0.02, filter: { type: "hp", freq: 3000 }, env: { a: 0.0005, d: 0.006 }, gain: 0.25 });
    // Open hat on the offbeat, closed ticks between.
    out.push({ wave: "noise", at: t + beat / 2, dur: 0.22, filter: { type: "hp", freq: 7000 }, env: { a: 0.001, d: 0.08 }, gain: 0.32 });
    out.push({ wave: "noise", at: t + beat / 4, dur: 0.04, filter: { type: "hp", freq: 9000 }, env: { a: 0.0005, d: 0.01 }, gain: 0.14 });
    out.push({ wave: "noise", at: t + (3 * beat) / 4, dur: 0.04, filter: { type: "hp", freq: 9000 }, env: { a: 0.0005, d: 0.01 }, gain: 0.14 });
    // Claps on 2 and 4.
    if (b % 2 === 1) out.push({ wave: "noise", at: t, dur: 0.18, filter: { type: "bp", freq: 1500, q: 1.1 }, env: { a: 0.001, d: 0.05 }, repeat: { count: 3, every: 0.011, fade: 0.8 }, gain: 0.42 });
    // Octave bass: root on the beat, octave on the off.
    const root = roots[Math.floor(b / 4)]!;
    out.push({ wave: "saw", at: t, dur: beat * 0.42, freq: midi(root - 12), filter: { type: "lp", freq: [1400, 300], q: 3 }, env: { a: 0.004, d: 0.12, s: 0.4, r: 0.03 }, gain: 0.42 });
    out.push({ wave: "saw", at: t + beat / 2, dur: beat * 0.42, freq: midi(root), filter: { type: "lp", freq: [1600, 350], q: 3 }, env: { a: 0.004, d: 0.12, s: 0.4, r: 0.03 }, gain: 0.36 });
  }
  // String stabs on the "and" of 4 of each bar, a minor-ish seventh chord.
  for (let bar = 0; bar < 4; bar++) {
    const root = roots[bar]! + 12;
    for (const iv of [0, 3, 7, 10]) {
      out.push({ wave: "saw", at: bar * 4 * beat + 3.5 * beat, dur: 0.26, freq: midi(root + iv), vibrato: { rate: 5, depth: 0.1 }, filter: { type: "lp", freq: 2600 }, env: { a: 0.01, d: 0.12, s: 0.3, r: 0.05 }, gain: 0.14 });
    }
  }
  return out;
}

export const SPECTACLE_SOUNDS: readonly SoundDef[] = [
  // ----- Duck parade ------------------------------------------------------------
  {
    id: "duck.squeak",
    label: "Duck squeak",
    group: D,
    description: "A rubber duck swallows a shot or shoves a ship: a squeeze-toy squeak",
    bus: "effects",
    gain: 0.75,
    priority: 6,
    maxInstances: 4,
    minGap: 0.06,
    refDistance: 14,
    space: 0.3,
    pitchJitter: 0.08,
    synth: {
      layers: [
        { wave: "triangle", dur: 0.2, freq: [880, 1500], sweepTime: 0.06, sweepCurve: 0.6, vibrato: { rate: 38, depth: 0.5 }, filter: { type: "bp", freq: 1600, q: 1.4 }, env: { a: 0.004, h: 0.07, d: 0.05 }, drive: 1.3, gain: 0.8 },
        { wave: "noise", dur: 0.14, filter: { type: "bp", freq: 3200, q: 3 }, env: { a: 0.004, d: 0.04 }, gain: 0.25 },
        { wave: "triangle", at: 0.12, dur: 0.12, freq: [1400, 1000], filter: { type: "bp", freq: 1500, q: 1.2 }, env: { a: 0.004, d: 0.04 }, gain: 0.45 },
      ],
    },
  },
  {
    id: "duck.kazoo",
    label: "Duck parade march (loop)",
    group: D,
    description: "While the ducks parade: a marching kazoo band with tuba and snare, following the lead duck",
    bus: "music",
    loop: true,
    gain: 0.55,
    priority: 5,
    refDistance: 30,
    space: 0.35,
    synth: { loopFade: 0.05, layers: kazooMarch() },
  },
  {
    id: "duck.fanfare",
    label: "Duck parade fanfare",
    group: D,
    description: "Duck parade announced: a kazoo \"ta-ta-taaa\" and a quack",
    bus: "effects",
    gain: 0.6,
    priority: 8,
    maxInstances: 1,
    pitchJitter: 0,
    variants: 1,
    synth: {
      layers: [
        ...kazoo(0.55, 72, 0.13),
        ...kazoo(0.7, 72, 0.13),
        ...kazoo(0.85, 79, 0.55, 0.6),
        { wave: "saw", at: 1.5, dur: 0.22, freq: [520, 380], filter: { type: "bp", freq: 1100, q: 2.5 }, am: { rate: 45, depth: 0.5 }, env: { a: 0.01, d: 0.09 }, drive: 2, gain: 0.5 },
      ],
    },
  },
  // ----- Hat rain ---------------------------------------------------------------
  {
    id: "hat.whistle",
    label: "Hat rain whistle",
    group: H,
    description: "Hat rain announced: a slide whistle falling, then a twinkle",
    bus: "effects",
    gain: 0.55,
    priority: 8,
    maxInstances: 1,
    pitchJitter: 0,
    variants: 1,
    synth: {
      layers: [
        { wave: "sine", at: 0.55, dur: 1.1, freq: [2100, 420], sweepCurve: 1.4, vibrato: { rate: 7, depth: 0.25 }, env: { a: 0.03, d: 0.6, s: 0.6, r: 0.08 }, gain: 0.7 },
        { wave: "noise", at: 0.55, dur: 1.1, filter: { type: "bp", freq: [2100, 420], q: 6 }, env: { a: 0.03, s: 0.5 }, gain: 0.12 },
        { wave: "triangle", at: 1.7, dur: 0.4, freq: 1568, env: { a: 0.002, d: 0.1 }, repeat: { count: 4, every: 0.07, fade: 0.8, pitchStep: 3 }, gain: 0.3 },
      ],
    },
  },
  {
    id: "hat.pop",
    label: "Hat on",
    group: H,
    description: "A ship puts on a hat: a cork pop and a bright ding (higher with every hat)",
    bus: "effects",
    gain: 0.7,
    priority: 6,
    maxInstances: 4,
    minGap: 0.05,
    refDistance: 12,
    space: 0.25,
    synth: {
      layers: [
        { wave: "sine", dur: 0.09, freq: [380, 1200], sweepTime: 0.03, env: { a: 0.001, d: 0.025 }, gain: 0.8 },
        { wave: "noise", dur: 0.05, filter: { type: "bp", freq: 1800, q: 1.5 }, env: { a: 0.001, d: 0.012 }, gain: 0.5 },
        { wave: "triangle", at: 0.04, dur: 0.5, freq: 1318.5, env: { a: 0.002, d: 0.14 }, gain: 0.4 },
        { wave: "sine", at: 0.04, dur: 0.5, freq: 2637, env: { a: 0.002, d: 0.08 }, gain: 0.15 },
      ],
    },
  },
  // ----- Cat paw ----------------------------------------------------------------
  {
    id: "paw.mrrp",
    label: "Cat \"mrrp\"",
    group: P,
    description: "The cat's trill: on the tell and, pleased, when the paw withdraws",
    bus: "effects",
    gain: 0.9,
    priority: 9,
    maxInstances: 1,
    refDistance: 45,
    space: 0.5,
    pitchJitter: 0.04,
    synth: {
      layers: [
        // A purring roll that rises into a chirp: the r is a fast flutter, the vowel a formant.
        { wave: "saw", dur: 0.55, freq: [170, 420], sweepCurve: 1.8, am: { rate: 27, depth: 0.75 }, filter: { type: "bp", freq: [600, 1500], q: 2.4 }, filter2: { type: "peak", freq: 2600, q: 2, gain: 6 }, env: { a: 0.04, h: 0.3, d: 0.12, s: 0.3, r: 0.05 }, drive: 1.2, gain: 0.9 },
        { wave: "sine", dur: 0.55, freq: [170, 420], sweepCurve: 1.8, am: { rate: 27, depth: 0.6 }, env: { a: 0.04, h: 0.3, d: 0.12 }, gain: 0.45 },
        { wave: "brown", dur: 0.45, filter: { type: "lp", freq: 300 }, am: { rate: 27, depth: 0.9 }, env: { a: 0.05, d: 0.2 }, gain: 0.3 },
      ],
    },
  },
  {
    id: "paw.rumble",
    label: "Cat paw rumble",
    group: P,
    description: "The tell: the boundary wall groans and creaks where the paw will come through",
    bus: "effects",
    gain: 0.8,
    priority: 8,
    maxInstances: 1,
    refDistance: 40,
    space: 0.55,
    synth: {
      layers: [
        { wave: "brown", dur: 2.9, filter: { type: "lp", freq: [80, 260] }, env: { a: 1.2, s: 1, r: 0.3 }, gain: 1 },
        { wave: "saw", dur: 2.9, freq: [38, 52], filter: { type: "bp", freq: 240, q: 3 }, am: { rate: 3.3, depth: 0.7, random: true }, env: { a: 1.4, r: 0.3 }, gain: 0.5 },
        { wave: "crackle", density: 22, dur: 2.9, filter: { type: "bp", freq: 1900, q: 1 }, env: { a: 1.6 }, gain: 0.5 },
      ],
    },
  },
  {
    id: "paw.crash",
    label: "Cat paw breaks through",
    group: P,
    description: "The paw bursts through the boundary wall: a crash and falling rock",
    bus: "effects",
    gain: 1,
    priority: 9,
    maxInstances: 1,
    refDistance: 40,
    space: 0.6,
    synth: {
      layers: [
        { wave: "sine", dur: 0.9, freq: [95, 30], env: { a: 0.002, d: 0.3 }, drive: 2.2, gain: 1 },
        { wave: "noise", dur: 1.6, filter: { type: "lp", freq: [5200, 220], curve: 0.5 }, env: { a: 0.002, d: 0.45 }, gain: 0.95 },
        { wave: "crackle", density: 110, dur: 1.8, at: 0.05, filter: { type: "bp", freq: 1300, q: 0.7 }, env: { a: 0.02, d: 0.6 }, gain: 0.6 },
      ],
    },
  },
  {
    id: "paw.whoosh",
    label: "Cat paw swipe",
    group: P,
    description: "The swipe: a huge whoosh of air",
    bus: "effects",
    gain: 0.9,
    priority: 8,
    maxInstances: 1,
    refDistance: 35,
    space: 0.45,
    synth: {
      layers: [
        { wave: "noise", dur: 1.6, filter: { type: "bp", freq: [300, 1800], q: 1.3, time: 0.9, curve: 0.7 }, env: { a: 0.35, h: 0.3, d: 0.35 }, gain: 1 },
        { wave: "pink", dur: 1.6, filter: { type: "lp", freq: [200, 900] }, env: { a: 0.3, d: 0.5 }, gain: 0.6 },
      ],
    },
  },
  {
    id: "paw.scrape",
    label: "Claws on rock (loop)",
    group: P,
    description: "While the claws rake rock: grinding stone and a screech",
    bus: "effects",
    loop: true,
    gain: 0.8,
    priority: 7,
    refDistance: 25,
    space: 0.4,
    synth: {
      loopFade: 0.2,
      layers: [
        { wave: "crackle", density: 260, dur: 1.4, filter: { type: "bp", freq: 1500, q: 0.8 }, env: { a: 0.01 }, gain: 0.8 },
        { wave: "noise", dur: 1.4, filter: { type: "bp", freq: 3600, q: 7 }, am: { rate: 11, depth: 0.6, random: true }, env: { a: 0.01 }, gain: 0.35 },
        { wave: "brown", dur: 1.4, filter: { type: "lp", freq: 240 }, am: { rate: 6, depth: 0.5, random: true }, env: { a: 0.01 }, gain: 0.6 },
      ],
    },
  },
  // ----- Disco inferno ----------------------------------------------------------
  {
    id: "disco.scratch",
    label: "Record scratch",
    group: S,
    description: "Disco inferno announced: the record scratches, the lights are about to drop",
    bus: "effects",
    gain: 0.7,
    priority: 8,
    maxInstances: 1,
    pitchJitter: 0,
    variants: 1,
    synth: {
      layers: [
        { wave: "noise", at: 0.5, dur: 0.18, filter: { type: "bp", freq: [600, 2600], q: 4 }, env: { a: 0.005, d: 0.08 }, gain: 0.8 },
        { wave: "noise", at: 0.68, dur: 0.22, filter: { type: "bp", freq: [2800, 500], q: 4 }, env: { a: 0.005, d: 0.1 }, gain: 0.8 },
        { wave: "saw", at: 0.5, dur: 0.4, freq: [140, 520], sweepTime: 0.18, filter: { type: "lp", freq: 1800 }, env: { a: 0.005, d: 0.15 }, gain: 0.35 },
      ],
    },
  },
  {
    id: "disco.drop",
    label: "Mirror ball drop",
    group: S,
    description: "The mirror ball comes down: a glittering descending shimmer",
    bus: "effects",
    gain: 0.6,
    priority: 6,
    maxInstances: 1,
    refDistance: 30,
    space: 0.6,
    synth: {
      layers: [
        { wave: "sine", dur: 0.25, freq: 2093, env: { a: 0.002, d: 0.08 }, repeat: { count: 12, every: 0.09, jitter: 0.01, fade: 0.93, pitchStep: -2 }, gain: 0.35 },
        { wave: "noise", dur: 1.4, filter: { type: "hp", freq: [9000, 4000] }, env: { a: 0.2, d: 0.6 }, gain: 0.2 },
      ],
    },
  },
  {
    id: "disco.loop",
    label: "Disco inferno (loop)",
    group: S,
    description: "While the lights are down: four on the floor, offbeat hats, claps, an octave bass and string stabs",
    bus: "music",
    loop: true,
    gain: 0.5,
    priority: 5,
    space: 0.3,
    synth: { loopFade: 0.02, layers: discoGroove() },
  },
  // ----- Tired announcer --------------------------------------------------------
  {
    id: "announcer.yawn",
    label: "The announcer yawns",
    group: T,
    description: "Tired announcer announced: a long, robotic yawn",
    bus: "voice",
    gain: 0.6,
    priority: 9,
    maxInstances: 1,
    pitchJitter: 0,
    variants: 1,
    synth: {
      layers: [
        { wave: "saw", at: 0.6, dur: 1.6, freq: [260, 120], sweepCurve: 1.6, vibrato: { rate: 5, depth: 0.3 }, filter: { type: "bp", freq: [1100, 500], q: 3 }, filter2: { type: "peak", freq: 2400, q: 2, gain: 5 }, env: { a: 0.25, h: 0.7, d: 0.4 }, gain: 0.8 },
        { wave: "pink", at: 0.6, dur: 1.6, filter: { type: "bp", freq: [1200, 500], q: 2 }, env: { a: 0.3, h: 0.6, d: 0.4 }, gain: 0.25 },
      ],
    },
  },
  {
    id: "killcam.rewind",
    label: "Kill-cam rewind",
    group: T,
    description: "A kill-cam starts: a tape rewinding with a whir and a clunk",
    bus: "effects",
    gain: 0.55,
    priority: 7,
    maxInstances: 1,
    pitchJitter: 0.02,
    synth: {
      layers: [
        { wave: "saw", dur: 0.45, freq: [900, 2400], vibrato: { rate: 30, depth: 0.4 }, filter: { type: "bp", freq: [1200, 3200], q: 2 }, env: { a: 0.02, h: 0.3, d: 0.06 }, gain: 0.4 },
        { wave: "noise", dur: 0.45, filter: { type: "hp", freq: 4000 }, am: { rate: 45, depth: 0.6 }, env: { a: 0.02, h: 0.3, d: 0.06 }, gain: 0.25 },
        { wave: "noise", at: 0.46, dur: 0.06, filter: { type: "bp", freq: 900, q: 1.5 }, env: { a: 0.001, d: 0.02 }, gain: 0.7 },
        { wave: "sine", at: 0.46, dur: 0.08, freq: [180, 90], env: { a: 0.001, d: 0.03 }, gain: 0.5 },
      ],
    },
  },
];
