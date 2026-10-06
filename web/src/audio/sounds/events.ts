// Sounds of the comedy events of M4 C1 (game/events/): Bored level's yawn and
// the creaking level, Level hiccups' "hic", Gravity coffee break's clink,
// slurp and lounge music, Wrong genre's power-on bleep, chiptune loop and
// 1-UP jingle, Big head day's inflating squeak, honk and pop, Rubber
// chickens' squeak, squawk and the giant egg's crack, Moving day's tape,
// hop, thump and flaps, Everything is cake's party horn. All procedural
// (dsp.ts); every one plays in `?audio=demo`.

import type { Layer } from "../dsp.ts";
import type { SoundDef } from "../library.ts";

const G = "Events";

/** Equal-tempered frequency of a MIDI note. */
export function midi(n: number): number {
  return 440 * Math.pow(2, (n - 69) / 12);
}

/** One chiptune note: a 25 % square with a plucky envelope. */
function chipNote(at: number, note: number, dur: number, gain = 0.32, duty = 0.25): Layer {
  return { wave: "square", duty, at, dur, freq: midi(note), filter: { type: "lp", freq: 7000 }, env: { a: 0.002, d: dur * 0.7, s: 0.45, r: 0.015 }, gain };
}

/** Bars of the Wrong genre loop at 150 bpm: an eighth is 0.2 s, a bar 1.6 s. */
const CHIP_EIGHTH = 0.2;
/** Melody in eighths (MIDI, 0 = rest): a bouncy C major platformer tune. */
const CHIP_MELODY = [
  72, 76, 79, 76, 84, 79, 76, 79,
  77, 81, 84, 81, 77, 81, 84, 81,
  79, 83, 86, 83, 79, 83, 86, 89,
  88, 86, 84, 79, 76, 79, 84, 0,
];
/** Bass in quarters. */
const CHIP_BASS = [48, 55, 48, 55, 41, 48, 41, 48, 43, 50, 43, 50, 48, 43, 48, 48];

/** The Wrong genre chiptune loop (6.4 s). */
export function chiptuneLayers(): Layer[] {
  const out: Layer[] = [];
  CHIP_MELODY.forEach((n, i) => {
    if (n > 0) out.push(chipNote(i * CHIP_EIGHTH, n, CHIP_EIGHTH * 0.85));
  });
  // An echo an octave down, quieter, for body.
  CHIP_MELODY.forEach((n, i) => {
    if (n > 0 && i % 2 === 0) out.push(chipNote(i * CHIP_EIGHTH + 0.1, n - 12, CHIP_EIGHTH * 0.6, 0.12, 0.5));
  });
  CHIP_BASS.forEach((n, i) => {
    out.push({ wave: "triangle", at: i * 0.4, dur: 0.36, freq: midi(n), env: { a: 0.003, d: 0.25, s: 0.6, r: 0.02 }, gain: 0.55 });
  });
  for (let i = 0; i < 32; i++) {
    // Noise drums: a kick-ish thump on the beats, a hat on the offbeats, a snare on 2 and 4.
    if (i % 2 === 0) out.push({ wave: "sine", at: i * CHIP_EIGHTH, dur: 0.12, freq: [160, 50], env: { a: 0.001, d: 0.05 }, gain: i % 4 === 0 ? 0.55 : 0.3 });
    out.push({ wave: "noise", at: i * CHIP_EIGHTH + (i % 2 ? 0 : 0.1), dur: 0.04, filter: { type: "hp", freq: 7000 }, env: { a: 0.001, d: 0.012 }, gain: 0.18 });
    if (i % 4 === 2) out.push({ wave: "noise", at: i * CHIP_EIGHTH, dur: 0.12, filter: { type: "bp", freq: 1800, q: 0.7 }, env: { a: 0.001, d: 0.05 }, gain: 0.35 });
  }
  return out;
}

/** An electric piano chord (FM sines, the bell fading), for the lounge loop. */
function epChord(at: number, notes: readonly number[], dur: number): Layer[] {
  return notes.map((n) => ({ wave: "sine" as const, at, dur, freq: midi(n), fm: { ratio: 1, index: [1.6, 0.15] as const }, env: { a: 0.004, d: 1.1, s: 0.25, r: 0.2 }, gain: 0.22 }));
}

