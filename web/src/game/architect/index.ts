// The Architect's client (M4 C4, milestone-4-plan.md §3.5 and wave C):
// one object per match that main.ts calls once per frame. For the round's
// architect it flies the god camera (camera.ts: free flight or the map view
// with the cut), aims and places strokes (number keys, the R wheel, the
// slider panel; mouse wheel sizes; left click sends an `ArchitectAction`;
// the server answers by telegraphing it, or silently refuses: the tracker
// then says why it probably did), and draws the placement preview and a
// pillar over every runner. For everyone it draws the exit gate and every
// telegraphed and acting stroke (visuals.ts), plays their sounds, and shows
// the round banners, the architect plate and (architect) the dock
// (ui/architect.ts). Runner alerts and the exit and stroke markers are in
// the mode HUD (ui/modeHud.ts).

import type { InputState } from "../../input/input.ts";
import * as q from "../../math/quat.ts";
import type { Mat4 } from "../../math/mat4.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { AudioEngine } from "../../audio/engine.ts";
import type { PointLight } from "../../renderer/lights.ts";
import type { ModeVolume } from "../../renderer/modeFx.ts";
import { decodeShareCode } from "../../renderer/atmosphere.ts";
import { ArchitectHud, architectHudModel, radialSector, type ArchitectBanner, type ArchitectToast } from "../../ui/architect.ts";
import { nameOf, projectMarker, type Marker } from "../../ui/modeHud.ts";
import type { SliderValues } from "../../ui/workshop.ts";
import { sideCss } from "../sides.ts";
import type { ModeState } from "../modeState.ts";
import type { VfxSink } from "../vfxLibrary.ts";
import type { ShipState } from "../views.ts";
import { ArchitectCamera, type ArchitectPose, type Cutaway } from "./camera.ts";
import {
  architectView,
  ArchitectTracker,
  DEFAULT_ARCHITECT_RULES,
  parseArchitectRules,
  strokeDef,
  strokeProblem,
  STROKES,
  type ArchitectEvent,
  type ArchitectRules,
  type ArchitectView,
} from "./model.ts";
import { architectLights, architectVolumes, ArchitectFx, type ArchitectSceneInput, type StrokePreview } from "./visuals.ts";

export { ArchitectCamera } from "./camera.ts";
export * from "./model.ts";

/** What the client needs from the wasm game (a subset of `GameHandle`). */
export interface ArchitectGame {
  architectActionPacket?(stroke: number, x: number, y: number, z: number, radius: number): Uint8Array | undefined;
  architectRules?(): string | undefined;
  raycast?(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, max: number): number;
}

export interface ArchitectFrameInput {
  dt: number;
  time: number;
  nowTick: number;
  /** The frame's sampled input (the architect's camera and strokes read it). */
  input: InputState;
  mode: ModeState;
  own: number;
  names: ReadonlyMap<number, string>;
  /** Level box [minX, minY, minZ, maxX, maxY, maxZ]. */
  bounds: ArrayLike<number>;
  /** Remote ships (runners). */
  ships: readonly ShipState[];
  lookScale: number;
  invertY: boolean;
  /** Where the god camera starts (the current camera). */
  start: ArchitectPose;
  /** The match's share code (the slider panel's resting values). */
  shareCode: string;
  /** Results or lobby showing: nothing to aim. */
  quiet: boolean;
}

export interface ArchitectFrame {
  /** The match is The Architect. */
  active: boolean;
  /** The local player is the round's architect: `pose` is the camera. */
  amArchitect: boolean;
  pose: ArchitectPose | null;
  /** The renderer's cut (the map view), null otherwise. */
  cutaway: Cutaway | null;
  volumes: ModeVolume[];
  lights: PointLight[];
  /** Fog multiplier (the map view sees far). */
  fogScale: number;
  /** Headlight multiplier: the map view lights the floors from up high like a desk lamp. */
  headlightScale: number;
}

