// WebAudio engine (M4 A6, milestone-4-plan.md §3.7).
//
// - Silent until the first user gesture: no AudioContext exists before the
//   page has seen a click, key or touch (browsers would refuse to start it
//   anyway, and the smoke test checks it). `play` before that is a no-op.
// - Sounds are SoundDefs (library.ts) rendered by dsp.ts into AudioBuffers:
//   lazily on first use, and in the background in small time slices right
//   after the context starts, so the first shot of a match does not stall.
// - Mixer: per-voice gain -> (distance low-pass -> panner) -> bus (effects,
//   music, voice) -> master -> limiter -> speakers, plus a reverb send per
//   voice (wetter with distance) into one convolver.
// - Voice cap (32): a new sound beyond the cap replaces the least important
//   voice (priority, audibility, age) or is dropped if it matters less;
//   per-sound instance limits and minimum gaps keep bursts (a shotgun, a
//   64-sound pile-up) from stacking.
// - Positional sounds take a world position (u); the listener follows the
//   camera (`setListener`). Sounds without a position (your own ship, UI,
//   the announcer) play centred.

import type { Vec3 } from "../math/vec3.ts";
import { impulseResponse, renderSynth } from "./dsp.ts";
import { allSounds, mixOf, soundDef, type Bus, type SoundDef } from "./library.ts";
import { busGain, loadAudioSettings, onAudioSettings, type AudioSettings } from "./settings.ts";

/** Most voices sounding at once. */
export const VOICE_CAP = 32;
/** Estimated gain below which a positional sound is not started at all. */
export const CULL_GAIN = 0.004;
/** Fade (s) of a stolen or stopped voice. */
const STEAL_FADE = 0.02;
/** Reverb decay (s to -60 dB). */
const REVERB_SECS = 2.2;
/** Background render slice (ms of main-thread time per timer tick). */
const PRERENDER_SLICE_MS = 6;

export interface PlayOptions {
  /** World position (u); absent = centred (own ship, UI). */
  position?: Vec3 | null | undefined;
  /** Extra linear gain [1]. */
  gain?: number;
  /** Playback rate (pitch and speed) [1, with the sound's jitter]. */
  rate?: number;
  /** Start delay (s) [0]. */
  delay?: number;
  /** Variant index (default random). */
  variant?: number;
  /** Override the sound's priority. */
  priority?: number;
}

export interface LoopHandle {
  /** False once stopped or stolen. */
  readonly playing: boolean;
  setGain(gain: number, ramp?: number): void;
  setRate(rate: number, ramp?: number): void;
  setPosition(p: Vec3): void;
  stop(fade?: number): void;
}

export interface EngineStats {
  /** "locked" before the first gesture, else the context's state. */
  state: string;
  voices: number;
  peakVoices: number;
  started: number;
  stolen: number;
  dropped: number;
  culled: number;
  /** Sounds with every variant rendered / registered. */
  rendered: number;
  total: number;
}

/** What the engine needs from the page (a fake in Node tests). */
export interface AudioEnv {
  /** Creates the context (only ever called after a gesture). */
  createContext(): AudioContext | null;
  /** Where gestures are heard (window); null = never unlock by itself. */
  gestureTarget: EventTarget | null;
  /** True when the page already had a user gesture (navigator.userActivation.hasBeenActive). */
  activated(): boolean;
  /** The page's document for visibility (suspend while hidden); null = none. */
  doc: Document | null;
  /** Schedules background work. */
  defer(fn: () => void): void;
}

interface Voice {
  serial: number;
  def: SoundDef;
  key: string;
  source: AudioBufferSourceNode;
  gain: GainNode;
  panner: PannerNode | null;
  filter: BiquadFilterNode | null;
  send: GainNode | null;
  started: number;
  /** Context time it ends (Infinity: loop). */
  end: number;
  priority: number;
  audible: number;
  refDistance: number;
  stopped: boolean;
}

const GESTURES = ["pointerdown", "mousedown", "keydown", "touchend", "click"];

function dist(a: Vec3, b: Vec3): number {
  return Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z);
}

/** Inverse-distance gain of the PannerNode model (rolloff 1). */
export function distanceGain(d: number, ref: number): number {
  return ref / Math.max(ref, d);
}

/** Low-pass cutoff (Hz) for a sound `d` u away: far sounds lose their highs. */
export function distanceCutoff(d: number): number {
  return Math.max(900, Math.min(20000, 20000 / (1 + d / 45)));
}

