// The announcer's gibberish voice (M4 A6, milestone-4-plan.md §3.7): a small
// formant synthesizer that "speaks" a caption line as made-up syllables
// shaped by its letters. Each vowel group of a word becomes a syllable whose
// vowel colour comes from its first vowel letter and whose onset is a
// consonant sound of the letter before it (plosive burst, hiss, nasal hum,
// glide). Words keep their rhythm, punctuation adds pauses, CAPS and "!"
// words are stressed, and the voice pack's parameters (pitch, range, pace,
// melody, breath, formant size) give each announcer a personality. The same
// line in the same pack always sounds the same (seeded by the words).
//
// Pure maths like dsp.ts; returns the samples and each word's time span so
// the caption (ui/announcer.ts) can reveal words as they are spoken.

import { rng32 } from "./dsp.ts";

export interface VoiceParams {
  /** Base pitch (Hz). */
  pitch: number;
  /** Random pitch movement between syllables (semitones). */
  range: number;
  /** Syllables per second. */
  rate: number;
  /** Pitch drift over the phrase (semitones; positive falls, negative rises). */
  declination: number;
  /** Melody of the last word: fall (statement), rise (question, eager), singsong (rise-fall, sweetly), flat (monotone). */
  ending: "fall" | "rise" | "singsong" | "flat";
  /** Breathiness 0..1 (aspiration noise mixed into the voice). */
  breath: number;
  /** Formant scale (1 = adult, 1.2 = small cartoon throat). */
  formant: number;
  /** Vibrato depth (semitones) and rate (Hz). */
  vibrato: number;
  vibratoRate: number;
  /** Punctuation pause multiplier. */
  pause: number;
  /** Random syllable length spread (fraction). */
  jitter: number;
  /** Source brightness 0..1 (how buzzy the voice is). */
  bright: number;
  /** Extra pitch of stressed words (semitones). */
  emphasis: number;
  /** Pitch glide time between syllables (s). */
  glide: number;
}

export interface Speech {
  samples: Float32Array;
  /** Start and end (s) of each word of the line, in order. */
  words: { text: string; start: number; end: number }[];
  duration: number;
}

type Formants = readonly [number, number, number];

/** Vowel formants (Hz, F1-F3) of an adult voice. */
const VOWELS: Readonly<Record<string, Formants>> = {
  a: [730, 1090, 2440],
  e: [530, 1840, 2480],
  i: [300, 2200, 2950],
  o: [570, 840, 2410],
  u: [330, 900, 2240],
  y: [300, 2100, 2900],
};

type ConsonantKind = "plosive" | "voicedPlosive" | "sibilant" | "hush" | "fricative" | "aspirate" | "nasal" | "glide";

interface Consonant {
  kind: ConsonantKind;
  /** Noise colour: filter centre (Hz). */
  noise: number;
  /** Formants for voiced consonants. */
  formants?: Formants;
  /** Length (s). */
  len: number;
}

const CONSONANTS: Readonly<Record<string, Consonant>> = {
  p: { kind: "plosive", noise: 900, len: 0.05 },
  b: { kind: "voicedPlosive", noise: 800, len: 0.04 },
  t: { kind: "plosive", noise: 4200, len: 0.05 },
  d: { kind: "voicedPlosive", noise: 3500, len: 0.04 },
  k: { kind: "plosive", noise: 2100, len: 0.055 },
  c: { kind: "plosive", noise: 2100, len: 0.055 },
  q: { kind: "plosive", noise: 1900, len: 0.055 },
  g: { kind: "voicedPlosive", noise: 1900, len: 0.045 },
  s: { kind: "sibilant", noise: 6200, len: 0.075 },
  z: { kind: "sibilant", noise: 5500, len: 0.06 },
  x: { kind: "sibilant", noise: 5000, len: 0.08 },
  j: { kind: "hush", noise: 2600, len: 0.07 },
  f: { kind: "fricative", noise: 3200, len: 0.065 },
  v: { kind: "fricative", noise: 2800, len: 0.05 },
  h: { kind: "aspirate", noise: 1500, len: 0.05 },
  m: { kind: "nasal", noise: 0, formants: [280, 1100, 2300], len: 0.065 },
  n: { kind: "nasal", noise: 0, formants: [280, 1600, 2500], len: 0.06 },
  l: { kind: "glide", noise: 0, formants: [360, 1300, 2700], len: 0.05 },
  r: { kind: "glide", noise: 0, formants: [420, 1300, 1650], len: 0.055 },
  w: { kind: "glide", noise: 0, formants: [300, 700, 2200], len: 0.05 },
};