const INACTIVE: ArchitectFrame = { active: false, amArchitect: false, pose: null, cutaway: null, volumes: [], lights: [], fogScale: 1, headlightScale: 1 };

/** Default stroke size (u). */
export const DEFAULT_RADIUS = 18;
/** Size step per wheel notch (u). */
export const RADIUS_STEP = 2;
/** Aim reach of the reticle (u). */
export const AIM_REACH = 260;
/** Without a hit the stroke goes this far ahead (u). */
export const AIM_FALLBACK = 45;
/** The map view: fog thinned to this share, the headlight this much stronger (it falls off as 1 / (1 + 0.004 d²): about 1 at the floor 120 u below). */
export const OVERVIEW_FOG = 0.25;
export const OVERVIEW_HEADLIGHT = 55;
/** Runner markers float this far above the ship (u). */
export const RUNNER_MARKER_LIFT = 1.6;
/** Banner and toast times (s). */
const BANNER_SECS = 4.5;
const TOAST_SECS = 2.6;

/**
 * Where a stroke aimed along `dir` from `origin` goes: on the rock the ray
 * hits (`hit` u away), pushed into it for Enclosure− (it opens rock) and
 * pulled back into the open for the well, the ink and the events; without
 * a hit `AIM_FALLBACK` ahead. Kept inside the level box.
 */
export function aimPoint(origin: Vec3, dir: Vec3, hit: number, stroke: number, radius: number, bounds: ArrayLike<number>): Vec3 {
  let d: number;
  if (hit >= 0) {
    const into = stroke === 1 ? radius * 0.35 : stroke === 0 || stroke === 2 || stroke === 4 ? 0 : -Math.min(radius * 0.5, hit * 0.5);
    d = Math.max(0, hit + into);
  } else {
    d = AIM_FALLBACK;
  }
  const b = (i: number): number => bounds[i] ?? 0;
  const c = (v: number, lo: number, hi: number): number => Math.max(lo, Math.min(hi, v));
  const p = { x: origin.x + dir.x * d, y: origin.y + dir.y * d, z: origin.z + dir.z * d };
  return bounds.length >= 6 ? { x: c(p.x, b(0), b(3)), y: c(p.y, b(1), b(4)), z: c(p.z, b(2), b(5)) } : p;
}

/** The share code's sliders as the panel's values (null when it does not decode). */
export function worldSliders(code: string): SliderValues | null {
  const w = decodeShareCode(code);
  return w ? { ...w.sliders } : null;
}

export class ArchitectClient {
  private readonly game: ArchitectGame;
  private readonly send: (packet: Uint8Array) => boolean;
  private readonly audio: AudioEngine | null;
  private readonly hud: ArchitectHud;
  private readonly fx: ArchitectFx | null;
  private readonly tracker = new ArchitectTracker();
  private rules: ArchitectRules | null = null;
  private camera: ArchitectCamera | null = null;
  private armed = 0;
  private radius = DEFAULT_RADIUS;
  private radialOpen = false;
  private radialX = 0;
  private radialY = 0;
  private prevPrimary = false;
  private prevSecondary = false;
  private banner: { b: ArchitectBanner; until: number } | null = null;
  private toast: { t: ArchitectToast; until: number } | null = null;
  private sliders: SliderValues | null = null;
  private sliderCode = "";
  private lastView: ArchitectView | null = null;
  /** The frame clock (s), for toasts. */
  private clock = 0;

  constructor(ui: HTMLElement, game: ArchitectGame, send: (packet: Uint8Array) => boolean, sink: VfxSink | null, audio: AudioEngine | null) {
    this.game = game;
    this.send = send;
    this.audio = audio;
    this.fx = sink ? new ArchitectFx(sink) : null;
    this.hud = new ArchitectHud(ui, { arm: (i) => this.arm(i) });
  }

  /** The tracker (rounds played this match: the results screen's round lines). */
  get rounds(): ArchitectTracker["history"] {
    return this.tracker.history;
  }

  /** The last frame's view (null outside The Architect). */
  get view(): ArchitectView | null {
    return this.lastView;
  }

