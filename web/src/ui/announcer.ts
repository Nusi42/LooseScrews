// The announcer (M4 A6, milestone-4-plan.md §3.7): a caption character at the
// top left (portrait, name, line) that speaks event announcements, first
// blood, multi-kills, cave-ins and the Tired announcer's kill-cam narration
// in a synthesized gibberish voice (audio/voice.ts). One of three voice
// packs per match (audio/announcer/), picked from the match seed. A line
// starts with its sting (fanfare, gong, chiptune run or a blip), the words
// appear as they are spoken, the effects duck under the voice, and the
// portrait's mouth moves while it talks. Captions show even without sound
// (before the first gesture, muted): the voice's timing still paces them.
//
// Lines queue (at most `MAX_QUEUE` waiting; an event announcement jumps the
// queue) so a burst of kills does not stack captions.

import type { AudioEngine } from "../audio/engine.ts";
import { fillLine, LinePicker, type LineVars, type Trigger, type VoicePack } from "../audio/announcer/lines.ts";
import { renderSpeech, type Speech } from "../audio/voice.ts";

/** Speech is rendered at this rate (voices need nothing above 12 kHz; half the cost of 48 kHz). */
export const SPEECH_RATE = 24000;
/** Lines waiting behind the one on screen. */
export const MAX_QUEUE = 2;
/** Caption time after the voice ends (s). */
const LINGER = 1.6;
/** Slide-out time (ms; announcer.css). */
const HIDE_MS = 320;

/** The sting before a trigger's line and the delay (s) until the voice starts. */
export function stingFor(trigger: Trigger): { id: string; lead: number } {
  if (trigger.startsWith("event.")) return { id: "sting.event", lead: 0.62 };
  if (trigger === "firstBlood") return { id: "sting.firstBlood", lead: 0.45 };
  if (trigger.startsWith("multiKill")) return { id: "sting.multiKill", lead: 0.4 };
  return { id: "caption.blip", lead: 0.12 };
}

