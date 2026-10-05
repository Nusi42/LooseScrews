import assert from "node:assert/strict";
import { test } from "node:test";
import { defDuration, envelopeAt, levels, loopable, renderSynth, sweepAt } from "./dsp.ts";
import { AudioEngine, CULL_GAIN, VOICE_CAP, distanceCutoff, distanceGain, type AudioEnv } from "./engine.ts";
import { allSounds, mixOf, registerSounds, soundDef } from "./library.ts";
import { BUILTIN_SOUNDS } from "./sounds/index.ts";
import { AUDIO_STORAGE_KEY, DEFAULT_AUDIO, busGain, loadAudioSettings, readAudioSettings, saveAudioSettings, type KeyValueStore } from "./settings.ts";
import { renderSpeech, syllables, words } from "./voice.ts";
import { ALL_TRIGGERS, EVENT_IDS, LinePicker, fillLine, linesFor } from "./announcer/lines.ts";
import { PACKS, packForSeed } from "./announcer/packs.ts";
import { GameAudio, fireSound, impactSound, type AudioFrame } from "./game.ts";
import { CUE, NO_SHIP } from "../game/mechanics.ts";
import { CUE_HIT } from "../game/vfx.ts";
import type { ProjectileState, ShipState } from "../game/views.ts";

registerSounds(BUILTIN_SOUNDS);

// ----- a fake WebAudio context ----------------------------------------------------------

class FakeParam {
  value: number;
  constructor(v = 0) {
    this.value = v;
  }
  setValueAtTime(v: number): this {
    this.value = v;
    return this;
  }
  setTargetAtTime(v: number): this {
    this.value = v;
    return this;
  }
  linearRampToValueAtTime(v: number): this {
    this.value = v;
    return this;
  }
  cancelScheduledValues(): this {
    return this;
  }
}

class FakeNode {
  connections = 0;
  connect<T>(n: T): T {
    this.connections++;
    return n;
  }
  disconnect(): void {
    this.connections = 0;
  }
}

class FakeSource extends FakeNode {
  buffer: unknown = null;
  loop = false;
  playbackRate = new FakeParam(1);
  onended: (() => void) | null = null;
  started = false;
  stopAt: number | null = null;
  constructor(ctx: FakeContext) {
    super();
    ctx.sources.push(this);
  }
  start(): void {
    this.started = true;
  }
  stop(at: number): void {
    this.stopAt = at;
  }
  /** Sounding: started and not told to stop. */
  get running(): boolean {
    return this.started && this.stopAt === null;
  }
}

class FakeContext {
  state = "running";
  currentTime = 0;
  sampleRate = 8000;
  destination = new FakeNode();
  sources: FakeSource[] = [];
  listener = {
    positionX: new FakeParam(),
    positionY: new FakeParam(),
    positionZ: new FakeParam(),
    forwardX: new FakeParam(),
    forwardY: new FakeParam(),
    forwardZ: new FakeParam(-1),
    upX: new FakeParam(),
    upY: new FakeParam(1),
    upZ: new FakeParam(),
  };
  resume(): Promise<void> {
    return Promise.resolve();
  }
  suspend(): Promise<void> {
    return Promise.resolve();
  }
  close(): Promise<void> {
    return Promise.resolve();
  }
  addEventListener(): void {}
  createGain(): unknown {
    return Object.assign(new FakeNode(), { gain: new FakeParam(1) });
  }
  createBiquadFilter(): unknown {
    return Object.assign(new FakeNode(), { type: "lowpass", frequency: new FakeParam(350), Q: new FakeParam(1) });
  }
  createPanner(): unknown {
    return Object.assign(new FakeNode(), { positionX: new FakeParam(), positionY: new FakeParam(), positionZ: new FakeParam(), panningModel: "", distanceModel: "", refDistance: 1, rolloffFactor: 1, maxDistance: 1 });
  }
  createDynamicsCompressor(): unknown {
    return Object.assign(new FakeNode(), { threshold: new FakeParam(), knee: new FakeParam(), ratio: new FakeParam(), attack: new FakeParam(), release: new FakeParam() });
  }
  createConvolver(): unknown {
    return Object.assign(new FakeNode(), { normalize: true, buffer: null });
  }
  createBufferSource(): unknown {
    return new FakeSource(this);
  }
  createBuffer(channels: number, length: number, sampleRate: number): unknown {
    const data = Array.from({ length: channels }, () => new Float32Array(length));
    return { length, sampleRate, numberOfChannels: channels, duration: length / sampleRate, getChannelData: (c: number) => data[c] };
  }
  get running(): number {
    return this.sources.filter((s) => s.running).length;
  }
}

