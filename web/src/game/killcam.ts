// Kill-cam (M4 C3; milestone-4-plan.md §3.2 Tired announcer, §3.7): while
// the Tired announcer event runs, every kill plays a 3 s replay of the
// killer's view for everyone, picture in picture, narrated by the
// announcer. The simulation schedules them (B3: `CueKind::KillCam` with
// killer, victim, the death position and the delay until its slot; u16::MAX
// = narrated only), so every client shows the same kill-cams at the same
// time. The client keeps the last seconds of what it drew (ships, the own
// ship, projectiles, entities; KillCamRecorder) and replays them from a
// chase camera behind the killer (the victim's view for kills by the level)
// in a small window drawn by a second renderer on the same GPU device that
// shares the main renderer's terrain meshes (Renderer.shareTerrain).
//
// The maths (recording window, clip timeline, camera) is pure and tested;
// KillCamWindow is the DOM and GPU side.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { CameraView, Instance, Renderer } from "../renderer/renderer.ts";
import type { Atmosphere } from "../renderer/atmosphere.ts";
import type { PointLight } from "../renderer/lights.ts";
import type { Cue } from "./mechanics.ts";
import type { EntityState, ProjectileState, ShipState } from "./views.ts";

/** `ls_sim::CueKind::KillCam`. */
export const KILLCAM_CUE = 19;
/** Cue value of a kill that is only narrated (the queue was full). */
export const NARRATED_ONLY = 65535;
/** data/events/tired_announcer.ron `killcam_secs`. */
export const KILLCAM_SECS = 3;
/** Seconds of the replay after the kill (the blast); the rest comes before it. */
export const AFTER_KILL = 0.4;
/** Vertical field of view of the kill-cam (rad): a little tele, the pair fills the window. */
export const KILLCAM_FOV = 0.85;
/** Seconds the recorder keeps. */
export const RECORD_SECS = KILLCAM_SECS + 1;
/** Ship id meaning "none" (the level killed). */
const NONE = 255;

/** One recorded frame: what the game drew at `time` (client seconds). */
export interface ReplayFrame {
  time: number;
  /** Every ship, the own one included. */
  ships: ShipState[];
  shots: readonly ProjectileState[];
  entities: readonly EntityState[];
}

/** Keeps the last RECORD_SECS of frames. */
export class KillCamRecorder {
  private frames: ReplayFrame[] = [];

  push(f: ReplayFrame): void {
    this.frames.push(f);
    const cut = f.time - RECORD_SECS;
    let k = 0;
    while (k < this.frames.length && this.frames[k]!.time < cut) k++;
    if (k > 0) this.frames.splice(0, k);
  }

  /** Frames with `from <= time <= to` (oldest first). */
  window(from: number, to: number): ReplayFrame[] {
    return this.frames.filter((f) => f.time >= from && f.time <= to);
  }

  clear(): void {
    this.frames = [];
  }

  get size(): number {
    return this.frames.length;
  }
}

/** One kill-cam: who, where, when it plays and the frames it shows. */
export interface KillCamClip {
  killer: number;
  victim: number;
  /** Where the victim died. */
  at: Vec3;
  /** Client time of the kill. */
  killTime: number;
  /** Client time the window starts playing. */
  start: number;
  frames: ReplayFrame[];
}

/** Frame of a clip shown `u` (0..1) into its playback (nearest recorded frame). */
export function clipFrame(clip: KillCamClip, u: number): ReplayFrame | null {
  const fs = clip.frames;
  if (fs.length === 0) return null;
  const t = clip.killTime + AFTER_KILL - KILLCAM_SECS + Math.min(1, Math.max(0, u)) * KILLCAM_SECS;
  let best = fs[0]!;
  for (const f of fs) if (Math.abs(f.time - t) < Math.abs(best.time - t)) best = f;
  return best;
}

/** Replay time (s, relative to the kill: negative before it) at `u`. */
export function clipClock(u: number): number {
  return AFTER_KILL - KILLCAM_SECS + Math.min(1, Math.max(0, u)) * KILLCAM_SECS;
}

const smooth = (a: number, b: number, x: number): number => {
  const t = Math.min(1, Math.max(0, (x - a) / (b - a)));
  return t * t * (3 - 2 * t);
};

/**
 * The kill-cam's camera in a frame: behind and a little above the killer,
 * looking where it looks and, as the kill nears, at the victim. Without
 * the killer (the level, or out of sight): circling the victim.
 */