export class AudioEngine {
  private readonly env: AudioEnv;
  private ctx: AudioContext | null = null;
  private master: GainNode | null = null;
  private buses = new Map<Bus, GainNode>();
  private reverb: ConvolverNode | null = null;
  private duckGain: GainNode | null = null;
  private voices: Voice[] = [];
  private serial = 0;
  private lastStart = new Map<string, number>();
  private buffers = new Map<string, AudioBuffer[]>();
  private listenerPos: Vec3 = { x: 0, y: 0, z: 0 };
  private settings: AudioSettings;
  private prerendering = false;
  private unsubscribe: (() => void) | null = null;
  private readonly counts = { peakVoices: 0, started: 0, stolen: 0, dropped: 0, culled: 0 };
  private readonly onGesture = (): void => this.unlock();
  private readonly onVisibility = (): void => {
    if (!this.ctx || !this.env.doc) return;
    if (this.env.doc.hidden) void this.ctx.suspend().catch(() => {});
    else void this.ctx.resume().catch(() => {});
  };

  constructor(env: AudioEnv, settings: AudioSettings = loadAudioSettings()) {
    this.env = env;
    this.settings = settings;
    this.unsubscribe = onAudioSettings((s) => this.applySettings(s));
    if (env.activated()) this.unlock();
    else for (const g of GESTURES) env.gestureTarget?.addEventListener(g, this.onGesture, { capture: true, passive: true });
  }

  /** The context, null until the first gesture. */
  get context(): AudioContext | null {
    return this.ctx;
  }

  get unlocked(): boolean {
    return this.ctx !== null;
  }

  /** Creates and starts the context (call from a user gesture; the constructor listens for one). */
  unlock(): void {
    if (this.ctx) {
      if (this.ctx.state === "suspended" && !this.env.doc?.hidden) void this.ctx.resume().catch(() => {});
      return;
    }
    const ctx = this.env.createContext();
    if (!ctx) return;
    this.ctx = ctx;
    this.build(ctx);
    void ctx.resume().catch(() => {});
    // Keep listening until the context really runs (some gestures, like Escape, do not count).
    if (ctx.state === "running") this.removeGestureListeners();
    else
      ctx.addEventListener?.("statechange", () => {
        if (ctx.state === "running") this.removeGestureListeners();
      });
    this.env.doc?.addEventListener("visibilitychange", this.onVisibility);
    this.prerender();
  }

  private removeGestureListeners(): void {
    for (const g of GESTURES) this.env.gestureTarget?.removeEventListener(g, this.onGesture, { capture: true });
  }

  private build(ctx: AudioContext): void {
    const limiter = ctx.createDynamicsCompressor();
    limiter.threshold.value = -6;
    limiter.knee.value = 6;
    limiter.ratio.value = 12;
    limiter.attack.value = 0.003;
    limiter.release.value = 0.25;
    limiter.connect(ctx.destination);
    const master = ctx.createGain();
    master.connect(limiter);
    this.master = master;
    // Effects pass through a ducker the announcer pulls down while it speaks.
    const duck = ctx.createGain();
    duck.connect(master);
    this.duckGain = duck;
    for (const bus of ["effects", "music", "voice"] as const) {
      const g = ctx.createGain();
      g.connect(bus === "effects" ? duck : master);
      this.buses.set(bus, g);
    }
    const reverb = ctx.createConvolver();
    const [l, r] = impulseResponse(ctx.sampleRate, REVERB_SECS);
    const ir = ctx.createBuffer(2, l.length, ctx.sampleRate);
    ir.getChannelData(0).set(l);
    ir.getChannelData(1).set(r);
    reverb.normalize = false;
    reverb.buffer = ir;
    reverb.connect(duck);
    this.reverb = reverb;
    this.applySettings(this.settings);
  }

  /** Applies volumes (smoothly). */
  applySettings(s: AudioSettings): void {
    this.settings = s;
    const ctx = this.ctx;
    if (!ctx || !this.master) return;
    const t = ctx.currentTime;
    this.master.gain.setTargetAtTime(busGain(s, "master"), t, 0.03);
    for (const [bus, g] of this.buses) g.gain.setTargetAtTime(busGain(s, bus), t, 0.03);
  }