function fakeEnv(activated = false): { env: AudioEnv; target: EventTarget; contexts: FakeContext[] } {
  const target = new EventTarget();
  const contexts: FakeContext[] = [];
  const env: AudioEnv = {
    createContext: () => {
      const c = new FakeContext();
      contexts.push(c);
      return c as unknown as AudioContext;
    },
    gestureTarget: target,
    activated: () => activated,
    doc: null,
    defer: () => {}, // no background rendering in tests
  };
  return { env, target, contexts };
}

// ----- synthesis --------------------------------------------------------------------------

test("sweeps and envelopes", () => {
  assert.equal(sweepAt(440, 0.5, 0), 440);
  assert.ok(Math.abs(sweepAt([100, 400], 0.5, 0) - 200) < 1e-9, "exponential midpoint");
  assert.equal(sweepAt(undefined, 0.3, 7), 7);
  assert.ok(envelopeAt({ a: 0.1 }, 0.05, 1) > 0.49 && envelopeAt({ a: 0.1 }, 0.05, 1) < 0.51);
  assert.ok(envelopeAt({ a: 0.001, d: 0.1 }, 0.3, 1) < 0.06, "decays");
  assert.equal(envelopeAt({ a: 0.01, r: 0.1 }, 1, 1), 0, "released at the end");
});

test("loopable crossfades the tail into the head", () => {
  const b = new Float32Array(300).fill(0.5);
  const l = loopable(b, 60);
  assert.equal(l.length, 240);
  assert.ok(Math.abs((l[0] ?? 0) - 0.5) < 1e-6);
});

test("every built-in sound renders: unique ids, finite, normalized, not silent, right length", () => {
  const ids = new Set<string>();
  for (const d of BUILTIN_SOUNDS) {
    assert.ok(!ids.has(d.id), `duplicate ${d.id}`);
    ids.add(d.id);
    assert.match(d.id, /^[a-z]+(\.[a-zA-Z]+)?$/, d.id);
    assert.ok(d.label && d.description && d.group, d.id);
    const sr = 8000;
    const s = renderSynth(d.synth, sr, 1);
    const { peak, rms } = levels(s);
    assert.ok(s.every((x) => Number.isFinite(x)), `${d.id} finite`);
    assert.ok(peak <= (d.synth.peak ?? 0.9) + 1e-6 && peak > 0.5, `${d.id} peak ${peak}`);
    assert.ok(rms > 0.01, `${d.id} not silent (${rms})`);
    const expected = defDuration(d.synth) * sr;
    if (d.synth.loopFade) assert.ok(s.length < expected, `${d.id} loop shortened by its crossfade`);
    else assert.ok(Math.abs(s.length - expected) <= 2, `${d.id} length`);
    if (d.loop) assert.ok(d.synth.loopFade, `${d.id} loops need a crossfade`);
  }
});

test("renders are repeatable per variant and differ between variants", () => {
  const def = soundDef("explosion.small");
  assert.ok(def);
  const a = renderSynth(def.synth, 8000, 1);
  const b = renderSynth(def.synth, 8000, 1);
  const c = renderSynth(def.synth, 8000, 2);
  assert.deepEqual(a, b);
  assert.notDeepEqual(a, c);
});

// ----- voice and announcer ------------------------------------------------------------------