/** The pack's portrait: a scrap-built broadcast robot with a screen face (SVG, mouth class `mouth`). */
export function portraitSvg(pack: VoicePack): string {
  const a = pack.accent;
  const face: Record<VoicePack["id"], string> = {
    // Doug: heavy lids, eye bags, flat mouth, a bent antenna.
    tired: `
      <path d="M19 27.5 a4 4 0 0 0 8 0 z M37 27.5 a4 4 0 0 0 8 0 z" fill="${a}"/>
      <path d="M18 27 h10 M36 27 h10" stroke="${a}" stroke-width="2" stroke-linecap="round"/>
      <path d="M19.5 34 q3.5 1.6 7 0 M37.5 34 q3.5 1.6 7 0" stroke="${a}" stroke-opacity=".45" stroke-width="1.2" fill="none" stroke-linecap="round"/>
      <g class="mouth"><path d="M26 39.5 q6 -1.4 12 0" stroke="${a}" stroke-width="2.2" fill="none" stroke-linecap="round"/></g>`,
    // Sparky: huge shiny eyes, raised brows, a wide open grin.
    overexcited: `
      <path d="M17 20.5 l8 -2.5 M47 20.5 l-8 -2.5" stroke="${a}" stroke-width="2" stroke-linecap="round"/>
      <circle cx="22.5" cy="27" r="5" fill="${a}"/><circle cx="41.5" cy="27" r="5" fill="${a}"/>
      <circle cx="24.3" cy="25.2" r="1.7" fill="#fff"/><circle cx="43.3" cy="25.2" r="1.7" fill="#fff"/>
      <g class="mouth"><path d="M24 35.5 h16 q0 7 -8 7 q-8 0 -8 -7 z" fill="${a}"/><path d="M27.5 39.6 q4.5 2.4 9 0" stroke="#0d1118" stroke-opacity=".5" stroke-width="1.4" fill="none"/></g>`,
    // Brenda: closed smiling eyes, one raised brow, a tight little smile.
    passiveAggressive: `
      <path d="M18 21.5 l7 -1 M39 18.5 l7 2.4" stroke="${a}" stroke-width="2" stroke-linecap="round"/>
      <path d="M18.5 28.5 q4 -5 8 0 M37.5 28.5 q4 -5 8 0" stroke="${a}" stroke-width="2.2" fill="none" stroke-linecap="round"/>
      <circle cx="17" cy="34" r="2.2" fill="${a}" fill-opacity=".35"/><circle cx="47" cy="34" r="2.2" fill="${a}" fill-opacity=".35"/>
      <g class="mouth"><path d="M27 37.5 q5 3 10 0" stroke="${a}" stroke-width="2.2" fill="none" stroke-linecap="round"/></g>`,
  };
  const antenna: Record<VoicePack["id"], string> = {
    tired: `<path d="M32 10 L35 5.5 L40.5 6.5" stroke="#8b93a7" stroke-width="2" fill="none" stroke-linecap="round" stroke-linejoin="round"/><circle cx="41" cy="6.6" r="2.2" fill="${a}" fill-opacity=".55"/>`,
    overexcited: `<path d="M32 10 V4" stroke="#8b93a7" stroke-width="2" stroke-linecap="round"/><path d="M32 0.4 l1.2 2.4 2.6 0.4 -1.9 1.8 0.5 2.6 -2.4 -1.3 -2.4 1.3 0.5 -2.6 -1.9 -1.8 2.6 -0.4 z" fill="${a}"/>`,
    passiveAggressive: `<path d="M32 10 V5" stroke="#8b93a7" stroke-width="2" stroke-linecap="round"/><path d="M32 4.6 l-5 -2.6 v5.2 z M32 4.6 l5 -2.6 v5.2 z" fill="${a}"/><circle cx="32" cy="4.6" r="1.4" fill="${a}"/>`,
  };
  return `<svg viewBox="0 0 64 64" aria-hidden="true">
    <defs><radialGradient id="ann-glow-${pack.id}" cx="50%" cy="45%" r="65%"><stop offset="0" stop-color="${a}" stop-opacity=".22"/><stop offset="1" stop-color="${a}" stop-opacity="0"/></radialGradient></defs>
    ${antenna[pack.id]}
    <rect x="5" y="25" width="4" height="12" rx="1.5" fill="#4a5163"/><rect x="55" y="25" width="4" height="12" rx="1.5" fill="#4a5163"/>
    <rect x="8" y="10" width="48" height="46" rx="12" fill="#2b3142" stroke="#5b6378" stroke-width="1.2"/>
    <circle cx="13" cy="15" r="1.2" fill="#6c7489"/><circle cx="51" cy="15" r="1.2" fill="#6c7489"/><circle cx="13" cy="51" r="1.2" fill="#6c7489"/><circle cx="51" cy="51" r="1.2" fill="#6c7489"/>
    <rect x="12.5" y="15.5" width="39" height="31" rx="7" fill="#0d1118"/>
    <rect x="12.5" y="15.5" width="39" height="31" rx="7" fill="url(#ann-glow-${pack.id})"/>
    ${face[pack.id]}
    <path d="M24 51 h16 M26 53.5 h12" stroke="#5b6378" stroke-width="1.3" stroke-linecap="round"/>
  </svg>`;
}

interface Pending {
  trigger: Trigger;
  text: string;
}

export class Announcer {
  readonly root: HTMLElement;
  private readonly engine: AudioEngine | null;
  private pack: VoicePack;
  private readonly picker: LinePicker;
  private readonly portrait: HTMLElement;
  private readonly name: HTMLElement;
  private readonly title: HTMLElement;
  private readonly line: HTMLElement;
  private queue: Pending[] = [];
  private busy = false;
  private timers: ReturnType<typeof setTimeout>[] = [];
  private disposed = false;
  /** Lines said so far (tests, smoke). */
  readonly said: string[] = [];

  constructor(parent: HTMLElement, engine: AudioEngine | null, pack: VoicePack, rnd: () => number = Math.random) {
    this.engine = engine;
    this.pack = pack;
    this.picker = new LinePicker(rnd);
    const root = document.createElement("div");
    root.className = "announcer";
    root.setAttribute("role", "status");
    root.setAttribute("aria-live", "polite");
    root.innerHTML = `
      <div class="announcer-portrait"></div>
      <div class="announcer-body">
        <div class="announcer-head"><span class="announcer-name"></span><span class="announcer-title"></span></div>
        <div class="announcer-line"></div>
      </div>`;
    parent.appendChild(root);
    this.root = root;
    this.portrait = root.querySelector(".announcer-portrait") as HTMLElement;
    this.name = root.querySelector(".announcer-name") as HTMLElement;
    this.title = root.querySelector(".announcer-title") as HTMLElement;
    this.line = root.querySelector(".announcer-line") as HTMLElement;
    this.setPack(pack);
  }