  /** Lowers the effects (and their reverb) to `level` for `secs`, so the announcer stays clear. */
  duck(secs: number, level = 0.55): void {
    const ctx = this.ctx;
    if (!ctx || !this.duckGain) return;
    const t = ctx.currentTime;
    const g = this.duckGain.gain;
    g.cancelScheduledValues(t);
    g.setTargetAtTime(level, t, 0.05);
    g.setTargetAtTime(1, t + Math.max(0.1, secs), 0.25);
  }

  /** Moves the listener (camera): position and its forward and up directions. */
  setListener(position: Vec3, forward: Vec3, up: Vec3): void {
    this.listenerPos = position;
    const ctx = this.ctx;
    if (!ctx) return;
    const l = ctx.listener;
    if (l.positionX) {
      const t = ctx.currentTime;
      l.positionX.setValueAtTime(position.x, t);
      l.positionY.setValueAtTime(position.y, t);
      l.positionZ.setValueAtTime(position.z, t);
      l.forwardX.setValueAtTime(forward.x, t);
      l.forwardY.setValueAtTime(forward.y, t);
      l.forwardZ.setValueAtTime(forward.z, t);
      l.upX.setValueAtTime(up.x, t);
      l.upY.setValueAtTime(up.y, t);
      l.upZ.setValueAtTime(up.z, t);
    } else {
      l.setPosition?.(position.x, position.y, position.z);
      l.setOrientation?.(forward.x, forward.y, forward.z, up.x, up.y, up.z);
    }
  }

  /** Plays sound `id`; returns false when it did not start (locked, unknown, culled, limited). */
  play(id: string, o: PlayOptions = {}): boolean {
    return this.start(id, o, false) !== null;
  }

  /** Starts a looping sound; null when it could not start. */
  loop(id: string, o: PlayOptions = {}): LoopHandle | null {
    const v = this.start(id, o, true);
    if (!v) return null;
    const ctx = this.ctx as AudioContext;
    // A loop's gain node holds the base gain; the handle scales it.
    const base = v.gain.gain.value;
    return {
      get playing() {
        return !v.stopped;
      },
      setGain: (g, ramp = 0.08) => {
        if (!v.stopped) v.gain.gain.setTargetAtTime(base * Math.max(0, g), ctx.currentTime, ramp / 3);
        v.audible = Math.max(0, g);
      },
      setRate: (r, ramp = 0.08) => {
        if (!v.stopped) v.source.playbackRate.setTargetAtTime(Math.max(0.05, r), ctx.currentTime, ramp / 3);
      },
      setPosition: (p) => {
        if (v.stopped || !v.panner) return;
        const t = ctx.currentTime;
        v.panner.positionX.setValueAtTime(p.x, t);
        v.panner.positionY.setValueAtTime(p.y, t);
        v.panner.positionZ.setValueAtTime(p.z, t);
        const d = dist(p, this.listenerPos);
        v.filter?.frequency.setTargetAtTime(distanceCutoff(d), t, 0.05);
      },
      stop: (fade = 0.15) => this.stopVoice(v, fade),
    };
  }

  /** Plays raw mono samples at `sampleRate` (the announcer's speech) on `bus`, centred. */
  playSamples(samples: Float32Array, sampleRate: number, bus: Bus = "voice", gain = 1, priority = 10): boolean {
    const ctx = this.ctx;
    if (!ctx || samples.length === 0) return false;
    const buf = ctx.createBuffer(1, samples.length, sampleRate);
    buf.getChannelData(0).set(samples);
    const def: SoundDef = { id: "__samples", label: "", group: "", description: "", bus, synth: { layers: [] }, priority, space: 0.08, maxInstances: 4, minGap: 0 };
    return this.startBuffer(def, buf, { gain }, false, 1) !== null;
  }

  private start(id: string, o: PlayOptions, loop: boolean): Voice | null {
    const ctx = this.ctx;
    if (!ctx) return null;
    const def = soundDef(id);
    if (!def) return null;
    const mix = mixOf(def);
    const variants = this.buffersFor(def, o.variant);
    const buf = variants[o.variant !== undefined ? Math.abs(o.variant) % variants.length : Math.floor(Math.random() * variants.length)];
    if (!buf) return null;
    const jitter = loop ? 0 : mix.pitchJitter * (Math.random() * 2 - 1);
    return this.startBuffer(def, buf, o, loop || def.loop === true, (o.rate ?? 1) * (1 + jitter));
  }