test("syllables follow the vowel groups", () => {
  assert.equal(syllables("Rivet").length, 2);
  assert.equal(syllables("blood").length, 1);
  assert.equal(syllables("rate").length, 1, "silent final e");
  assert.equal(syllables("overexcitedly").length, 4, "capped at 4");
  assert.deepEqual(
    words("Three, two, one… BELT UP!").map((w) => w.text),
    ["Three", "two", "one", "BELT", "UP"],
  );
});

test("speech has one span per word in order, and repeats exactly", () => {
  const pack = PACKS[1];
  assert.ok(pack);
  const line = "DOUBLE KILL! Rivet is heating up, folks.";
  const a = renderSpeech(line, pack.voice, 16000, 1);
  const b = renderSpeech(line, pack.voice, 16000, 1);
  assert.deepEqual(a.samples, b.samples);
  assert.equal(a.words.length, words(line).length);
  for (let i = 1; i < a.words.length; i++) assert.ok((a.words[i]?.start ?? 0) >= (a.words[i - 1]?.end ?? 0));
  assert.ok(a.duration > 0.8 && a.duration < 5, `duration ${a.duration}`);
  assert.ok(levels(a.samples).rms > 0.02);
});

test("the three voice packs differ in pace and pitch", () => {
  const line = "First blood. Rivet. Good for you.";
  const durs = PACKS.map((p) => renderSpeech(line, p.voice, 16000, 1).duration);
  const tired = durs[0] ?? 0;
  const excited = durs[1] ?? 0;
  assert.ok(tired > excited * 1.4, `tired ${tired} s vs overexcited ${excited} s`);
  assert.ok((PACKS[1]?.voice.pitch ?? 0) > (PACKS[0]?.voice.pitch ?? 0) * 1.8);
});

test("every pack covers every trigger with known placeholders", () => {
  assert.equal(EVENT_IDS.length, 18);
  for (const p of PACKS) {
    for (const t of ALL_TRIGGERS) {
      const ls = p.lines[t];
      assert.ok(ls && ls.length > 0, `${p.id} lacks ${t}`);
      for (const l of ls) for (const m of l.matchAll(/\{(\w+)\}/g)) assert.ok(["killer", "victim", "target", "winner"].includes(m[1] ?? ""), `${p.id}/${t}: ${l}`);
    }
  }
  assert.equal(fillLine("{killer} got {victim}", { killer: "Ana" }), "Ana got someone");
  assert.equal(fillLine("{winner} wins", {}), "nobody wins");
});

test("packs spread over seeds; the line picker does not repeat itself", () => {
  const seen = new Set<string>();
  for (let s = 0; s < 64; s++) seen.add(packForSeed(s * 7919).id);
  assert.equal(seen.size, 3);
  const p = PACKS[0];
  assert.ok(p);
  const picker = new LinePicker(() => 0.99);
  let last = "";
  for (let i = 0; i < 10; i++) {
    const l = picker.pick(p, "matchStart");
    assert.notEqual(l, last);
    assert.ok(linesFor(p, "matchStart").includes(l));
    last = l;
  }
});

// ----- settings -----------------------------------------------------------------------------

function memoryStore(init: Record<string, string> = {}): KeyValueStore {
  const m = new Map(Object.entries(init));
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => void m.set(k, v) };
}

test("audio settings: defaults, clamping, persistence, gain curve and mute", () => {
  assert.deepEqual(loadAudioSettings(memoryStore()), DEFAULT_AUDIO);
  assert.deepEqual(loadAudioSettings(memoryStore({ [AUDIO_STORAGE_KEY]: "{broken" })), DEFAULT_AUDIO);
  assert.deepEqual(readAudioSettings({ master: 140, effects: -3, music: "x", voice: 50.4, muted: 1 }), { master: 100, effects: 0, music: DEFAULT_AUDIO.music, voice: 50, muted: false });
  const store = memoryStore();
  saveAudioSettings({ master: 40, effects: 70, music: 0, voice: 100, muted: true }, store);
  const back = loadAudioSettings(store);
  assert.deepEqual(back, { master: 40, effects: 70, music: 0, voice: 100, muted: true });
  assert.equal(busGain(back, "master"), 0, "muted");
  assert.ok(Math.abs(busGain({ ...back, muted: false }, "master") - 0.16) < 1e-12);
  assert.equal(busGain(back, "voice"), 1);
  const throwing: KeyValueStore = {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("blocked");
    },
  };
  assert.deepEqual(loadAudioSettings(throwing), DEFAULT_AUDIO);
  saveAudioSettings(DEFAULT_AUDIO, throwing);
});