  /** Arms stroke `index` (cards, keys, the wheel, the slider panel). */
  arm(index: number): void {
    if (!strokeDef(index)) return;
    if (index !== this.armed) this.audio?.play("architect.pick");
    this.armed = index;
  }

  update(f: ArchitectFrameInput): ArchitectFrame {
    const view = architectView(f.mode);
    this.lastView = view;
    this.clock = f.time;
    if (!view) {
      this.hud.show(false);
      return INACTIVE;
    }
    if (!this.rules) this.rules = parseArchitectRules(this.game.architectRules?.()) ?? { ...DEFAULT_ARCHITECT_RULES };
    const rules = this.rules;
    if (f.shareCode !== this.sliderCode) {
      this.sliderCode = f.shareCode;
      this.sliders = worldSliders(f.shareCode);
    }
    const amArchitect = view.architect === f.own && f.own >= 0 && f.mode.running;
    const events = this.tracker.update(view, f.own, f.nowTick, rules);
    this.onEvents(events, view, f);

    // The god camera (architect only).
    let pose: ArchitectPose | null = null;
    let cutaway: Cutaway | null = null;
    let preview: StrokePreview | null = null;
    let problem: string | null = null;
    let aimDistance: number | null = null;
    const input = f.input;
    if (amArchitect && !f.quiet) {
      if (!this.camera) {
        // A new architect starts in the map view over the runners' start, looking down the level.
        this.camera = new ArchitectCamera(f.start);
        const sp = rules.spawns;
        if (sp.length > 0 && f.bounds.length >= 6) {
          const n = sp.length;
          const mid = sp.reduce((a, p) => ({ x: a.x + p.x / n, y: a.y + p.y / n, z: a.z + p.z / n }), { x: 0, y: 0, z: 0 });
          this.camera.lookDownAt(mid, f.bounds);
        }
      }
      const cam = this.camera;
      if (input.cyclePaintColor) cam.toggle(f.bounds);
      // Stroke picks: number keys, the wheel (hold R), mouse wheel for the size.
      if (input.weaponKey >= 1 && input.weaponKey <= STROKES.length) this.arm(input.weaponKey - 1);
      if (input.cyclePrimary !== 0) this.radius = clampRadius(this.radius - input.cyclePrimary * RADIUS_STEP, rules);
      const radial = input.scavenge && input.pointerLocked;
      let look = input;
      if (radial) {
        if (!this.radialOpen) {
          this.radialX = 0;
          this.radialY = 0;
        }
        this.radialX += input.lookX;
        this.radialY += input.lookY;
        const len = Math.hypot(this.radialX, this.radialY);
        if (len > 140) {
          this.radialX *= 140 / len;
          this.radialY *= 140 / len;
        }
        look = { ...input, lookX: 0, lookY: 0 };
      } else if (this.radialOpen) {
        const pick = radialSector(this.radialX, this.radialY);
        if (pick >= 0) this.arm(pick);
      }
      this.radialOpen = radial;
      pose = cam.update(f.dt, look, f.lookScale, f.bounds, f.invertY);
      cutaway = cam.cutaway(f.bounds);
      this.radius = clampRadius(this.radius, rules);

      // Aim: from the cut plane in the map view (the clipped rock is not there to hit).
      const def = strokeDef(this.armed);
      if (def && !view.roundOver) {
        const dir = q.forward(pose.orientation);
        let origin = pose.position;
        if (cutaway && dir.y < -1e-3 && origin.y > cutaway.max.y) {
          const t = (cutaway.max.y - origin.y) / dir.y;
          origin = { x: origin.x + dir.x * t, y: cutaway.max.y, z: origin.z + dir.z * t };
        }
        const hit = this.game.raycast?.(origin.x, origin.y, origin.z, dir.x, dir.y, dir.z, AIM_REACH) ?? -1;
        const center = aimPoint(origin, dir, hit, def.index, this.radius, f.bounds);
        aimDistance = Math.hypot(center.x - pose.position.x, center.y - pose.position.y, center.z - pose.position.z);
        problem = strokeProblem({
          view,
          rules,
          own: f.own,
          nowTick: f.nowTick,
          stroke: def.index,
          center,
          radius: this.radius,
          bounds: f.bounds,
          cooldownUntil: this.tracker.cooldownUntil,
          verticalityUsed: this.tracker.verticalityUsed,
          mood: f.mode.mood,
        });
        preview = { def, center, radius: this.radius, ok: problem === null };
        const click = input.firePrimary && !this.prevPrimary && input.pointerLocked && !radial;
        if (click && !this.tracker.waiting) {
          if (problem) {
            this.say(problem, "bad");
            this.audio?.play("architect.denied");
          } else {
            const packet = this.game.architectActionPacket?.(def.index, center.x, center.y, center.z, this.radius);
            if (packet && this.send(packet)) {
              this.tracker.sent(def.index, f.nowTick, null);
              this.audio?.play("architect.place");
            } else {
              this.say("Not connected: strokes need a server", "bad");
            }
          }
        }
      }
      if (input.fireSecondary && !this.prevSecondary && input.pointerLocked) this.armed = -1;
    } else {
      this.camera = null;
      this.radialOpen = false;
    }
    this.prevPrimary = input.firePrimary;
    this.prevSecondary = input.fireSecondary;

    // World look for everyone.
    const runners = amArchitect
      ? f.ships.filter((s) => s.alive && view.runners.some((r) => r.id === s.id && !r.finished)).map((s) => ({ id: s.id, side: s.team, pos: s.position }))
      : [];
    const scene: ArchitectSceneInput = {
      view,
      rules,
      nowTick: f.nowTick,
      time: f.time,
      amArchitect,
      preview: amArchitect ? preview : null,
      runners,
      overviewTarget: cutaway && this.camera ? this.camera.target : null,
      camera: pose?.position ?? f.start.position,
    };
    this.fx?.update(scene, events, f.dt);
    const volumes = f.quiet ? [] : architectVolumes(scene);
    const lights = f.quiet ? [] : architectLights(scene);

    // HUD.
    const now = f.time;
    if (this.banner && now > this.banner.until && !view.roundOver) this.banner = null;
    if (this.toast && now > this.toast.until) this.toast = null;
    this.hud.update(
      architectHudModel({
        view,
        rules,
        own: f.own,
        nowTick: f.nowTick,
        names: f.names,
        armed: amArchitect ? this.armed : -1,
        radius: this.radius,
        problem,
        aimDistance,
        cooldownUntil: this.tracker.cooldownUntil,
        verticalityUsed: this.tracker.verticalityUsed,
        mood: f.mode.mood,
        overview: this.camera?.mode === "overview",
        pointerLocked: input.pointerLocked,
        radial: { open: this.radialOpen, selected: this.radialOpen ? radialSector(this.radialX, this.radialY) : -1 },
        banner: this.banner?.b ?? null,
        toast: this.toast?.t ?? null,
        waiting: this.tracker.waiting,
        sliders: this.sliders,
      }),
    );
    this.hud.show(!f.quiet);
    return { active: true, amArchitect, pose, cutaway, volumes, lights, fogScale: cutaway ? OVERVIEW_FOG : 1, headlightScale: cutaway ? OVERVIEW_HEADLIGHT : 1 };
  }

