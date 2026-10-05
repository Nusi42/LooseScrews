// Camera and scale cues per frame (visual-quality-plan.md VQ1): the field of
// view from the view settings with the boost kick, the fit ring's probes
// (one ray and one ship-sized sphere sweep in ls-wasm) and the cockpit hull
// frame's state. main.ts and the visual gallery call it; the maths lives in
// camera.ts, ui/fitRing.ts and renderer/hullFrame.ts.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { Rgb } from "../renderer/renderer.ts";
import type { HullFrameState } from "../renderer/hullFrame.ts";
import { FIT_BLOCKED_MARGIN, FIT_RING_RANGE, fitRingModel, HIDDEN_RING, parseSweep, type FitRingModel } from "../ui/fitRing.ts";
import { loadViewSettings, type ViewSettings } from "../ui/viewSettings.ts";
import type { GameHandle } from "../wasm/api.ts";
import { FovKick, fovYFor } from "./camera.ts";
import { TINKER_RADIUS } from "./views.ts";

/** The probes the fit ring needs (a subset of the ls-wasm Game). */
export type ProbeGame = Pick<GameHandle, "raycast" | "sweepSphere" | "shipRadius">;

/** Sweep reach: a bit beyond the ring's range, so the fade-in starts smoothly. */
export const SWEEP_RANGE = FIT_RING_RANGE + 1;
/** The probes run every this many frames (30 Hz at 60 fps); the ring keeps its last state in between. */
export const PROBE_EVERY_FRAMES = 2;

/** Runs the fit ring's probes from `eye` along `orientation`'s forward. */
export function probeFitRing(
  game: ProbeGame,
  eye: Vec3,
  orientation: Quat,
  radius: number,
  fovY: number,
  viewportHeight: number,
): FitRingModel {
  if (!game.sweepSphere || !game.raycast) return HIDDEN_RING;
  const f = q.forward(orientation);
  const contact = parseSweep(game.sweepSphere(eye.x, eye.y, eye.z, f.x, f.y, f.z, radius, SWEEP_RANGE));
  if (!contact) return HIDDEN_RING;
  // The ray only has to tell whether the aim point lies beyond the contact
  // (blocked) or at it: it stops just past the blocked margin.
  const depth = (contact.point.x - eye.x) * f.x + (contact.point.y - eye.y) * f.y + (contact.point.z - eye.z) * f.z;
  const aim = game.raycast(eye.x, eye.y, eye.z, f.x, f.y, f.z, Math.max(0, depth) + FIT_BLOCKED_MARGIN + 0.05);
  return fitRingModel({ contact, aim: aim < 0 ? null : aim, eye, forward: f, radius, fovY, viewportHeight });
}

export interface ScaleCueFrame {
  /** Seconds since the last frame. */
  dt: number;
  /** Canvas aspect (width / height) and CSS height. */
  aspect: number;
  viewportHeight: number;
  /** The own ship flies (alive, not spectating, not riding a guided missile, no results screen). */
  flying: boolean;
  /** Boost held with boost left. */
  boosting: boolean;
}

export class ScaleCues {
  readonly settings: ViewSettings;
  private readonly kick = new FovKick();
  private frames = 0;
  private ring: FitRingModel = HIDDEN_RING;

  constructor(settings: ViewSettings = loadViewSettings()) {
    this.settings = settings;
  }

  /** Vertical FOV (rad) for this frame: the setting at the canvas aspect plus the eased boost kick. */
  fovY(f: ScaleCueFrame): number {
    return fovYFor(this.settings.fov, f.aspect, this.kick.update(f.dt, f.flying && f.boosting));
  }

  /** The fit ring for the own ship at `eye` (hidden when off or not flying). */
  fitRing(f: ScaleCueFrame, game: ProbeGame, eye: Vec3, orientation: Quat, fovY: number): FitRingModel {
    if (!this.settings.fitRing || !f.flying) {
      this.frames = 0;
      return (this.ring = HIDDEN_RING);
    }
    if (this.frames++ % PROBE_EVERY_FRAMES === 0) {
      this.ring = probeFitRing(game, eye, orientation, game.shipRadius ?? TINKER_RADIUS, fovY, f.viewportHeight);
    }
    return this.ring;
  }

  /** The cockpit frame's state (null hides it). */
  hullFrame(f: ScaleCueFrame, shipType: number, radius: number, color: Rgb): HullFrameState | null {
    if (!this.settings.hullFrame || !f.flying) return null;
    return { shipType, radius, color, glow: f.boosting ? 1 : 0 };
  }

  reset(): void {
    this.kick.reset();
    this.frames = 0;
    this.ring = HIDDEN_RING;
  }
}