// ----- engine -------------------------------------------------------------------------------

test("no AudioContext before a user gesture; the first gesture creates it", () => {
  const { env, target, contexts } = fakeEnv(false);
  const e = new AudioEngine(env, DEFAULT_AUDIO);
  assert.equal(contexts.length, 0);
  assert.equal(e.play("fire.bolt"), false, "silent while locked");
  assert.equal(e.stats().state, "locked");
  target.dispatchEvent(new Event("pointerdown"));
  assert.equal(contexts.length, 1);
  assert.equal(e.play("fire.bolt"), true);
  target.dispatchEvent(new Event("keydown"));
  assert.equal(contexts.length, 1, "one context only");
  e.dispose();
});

test("an engine made after a gesture (sticky activation) starts at once", () => {
  const { env, contexts } = fakeEnv(true);
  const e = new AudioEngine(env, DEFAULT_AUDIO);
  assert.equal(contexts.length, 1);
  e.dispose();
});

test("the voice cap holds under a 64-sound burst", () => {
  const { env, contexts } = fakeEnv(true);
  const e = new AudioEngine(env, DEFAULT_AUDIO);
  const ctx = contexts[0] as FakeContext;
  const ids = allSounds().filter((d) => !d.loop && d.bus === "effects").map((d) => d.id);
  let started = 0;
  for (let i = 0; i < 64; i++) {
    const a = (i / 64) * Math.PI * 2;
    if (e.play(ids[i % ids.length] as string, { position: { x: Math.cos(a) * 10, y: 0, z: Math.sin(a) * 10 } })) started++;
    assert.ok(e.voiceCount <= VOICE_CAP, `voices ${e.voiceCount} after ${i + 1}`);
    assert.ok(ctx.running <= VOICE_CAP, `running sources ${ctx.running}`);
  }
  assert.ok(started >= VOICE_CAP, `${started} started`);
  assert.equal(e.voiceCount, VOICE_CAP);
  const s = e.stats();
  assert.ok(s.peakVoices <= VOICE_CAP);
  assert.equal(s.stolen + s.dropped + VOICE_CAP, 64, `stolen ${s.stolen} dropped ${s.dropped}`);
  // A burst of one sound: its instance limit and minimum gap keep it to a few voices.
  const e2 = new AudioEngine(fakeEnv(true).env, DEFAULT_AUDIO);
  for (let i = 0; i < 64; i++) e2.play("fire.pellet", { position: { x: i, y: 0, z: 0 } });
  assert.ok(e2.voiceCount <= (mixOf(soundDef("fire.pellet")!).maxInstances), `pellets ${e2.voiceCount}`);
  e.dispose();
  e2.dispose();
});

test("important sounds steal from quiet ones; unimportant ones are dropped when full", () => {
  const { env, contexts } = fakeEnv(true);
  const e = new AudioEngine(env, DEFAULT_AUDIO);
  const ctx = contexts[0] as FakeContext;
  const ids = allSounds().filter((d) => !d.loop && (d.priority ?? 5) <= 4).map((d) => d.id);
  for (let i = 0; i < 400 && e.voiceCount < VOICE_CAP; i++) {
    ctx.currentTime += 0.005;
    e.play(ids[i % ids.length] as string, { position: { x: 30 + (i % 20), y: 0, z: 0 } });
  }
  assert.equal(e.voiceCount, VOICE_CAP);
  assert.equal(e.play("tectonic.blast"), true, "priority 10 steals");
  const before = e.stats().dropped;
  assert.equal(e.play("impact.sparks", { position: { x: 40, y: 0, z: 0 }, gain: 0.5, priority: 0 }), false, "quiet low priority is dropped");
  assert.equal(e.stats().dropped, before + 1);
  e.dispose();
});