  private startBuffer(def: SoundDef, buf: AudioBuffer, o: PlayOptions, loop: boolean, rate: number): Voice | null {
    const ctx = this.ctx as AudioContext;
    const mix = mixOf(def);
    const now = ctx.currentTime;
    this.reap(now);
    const pos = o.position ?? null;
    const d = pos ? dist(pos, this.listenerPos) : 0;
    const gain = mix.gain * (o.gain ?? 1);
    const audible = gain * (pos ? distanceGain(d, mix.refDistance) : 1);
    if (pos && audible < CULL_GAIN) {
      this.counts.culled++;
      return null;
    }
    // Minimum gap per sound (separately for centred and positional plays).
    const key = `${def.id}/${pos ? "w" : "c"}`;
    const last = this.lastStart.get(key);
    if (!loop && last !== undefined && now - last < mix.minGap) {
      this.counts.dropped++;
      return null;
    }
    const priority = o.priority ?? mix.priority;
    // Instance limit: the oldest instance of this sound makes room.
    const same = this.voices.filter((v) => v.def.id === def.id && !v.stopped);
    if (same.length >= mix.maxInstances) {
      const oldest = same.reduce((a, b) => (a.started <= b.started ? a : b));
      this.stopVoice(oldest, STEAL_FADE);
      this.counts.stolen++;
    }
    // Voice cap: steal the least important voice, or give up.
    const live = this.voices.filter((v) => !v.stopped);
    if (live.length >= VOICE_CAP) {
      const victim = live.reduce((a, b) => (importance(a, now) <= importance(b, now) ? a : b));
      const mine = priority + 4 * Math.min(1, audible);
      if (mine < importance(victim, now)) {
        this.counts.dropped++;
        return null;
      }
      this.stopVoice(victim, STEAL_FADE);
      this.counts.stolen++;
    }

    const at = now + Math.max(0, o.delay ?? 0);
    const source = ctx.createBufferSource();
    source.buffer = buf;
    source.loop = loop;
    source.playbackRate.value = Math.max(0.05, rate);
    const g = ctx.createGain();
    g.gain.value = gain;
    source.connect(g);
    let panner: PannerNode | null = null;
    let filter: BiquadFilterNode | null = null;
    const bus = this.buses.get(def.bus) as GainNode;
    if (pos) {
      filter = ctx.createBiquadFilter();
      filter.type = "lowpass";
      filter.frequency.value = distanceCutoff(d);
      filter.Q.value = 0.5;
      panner = ctx.createPanner();
      panner.panningModel = "HRTF";
      panner.distanceModel = "inverse";
      panner.refDistance = mix.refDistance;
      panner.rolloffFactor = 1;
      panner.maxDistance = 100000;
      panner.positionX.value = pos.x;
      panner.positionY.value = pos.y;
      panner.positionZ.value = pos.z;
      g.connect(filter);
      filter.connect(panner);
      panner.connect(bus);
    } else {
      g.connect(bus);
    }
    let send: GainNode | null = null;
    if (this.reverb && mix.space > 0) {
      send = ctx.createGain();
      // Distant sounds are wetter (and the send follows the bus volume at the start).
      const far = pos ? Math.min(1, d / 80) : 0;
      send.gain.value = mix.space * (0.6 + 0.6 * far) * busGain(this.settings, def.bus);
      (panner ?? g).connect(send);
      send.connect(this.reverb);
    }
    const v: Voice = {
      serial: ++this.serial,
      def,
      key,
      source,
      gain: g,
      panner,
      filter,
      send,
      started: at,
      end: loop ? Infinity : at + buf.duration / Math.max(0.05, rate) + 0.05,
      priority,
      audible: Math.min(1, audible),
      refDistance: mix.refDistance,
      stopped: false,
    };
    source.onended = () => this.release(v);
    source.start(at);
    this.voices.push(v);
    this.lastStart.set(key, now);
    this.counts.started++;
    this.counts.peakVoices = Math.max(this.counts.peakVoices, this.voices.filter((x) => !x.stopped).length);
    return v;
  }

  private stopVoice(v: Voice, fade: number): void {
    if (v.stopped || !this.ctx) return;
    v.stopped = true;
    const t = this.ctx.currentTime;
    try {
      v.gain.gain.cancelScheduledValues(t);
      v.gain.gain.setValueAtTime(v.gain.gain.value, t);
      v.gain.gain.linearRampToValueAtTime(0, t + fade);
      v.source.stop(t + fade + 0.005);
    } catch {
      /* already stopped */
    }
    v.end = Math.min(v.end, t + fade + 0.01);
  }