/** The coffee break's lounge loop (6.4 s): Cmaj7 Am7 Dm7 G7 on a soft e-piano, a walking bass, brushes. */
export function loungeLayers(): Layer[] {
  const chords: number[][] = [
    [60, 64, 67, 71],
    [57, 60, 64, 67],
    [62, 65, 69, 72],
    [55, 59, 62, 65],
  ];
  const bass = [36, 40, 33, 36, 38, 41, 31, 35];
  const out: Layer[] = [];
  chords.forEach((c, i) => {
    out.push(...epChord(i * 1.6, c, 1.5));
    out.push(...epChord(i * 1.6 + 1.0, c.slice(1), 0.55).map((l) => ({ ...l, gain: 0.12 })));
  });
  bass.forEach((n, i) => out.push({ wave: "triangle", at: i * 0.8, dur: 0.75, freq: midi(n), filter: { type: "lp", freq: 600 }, env: { a: 0.01, d: 0.5, s: 0.4, r: 0.05 }, gain: 0.5 }));
  for (let i = 0; i < 16; i++) {
    // Brushes with a swing.
    out.push({ wave: "pink", at: i * 0.4 + (i % 2 ? 0.07 : 0), dur: 0.2, filter: { type: "bp", freq: 5200, q: 0.6 }, env: { a: 0.02, d: 0.08 }, gain: i % 2 ? 0.1 : 0.16 });
  }
  return out;
}

