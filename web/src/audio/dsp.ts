// Procedural sound synthesis (M4 A6, milestone-4-plan.md §3.7): renders a
// sound definition (sounds/*.ts, sfxr-style parameter sets) into a mono
// Float32Array. Pure maths, no WebAudio: the engine (engine.ts) copies the
// samples into AudioBuffers, Node tests and `web/scripts/audio-preview.ts`
// render them directly. Each layer is one source (oscillator, noise, plucked
// string, modal resonator) with its own pitch sweep, envelope, filter sweep,
// FM, tremolo and saturation; layers are summed, DC-filtered and normalized
// to the definition's peak.
//
// Random parts (noise, jitter, grain timing) come from a seeded RNG, so a
// sound's variants are repeatable (variant k = seed k).

/** Oscillator shapes. `pluck` is a Karplus-Strong string, `modal` a struck metal body (inharmonic partials). */
export type Wave = "sine" | "triangle" | "saw" | "square" | "noise" | "pink" | "brown" | "crackle" | "pluck" | "modal";

/** A value that may sweep: a constant, or [from, to] (exponential for frequencies). */
export type Sweep = number | readonly [number, number];

export interface Envelope {
  /** Attack (s, linear rise). */
  a?: number;
  /** Hold at full level after the attack (s). */
  h?: number;
  /** Decay time constant toward `s` (s); 0 or absent = no decay. */
  d?: number;
  /** Sustain level 0..1 the decay settles at [0 when `d` is set, else 1]. */
  s?: number;
  /** Release fade at the end of the layer (s, raised cosine) [0.005]. */
  r?: number;
}

export interface FilterDef {
  type: "lp" | "hp" | "bp" | "notch" | "peak";
  /** Cutoff / centre (Hz), or a sweep over `time`. */
  freq: Sweep;
  /** Resonance [0.707]. */
  q?: number;
  /** Peak gain (dB, `peak` only) [6]. */
  gain?: number;
  /** Sweep time (s) [the layer's duration]. */
  time?: number;
  /** Sweep curve exponent (1 = exponential in log frequency; >1 slow start, <1 fast start) [1]. */
  curve?: number;
}

export interface Layer {
  wave: Wave;
  /** Length of the layer (s). */
  dur: number;
  /** Start offset (s) [0]. */
  at?: number;
  /** Linear gain [1]. */
  gain?: number;
  /** Frequency (Hz) or a sweep; ignored by noise. */
  freq?: Sweep;
  /** Pitch sweep time (s) [dur]. */
  sweepTime?: number;
  /** Pitch sweep curve (1 = exponential; >1 slow start, <1 fast start) [1]. */
  sweepCurve?: number;
  /** Random pitch per variant (fraction, e.g. 0.05 = ±5 %) [0]. */
  jitter?: number;
  /** Square duty 0..1 [0.5]. */
  duty?: number;
  /** Vibrato: rate (Hz) and depth (semitones). */
  vibrato?: { rate: number; depth: number };
  /** FM: a sine modulator at ratio x carrier, index (radians, may sweep). */
  fm?: { ratio: number; index: Sweep };
  /** Tremolo: rate (Hz), depth 0..1; `random` makes it a jittery flutter. */
  am?: { rate: number; depth: number; random?: boolean };
  env?: Envelope;
  filter?: FilterDef;
  /** Second filter in series. */
  filter2?: FilterDef;
  /** tanh drive (1 = gentle, 4 = crunchy) [0 = clean]. */
  drive?: number;
  /** Crackle density (impulses per second) for `crackle` [400]. */
  density?: number;
  /** Plucked string: decay (s to -60 dB) and brightness 0..1. */
  pluck?: { decay: number; bright: number };
  /** Modal body: partial ratios, their decay times (s) and gains. */
  modal?: { ratios: readonly number[]; decays: readonly number[]; gains?: readonly number[] };
  /** Repeat the layer: `count` copies `every` s apart (± jitter s), each `fade` x the previous gain. */
  repeat?: { count: number; every: number; jitter?: number; fade?: number; pitchStep?: number };
}

export interface SynthDef {
  layers: readonly Layer[];
  /** Peak level after normalization (0..1) [0.9]. */
  peak?: number;
  /** Seamless loop: crossfade this many seconds of the tail into the head. */
  loopFade?: number;
}