test("far sounds are culled, distance shapes gain and cutoff", () => {
  const { env } = fakeEnv(true);
  const e = new AudioEngine(env, DEFAULT_AUDIO);
  assert.equal(distanceGain(4, 8), 1);
  assert.equal(distanceGain(80, 8), 0.1);
  assert.ok(distanceCutoff(0) === 20000 && distanceCutoff(200) < 5000 && distanceCutoff(10000) === 900);
  const ref = mixOf(soundDef("impact.sparks")!).refDistance;
  const gain = mixOf(soundDef("impact.sparks")!).gain;
  const far = (ref * gain) / CULL_GAIN + 50;
  assert.equal(e.play("impact.sparks", { position: { x: far, y: 0, z: 0 } }), false);
  assert.equal(e.stats().culled, 1);
  e.dispose();
});

test("background pre-rendering finishes every variant of every sound in small slices", () => {
  const { env } = fakeEnv(true);
  const queue: (() => void)[] = [];
  env.defer = (fn) => void queue.push(fn);
  const e = new AudioEngine(env, DEFAULT_AUDIO);
  let slices = 0;
  while (queue.length > 0 && slices < 10000) {
    (queue.shift() as () => void)();
    slices++;
  }
  const s = e.stats();
  assert.equal(s.rendered, s.total);
  assert.ok(slices > 1, `${slices} slices`);
  e.dispose();
});

test("loops run until stopped and survive their own updates", () => {
  const { env, contexts } = fakeEnv(true);
  const e = new AudioEngine(env, DEFAULT_AUDIO);
  const h = e.loop("boost.loop", { gain: 0 });
  assert.ok(h);
  h.setGain(0.7);
  h.setRate(1.1);
  assert.ok(h.playing);
  assert.equal(e.voiceCount, 1);
  const src = (contexts[0] as FakeContext).sources.at(-1);
  assert.equal(src?.loop, true);
  h.stop(0.1);
  assert.equal(h.playing, false);
  assert.equal(e.voiceCount, 0);
  assert.equal(e.playSamples(new Float32Array(2400), 24000), true);
  e.dispose();
});

// ----- game sounds ----------------------------------------------------------------------------

class RecordingEngine {
  plays: { id: string; positional: boolean; rate: number }[] = [];
  loops: { id: string; stopped: boolean; gain: number }[] = [];
  play(id: string, o: { position?: unknown; rate?: number } = {}): boolean {
    this.plays.push({ id, positional: Boolean(o.position), rate: o.rate ?? 1 });
    return true;
  }
  loop(id: string, o: { gain?: number } = {}): unknown {
    const rec = { id, stopped: false, gain: o.gain ?? 1 };
    this.loops.push(rec);
    return {
      get playing() {
        return !rec.stopped;
      },
      setGain: (g: number) => void (rec.gain = g),
      setRate: () => {},
      setPosition: () => {},
      stop: () => void (rec.stopped = true),
    };
  }
  setListener(): void {}
  stopAll(): void {}
  ids(): string[] {
    return this.plays.map((p) => p.id);
  }
}

const IDENT = { x: 0, y: 0, z: 0, w: 1 };
const VISUALS = [
  { shape: "bolt", color: [1, 1, 1] as [number, number, number], scale: 1 },
  { shape: "rocket", color: [1, 1, 1] as [number, number, number], scale: 1.8 },
];

function ship(id: number, x: number, alive = true, boosting = false): ShipState {
  return { id, team: 0, alive, boosting, position: { x, y: 0, z: 0 }, orientation: IDENT, shipType: 1, radius: 1, gadgetActive: false };
}

function frame(over: Partial<AudioFrame>): AudioFrame {
  return {
    time: 0,
    dt: 1 / 60,
    player: 0,
    online: true,
    stage: "match",
    camera: { position: { x: 0, y: 0, z: 0 }, orientation: IDENT },
    pose: { position: { x: 0, y: 0, z: 0 }, orientation: IDENT, velocity: { x: 0, y: 0, z: 0 }, alive: true },
    ships: [ship(1, 20), ship(2, 40)],
    projectiles: [],
    entities: [],
    cues: [],
    visuals: VISUALS,
    boosting: false,
    gadgetOf: () => "",
    gadgetIds: ["shield", "cloak"],
    names: new Map([
      [0, "Me"],
      [1, "Rivet"],
      [2, "Sprocket"],
    ]),
    ...over,
  };
}