const VOWEL_LETTERS = "aeiouy";

/** A piece of the timeline. */
interface Segment {
  start: number;
  len: number;
  /** Voicing level 0..1. */
  voice: number;
  /** Noise level 0..1 and its filter. */
  noise: number;
  noiseFreq: number;
  noiseType: "hp" | "bp" | "formant";
  formants: Formants;
  /** Pitch (semitones over the base) at the segment's start. */
  semis: number;
  /** Loudness 0..1. */
  level: number;
  /** Burst: noise only in the first 12 ms (plosive release). */
  burst?: boolean;
}

interface Syllable {
  onset: string;
  vowel: string;
}

/** Splits a word into syllables (vowel groups) with the consonant before each. */
export function syllables(word: string): Syllable[] {
  const w = word.toLowerCase().replace(/[^a-z0-9]/g, "");
  if (/^\d+$/.test(w)) {
    // Numbers: one syllable per digit pair, "n" onsets ("twenty-ish").
    return Array.from({ length: Math.min(4, Math.max(1, Math.ceil(w.length / 1.5))) }, (_, i) => ({ onset: i % 2 ? "t" : "n", vowel: i % 2 ? "e" : "i" }));
  }
  const out: Syllable[] = [];
  let i = 0;
  let onset = "";
  while (i < w.length) {
    const ch = w[i] as string;
    if (VOWEL_LETTERS.includes(ch) && !(ch === "y" && i === 0)) {
      out.push({ onset, vowel: ch });
      onset = "";
      while (i < w.length && VOWEL_LETTERS.includes(w[i] as string)) i++;
      continue;
    }
    onset = ch;
    i++;
  }
  if (out.length === 0) out.push({ onset: w[0] ?? "m", vowel: "u" });
  // Silent final e ("rate"): merge into the previous syllable.
  if (out.length > 1 && w.endsWith("e") && out[out.length - 1]?.vowel === "e" && !/[aeiouy]e$/.test(w)) out.pop();
  return out.slice(0, 4);
}

/** Small string hash (seeds a word's pitch pattern so it repeats). */
function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

/** Splits a line into words with the punctuation that follows each. */
export function words(line: string): { text: string; punct: string }[] {
  const out: { text: string; punct: string }[] = [];
  const re = /([A-Za-z0-9'’]+)([^A-Za-z0-9'’]*)/g;
  for (let m = re.exec(line); m; m = re.exec(line)) out.push({ text: m[1] ?? "", punct: (m[2] ?? "").trim() });
  return out;
}

const PAUSES: Readonly<Record<string, number>> = { ",": 0.13, ";": 0.18, ":": 0.18, ".": 0.24, "!": 0.22, "?": 0.24, "…": 0.38, "...": 0.38, "-": 0.08, "—": 0.15 };

function pauseFor(punct: string): number {
  if (!punct) return 0;
  if (punct.includes("…") || punct.includes("...")) return PAUSES["…"] as number;
  let p = 0;
  for (const ch of punct) p = Math.max(p, PAUSES[ch] ?? 0);
  return p;
}

const scaleF = (f: Formants, k: number): Formants => [f[0] * k, f[1] * k, f[2] * k];