export function killCamPose(frame: ReplayFrame, killer: number, victim: number, at: Vec3, u: number): { position: Vec3; orientation: Quat } {
  const k = killer !== NONE ? frame.ships.find((s) => s.id === killer && s.alive) : undefined;
  const v = frame.ships.find((s) => s.id === victim);
  const target = v && v.alive ? v.position : at;
  if (k) {
    const fwd = q.forward(k.orientation);
    const upv = q.up(k.orientation);
    const back = 1.5 + 2.8 * k.radius;
    const cam = v3.add(v3.addScaled(k.position, fwd, -back), v3.scale(upv, 0.45 + 0.9 * k.radius));
    const ahead = v3.addScaled(k.position, fwd, 30);
    const look = v3.lerp(ahead, target, smooth(0.35, 0.8, u));
    const d = v3.sub(look, cam);
    return { position: cam, orientation: q.lookRotation(v3.length(d) > 1e-3 ? d : fwd, upv) };
  }
  // The level did it (or the killer is gone): a slow orbit around the victim.
  const a = 0.6 + u * 1.1;
  const r = 9;
  const cam = v3.add(target, { x: Math.cos(a) * r, y: 3.5, z: Math.sin(a) * r });
  return { position: cam, orientation: q.lookRotation(v3.sub(target, cam), { x: 0, y: 1, z: 0 }) };
}

/** Kill-cam cues of a frame as clips to capture: [killer, victim, at, start delay (s)], or null for narrated-only. */
export function parseKillCam(c: Cue): { killer: number; victim: number; at: Vec3; delay: number | null } | null {
  if (c.kind !== KILLCAM_CUE) return null;
  return { killer: c.source, victim: c.target, at: c.position, delay: c.value >= NARRATED_ONLY ? null : c.value / 60 };
}

/** The schedule: cues in, clips out when their frames are recorded and their slot comes. */
export class KillCamDirector {
  readonly recorder = new KillCamRecorder();
  /** Clips waiting for their AFTER_KILL frames or their start. */
  private pending: KillCamClip[] = [];
  private current: KillCamClip | null = null;

  reset(): void {
    this.recorder.clear();
    this.pending = [];
    this.current = null;
  }

  /**
   * Takes this frame's cues (after recording the frame). Returns the kills to
   * narrate now: every kill-cam cue (spoken when its window starts, or at
   * once when only narrated).
   */
  onCues(cues: readonly Cue[], time: number): Array<{ killer: number; victim: number; narratedOnly: boolean }> {
    const say: Array<{ killer: number; victim: number; narratedOnly: boolean }> = [];
    for (const c of cues) {
      const k = parseKillCam(c);
      if (!k) continue;
      if (k.delay === null) {
        say.push({ killer: k.killer, victim: k.victim, narratedOnly: true });
        continue;
      }
      // The window opens AFTER_KILL late so the blast is on tape.
      this.pending.push({ killer: k.killer, victim: k.victim, at: k.at, killTime: time, start: time + k.delay + AFTER_KILL, frames: [] });
    }
    return say;
  }

  /** The clip playing at `time` (and how far, 0..1), after collecting frames of the pending ones. */
  update(time: number): { clip: KillCamClip; u: number; started: boolean } | null {
    let started = false;
    for (const p of this.pending) {
      if (p.frames.length === 0 && time >= p.killTime + AFTER_KILL) p.frames = this.recorder.window(p.killTime + AFTER_KILL - KILLCAM_SECS - 0.05, p.killTime + AFTER_KILL + 0.05);
    }
    if (this.current && time >= this.current.start + KILLCAM_SECS) this.current = null;
    if (!this.current) {
      const next = this.pending.findIndex((p) => p.frames.length > 0 && time >= p.start);
      if (next >= 0) {
        this.current = this.pending.splice(next, 1)[0]!;
        started = true;
      }
    }
    // Clips that waited far too long (a stalled tab) are dropped.
    this.pending = this.pending.filter((p) => time < p.start + KILLCAM_SECS);
    if (!this.current) return null;
    return { clip: this.current, u: Math.min(1, Math.max(0, (time - this.current.start) / KILLCAM_SECS)), started };
  }
}

/** The synthetic blast at the victim's death (the window has no particles): fireball and core sizes over the replay time after the kill. */
export function blastAt(clock: number): { fire: number; core: number; opacity: number } | null {
  if (clock < 0 || clock > AFTER_KILL + 0.2) return null;
  const k = clock / (AFTER_KILL + 0.2);
  return { fire: 1.2 + 5.5 * Math.sqrt(k), core: 2.2 * (1 - k), opacity: 0.9 * (1 - k) };
}

