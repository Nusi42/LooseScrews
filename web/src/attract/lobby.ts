// Lobby flythrough (first-impression pass, FI): while the lobby card shows
// and the mouse is free (the player reads the card, picks a team or waits),
// the camera leaves the ship and flies a cinematic path through the next
// level (cinematic.ts); the other pilots warming up are the action. Locking
// the mouse (click to fly) returns to the cockpit. Starts from the current
// view, so the move out of the cockpit is smooth.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { fovYFor } from "../game/camera.ts";
import type { LocalPose, ShipState } from "../game/views.ts";
import type { CameraView } from "../renderer/renderer.ts";
import type { GameHandle } from "../wasm/api.ts";
import { CINEMATIC_FOV_DEG, frameYawFor } from "./backdrop.ts";
import { CinematicCamera, DEFAULT_CINEMATIC } from "./cinematic.ts";
import { FlightPath, wasmProbe } from "./flightPath.ts";

/** The own ship shows once the camera is this far from it (u), so the camera never starts inside the hull. */
export const OWN_SHIP_MIN_DISTANCE = 2.5;

export class LobbyCinematic {
  private camera: CinematicCamera | null = null;
  private pose: { position: Vec3; orientation: Quat } | null = null;

  /**
   * The cinematic pose this frame, or null when it should not run (`want`
   * false, or the game has no level probes). `from` is the view to start at.
   */
  update(dt: number, game: Pick<GameHandle, "raycast" | "sweepSphere">, want: boolean, from: CameraView, aspect = 16 / 9): { position: Vec3; orientation: Quat } | null {
    if (!want) {
      this.camera = null;
      this.pose = null;
      return null;
    }
    if (!this.camera) {
      const probe = wasmProbe(game);
      if (!probe) return null;
      const path = new FlightPath(probe, from.position, q.forward(from.orientation), Math.random);
      this.camera = new CinematicCamera(path, { ...DEFAULT_CINEMATIC, easeIn: 3.5, frameYaw: frameYawFor(aspect) }, from.orientation);
    }
    this.pose = this.camera.update(dt);
    return this.pose;
  }

  /** True while the cinematic camera runs. */
  get active(): boolean {
    return this.camera !== null;
  }

  /** Vertical FOV of the cinematic camera at `aspect`. */
  fovY(aspect: number): number {
    return fovYFor(CINEMATIC_FOV_DEG, aspect);
  }

  /**
   * The own ship as the cinematic camera sees it (first person never draws
   * it); not alive (hidden) while the camera is still close to it.
   */
  ownShip(id: number, team: number, pose: LocalPose, shipType: number, radius: number): ShipState {
    const far = this.pose ? v3.length(v3.sub(this.pose.position, pose.position)) > OWN_SHIP_MIN_DISTANCE : false;
    return { id, team, alive: pose.alive && far, boosting: false, position: pose.position, orientation: pose.orientation, shipType, radius, gadgetActive: false };
  }

  /** A new match: start over from the next view. */
  reset(): void {
    this.camera = null;
    this.pose = null;
  }
}