/** Lays out the line as segments and word spans. */
function plan(line: string, v: VoiceParams, rnd: () => number): { segs: Segment[]; spans: Speech["words"]; end: number } {
  const ws = words(line);
  const segs: Segment[] = [];
  const spans: Speech["words"] = [];
  const syl = 1 / Math.max(1, v.rate);
  let t = 0.04;
  const total = ws.reduce((n, w) => n + syllables(w.text).length, 0);
  let k = 0;
  ws.forEach((w, wi) => {
    const stressed = (w.text.length > 1 && w.text === w.text.toUpperCase() && /[A-Z]/.test(w.text)) || w.punct.includes("!");
    const last = wi === ws.length - 1 || /[.!?…]/.test(w.punct);
    const wr = rng32(hash(w.text.toLowerCase()) ^ 0x5bd1e995);
    const ss = syllables(w.text);
    const start = t;
    ss.forEach((s, si) => {
      const progress = total > 1 ? k / (total - 1) : 0;
      let semis = -v.declination * progress + (wr() * 2 - 1) * v.range * 0.5 + (si === 0 ? v.range * 0.2 : 0);
      if (stressed) semis += v.emphasis;
      if (last) {
        const p = ss.length > 1 ? si / (ss.length - 1) : 1;
        if (v.ending === "fall") semis -= 2.5 * p + 1;
        else if (v.ending === "rise") semis += 4 * p + 1;
        else if (v.ending === "singsong") semis += si === 0 ? 3.5 : -1.5;
        else semis = -v.declination * 0.8;
      }
      const level = (si === 0 ? 1 : 0.8) * (stressed ? 1 : 0.85);
      const vowel = scaleF(VOWELS[s.vowel] ?? (VOWELS["a"] as Formants), v.formant);
      const c = CONSONANTS[s.onset];
      if (c) {
        const len = c.len * (0.8 + 0.4 * rnd());
        const voiced = c.kind === "nasal" || c.kind === "glide" || c.kind === "voicedPlosive";
        const f = c.formants ? scaleF(c.formants, v.formant) : vowel;
        if (c.kind === "plosive" || c.kind === "voicedPlosive") {
          // Closure (silence or a murmur), then the burst.
          segs.push({ start: t, len: len * 0.6, voice: c.kind === "voicedPlosive" ? 0.25 : 0, noise: 0, noiseFreq: c.noise, noiseType: "bp", formants: [250, 800, 2200], semis, level });
          segs.push({ start: t + len * 0.6, len: len * 0.4, voice: c.kind === "voicedPlosive" ? 0.4 : 0, noise: 0.9, noiseFreq: c.noise, noiseType: c.noise > 3000 ? "hp" : "bp", formants: vowel, semis, level, burst: true });
        } else if (c.kind === "aspirate") {
          segs.push({ start: t, len, voice: 0, noise: 0.7, noiseFreq: c.noise, noiseType: "formant", formants: vowel, semis, level });
        } else if (!voiced) {
          segs.push({ start: t, len, voice: c.kind === "fricative" && s.onset === "v" ? 0.3 : 0, noise: c.kind === "fricative" ? 0.35 : 0.6, noiseFreq: c.noise, noiseType: c.kind === "hush" ? "bp" : "hp", formants: vowel, semis, level });
        } else {
          segs.push({ start: t, len, voice: c.kind === "nasal" ? 0.6 : 0.8, noise: 0, noiseFreq: 0, noiseType: "bp", formants: f, semis, level });
        }
        t += len;
      }
      const vl = syl * (0.75 + v.jitter * (rnd() * 2 - 1)) * (stressed ? 1.15 : 1) * (last && si === ss.length - 1 ? 1.35 : 1);
      segs.push({ start: t, len: vl, voice: 1, noise: v.breath * 0.25, noiseFreq: 0, noiseType: "formant", formants: vowel, semis, level });
      t += vl;
      k++;
    });
    spans.push({ text: w.text, start, end: t });
    t += 0.035 + pauseFor(w.punct) * v.pause;
  });
  return { segs, spans, end: t + 0.12 };
}

/** One biquad band-pass (constant peak gain) with a settable centre. */
class BandPass {
  private b0 = 0;
  private b2 = 0;
  private a1 = 0;
  private a2 = 0;
  private x1 = 0;
  private x2 = 0;
  private y1 = 0;
  private y2 = 0;
  set(f: number, q: number, sr: number, type: "bp" | "hp" = "bp"): void {
    const w = (2 * Math.PI * Math.min(sr * 0.45, Math.max(40, f))) / sr;
    const alpha = Math.sin(w) / (2 * q);
    const a0 = 1 + alpha;
    const cw = Math.cos(w);
    if (type === "bp") {
      this.b0 = alpha / a0;
      this.b2 = -alpha / a0;
      this.b1 = 0;
    } else {
      this.b0 = (1 + cw) / 2 / a0;
      this.b1 = -(1 + cw) / a0;
      this.b2 = this.b0;
    }
    this.a1 = (-2 * cw) / a0;
    this.a2 = (1 - alpha) / a0;
  }
  private b1 = 0;
  run(x: number): number {
    const y = this.b0 * x + this.b1 * this.x1 + this.b2 * this.x2 - this.a1 * this.y1 - this.a2 * this.y2;
    this.x2 = this.x1;
    this.x1 = x;
    this.y2 = this.y1;
    this.y1 = Math.abs(y) < 1e-20 ? 0 : y;
    return this.y1;
  }
}

