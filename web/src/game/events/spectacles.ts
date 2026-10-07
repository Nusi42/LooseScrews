// M4 C3's spectacle event visuals behind one object (milestone-4-plan.md
// §3.2, wave C "C3"): Duck parade, Hat rain, Cat paw, Disco inferno and the
// Tired announcer's kill-cams. main.ts creates one per game and calls, each
// frame: `update` (after the world visuals), `addToScene` (after the scene
// builder, which leaves the props to us: `skipKinds`), uses `lightLevel`,
// `lights` and `shake`, and after the main render `renderKillCam`. The
// gallery drives the same object with synthetic states.
//
// Also here: the five events' own sting when the EventAnnounce cue names one
// of them (the announcer's `event.<id>` line and the event card belong to
// C1's hub, game/events/index.ts, for all 18 events) and the own hat count
// chip.

import type { PointLight } from "../../renderer/lights.ts";
import type { WorldVolume } from "../../renderer/wells.ts";
import type { Renderer, SceneInstances } from "../../renderer/renderer.ts";
import type { LineVars, Trigger } from "../../audio/announcer/lines.ts";
import { EVENT_IDS } from "../../audio/announcer/lines.ts";
import { buildScene } from "../scene.ts";
import type { VfxSink } from "../vfxLibrary.ts";
import type { ShipState, VisualDef } from "../views.ts";
import { blastScene, clipClock, clipFrame, KillCamDirector, killCamPose, KillCamWindow, type KillCamClip } from "../killcam.ts";
import { CatPawFx } from "./catPaw.ts";
import { DiscoInfernoFx } from "./discoInferno.ts";
import { DuckParadeFx } from "./duckParade.ts";
import { HatRainFx, REVEAL_HATS } from "./hatRain.ts";
import { NONE, SPECTACLE_CUE, type SpectacleInput, type SpectacleSoundEngine } from "./spectacleCommon.ts";

/** The events this module draws (data/events ids). */
export const SPECTACLE_EVENTS = ["duck_parade", "hat_rain", "cat_paw", "disco_inferno", "tired_announcer"] as const;

/** Entity kinds drawn here instead of by the scene builder. */
export const SPECTACLE_KINDS: ReadonlySet<string> = new Set(["duck", "hat", "catPaw", "discoBall"]);

/** Sting of each event's announcement (audio/sounds/spectacles.ts). */
const STINGS: Readonly<Record<string, string>> = {
  duck_parade: "duck.fanfare",
  hat_rain: "hat.whistle",
  cat_paw: "paw.mrrp",
  disco_inferno: "disco.scratch",
  tired_announcer: "announcer.yawn",
};

export interface SpectacleFrame {
  lights: PointLight[];
  /** World effect volumes (renderer/wells.ts) to draw with the world visuals' (Cat paw's cracking wall). */
  volumes: WorldVolume[];
  /** Sun and ambient factor for the renderer (RenderSettings.lightLevel). */
  lightLevel: number;
  /** Factor on the headlight (Disco inferno turns it down too, so the spots and trails carry the room). */
  headlight: number;
  shake: number;
  /** The own hats (0 = none) and whether everyone sees you for them. */
  ownHats: number;
  revealed: boolean;
}

/** Everything the kill-cam window needs to draw a replay frame. */
export interface KillCamDraw {
  visuals: readonly VisualDef[];
  teamOf: (id: number) => number | undefined;
  teams: number;
  /** Creates the window's renderer (main.ts: `Renderer.create(canvas, { device, quality: "low", particles: false, shadows: false })`). */
  create: (canvas: HTMLCanvasElement, device: GPUDevice) => Promise<Renderer>;
}

export class SpectacleVisuals {
  readonly ducks = new DuckParadeFx();
  readonly hats = new HatRainFx();
  readonly paw = new CatPawFx();
  readonly disco = new DiscoInfernoFx();
  readonly killcam = new KillCamDirector();
  readonly enabled: boolean;
  private readonly say: (t: Trigger, v?: LineVars) => void;
  private readonly engine: SpectacleSoundEngine | null;
  private window: KillCamWindow | null = null;
  private readonly ui: HTMLElement | null;
  private chip: HTMLDivElement | null = null;
  private playing: { clip: KillCamClip; u: number } | null = null;
  private last: SpectacleInput | null = null;