test("fire and impact sounds by shape", () => {
  assert.deepEqual(fireSound("bolt", 1), { id: "fire.bolt", rate: 1 });
  assert.ok((fireSound("rocket", 1.8)?.rate ?? 1) < 1, "bigger shots sound lower");
  assert.equal(fireSound("spike", 1.2), null, "the Railspike cue plays it");
  assert.equal(fireSound("blob", 1)?.id, "fire.sneeze");
  assert.equal(impactSound(VISUALS[0]), "impact.sparks");
  assert.equal(impactSound(VISUALS[1]), "explosion.big");
  assert.equal(impactSound({ shape: "rocket", color: [1, 1, 1], scale: 1 }), "explosion.small");
});

test("GameAudio: shots, impacts, deaths, first blood, multi-kills", () => {
  const eng = new RecordingEngine();
  const said: { t: string; v: Record<string, unknown> }[] = [];
  const g = new GameAudio(eng as unknown as AudioEngine, { say: (t, v) => said.push({ t, v: { ...v } }) });
  g.update(frame({ time: 0 }));
  // Ship 1 fires a rocket: new projectile next to its owner.
  const shot: ProjectileState = { kind: 0, owner: 1, position: { x: 21, y: 0, z: 0 }, velocity: { x: -60, y: 0, z: 0 }, visual: 1, state: 0 };
  g.update(frame({ time: 0.016, projectiles: [shot] }));
  assert.deepEqual(eng.plays.at(-1), { id: "fire.rocket", positional: true, rate: fireSound("rocket", 1.8)?.rate ?? 0 });
  // It flies, then vanishes in rock: an explosion.
  g.update(frame({ time: 0.033, projectiles: [{ ...shot, position: { x: 20, y: 0, z: 0 } }] }));
  g.update(frame({ time: 0.05, projectiles: [] }));
  assert.equal(eng.plays.at(-1)?.id, "explosion.big");
  // Own shot: centred.
  const own: ProjectileState = { kind: 0, owner: 0, position: { x: 0.5, y: 0, z: 0 }, velocity: { x: 0, y: 0, z: -80 }, visual: 0, state: 0 };
  g.update(frame({ time: 0.066, projectiles: [own] }));
  assert.deepEqual(eng.plays.at(-1), { id: "fire.bolt", positional: false, rate: 1 });
  // Ship 1 hits ship 2 and kills it: clang, explosion, first blood.
  const hit = { kind: CUE_HIT, source: 1, target: 2, position: { x: 40, y: 0, z: 0 }, value: 20, tick: 0 };
  const n = eng.plays.length;
  g.update(frame({ time: 1, cues: [hit] }));
  assert.ok(eng.ids().slice(n).includes("impact.hull"));
  g.update(frame({ time: 1.2, ships: [ship(1, 20), ship(2, 40, false)] }));
  assert.equal(eng.plays.at(-1)?.id, "explosion.ship");
  assert.deepEqual(said.at(-1), { t: "firstBlood", v: { killer: "Rivet", victim: "Sprocket" } });
  // Ship 1 also gets me within 4 s: a double kill, my explosion is centred with a power-down.
  g.update(frame({ time: 2, ships: [ship(1, 20), ship(2, 40, false)], cues: [{ ...hit, target: 0 }] }));
  assert.equal(eng.plays.at(-1)?.id, "impact.hullOwn");
  g.update(frame({ time: 2.1, ships: [ship(1, 20), ship(2, 40, false)], pose: { position: { x: 0, y: 0, z: 0 }, orientation: IDENT, velocity: { x: 0, y: 0, z: 0 }, alive: false } }));
  assert.deepEqual(eng.ids().slice(-2), ["explosion.ship", "ship.powerdown"]);
  assert.equal(said.at(-1)?.t, "multiKill2");
});