  private release(v: Voice): void {
    v.stopped = true;
    const i = this.voices.indexOf(v);
    if (i >= 0) this.voices.splice(i, 1);
    try {
      v.source.disconnect();
      v.gain.disconnect();
      v.filter?.disconnect();
      v.panner?.disconnect();
      v.send?.disconnect();
    } catch {
      /* gone */
    }
  }

  /** Drops voices whose time is over (in case `onended` did not fire). */
  private reap(now: number): void {
    for (const v of [...this.voices]) if (v.end < now - 0.25) this.release(v);
  }

  /** Voices sounding (not stopped). */
  get voiceCount(): number {
    return this.voices.filter((v) => !v.stopped).length;
  }

  /** Rendered buffers of `def`, rendering what is needed now. */
  private buffersFor(def: SoundDef, want?: number): AudioBuffer[] {
    let list = this.buffers.get(def.id);
    if (!list) {
      list = [];
      this.buffers.set(def.id, list);
    }
    const need = want !== undefined ? Math.min(mixOf(def).variants, Math.abs(want) + 1) : 1;
    while (list.length < need) list.push(this.renderVariant(def, list.length));
    return list;
  }

  private renderVariant(def: SoundDef, k: number): AudioBuffer {
    const ctx = this.ctx as AudioContext;
    const samples = renderSynth(def.synth, ctx.sampleRate, k + 1);
    const buf = ctx.createBuffer(1, samples.length, ctx.sampleRate);
    buf.getChannelData(0).set(samples);
    return buf;
  }

  /** Renders every registered sound's variants in the background, a few ms at a time. */
  prerender(): void {
    if (this.prerendering || !this.ctx) return;
    this.prerendering = true;
    const step = (): void => {
      const t0 = performance.now();
      let more = false;
      // First variant of everything, then the rest.
      outer: for (const pass of [1, Infinity]) {
        for (const def of allSounds()) {
          const want = Math.min(mixOf(def).variants, pass);
          for (let have = this.buffers.get(def.id)?.length ?? 0; have < want; have++) {
            if (performance.now() - t0 > PRERENDER_SLICE_MS) {
              more = true;
              break outer;
            }
            this.buffersFor(def, have);
          }
        }
      }
      if (more) this.env.defer(step);
      else this.prerendering = false;
    };
    this.env.defer(step);
  }

  stats(): EngineStats {
    const total = allSounds();
    return {
      state: this.ctx ? this.ctx.state : "locked",
      voices: this.voiceCount,
      ...this.counts,
      rendered: total.filter((d) => (this.buffers.get(d.id)?.length ?? 0) >= mixOf(d).variants).length,
      total: total.length,
    };
  }

  /** Stops every voice, or every voice of `bus` (match end). */
  stopAll(fade = 0.1, bus?: Bus): void {
    for (const v of [...this.voices]) if (!bus || v.def.bus === bus) this.stopVoice(v, fade);
  }

  dispose(): void {
    this.stopAll(0.01);
    this.removeGestureListeners();
    this.env.doc?.removeEventListener("visibilitychange", this.onVisibility);
    this.unsubscribe?.();
    this.unsubscribe = null;
    void this.ctx?.close().catch(() => {});
    this.ctx = null;
  }
}

/** How much a voice matters: priority, then audibility, minus age for one-shots. */
function importance(v: Voice, now: number): number {
  const age = Number.isFinite(v.end) ? Math.max(0, now - v.started) : 0;
  return v.priority + 4 * v.audible - 0.8 * age;
}

/** Engine wired to the page (window gestures, document visibility). */
export function browserAudioEnv(): AudioEnv {
  const w = typeof window === "undefined" ? null : window;
  return {
    createContext: () => {
      const Ctor = w?.AudioContext ?? (w as unknown as { webkitAudioContext?: typeof AudioContext } | null)?.webkitAudioContext;
      if (!Ctor) return null;
      try {
        return new Ctor({ latencyHint: "interactive" });
      } catch {
        return null;
      }
    },
    gestureTarget: w,
    activated: () => {
      const ua = typeof navigator === "undefined" ? undefined : (navigator as Navigator & { userActivation?: { hasBeenActive: boolean } }).userActivation;
      return ua?.hasBeenActive === true;
    },
    doc: typeof document === "undefined" ? null : document,
    defer: (fn) => setTimeout(fn, 0),
  };
}