  constructor(ui: HTMLElement | null, sink: VfxSink | null, enabled: boolean, engine: SpectacleSoundEngine | null, say: (t: Trigger, v?: LineVars) => void) {
    this.ui = ui;
    this.enabled = enabled;
    this.engine = enabled ? engine : null;
    this.say = say;
    for (const m of [this.ducks, this.hats, this.paw, this.disco]) {
      m.sink = enabled ? sink : null;
      m.engine = this.engine;
    }
  }

  /** Entity kinds the scene builder must leave to us (none when switched off: they draw as before). */
  get skipKinds(): ReadonlySet<string> {
    return this.enabled ? SPECTACLE_KINDS : new Set();
  }

  reset(): void {
    this.ducks.reset();
    this.hats.reset();
    this.paw.reset();
    this.disco.reset();
    this.killcam.reset();
    this.playing = null;
    this.window?.show(false);
  }

  update(i: SpectacleInput): SpectacleFrame {
    this.last = i;
    if (!this.enabled) return { lights: [], volumes: [], lightLevel: 1, headlight: 1, shake: 0, ownHats: 0, revealed: false };
    this.announce(i);
    this.ducks.update(i);
    this.hats.update(i);
    this.paw.update(i);
    this.disco.update(i);
    this.tired(i);
    const revealed = this.hats.ownHats >= REVEAL_HATS && i.mood.eventId === "hat_rain" && i.mood.phase === "active";
    this.updateChip(i.quiet ? 0 : this.hats.ownHats, revealed);
    return {
      lights: [...this.ducks.lights, ...this.hats.lights, ...this.paw.lights, ...this.disco.lights],
      volumes: this.paw.volumes,
      lightLevel: this.disco.lightLevel,
      headlight: 0.3 + 0.7 * this.disco.lightLevel,
      shake: this.paw.shake,
      ownHats: this.hats.ownHats,
      revealed,
    };
  }

  /** The announcement of one of our events: the event's sting (the hub says the line). */
  private announce(i: SpectacleInput): void {
    for (const c of i.cues) {
      if (c.kind !== SPECTACLE_CUE.eventAnnounce) continue;
      const id = EVENT_IDS[c.value];
      if (!id || !(SPECTACLE_EVENTS as readonly string[]).includes(id)) continue;
      const sting = STINGS[id];
      if (sting && id !== "cat_paw" && id !== "disco_inferno") this.engine?.play(sting, {});
    }
  }

  /** Tired announcer: record, schedule kill-cams, narrate. */
  private tired(i: SpectacleInput): void {
    const ships: ShipState[] = [...i.ships];
    if (i.own) ships.push({ id: i.own.id, team: i.own.team, alive: true, boosting: false, position: i.own.position, orientation: i.own.orientation, shipType: i.own.shipType, radius: i.own.radius, gadgetActive: false });
    this.killcam.recorder.push({ time: i.time, ships, shots: i.shots, entities: i.entities });
    const name = (id: number): string => (id === NONE ? "the level" : i.nameOf(id) || "someone");
    for (const k of this.killcam.onCues(i.cues, i.time)) this.say("killcam", { killer: name(k.killer), victim: name(k.victim) });
    const p = this.killcam.update(i.time);
    if (p?.started) {
      this.say("killcam", { killer: name(p.clip.killer), victim: name(p.clip.victim) });
      this.engine?.play("killcam.rewind", {});
    }
    this.playing = p ? { clip: p.clip, u: p.u } : null;
    // Warm the window's renderer up during the announcement (it takes a moment).
    if (i.mood.eventId === "tired_announcer") this.wantWindow = true;
  }

  private wantWindow = false;

  /** Our props into the scene. */
  addToScene(scene: SceneInstances): void {
    const i = this.last;
    if (!this.enabled || !i) return;
    this.ducks.addToScene(scene, i.time, i.up);
    this.hats.addToScene(scene, i.entities, i.ships, i.time, i.up);
    this.paw.addToScene(scene);
    this.disco.addToScene(scene, i.time, i.up);
  }