export const EVENT_SOUNDS: readonly SoundDef[] = [
  {
    id: "bored.yawn",
    label: "Bored level: yawn",
    group: G,
    description: "Bored level starts turning: the level's long, bored yawn",
    bus: "effects",
    gain: 0.75,
    priority: 8,
    maxInstances: 1,
    space: 0.6,
    pitchJitter: 0.02,
    synth: {
      layers: [
        { wave: "saw", dur: 2.8, freq: [170, 105], sweepCurve: 1.4, vibrato: { rate: 4.5, depth: 0.25 }, filter: { type: "bp", freq: [900, 330], q: 2.4 }, filter2: { type: "lp", freq: 2200 }, env: { a: 0.45, h: 0.9, d: 0.7, s: 0.2, r: 0.5 }, gain: 0.9 },
        { wave: "saw", dur: 2.8, freq: [171, 106], sweepCurve: 1.4, filter: { type: "bp", freq: [2300, 900], q: 3 }, env: { a: 0.5, h: 0.8, d: 0.6, s: 0.1, r: 0.5 }, gain: 0.35 },
        { wave: "pink", dur: 2.9, filter: { type: "bp", freq: [1400, 600], q: 1.2 }, env: { a: 0.3, h: 1.2, d: 0.8, r: 0.4 }, gain: 0.35 },
        // The jaw closing.
        { wave: "sine", at: 2.55, dur: 0.12, freq: [220, 140], env: { a: 0.002, d: 0.04 }, gain: 0.3 },
      ],
    },
  },
  {
    id: "bored.creak",
    label: "Bored level: creaking (loop)",
    group: G,
    description: "While the level turns over: stone groaning and creaking",
    bus: "effects",
    loop: true,
    gain: 0.55,
    priority: 6,
    space: 0.65,
    synth: {
      loopFade: 0.6,
      layers: [
        { wave: "saw", dur: 4.8, freq: 46, vibrato: { rate: 0.6, depth: 1.8 }, filter: { type: "bp", freq: 260, q: 3 }, am: { rate: 3, depth: 0.7, random: true }, env: { a: 1.2 }, gain: 0.8 },
        { wave: "brown", dur: 4.8, filter: { type: "lp", freq: 180 }, am: { rate: 0.7, depth: 0.6 }, env: { a: 1.2 }, gain: 0.7 },
        { wave: "crackle", density: 22, dur: 4.8, filter: { type: "bp", freq: 1300, q: 1 }, env: { a: 1.2 }, gain: 0.45 },
      ],
    },
  },
  {
    id: "hiccup.hic",
    label: "Level hiccups: hic",
    group: G,
    description: "Every hiccup of the level: a sharp \"hic\"",
    bus: "effects",
    gain: 0.85,
    priority: 8,
    maxInstances: 2,
    space: 0.45,
    pitchJitter: 0.06,
    synth: {
      layers: [
        { wave: "noise", dur: 0.05, filter: { type: "bp", freq: 2600, q: 2.5 }, env: { a: 0.001, d: 0.015 }, gain: 0.9 },
        { wave: "saw", at: 0.012, dur: 0.13, freq: [230, 360], filter: { type: "bp", freq: [700, 1500], q: 4 }, env: { a: 0.004, d: 0.05, r: 0.02 }, gain: 0.8 },
        { wave: "sine", at: 0.01, dur: 0.1, freq: [300, 520], env: { a: 0.002, d: 0.035 }, gain: 0.45 },
        { wave: "sine", dur: 0.18, freq: [120, 60], env: { a: 0.001, d: 0.05 }, gain: 0.6 },
      ],
    },
  },
  {
    id: "coffee.clink",
    label: "Coffee break: clink",
    group: G,
    description: "A mug lands on a well's core stone",
    bus: "effects",
    gain: 0.6,
    priority: 6,
    refDistance: 14,
    space: 0.4,
    synth: {
      layers: [
        { wave: "modal", dur: 0.9, freq: 1760, modal: { ratios: [1, 2.32, 4.25, 6.63], decays: [0.6, 0.35, 0.2, 0.12] }, gain: 0.8 },
        { wave: "noise", dur: 0.03, filter: { type: "hp", freq: 4000 }, env: { a: 0.001, d: 0.008 }, gain: 0.4 },
      ],
    },
  },
  {
    id: "coffee.slurp",
    label: "Coffee break: slurp",
    group: G,
    description: "A well takes a sip of its coffee, then sighs",
    bus: "effects",
    gain: 0.7,
    priority: 6,
    refDistance: 14,
    space: 0.35,
    synth: {
      layers: [
        { wave: "noise", dur: 0.6, filter: { type: "bp", freq: [500, 2600], q: 3 }, am: { rate: 45, depth: 0.85, random: true }, env: { a: 0.04, h: 0.35, r: 0.15 }, gain: 0.9 },
        { wave: "sine", dur: 0.55, freq: [260, 700], am: { rate: 30, depth: 0.7, random: true }, env: { a: 0.03, h: 0.3, r: 0.15 }, gain: 0.25 },
        { wave: "saw", at: 0.7, dur: 0.75, freq: [180, 130], filter: { type: "bp", freq: [800, 500], q: 2 }, env: { a: 0.08, d: 0.3, r: 0.2 }, gain: 0.35 },
        { wave: "pink", at: 0.68, dur: 0.8, filter: { type: "bp", freq: 1000, q: 1 }, env: { a: 0.1, d: 0.35 }, gain: 0.3 },
      ],
    },
  },
  {
    id: "coffee.lounge",
    label: "Coffee break: lounge music (loop)",
    group: G,
    description: "While gravity is on its break: soft lounge music",
    bus: "music",
    loop: true,
    gain: 0.45,
    priority: 7,
    space: 0.5,
    synth: { loopFade: 0.15, layers: loungeLayers() },
  },
  {
    id: "genre.start",
    label: "Wrong genre: power on",
    group: G,
    description: "The genre switches: an old console powering on",
    bus: "effects",
    gain: 0.55,
    priority: 9,
    maxInstances: 1,
    space: 0.1,
    pitchJitter: 0,
    variants: 1,
    synth: {
      layers: [
        { wave: "square", duty: 0.5, dur: 0.35, freq: [110, 1760], sweepCurve: 0.7, filter: { type: "lp", freq: 6000 }, env: { a: 0.002, h: 0.3, r: 0.04 }, gain: 0.35 },
        chipNote(0.38, 72, 0.07, 0.4, 0.5),
        chipNote(0.45, 79, 0.07, 0.4, 0.5),
        chipNote(0.52, 84, 0.07, 0.4, 0.5),
        chipNote(0.59, 88, 0.25, 0.45, 0.5),
      ],
    },
  },
  {
    id: "genre.chiptune",
    label: "Wrong genre: chiptune (loop)",
    group: G,
    description: "While the match is a side-scroller: a bouncy chiptune",
    bus: "music",
    loop: true,
    gain: 0.5,
    priority: 7,
    space: 0.1,
    synth: { loopFade: 0.02, layers: chiptuneLayers() },
  },
  {
    id: "genre.oneup",
    label: "Wrong genre: 1-UP",
    group: G,
    description: "The genre switches back: a 1-UP jingle",
    bus: "effects",
    gain: 0.55,
    priority: 9,
    maxInstances: 1,
    space: 0.1,
    pitchJitter: 0,
    variants: 1,
    synth: {
      layers: [
        chipNote(0, 79, 0.08, 0.4),
        chipNote(0.09, 84, 0.08, 0.4),
        chipNote(0.18, 88, 0.08, 0.4),
        chipNote(0.27, 91, 0.08, 0.4),
        chipNote(0.36, 96, 0.08, 0.4),
        { ...chipNote(0.45, 100, 0.45, 0.42), vibrato: { rate: 9, depth: 0.3 } },
        { wave: "triangle", at: 0.45, dur: 0.45, freq: midi(60), env: { a: 0.002, d: 0.3 }, gain: 0.4 },
      ],
    },
  },
  {
    id: "bighead.inflate",
    label: "Big head day: inflate",
    group: G,
    description: "Balloon heads blown up: a rubbery, squeaky stretch",
    bus: "effects",
    gain: 0.6,
    priority: 6,
    maxInstances: 2,
    space: 0.3,
    synth: {
      layers: [
        { wave: "saw", dur: 0.38, freq: [260, 560], vibrato: { rate: 24, depth: 0.6 }, filter: { type: "bp", freq: [1100, 2200], q: 3 }, am: { rate: 70, depth: 0.6, random: true }, env: { a: 0.02, h: 0.25, r: 0.08 }, gain: 0.8 },
        { wave: "pink", dur: 0.36, filter: { type: "bp", freq: 1800, q: 1.2 }, env: { a: 0.05, h: 0.2, r: 0.08 }, gain: 0.35 },
      ],
    },
  },
  {
    id: "bighead.honk",
    label: "Big head day: honk",
    group: G,
    description: "A hit on a big head: a clown horn's honk",
    bus: "effects",
    gain: 0.75,
    priority: 7,
    maxInstances: 4,
    minGap: 0.06,
    refDistance: 12,
    space: 0.3,
    synth: {
      layers: [
        { wave: "saw", dur: 0.3, freq: [380, 340], filter: { type: "bp", freq: 950, q: 2.2 }, env: { a: 0.01, h: 0.12, d: 0.08, r: 0.04 }, drive: 1.5, gain: 0.9 },
        { wave: "square", duty: 0.35, dur: 0.3, freq: [570, 510], filter: { type: "bp", freq: 1600, q: 2 }, env: { a: 0.01, h: 0.12, d: 0.08, r: 0.04 }, gain: 0.35 },
        { wave: "pink", dur: 0.08, filter: { type: "hp", freq: 2000 }, env: { a: 0.002, d: 0.03 }, gain: 0.25 },
      ],
    },
  },
  {
    id: "bighead.pop",
    label: "Big head day: pop",
    group: G,
    description: "Big head day ends: every balloon bursts",
    bus: "effects",
    gain: 0.9,
    priority: 8,
    maxInstances: 4,
    refDistance: 16,
    space: 0.4,
    synth: {
      layers: [
        { wave: "noise", dur: 0.08, filter: { type: "hp", freq: 900 }, env: { a: 0.0005, d: 0.015 }, gain: 1 },
        { wave: "sine", dur: 0.14, freq: [180, 60], env: { a: 0.001, d: 0.04 }, gain: 0.6 },
        { wave: "crackle", density: 200, at: 0.02, dur: 0.25, filter: { type: "bp", freq: 2400, q: 1 }, env: { a: 0.005, d: 0.07 }, gain: 0.45 },
      ],
    },
  },
  {
    id: "chicken.squeak",
    label: "Rubber chickens: squeak",
    group: G,
    description: "A rubber chicken is fired: a squeaky rubber squeal",
    bus: "effects",
    gain: 0.55,
    priority: 5,
    maxInstances: 4,
    minGap: 0.05,
    refDistance: 9,
    space: 0.2,
    pitchJitter: 0.08,
    synth: {
      layers: [
        { wave: "square", duty: 0.3, dur: 0.2, freq: [950, 1550], sweepTime: 0.07, vibrato: { rate: 32, depth: 0.7 }, filter: { type: "bp", freq: 2100, q: 2.5 }, env: { a: 0.006, h: 0.09, d: 0.05, r: 0.03 }, gain: 0.8 },
        { wave: "saw", dur: 0.2, freq: [960, 1560], sweepTime: 0.07, filter: { type: "bp", freq: 3400, q: 3 }, env: { a: 0.006, h: 0.08, d: 0.05, r: 0.03 }, gain: 0.3 },
        { wave: "noise", dur: 0.16, filter: { type: "bp", freq: 2800, q: 1.5 }, env: { a: 0.01, d: 0.05 }, gain: 0.2 },
      ],
    },
  },
  {
    id: "chicken.squawk",
    label: "Rubber chickens: squawk",
    group: G,
    description: "A rubber chicken hits: a squawk and a puff of feathers",
    bus: "effects",
    gain: 0.65,
    priority: 6,
    maxInstances: 3,
    minGap: 0.05,
    refDistance: 11,
    space: 0.3,
    pitchJitter: 0.07,
    synth: {
      layers: [
        { wave: "square", duty: 0.35, dur: 0.48, freq: [1500, 620], sweepCurve: 0.6, vibrato: { rate: 26, depth: 1.1 }, filter: { type: "bp", freq: [2400, 1300], q: 2.2 }, am: { rate: 40, depth: 0.5, random: true }, env: { a: 0.004, h: 0.18, d: 0.15, r: 0.06 }, gain: 0.85 },
        { wave: "noise", dur: 0.3, filter: { type: "bp", freq: 1500, q: 0.8 }, env: { a: 0.002, d: 0.08 }, gain: 0.45 },
        { wave: "sine", dur: 0.12, freq: [200, 90], env: { a: 0.001, d: 0.04 }, gain: 0.5 },
      ],
    },
  },
  {
    id: "chicken.egg",
    label: "Rubber chickens: giant egg",
    group: G,
    description: "The Tectonic Charge's giant egg cracks open",
    bus: "effects",
    gain: 0.85,
    priority: 8,
    maxInstances: 2,
    refDistance: 18,
    space: 0.4,
    synth: {
      layers: [
        { wave: "modal", dur: 0.5, freq: 900, modal: { ratios: [1, 1.7, 2.9, 4.1], decays: [0.25, 0.15, 0.1, 0.06] }, gain: 0.7 },
        { wave: "crackle", density: 400, dur: 0.18, filter: { type: "bp", freq: 3000, q: 1.2 }, env: { a: 0.001, d: 0.05 }, gain: 0.7 },
        { wave: "noise", at: 0.08, dur: 0.35, filter: { type: "lp", freq: [1800, 300] }, env: { a: 0.005, d: 0.12 }, gain: 0.6 },
        { wave: "square", duty: 0.35, at: 0.12, dur: 0.6, freq: [1300, 500], vibrato: { rate: 22, depth: 1.2 }, filter: { type: "bp", freq: 1800, q: 2 }, env: { a: 0.01, h: 0.25, d: 0.2 }, gain: 0.5 },
      ],
    },
  },
  {
    id: "moving.tape",
    label: "Moving day: packing tape",
    group: G,
    description: "Boxes taped shut: the screech of packing tape and a rip",
    bus: "effects",
    gain: 0.7,
    priority: 6,
    maxInstances: 2,
    refDistance: 14,
    space: 0.3,
    synth: {
      layers: [
        { wave: "noise", dur: 0.65, filter: { type: "bp", freq: [1800, 3600], q: 2.5 }, am: { rate: 90, depth: 0.8, random: true }, env: { a: 0.02, h: 0.5, r: 0.06 }, gain: 0.85 },
        { wave: "saw", dur: 0.62, freq: [820, 1150], filter: { type: "bp", freq: 2600, q: 5 }, am: { rate: 60, depth: 0.7, random: true }, env: { a: 0.03, h: 0.45, r: 0.06 }, gain: 0.25 },
        { wave: "noise", at: 0.66, dur: 0.07, filter: { type: "hp", freq: 2500 }, env: { a: 0.001, d: 0.02 }, gain: 0.8 },
      ],
    },
  },
  {
    id: "moving.hop",
    label: "Moving day: boing",
    group: G,
    description: "The boxes hop off to their new places: a springy boing",
    bus: "effects",
    gain: 0.6,
    priority: 6,
    maxInstances: 2,
    refDistance: 14,
    space: 0.3,
    synth: {
      layers: [
        { wave: "triangle", dur: 0.45, freq: [190, 760], sweepTime: 0.2, vibrato: { rate: 16, depth: 2.2 }, env: { a: 0.004, d: 0.22 }, gain: 0.9 },
        { wave: "sine", dur: 0.45, freq: [95, 380], sweepTime: 0.2, vibrato: { rate: 16, depth: 2 }, env: { a: 0.004, d: 0.2 }, gain: 0.4 },
      ],
    },
  },
  {
    id: "moving.thump",
    label: "Moving day: thump",
    group: G,
    description: "A box lands at its new place",
    bus: "effects",
    gain: 0.8,
    priority: 6,
    maxInstances: 2,
    refDistance: 14,
    space: 0.35,
    synth: {
      layers: [
        { wave: "sine", dur: 0.3, freq: [110, 48], env: { a: 0.001, d: 0.09 }, gain: 1 },
        { wave: "noise", dur: 0.25, filter: { type: "lp", freq: 700 }, env: { a: 0.001, d: 0.06 }, gain: 0.7 },
        { wave: "modal", dur: 0.25, freq: 210, modal: { ratios: [1, 1.6, 2.3], decays: [0.12, 0.08, 0.05] }, gain: 0.3 },
      ],
    },
  },
  {
    id: "moving.pop",
    label: "Moving day: flaps",
    group: G,
    description: "The boxes open: cardboard flaps flipping",
    bus: "effects",
    gain: 0.55,
    priority: 5,
    maxInstances: 2,
    refDistance: 12,
    space: 0.3,
    synth: {
      layers: [
        { wave: "noise", dur: 0.06, filter: { type: "bp", freq: 900, q: 1.4 }, env: { a: 0.002, d: 0.02 }, repeat: { count: 2, every: 0.09 }, gain: 0.9 },
        { wave: "sine", dur: 0.12, freq: [500, 900], env: { a: 0.002, d: 0.04 }, gain: 0.3 },
      ],
    },
  },
  {
    id: "cake.horn",
    label: "Everything is cake: party horn",
    group: G,
    description: "The terrain turns to cake: a party horn and confetti",
    bus: "effects",
    gain: 0.75,
    priority: 9,
    maxInstances: 1,
    space: 0.3,
    pitchJitter: 0.02,
    synth: {
      layers: [
        { wave: "saw", dur: 1.0, freq: [430, 545], sweepTime: 0.15, vibrato: { rate: 7, depth: 0.35 }, fm: { ratio: 2, index: 0.8 }, filter: { type: "bp", freq: 1500, q: 1.2 }, am: { rate: 35, depth: 0.25, random: true }, env: { a: 0.02, h: 0.7, d: 0.15, r: 0.12 }, drive: 1.2, gain: 0.85 },
        { wave: "saw", dur: 1.0, freq: [433, 548], sweepTime: 0.15, filter: { type: "bp", freq: 3000, q: 2 }, env: { a: 0.02, h: 0.7, d: 0.15, r: 0.12 }, gain: 0.3 },
        { wave: "crackle", density: 120, dur: 0.5, filter: { type: "bp", freq: 4000, q: 0.8 }, env: { a: 0.01, d: 0.18 }, gain: 0.4 },
        { wave: "noise", at: 0.95, dur: 0.1, filter: { type: "bp", freq: 2500, q: 1 }, env: { a: 0.001, d: 0.03 }, gain: 0.5 },
      ],
    },
  },
];