/** A seeded 32-bit RNG (mulberry32) returning [0, 1). */
export function rng32(seed: number): () => number {
  let a = seed >>> 0 || 0x9e3779b9;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Value of a sweep at progress `p` 0..1 (exponential between positive ends, else linear). */
export function sweepAt(v: Sweep | undefined, p: number, fallback: number, curve = 1): number {
  if (v === undefined) return fallback;
  if (typeof v === "number") return v;
  const k = Math.pow(Math.min(1, Math.max(0, p)), curve);
  const [a, b] = v;
  if (a > 0 && b > 0) return a * Math.pow(b / a, k);
  return a + (b - a) * k;
}

/** Envelope level at time t of a layer of length `dur`. */
export function envelopeAt(e: Envelope | undefined, t: number, dur: number): number {
  const a = e?.a ?? 0.002;
  const h = e?.h ?? 0;
  const d = e?.d ?? 0;
  const s = e?.s ?? (d > 0 ? 0 : 1);
  const r = Math.min(e?.r ?? 0.005, dur * 0.5);
  let v: number;
  if (t < a) v = t / a;
  else if (t < a + h) v = 1;
  else if (d > 0) v = s + (1 - s) * Math.exp(-(t - a - h) / d);
  else v = s;
  const left = dur - t;
  if (left < r) v *= left <= 0 ? 0 : 0.5 - 0.5 * Math.cos((Math.PI * left) / r);
  return v;
}

/** RBJ biquad, coefficients refreshed by `set`. */
class Biquad {
  private b0 = 1;
  private b1 = 0;
  private b2 = 0;
  private a1 = 0;
  private a2 = 0;
  private x1 = 0;
  private x2 = 0;
  private y1 = 0;
  private y2 = 0;

  set(type: FilterDef["type"], freq: number, q: number, sr: number, gainDb = 6): void {
    const f = Math.min(sr * 0.45, Math.max(10, freq));
    const w = (2 * Math.PI * f) / sr;
    const cw = Math.cos(w);
    const alpha = Math.sin(w) / (2 * Math.max(0.05, q));
    let b0: number, b1: number, b2: number, a0: number, a1: number, a2: number;
    switch (type) {
      case "lp":
        b0 = (1 - cw) / 2;
        b1 = 1 - cw;
        b2 = b0;
        a0 = 1 + alpha;
        a1 = -2 * cw;
        a2 = 1 - alpha;
        break;
      case "hp":
        b0 = (1 + cw) / 2;
        b1 = -(1 + cw);
        b2 = b0;
        a0 = 1 + alpha;
        a1 = -2 * cw;
        a2 = 1 - alpha;
        break;
      case "bp":
        // Constant 0 dB peak gain.
        b0 = alpha;
        b1 = 0;
        b2 = -alpha;
        a0 = 1 + alpha;
        a1 = -2 * cw;
        a2 = 1 - alpha;
        break;
      case "notch":
        b0 = 1;
        b1 = -2 * cw;
        b2 = 1;
        a0 = 1 + alpha;
        a1 = -2 * cw;
        a2 = 1 - alpha;
        break;
      case "peak": {
        const A = Math.pow(10, gainDb / 40);
        b0 = 1 + alpha * A;
        b1 = -2 * cw;
        b2 = 1 - alpha * A;
        a0 = 1 + alpha / A;
        a1 = -2 * cw;
        a2 = 1 - alpha / A;
        break;
      }
    }
    this.b0 = b0 / a0;
    this.b1 = b1 / a0;
    this.b2 = b2 / a0;
    this.a1 = a1 / a0;
    this.a2 = a2 / a0;
  }

  run(x: number): number {
    const y = this.b0 * x + this.b1 * this.x1 + this.b2 * this.x2 - this.a1 * this.y1 - this.a2 * this.y2;
    this.x2 = this.x1;
    this.x1 = x;
    this.y2 = this.y1;
    // Flush denormals.
    this.y1 = Math.abs(y) < 1e-20 ? 0 : y;
    return this.y1;
  }
}

/** Samples between filter coefficient updates during a sweep. */
const FILTER_STEP = 16;
/** Samples between pitch, FM index and envelope updates (linear in between for the envelope). */
const CONTROL = 16;

/** Runs `f` over `buf` in place with its sweep. */
function applyFilter(buf: Float32Array, f: FilterDef, sr: number): void {
  const bq = new Biquad();
  const time = Math.max(1e-4, f.time ?? buf.length / sr);
  const q = f.q ?? 0.707;
  const sweeping = typeof f.freq !== "number";
  bq.set(f.type, sweepAt(f.freq, 0, 1000, f.curve), q, sr, f.gain);
  for (let i = 0; i < buf.length; i++) {
    if (sweeping && i % FILTER_STEP === 0) bq.set(f.type, sweepAt(f.freq, i / sr / time, 1000, f.curve), q, sr, f.gain);
    buf[i] = bq.run(buf[i] ?? 0);
  }
}

/** PolyBLEP step correction (band-limits saw and square edges). */
function polyBlep(t: number, dt: number): number {
  if (t < dt) {
    const x = t / dt;
    return x + x - x * x - 1;
  }
  if (t > 1 - dt) {
    const x = (t - 1) / dt;
    return x * x + x + x + 1;
  }
  return 0;
}

/** Renders one copy of a layer (no repeat) into a new buffer of `n` samples. */
function renderLayerOnce(l: Layer, sr: number, rnd: () => number, pitchScale: number): Float32Array {
  const n = Math.max(1, Math.round(l.dur * sr));
  const out = new Float32Array(n);
  const jit = 1 + (l.jitter ?? 0) * (rnd() * 2 - 1);
  const ps = pitchScale * jit;
  const sweepTime = Math.max(1e-4, l.sweepTime ?? l.dur);
  const freqAt = (t: number): number => {
    let f = sweepAt(l.freq, t / sweepTime, 220, l.sweepCurve) * ps;
    if (l.vibrato) f *= Math.pow(2, (l.vibrato.depth * Math.sin(2 * Math.PI * l.vibrato.rate * t)) / 12);
    return f;
  };
  switch (l.wave) {
    case "noise":
      for (let i = 0; i < n; i++) out[i] = rnd() * 2 - 1;
      break;
    case "pink": {
      // Paul Kellet's economy pink filter.
      let b0 = 0, b1 = 0, b2 = 0;
      for (let i = 0; i < n; i++) {
        const w = rnd() * 2 - 1;
        b0 = 0.99765 * b0 + w * 0.099046;
        b1 = 0.963 * b1 + w * 0.2965164;
        b2 = 0.57 * b2 + w * 1.0526913;
        out[i] = (b0 + b1 + b2 + w * 0.1848) * 0.25;
      }
      break;
    }
    case "brown": {
      let y = 0;
      for (let i = 0; i < n; i++) {
        y = (y + (rnd() * 2 - 1) * 0.04) * 0.998;
        out[i] = y * 3;
      }
      break;
    }
    case "crackle": {
      // Sparse impulses of random height and sign (sparks, static, Geiger).
      const p = (l.density ?? 400) / sr;
      for (let i = 0; i < n; i++) if (rnd() < p) out[i] = (rnd() * 0.7 + 0.3) * (rnd() < 0.5 ? -1 : 1);
      break;
    }
    case "pluck": {
      // Karplus-Strong: a noise burst in a delay line with a damped average.
      const f0 = Math.max(20, freqAt(0));
      const period = sr / f0;
      const len = Math.max(2, Math.floor(period));
      const frac = period - len;
      const line = new Float32Array(len + 1);
      const bright = l.pluck?.bright ?? 0.5;
      let lp = 0;
      for (let i = 0; i < line.length; i++) {
        const w = rnd() * 2 - 1;
        lp = lp + (w - lp) * (0.2 + 0.8 * bright);
        line[i] = lp;
      }
      const decay = l.pluck?.decay ?? 1;
      // Per-period loss so the string falls 60 dB in `decay` seconds.
      const loss = Math.pow(0.001, period / sr / Math.max(0.01, decay));
      let idx = 0;
      let prev = 0;
      for (let i = 0; i < n; i++) {
        const a = line[idx] ?? 0;
        const b = line[(idx + 1) % line.length] ?? 0;
        const v = a + (b - a) * frac;
        out[i] = v;
        const avg = (v + prev) * 0.5 * loss;
        prev = v;
        line[idx] = avg * (0.5 + 0.5 * bright) + v * loss * (0.5 - 0.5 * bright);
        idx = (idx + 1) % line.length;
      }
      break;
    }
    case "modal": {
      const m = l.modal ?? { ratios: [1, 2.76, 5.4, 8.93], decays: [0.6, 0.4, 0.25, 0.15] };
      const f0 = freqAt(0);
      m.ratios.forEach((ratio, k) => {
        const f = f0 * ratio * (1 + (rnd() - 0.5) * 0.01);
        if (f >= sr * 0.45) return;
        const tau = (m.decays[k] ?? m.decays[m.decays.length - 1] ?? 0.3) / 6.9;
        const g = m.gains?.[k] ?? 1 / (1 + k * 0.6);
        // A decaying phasor: rotate and shrink a complex value each sample.
        const w = (2 * Math.PI * f) / sr;
        const ph = rnd() * Math.PI * 2;
        const decay = Math.exp(-1 / sr / tau);
        const c = Math.cos(w) * decay;
        const sn = Math.sin(w) * decay;
        let re = Math.cos(ph) * g;
        let im = Math.sin(ph) * g;
        for (let i = 0; i < n; i++) {
          out[i] = (out[i] ?? 0) + im;
          const r2 = re * c - im * sn;
          im = re * sn + im * c;
          re = r2;
        }
      });
      break;
    }
    default: {
      // Pitched oscillators with optional FM.
      let phase = rnd();
      let mphase = 0;
      const duty = l.duty ?? 0.5;
      // Pitch and FM index at control rate (every CONTROL samples).
      let f = freqAt(0);
      let idx = l.fm ? sweepAt(l.fm.index, 0, 0) : 0;
      for (let i = 0; i < n; i++) {
        if (i % CONTROL === 0) {
          const t = i / sr;
          f = freqAt(t);
          if (l.fm) idx = sweepAt(l.fm.index, t / l.dur, 0);
        }
        const dt = Math.min(0.5, f / sr);
        let ph = phase;
        if (l.fm) {
          ph = phase + (idx * Math.sin(2 * Math.PI * mphase)) / (2 * Math.PI);
          mphase += (f * l.fm.ratio) / sr;
          mphase -= Math.floor(mphase);
          ph -= Math.floor(ph);
        }
        let v: number;
        switch (l.wave) {
          case "sine":
            v = Math.sin(2 * Math.PI * ph);
            break;
          case "triangle":
            v = 1 - 4 * Math.abs(ph - 0.5);
            break;
          case "saw":
            v = 2 * ph - 1 - polyBlep(ph, dt);
            break;
          default: {
            // square
            v = ph < duty ? 1 : -1;
            v += polyBlep(ph, dt);
            v -= polyBlep((ph - duty + 1) % 1, dt);
            break;
          }
        }
        out[i] = v;
        phase += dt;
        phase -= Math.floor(phase);
      }
    }
  }
  if (l.filter) applyFilter(out, l.filter, sr);
  if (l.filter2) applyFilter(out, l.filter2, sr);
  // Tremolo / flutter.
  if (l.am) {
    const am = l.am;
    let flutter = 1;
    let target = 1;
    const step = Math.max(1, Math.round(sr / Math.max(1, am.rate)));
    for (let i = 0; i < n; i++) {
      let m: number;
      if (am.random) {
        if (i % step === 0) target = 1 - am.depth * rnd();
        flutter += (target - flutter) * 0.02;
        m = flutter;
      } else {
        m = 1 - am.depth * (0.5 - 0.5 * Math.cos(2 * Math.PI * am.rate * (i / sr)));
      }
      out[i] = (out[i] ?? 0) * m;
    }
  }
  const drive = l.drive ?? 0;
  const g = l.gain ?? 1;
  const norm = drive > 0 ? 1 / Math.tanh(1 + drive) : 1;
  // Envelope at control rate, linearly interpolated.
  let e0 = envelopeAt(l.env, 0, l.dur);
  let e1 = e0;
  for (let i = 0; i < n; i++) {
    const k = i % CONTROL;
    if (k === 0) {
      if (i > 0) e0 = e1;
      e1 = envelopeAt(l.env, Math.min(n - 1, i + CONTROL) / sr, l.dur);
    }
    let v = (out[i] ?? 0) * (e0 + ((e1 - e0) * k) / CONTROL);
    if (drive > 0) v = Math.tanh(v * (1 + drive)) * norm;
    out[i] = v * g;
  }
  return out;
}

/** Length of a definition (s): the end of its last layer (with repeats). */
export function defDuration(def: SynthDef): number {
  let end = 0;
  for (const l of def.layers) {
    const rep = l.repeat;
    const last = rep ? (rep.count - 1) * rep.every + (rep.jitter ?? 0) : 0;
    end = Math.max(end, (l.at ?? 0) + last + l.dur);
  }
  return end;
}

/** Renders `def` at sample rate `sr`; `seed` picks the variant, `pitch` scales every frequency. */
export function renderSynth(def: SynthDef, sr: number, seed = 1, pitch = 1): Float32Array {
  const rnd = rng32(seed * 0x2545f491 + 17);
  const total = Math.max(1, Math.ceil(defDuration(def) * sr) + 1);
  const out = new Float32Array(total);
  for (const l of def.layers) {
    const rep = l.repeat ?? { count: 1, every: 0 };
    let g = 1;
    for (let k = 0; k < rep.count; k++) {
      const at = (l.at ?? 0) + k * rep.every + (rep.jitter ?? 0) * rnd();
      const step = Math.pow(2, ((rep.pitchStep ?? 0) * k) / 12);
      const buf = renderLayerOnce(l, sr, rnd, pitch * step);
      const off = Math.round(at * sr);
      for (let i = 0; i < buf.length && off + i < total; i++) out[off + i] = (out[off + i] ?? 0) + (buf[i] ?? 0) * g;
      g *= rep.fade ?? 1;
    }
  }
  finish(out, def.peak ?? 0.9);
  if (def.loopFade) return loopable(out, Math.round(def.loopFade * sr));
  return out;
}

/** Removes DC and normalizes `buf` to `peak` in place. */
export function finish(buf: Float32Array, peak: number): void {
  // DC blocker (one-pole high-pass at ~10 Hz).
  let x1 = 0;
  let y1 = 0;
  for (let i = 0; i < buf.length; i++) {
    const x = buf[i] ?? 0;
    const y = x - x1 + 0.9987 * y1;
    x1 = x;
    y1 = y;
    buf[i] = y;
  }
  let max = 0;
  for (let i = 0; i < buf.length; i++) max = Math.max(max, Math.abs(buf[i] ?? 0));
  if (max < 1e-9) return;
  const k = peak / max;
  for (let i = 0; i < buf.length; i++) buf[i] = (buf[i] ?? 0) * k;
}

/** A buffer whose end flows into its start: the last `fade` samples are crossfaded into the head. */
export function loopable(buf: Float32Array, fade: number): Float32Array {
  const f = Math.min(fade, Math.floor(buf.length / 3));
  if (f <= 0) return buf;
  const n = buf.length - f;
  const out = new Float32Array(n);
  for (let i = 0; i < n; i++) out[i] = buf[i] ?? 0;
  for (let i = 0; i < f; i++) {
    const x = i / f;
    // Equal-power: the tail fades out as the head fades in.
    const gin = Math.sin((x * Math.PI) / 2);
    const gout = Math.cos((x * Math.PI) / 2);
    out[i] = (buf[i] ?? 0) * gin + (buf[n + i] ?? 0) * gout;
  }
  return out;
}

/** Peak and RMS (linear) of a buffer. */
export function levels(buf: Float32Array): { peak: number; rms: number } {
  let peak = 0;
  let sum = 0;
  for (let i = 0; i < buf.length; i++) {
    const v = buf[i] ?? 0;
    peak = Math.max(peak, Math.abs(v));
    sum += v * v;
  }
  return { peak, rms: Math.sqrt(sum / Math.max(1, buf.length)) };
}

/**
 * A stereo reverb impulse response: exponentially decaying noise that
 * darkens over time, with a short pre-delay and sparse early reflections.
 * `decay` is the time to -60 dB (s).
 */
export function impulseResponse(sr: number, decay: number, seed = 7): [Float32Array, Float32Array] {
  const n = Math.round(sr * decay);
  const pre = Math.round(sr * 0.012);
  const chans: [Float32Array, Float32Array] = [new Float32Array(n), new Float32Array(n)];
  chans.forEach((ch, c) => {
    const rnd = rng32(seed + c * 101);
    let lp = 0;
    for (let i = pre; i < n; i++) {
      const t = (i - pre) / sr;
      const env = Math.pow(0.001, t / decay);
      // One-pole low-pass that closes over the tail (air absorbs the highs first).
      const k = 0.85 * Math.exp(-t * 2.2) + 0.08;
      lp += ((rnd() * 2 - 1) - lp) * k;
      ch[i] = lp * env;
    }
    // Early reflections.
    for (let r = 0; r < 10; r++) {
      const at = pre + Math.round(sr * (0.006 + rnd() * 0.07));
      if (at < n) ch[at] = (ch[at] ?? 0) + (rnd() < 0.5 ? -1 : 1) * (0.5 - r * 0.035);
    }
  });
  // Normalize the energy so the wet level does not depend on the length.
  let e = 0;
  for (const ch of chans) for (let i = 0; i < n; i++) e += (ch[i] ?? 0) ** 2;
  const k = 1 / Math.sqrt(Math.max(1e-9, e / 2)) * 0.6;
  for (const ch of chans) for (let i = 0; i < n; i++) ch[i] = (ch[i] ?? 0) * k;
  return chans;
}
