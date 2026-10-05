// Weapon fire sounds, one per visual shape (data/weapons `visual.shape`),
// grouped by family: energy (bolt, orb, arc), kinetic (pellet, spike,
// harpoon), ordnance (rocket, missile, lob for mines, grenades and the
// Tectonic Charge) and specials (blob = Magnet Sneeze, paint). The game
// plays them a little lower for bigger shots (visual scale), so one sound
// serves every weapon of a shape (game.ts `fireSound`).

import type { SoundDef } from "../library.ts";

const G = "Weapons: fire";

export const WEAPON_SOUNDS: readonly SoundDef[] = [
  {
    id: "fire.bolt",
    label: "Energy bolt",
    group: G,
    description: "Scrap Laser, Scatter Laser, Corkscrew, Polite Laser: a bright zap that drops in pitch",
    bus: "effects",
    gain: 0.5,
    priority: 4,
    maxInstances: 6,
    minGap: 0.03,
    space: 0.18,
    synth: {
      layers: [
        { wave: "square", duty: 0.3, dur: 0.16, freq: [1700, 240], sweepCurve: 0.55, env: { a: 0.001, d: 0.05 }, filter: { type: "lp", freq: [7000, 1400] }, gain: 0.55 },
        { wave: "sine", dur: 0.2, freq: [950, 170], sweepCurve: 0.6, fm: { ratio: 2.01, index: [3.5, 0] }, env: { a: 0.001, d: 0.06 }, gain: 0.6 },
        { wave: "noise", dur: 0.03, env: { a: 0.0005, d: 0.006 }, filter: { type: "hp", freq: 3200 }, gain: 0.35 },
      ],
    },
  },
  {
    id: "fire.orb",
    label: "Plasma orb",
    group: G,
    description: "Plasma Repeater, Fusion Charger, Echo Cannon, Borrowed Time: a warbling, wet launch with a soft sub thump",
    bus: "effects",
    gain: 0.55,
    priority: 4,
    maxInstances: 6,
    minGap: 0.04,
    space: 0.25,
    synth: {
      layers: [
        { wave: "saw", dur: 0.34, freq: [440, 130], sweepCurve: 0.5, vibrato: { rate: 28, depth: 1.4 }, filter: { type: "lp", freq: [3200, 500], q: 4 }, env: { a: 0.003, d: 0.12 }, gain: 0.6 },
        { wave: "sine", dur: 0.26, freq: [170, 55], env: { a: 0.002, d: 0.08 }, gain: 0.85 },
        { wave: "noise", dur: 0.1, filter: { type: "bp", freq: 1300, q: 2 }, env: { a: 0.001, d: 0.03 }, gain: 0.3 },
      ],
    },
  },
  {
    id: "fire.arc",
    label: "Electric arc",
    group: G,
    description: "Leech Arc and Ricochet Arc: a crackling electric buzz",
    bus: "effects",
    gain: 0.5,
    priority: 4,
    maxInstances: 5,
    minGap: 0.05,
    space: 0.2,
    synth: {
      layers: [
        { wave: "crackle", density: 3200, dur: 0.36, filter: { type: "bp", freq: 3600, q: 1.1 }, env: { a: 0.002, d: 0.14 }, gain: 0.9 },
        { wave: "saw", dur: 0.36, freq: [92, 84], am: { rate: 70, depth: 0.9, random: true }, filter: { type: "lp", freq: 2600 }, env: { a: 0.002, d: 0.15 }, gain: 0.5 },
        { wave: "sine", dur: 0.12, freq: [2600, 1100], fm: { ratio: 1.5, index: 4 }, env: { a: 0.001, d: 0.04 }, gain: 0.22 },
      ],
    },
  },
  {
    id: "fire.pellet",
    label: "Kinetic round",
    group: G,
    description: "Rivet Gun and Scrap Shotgun: a hard crack with a metallic click and a thump",
    bus: "effects",
    gain: 0.55,
    priority: 4,
    maxInstances: 6,
    minGap: 0.035,
    space: 0.25,
    synth: {
      layers: [
        { wave: "noise", dur: 0.09, env: { a: 0.0004, d: 0.018 }, filter: { type: "hp", freq: 1400 }, filter2: { type: "lp", freq: 9000 }, gain: 0.95 },
        { wave: "sine", dur: 0.13, freq: [150, 48], sweepCurve: 0.5, env: { a: 0.0008, d: 0.035 }, drive: 1.5, gain: 1 },
        { wave: "modal", dur: 0.12, freq: 1850, modal: { ratios: [1, 2.31, 3.93], decays: [0.09, 0.06, 0.035] }, gain: 0.28 },
      ],
    },
  },
  {
    id: "fire.spike",
    label: "Railspike",
    group: G,
    description: "Railspike: a supersonic crack, a falling zap and the rail ringing out (heard map-wide: it reveals the shooter)",
    bus: "effects",
    gain: 0.85,
    priority: 8,
    maxInstances: 3,
    minGap: 0.05,
    refDistance: 16,
    space: 0.5,
    synth: {
      layers: [
        { wave: "noise", dur: 0.28, env: { a: 0.0003, d: 0.035 }, filter: { type: "lp", freq: [13000, 700], curve: 0.6 }, gain: 1 },
        { wave: "saw", dur: 0.48, freq: [4400, 85], sweepCurve: 0.32, env: { a: 0.0005, d: 0.13 }, filter: { type: "lp", freq: [9500, 380] }, gain: 0.55 },
        { wave: "modal", dur: 1.5, freq: 520, modal: { ratios: [1, 2.76, 5.4, 8.93, 13.3], decays: [1.5, 1.05, 0.65, 0.42, 0.3] }, gain: 0.32 },
        { wave: "sine", dur: 0.42, freq: [95, 34], env: { a: 0.001, d: 0.11 }, drive: 1, gain: 0.95 },
      ],
    },
  },
  {
    id: "fire.harpoon",
    label: "Harpoon",
    group: G,
    description: "Swap Harpoon: a taut cable twang and a whoosh",
    bus: "effects",
    gain: 0.6,
    priority: 5,
    space: 0.25,
    synth: {
      layers: [
        { wave: "pluck", dur: 1.0, freq: 98, pluck: { decay: 0.9, bright: 0.62 }, gain: 0.8 },
        { wave: "pluck", dur: 0.8, freq: 147.5, pluck: { decay: 0.6, bright: 0.5 }, gain: 0.35 },
        { wave: "noise", dur: 0.32, filter: { type: "bp", freq: [2200, 600], q: 1.6 }, env: { a: 0.004, d: 0.1 }, gain: 0.4 },
        { wave: "sine", dur: 0.1, freq: [190, 60], env: { a: 0.001, d: 0.03 }, gain: 0.6 },
      ],
    },
  },
  {
    id: "fire.rocket",
    label: "Rocket launch",
    group: G,
    description: "Bolt Rocket, Heavy Rocket, Swarm Pod, Quicksilver; Seeker and Eyeball a little higher: a launch thump and a rising whoosh",
    bus: "effects",
    gain: 0.75,
    priority: 6,
    maxInstances: 6,
    minGap: 0.04,
    space: 0.35,
    synth: {
      layers: [
        { wave: "noise", dur: 0.75, filter: { type: "bp", freq: [480, 2600], q: 1.3, time: 0.5 }, env: { a: 0.012, d: 0.24 }, gain: 1 },
        { wave: "sine", dur: 0.26, freq: [115, 44], env: { a: 0.001, d: 0.07 }, drive: 1.2, gain: 0.95 },
        { wave: "noise", dur: 0.95, filter: { type: "lp", freq: 850 }, env: { a: 0.04, h: 0.08, d: 0.3 }, gain: 0.55 },
        { wave: "crackle", density: 900, dur: 0.6, filter: { type: "bp", freq: 2400, q: 0.8 }, env: { a: 0.01, d: 0.2 }, gain: 0.25 },
      ],
    },
  },
  {
    id: "fire.lob",
    label: "Lobbed charge",
    group: G,
    description: "Prox Mine, Stalker Mine, Seed Grenade, Flash Pod, Tectonic Charge: a hollow launcher thoonk",
    bus: "effects",
    gain: 0.7,
    priority: 5,
    space: 0.3,
    synth: {
      layers: [
        { wave: "sine", dur: 0.18, freq: [260, 68], sweepCurve: 0.6, env: { a: 0.001, d: 0.05 }, drive: 1, gain: 1 },
        { wave: "noise", dur: 0.05, filter: { type: "bp", freq: 900, q: 1 }, env: { a: 0.0005, d: 0.01 }, gain: 0.5 },
        { wave: "triangle", dur: 0.09, freq: [640, 300], env: { a: 0.001, d: 0.02 }, gain: 0.22 },
        { wave: "modal", dur: 0.3, freq: 380, modal: { ratios: [1, 1.71, 2.53], decays: [0.25, 0.15, 0.1] }, gain: 0.16 },
      ],
    },
  },
  {
    id: "fire.paint",
    label: "Paint glob",
    group: G,
    description: "Paint Thrower: a wet splort and a rising bloop",
    bus: "effects",
    gain: 0.5,
    priority: 4,
    maxInstances: 5,
    minGap: 0.04,
    space: 0.2,
    synth: {
      layers: [
        { wave: "noise", dur: 0.2, filter: { type: "lp", freq: [3200, 380], q: 6 }, env: { a: 0.002, d: 0.06 }, gain: 0.85 },
        { wave: "sine", dur: 0.11, freq: [280, 900], fm: { ratio: 1, index: 2 }, env: { a: 0.002, d: 0.04 }, gain: 0.5 },
      ],
    },
  },
  {
    id: "fire.sneeze",
    label: "Magnet Sneeze",
    group: G,
    description: "Magnet Sneeze: a breathy \"hah…\" and a sharp \"CHOO\"",
    bus: "effects",
    gain: 0.7,
    priority: 6,
    space: 0.25,
    pitchJitter: 0.06,
    synth: {
      layers: [
        // "hah": breath through an open vowel, rising.
        { wave: "noise", dur: 0.34, filter: { type: "bp", freq: [800, 1350], q: 5 }, env: { a: 0.22, r: 0.08 }, gain: 0.5 },
        { wave: "saw", dur: 0.34, freq: [210, 300], filter: { type: "bp", freq: [750, 1200], q: 6 }, env: { a: 0.2, r: 0.08 }, gain: 0.25 },
        // "CHOO": a fricative burst, then a short rounded vowel.
        { wave: "noise", at: 0.38, dur: 0.16, filter: { type: "hp", freq: 2600 }, env: { a: 0.002, d: 0.05 }, gain: 1 },
        { wave: "saw", at: 0.41, dur: 0.2, freq: [330, 200], filter: { type: "bp", freq: 650, q: 5 }, env: { a: 0.004, d: 0.07 }, gain: 0.7 },
        { wave: "noise", at: 0.41, dur: 0.2, filter: { type: "bp", freq: [1100, 600], q: 4 }, env: { a: 0.004, d: 0.07 }, gain: 0.45 },
      ],
    },
  },
];