  /** Banners, toasts and sounds from the tracker's events. */
  private onEvents(events: readonly ArchitectEvent[], view: ArchitectView, f: ArchitectFrameInput): void {
    const a = this.audio;
    const kicker = `Round ${view.round + 1} of ${view.rounds}`;
    for (const e of events) {
      switch (e.kind) {
        case "roundStart": {
          const mine = e.architect === f.own && f.own >= 0;
          const who = nameOf(f.names, e.architect);
          this.banner = {
            b:
              e.architect < 0
                ? { kicker, title: "Find the exit", sub: "No architect yet: the level is all yours", tone: "runner" }
                : mine
                  ? { kicker, title: "You are the architect", sub: "Reshape the level. Nobody gets out.", tone: "architect" }
                  : { kicker, title: "Reach the exit", sub: `${who} is the architect. Watch the glowing spheres.`, tone: "runner" },
            until: f.time + BANNER_SECS,
          };
          break;
        }
        case "roundOver": {
          const r = e.result;
          const mine = r.architect === f.own;
          const archName = r.architect >= 0 ? nameOf(f.names, r.architect) : "The level";
          const escaped = `${r.escaped} of ${r.runners} ${r.runners === 1 ? "runner" : "runners"} escaped`;
          this.banner = {
            b: r.runnersWon
              ? { kicker: `Round ${r.round + 1} over`, title: "The runners win the round", sub: escaped, tone: mine ? "loss" : "win" }
              : { kicker: `Round ${r.round + 1} over`, title: `${mine ? "You" : archName} win${mine ? "" : "s"} the round`, sub: `${escaped}. The architect scores the rest.`, tone: mine ? "win" : "loss" },
            until: f.time + BANNER_SECS,
          };
          a?.play("architect.round");
          break;
        }
        case "telegraph":
          a?.play("architect.telegraph", { position: e.stroke.def.levelWide ? null : e.stroke.pos });
          break;
        case "act": {
          const d = e.stroke.def;
          const id = d.changesRock ? "architect.rock" : d.index === 3 ? "architect.well" : d.index === 6 ? "architect.ink" : "architect.event";
          a?.play(id, { position: d.levelWide ? null : e.stroke.pos });
          break;
        }
        case "refused":
          this.say(`Refused: ${e.reason}`, "bad");
          a?.play("architect.denied");
          break;
        case "escaped":
          if (e.runner === f.own) this.say("You made it out!", "good");
          else this.say(`${nameOf(f.names, e.runner)} escaped`, view.architect === f.own ? "bad" : "info");
          a?.play("architect.escape");
          break;
        default:
          break;
      }
    }
  }