/** Blast instances and light for a replay frame (`clock` = s after the kill). */
export function blastScene(at: Vec3, clock: number): { translucent: Instance[]; opaque: Instance[]; light: PointLight | null } {
  const b = blastAt(clock);
  if (!b) return { translucent: [], opaque: [], light: null };
  return {
    translucent: [
      { position: at, rotation: q.identity(), scale: b.fire, color: [1, 0.45, 0.12], emissive: 1, opacity: b.opacity * 0.55 },
      { position: at, rotation: q.identity(), scale: b.fire * 0.6, color: [1, 0.8, 0.4], emissive: 1, opacity: b.opacity },
    ],
    opaque: b.core > 0.05 ? [{ position: at, rotation: q.identity(), scale: b.core, color: [1, 0.95, 0.8], emissive: 1 }] : [],
    light: { position: at, color: [1, 0.6, 0.25], intensity: 120 * b.opacity, range: 30 },
  };
}

/** The window: a canvas with its own renderer (sharing the terrain), a frame, labels and a timeline. */
export class KillCamWindow {
  readonly root: HTMLDivElement;
  private readonly canvas: HTMLCanvasElement;
  private readonly title: HTMLSpanElement;
  private readonly bar: HTMLDivElement;
  private readonly stamp: HTMLDivElement;
  private renderer: Renderer | null = null;
  private creating: Promise<void> | null = null;
  private disposed = false;
  private atmosphere: Atmosphere | null | undefined = undefined;
  /** GPU time of the window's last frame (ms; null without timestamp queries). */
  gpuMs: number | null = null;

  constructor(parent: HTMLElement) {
    this.root = document.createElement("div");
    this.root.className = "killcam";
    this.root.innerHTML = `
      <div class="killcam-head"><span class="killcam-rec"></span><span class="killcam-label">Kill-cam</span><span class="killcam-title"></span></div>
      <div class="killcam-screen"><canvas></canvas><div class="killcam-scan"></div><div class="killcam-stamp"></div></div>
      <div class="killcam-time"><div class="killcam-bar"></div></div>`;
    this.canvas = this.root.querySelector("canvas")!;
    this.title = this.root.querySelector(".killcam-title")!;
    this.bar = this.root.querySelector(".killcam-bar")!;
    this.stamp = this.root.querySelector(".killcam-stamp")!;
    parent.appendChild(this.root);
  }

  /** Creates the window's renderer on `main`'s device (once; resolves when ready). */
  prepare(main: Renderer, create: (canvas: HTMLCanvasElement, device: GPUDevice) => Promise<Renderer>): Promise<void> {
    this.creating ??= create(this.canvas, main.device)
      .then((r) => {
        if (this.disposed) {
          r.destroy();
          return;
        }
        r.shareTerrain(main);
        this.renderer = r;
      })
      .catch((err: unknown) => console.warn("kill-cam:", err));
    return this.creating;
  }

  get ready(): boolean {
    return this.renderer !== null;
  }

  /** Shows the window with its title (killer and victim names). */
  show(on: boolean, title = ""): void {
    this.root.classList.toggle("shown", on);
    this.root.classList.remove("standby");
    if (title) this.title.textContent = title;
  }

  /** The announcement: the window comes up on static, the announcer dozing ("Zzz"). */
  standby(title: string): void {
    this.root.classList.add("shown", "standby");
    this.title.textContent = title;
    this.stamp.textContent = "Zzz";
    this.stamp.classList.add("on");
    this.bar.style.transform = "scaleX(0)";
  }

  /** Draws one replay frame. */
  render(view: { position: Vec3; orientation: Quat }, scene: Parameters<Renderer["setScene"]>[0], lights: readonly PointLight[], u: number, stamp: string, main: Renderer, time: number): void {
    this.bar.style.transform = `scaleX(${Math.min(1, Math.max(0, u)).toFixed(3)})`;
    this.stamp.textContent = stamp;
    this.stamp.classList.toggle("on", stamp !== "");
    const r = this.renderer;
    if (!r) return;
    // The world's sky and grade (set once per match: the bake is not free).
    if (this.atmosphere !== main.atmosphere) {
      this.atmosphere = main.atmosphere;
      r.setAtmosphere(main.atmosphere);
    }
    r.settings = { ...main.settings, post: { ...main.settings.post } };
    r.timeOverride = time;
    r.setScene(scene);
    r.setLights(lights);
    const cam: CameraView = { position: view.position, orientation: view.orientation, fovY: KILLCAM_FOV, near: 0.05, far: 600 };
    r.render(cam);
    this.gpuMs = r.gpuTimings()?.total ?? null;
  }

  dispose(): void {
    this.disposed = true;
    this.renderer?.destroy();
    this.renderer = null;
    this.root.remove();
  }
}