/** Renders `line` in `voice` at `sr`; `seed` varies breath noise and syllable lengths. */
export function renderSpeech(line: string, voice: VoiceParams, sr: number, seed = 1): Speech {
  const rnd = rng32(seed * 7919 + hash(line));
  const { segs, spans, end } = plan(line, voice, rnd);
  const n = Math.max(1, Math.ceil(end * sr));
  const out = new Float32Array(n);
  const f1 = new BandPass();
  const f2 = new BandPass();
  const f3 = new BandPass();
  const nf = new BandPass();
  const bw = [90, 110, 150];
  // Smoothed parameters (one-pole toward the current segment's targets).
  let F: [number, number, number] = [...(segs[0]?.formants ?? [500, 1500, 2500])] as [number, number, number];
  let semis = segs[0]?.semis ?? 0;
  let voiceAmp = 0;
  let noiseAmp = 0;
  let nfreq = 2000;
  let phase = 0;
  let lp = 0;
  let si = 0;
  const kF = 1 - Math.exp(-1 / (sr * 0.022));
  const kP = 1 - Math.exp(-1 / (sr * Math.max(0.005, voice.glide)));
  const kA = 1 - Math.exp(-1 / (sr * 0.008));
  const STEP = 32;
  for (let i = 0; i < n; i++) {
    const t = i / sr;
    while (si < segs.length - 1 && t >= (segs[si] as Segment).start + (segs[si] as Segment).len) si++;
    const s = segs[si] as Segment;
    const inSeg = t >= s.start && t < s.start + s.len;
    const tv = inSeg ? s.voice * s.level : 0;
    let tn = inSeg ? s.noise * s.level : 0;
    if (inSeg && s.burst && t - s.start > 0.012) tn *= 0.15;
    voiceAmp += (tv - voiceAmp) * kA;
    noiseAmp += (tn - noiseAmp) * (s.burst ? 0.5 : kA);
    semis += (s.semis - semis) * kP;
    for (let j = 0; j < 3; j++) F[j] = (F[j] as number) + ((s.formants[j] as number) - (F[j] as number)) * kF;
    if (s.noiseFreq > 0) nfreq = s.noiseFreq;
    if (i % STEP === 0) {
      f1.set(F[0], F[0] / bw[0]!, sr);
      f2.set(F[1], F[1] / bw[1]!, sr);
      f3.set(F[2], F[2] / bw[2]!, sr);
      nf.set(nfreq, s.noiseType === "hp" ? 0.7 : 2.2, sr, s.noiseType === "hp" ? "hp" : "bp");
    }
    // Glottal source: a band-limited saw, darkened by a one-pole low-pass.
    const vib = voice.vibrato * Math.sin(2 * Math.PI * voice.vibratoRate * t);
    const f0 = voice.pitch * Math.pow(2, (semis + vib) / 12);
    const dt = f0 / sr;
    phase += dt;
    if (phase >= 1) phase -= 1;
    let saw = 2 * phase - 1;
    if (phase < dt) {
      const x = phase / dt;
      saw -= x + x - x * x - 1;
    } else if (phase > 1 - dt) {
      const x = (phase - 1) / dt;
      saw -= x * x + x + x + 1;
    }
    lp += (saw - lp) * (0.12 + 0.5 * voice.bright);
    const white = rnd() * 2 - 1;
    const src = lp * voiceAmp + white * (voice.breath * 0.35 * voiceAmp + (s.noiseType === "formant" ? noiseAmp : 0));
    let y = f1.run(src) * 1.0 + f2.run(src) * 0.55 + f3.run(src) * 0.3;
    if (s.noiseType !== "formant") y += nf.run(white) * noiseAmp * 0.5;
    out[i] = y;
  }
  // Normalize and a gentle broadcast saturation.
  let max = 0;
  for (let i = 0; i < n; i++) max = Math.max(max, Math.abs(out[i] ?? 0));
  const g = max > 1e-9 ? 1.6 / max : 0;
  for (let i = 0; i < n; i++) out[i] = Math.tanh((out[i] ?? 0) * g) * 0.82;
  return { samples: out, words: spans, duration: n / sr };
}