  get voicePack(): VoicePack {
    return this.pack;
  }

  /** Switches the voice pack (a new match). */
  setPack(pack: VoicePack): void {
    this.pack = pack;
    this.root.dataset["pack"] = pack.id;
    this.root.style.setProperty("--ann-accent", pack.accent);
    this.portrait.innerHTML = portraitSvg(pack);
    this.name.textContent = pack.name;
    this.title.textContent = pack.title;
  }

  /** Says a line of `trigger` (with `{placeholders}` filled from `vars`). */
  say(trigger: Trigger, vars: LineVars = {}): void {
    if (this.disposed) return;
    this.sayText(trigger, fillLine(this.picker.pick(this.pack, trigger), vars));
  }

  /** Says `text` as a line of `trigger` (its sting). */
  sayText(trigger: Trigger, text: string): void {
    if (this.disposed) return;
    const item = { trigger, text };
    if (trigger.startsWith("event.")) this.queue.unshift(item);
    else this.queue.push(item);
    if (this.queue.length > MAX_QUEUE) this.queue.length = MAX_QUEUE;
    if (!this.busy) this.next();
  }

  /** Shows a line of `trigger` at once, every word visible, mid-word (gallery stills; no sound). */
  still(trigger: Trigger, vars: LineVars = {}): void {
    this.clear();
    const text = fillLine(this.picker.pick(this.pack, trigger), vars);
    this.said.push(text);
    this.line.innerHTML = text
      .split(/\s+/)
      .filter(Boolean)
      .map((w) => `<span class="w">${w.replace(/[&<>]/g, (c) => (c === "&" ? "&amp;" : c === "<" ? "&lt;" : "&gt;"))}</span>`)
      .join(" ");
    this.root.classList.add("still", "shown", "speaking");
  }

  /** Drops everything waiting and hides the caption (a new match). */
  clear(): void {
    for (const t of this.timers) clearTimeout(t);
    this.timers = [];
    this.queue = [];
    this.busy = false;
    this.root.classList.remove("shown", "speaking", "still");
  }

  private later(ms: number, fn: () => void): void {
    this.timers.push(setTimeout(fn, ms));
  }

  private next(): void {
    const item = this.queue.shift();
    if (!item) {
      this.busy = false;
      return;
    }
    this.busy = true;
    this.said.push(item.text);
    const speech: Speech = renderSpeech(item.text, this.pack.voice, SPEECH_RATE, this.said.length);
    const sting = stingFor(item.trigger);
    const engine = this.engine;
    engine?.play(sting.id);
    // Words appear as they are spoken.
    this.line.innerHTML = "";
    const words = item.text.split(/\s+/).filter(Boolean);
    words.forEach((w, i) => {
      const span = document.createElement("span");
      span.className = "w";
      span.textContent = w;
      const at = sting.lead + (speech.words[i]?.start ?? speech.duration * (i / Math.max(1, words.length)));
      span.style.animationDelay = `${at.toFixed(3)}s`;
      this.line.appendChild(span);
      this.line.appendChild(document.createTextNode(" "));
    });
    this.root.classList.remove("shown");
    // Restart the slide-in.
    void this.root.offsetWidth;
    this.root.classList.add("shown");
    this.later(sting.lead * 1000, () => {
      if (engine?.playSamples(speech.samples, SPEECH_RATE, "voice", 1)) engine.duck(speech.duration + 0.2);
      this.root.classList.add("speaking");
      this.later(speech.duration * 1000, () => this.root.classList.remove("speaking"));
    });
    const hold = (sting.lead + speech.duration + LINGER) * 1000;
    this.later(hold, () => {
      this.root.classList.remove("shown");
      this.later(HIDE_MS, () => this.next());
    });
  }

  dispose(): void {
    this.disposed = true;
    this.clear();
    this.root.remove();
  }
}