  private say(text: string, tone: ArchitectToast["tone"]): void {
    this.toast = { t: { text, tone }, until: this.clock + TOAST_SECS };
  }

  /** Markers over the runners for the architect (seen through rock), projected like the objective markers. */
  runnerMarkers(view: ArchitectView | null, own: number, ships: readonly ShipState[], names: ReadonlyMap<number, string>, viewProj: Mat4, camera: Vec3, width: number, height: number): Marker[] {
    if (!view || view.architect !== own || own < 0) return [];
    const out: Marker[] = [];
    for (const s of ships) {
      const r = view.runners.find((x) => x.id === s.id);
      if (!s.alive || !r || r.finished) continue;
      // Above the ship, so the badge does not hide it.
      const p = projectMarker(viewProj, { x: s.position.x, y: s.position.y + RUNNER_MARKER_LIFT, z: s.position.z }, width, height);
      out.push({
        id: 1000 + s.id,
        kind: -1,
        label: nameOf(names, s.id),
        detail: `${Math.round(Math.hypot(s.position.x - camera.x, s.position.y - camera.y, s.position.z - camera.z))} u${r.lives >= 0 ? ` · ${r.lives} ${r.lives === 1 ? "life" : "lives"}` : ""}`,
        color: sideCss(s.team, 0),
        icon: "runner",
        ...p,
      });
    }
    return out;
  }

  /** A new match (transition): forget the rounds, the camera and the rules. */
  reset(): void {
    this.tracker.reset();
    this.rules = null;
    this.camera = null;
    this.banner = null;
    this.toast = null;
    this.armed = 0;
    this.radius = DEFAULT_RADIUS;
    this.lastView = null;
  }

  dispose(): void {
    this.hud.dispose();
  }
}

function clampRadius(r: number, rules: ArchitectRules): number {
  return Math.max(rules.minRadius, Math.min(rules.maxRadius, r));
}