  /** The kill-cam window: creates it when the event comes, draws the playing clip (after the main render). */
  renderKillCam(main: Renderer, d: KillCamDraw): void {
    const i = this.last;
    if (!this.enabled || !i || !this.ui) return;
    if ((this.wantWindow || this.playing) && !this.window) {
      this.window = new KillCamWindow(this.ui);
      void this.window.prepare(main, d.create);
    }
    const w = this.window;
    if (!w) return;
    // Results or lobby: no kill-cam over them (a kill just before the end would play on top).
    if (i.quiet) {
      w.show(false);
      return;
    }
    const p = this.playing;
    if (!p) {
      if (i.mood.eventId === "tired_announcer" && i.mood.phase === "announce") w.standby("Kill-cams for everyone");
      else w.show(false);
      return;
    }
    const frame = clipFrame(p.clip, p.u);
    const name = (id: number): string => (id === NONE ? "The level" : i.nameOf(id) || "?");
    w.show(true, `${name(p.clip.killer)} ▸ ${name(p.clip.victim)}`);
    if (!frame) return;
    const pose = killCamPose(frame, p.clip.killer, p.clip.victim, p.clip.at, p.u);
    const scene = buildScene(
      frame.ships.filter((s) => s.alive),
      frame.shots,
      frame.entities,
      d.visuals,
      d.teamOf,
      frame.time,
      { teams: d.teams, vfx: false, skipKinds: new Set([...SPECTACLE_KINDS, "pigmentPickup"]) },
    );
    const clock = clipClock(p.u);
    const blast = blastScene(p.clip.at, clock);
    (scene.translucent.sphere ??= []).push(...blast.translucent);
    (scene.opaque.orb ??= []).push(...blast.opaque);
    const lights: PointLight[] = blast.light ? [blast.light] : [];
    // The killer's engine and the victim lit a little, so the pair reads in a dark cave.
    for (const s of frame.ships) if (s.alive && (s.id === p.clip.killer || s.id === p.clip.victim)) lights.push({ position: s.position, color: s.id === p.clip.victim ? [1, 0.5, 0.4] : [0.6, 0.8, 1], intensity: 6, range: 12 });
    const stamp = clock >= 0 ? "K.O." : "";
    w.render(pose, scene, lights, p.u, stamp, main, frame.time);
  }

  /** Whether a kill-cam plays (tests, smoke). */
  get killCamPlaying(): boolean {
    return this.playing !== null;
  }

  /** The window's last GPU time (ms) for the debug overlay and the gallery. */
  get killCamGpuMs(): number | null {
    return this.window?.gpuMs ?? null;
  }

  private updateChip(hats: number, revealed: boolean): void {
    if (!this.ui) return;
    if (hats <= 0) {
      this.chip?.classList.remove("shown");
      return;
    }
    if (!this.chip) {
      this.chip = document.createElement("div");
      this.chip.className = "hat-chip";
      this.ui.appendChild(this.chip);
    }
    const text = `<svg viewBox="0 0 32 32" aria-hidden="true"><path d="M5 23 h22 a2 2 0 0 1 0 3 h-22 a2 2 0 0 1 0-3z M10 22 l1.5-14 a1.5 1.5 0 0 1 1.5-1.3 h6 a1.5 1.5 0 0 1 1.5 1.3 l1.5 14z" fill="currentColor"/><rect x="10.6" y="17.4" width="10.8" height="2.4" fill="#e8434f"/></svg><span class="hat-count">×${hats}</span>${revealed ? '<span class="hat-reveal">Everyone sees you</span>' : ""}`;
    if (this.chip.innerHTML !== text) this.chip.innerHTML = text;
    this.chip.classList.add("shown");
    this.chip.classList.toggle("revealed", revealed);
  }

  dispose(): void {
    this.reset();
    this.window?.dispose();
    this.window = null;
    this.chip?.remove();
    this.chip = null;
  }
}
