// Audio entry points (M4 A6). `audioEngine()` is the page's one engine
// (created on first use; silent until the first user gesture);
// `MatchAudio` is what main.ts runs per game: the game sounds (game.ts) and
// the announcer caption (ui/announcer.ts) with the match's voice pack.
// `?audio=demo` (demo.ts) lists every sound.
//
// For other packages: play a sound with `audioEngine().play(id, { position })`
// (or a loop with `.loop`), add sounds in sounds/*.ts, make the announcer
// speak with `MatchAudio.announcer.say("event.<id>", { target })`.
//
// `window.__lsAudio` (stats, play, say) serves the browser smoke test and
// headless screenshots.

import { hashSeed } from "../renderer/rockTextures.ts";
import { Announcer } from "../ui/announcer.ts";
import { packForSeed } from "./announcer/packs.ts";
import type { Trigger, LineVars } from "./announcer/lines.ts";
import { AudioEngine, browserAudioEnv, type EngineStats } from "./engine.ts";
import { GameAudio, type AudioFrame } from "./game.ts";
import { registerSounds, type Bus } from "./library.ts";
import { BUILTIN_SOUNDS } from "./sounds/index.ts";

registerSounds(BUILTIN_SOUNDS);

let engine: AudioEngine | null = null;
let current: MatchAudio | null = null;

export interface AudioDebug {
  stats(): EngineStats;
  play(id: string): boolean;
  say(trigger: Trigger, vars?: LineVars): void;
  /** Lines the announcer said in this game. */
  said(): string[];
}

/** The page's audio engine (installs the gesture listeners on first call). */
export function audioEngine(): AudioEngine {
  if (!engine) {
    engine = new AudioEngine(browserAudioEnv());
    if (typeof window !== "undefined") {
      const debug: AudioDebug = {
        stats: () => (engine as AudioEngine).stats(),
        play: (id) => (engine as AudioEngine).play(id),
        say: (t, v) => current?.announcer.say(t, v),
        said: () => [...(current?.announcer.said ?? [])],
      };
      (window as unknown as { __lsAudio?: AudioDebug }).__lsAudio = debug;
    }
  }
  return engine;
}

/** A short sample of a bus after its volume slider moved (start screen). */
export function previewBus(bus: Bus | "master"): void {
  const e = audioEngine();
  e.unlock();
  if (bus === "voice") e.play("caption.blip");
  else if (bus === "music") e.play("sting.multiKill");
  else e.play("fire.bolt");
}

/** The voice pack seed of a world (its share code), until the match seed reaches the client. */
export function matchSeed(shareCode: string): number {
  return hashSeed(shareCode);
}

/** Sound and announcer of one game (main.ts: create, `update` per frame, `dispose`). */
export class MatchAudio {
  readonly announcer: Announcer;
  private readonly sound: GameAudio;

  constructor(ui: HTMLElement, shareCode: string) {
    const e = audioEngine();
    this.announcer = new Announcer(ui, e, packForSeed(matchSeed(shareCode)));
    this.sound = new GameAudio(e, this.announcer);
    current = this;
  }

  /** A new match on the same connection: new level, new voice pack. */
  newMatch(shareCode: string): void {
    this.sound.reset();
    this.announcer.clear();
    this.announcer.setPack(packForSeed(matchSeed(shareCode)));
  }

  update(f: AudioFrame): void {
    this.sound.update(f);
  }

  dispose(): void {
    this.sound.dispose();
    this.announcer.dispose();
    if (current === this) current = null;
  }
}

export type { AudioFrame } from "./game.ts";