test("GameAudio: cues, rumble loop, boost loop and pickups", () => {
  const eng = new RecordingEngine();
  const g = new GameAudio(eng as unknown as AudioEngine, null);
  const cue = (kind: number, o: Partial<{ source: number; target: number; value: number }> = {}) => ({ kind, source: o.source ?? 1, target: o.target ?? NO_SHIP, position: { x: 10, y: 0, z: 0 }, value: o.value ?? 0, tick: 0 });
  g.update(frame({ time: 0 }));
  g.update(frame({ time: 0.1, cues: [cue(CUE.politeWarning, { target: 0, value: 60 })] }));
  assert.deepEqual(eng.plays.at(-1), { id: "polite.chime", positional: false, rate: 1 });
  g.update(frame({ time: 0.2, cues: [cue(CUE.tectonicArmed, { value: 120 })] }));
  assert.equal(eng.loops.at(-1)?.id, "tectonic.rumble");
  g.update(frame({ time: 1.2 }));
  assert.ok((eng.loops.at(-1)?.gain ?? 0) > 0.35, "the rumble swells");
  g.update(frame({ time: 2.2, cues: [cue(CUE.shockwave, { value: 60 })] }));
  assert.equal(eng.plays.at(-1)?.id, "tectonic.blast");
  assert.equal(eng.loops.at(-1)?.stopped, true);
  g.update(frame({ time: 2.3, cues: [cue(CUE.gadget, { value: 0 })] }));
  assert.equal(eng.plays.at(-1)?.id, "gadget.shield");
  // Boost: a loop while held, stopped after release.
  for (let i = 0; i < 10; i++) g.update(frame({ time: 3 + i / 60, boosting: true }));
  const boost = eng.loops.find((l) => l.id === "boost.loop");
  assert.ok(boost && !boost.stopped && boost.gain > 0.5);
  for (let i = 0; i < 90; i++) g.update(frame({ time: 4 + i / 60, boosting: false }));
  assert.equal(boost.stopped, true);
  // A pickup next to me vanishes: the pickup chime.
  const pickup = { id: 9, kind: "weaponPickup", visual: 0, owner: 255, team: 255, position: { x: 2, y: 0, z: 0 }, velocity: { x: 0, y: 0, z: 0 }, rotation: IDENT, state: [0, 0, 0, 0] as [number, number, number, number], radius: 1 };
  g.update(frame({ time: 6, entities: [pickup] }));
  g.update(frame({ time: 6.02, entities: [] }));
  assert.equal(eng.plays.at(-1)?.id, "pickup");
});

test("GameAudio announces the match start after the countdown, not when joining mid-match", () => {
  const said: string[] = [];
  const g = new GameAudio(new RecordingEngine() as unknown as AudioEngine, { say: (t: string) => void said.push(t) });
  g.update(frame({ stage: "match" }));
  assert.equal(said.length, 0);
  const g2 = new GameAudio(new RecordingEngine() as unknown as AudioEngine, { say: (t: string) => void said.push(t) });
  g2.update(frame({ stage: "countdown" }));
  g2.update(frame({ stage: "match" }));
  assert.deepEqual(said, ["matchStart"]);
  const g3 = new GameAudio(new RecordingEngine() as unknown as AudioEngine, { say: (t: string) => void said.push(t) });
  g3.update(frame({ stage: "match", online: false }));
  assert.deepEqual(said, ["matchStart", "matchStart"], "practice starts at once");
});

test("announcer event ids are the comedy events of data/events (ls-sim EventKind)", async () => {
  const { readdirSync } = await import("node:fs");
  const { fileURLToPath } = await import("node:url");
  const dir = fileURLToPath(new URL("../../../data/events/", import.meta.url));
  const files = readdirSync(dir)
    .filter((f) => f.endsWith(".ron") && f !== "director.ron")
    .map((f) => f.slice(0, -4))
    .sort();
  assert.deepEqual([...EVENT_IDS].sort(), files);
});
