// Bored level (M4 C1, milestone-4-plan.md §3.2): the level's up turns 90
// degrees about a seeded horizontal axis over 5 s and stays turned. B5 turns
// the sky, sun, fog and shadows with the replicated up (renderer/turn.ts);
// horizon assist rolls every pilot through the turn (ls-sim). C1 adds:
// - the tell: sleepy Z's drifting over the screen while it is announced;
// - a long bored yawn as the turn starts and the level's stone creaking
//   through it;
// - grit sliding off everything toward the new down around the camera;
// - an arrow on a ring around the crosshair pointing at the new down while
//   the level turns (and a moment after).

import * as q from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { CameraView } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { lin } from "../vfxLibrary.ts";
import { hash01, smooth, type EventClock } from "./fx.ts";
import type { EventEnv, EventInput, EventOutput, EventVisual } from "./types.ts";

/** Screen angle (rad, clockwise from straight down the screen) of a world direction, and how far it leans into the screen plane (0..1). */
export function screenAngle(cam: CameraView, dir: Vec3): { angle: number; inPlane: number } {
  const x = v3.dot(dir, q.right(cam.orientation));
  const y = v3.dot(dir, q.up(cam.orientation));
  return { angle: Math.atan2(-x, -y), inPlane: Math.hypot(x, y) };
}

/** Sleepy letters while the event is announced (t -3..0): in, hold, out. */
export function zzzAmount(t: number): number {
  return smooth(-3, -2.4, t) * (1 - smooth(-0.4, 0.2, t));
}

class BoredLevel implements EventVisual {
  private readonly env: EventEnv;
  private yawned = false;
  private creak: { stop(f?: number): void } | null = null;
  private gritAt = 0;

  constructor(env: EventEnv) {
    this.env = env;
  }

  update(i: EventInput, clock: EventClock, out: EventOutput): boolean {
    const sound = i.quiet ? null : this.env.sound;
    const turn = i.fx.bored?.turnSecs ?? 5;
    if (clock.over === null && clock.phase === "announce") out.hud.zzz = zzzAmount(clock.t);
    if (clock.t >= 0 && !this.yawned && clock.over === null) {
      this.yawned = true;
      sound?.play("bored.yawn");
      this.creak = sound?.loop("bored.creak", { gain: 0.8 }) ?? null;
    }
    const t = clock.over === null ? clock.t : turn + 1 + clock.over;
    if (this.creak && t > turn) {
      this.creak.stop(0.8);
      this.creak = null;
    }
    // The new down on the HUD while it turns and a moment after.
    const show = smooth(-0.2, 0.3, t) * (1 - smooth(turn + 1.2, turn + 2.2, t));
    if (show > 0) {
      const s = screenAngle(i.camera, v3.scale(i.up, -1));
      out.hud.down = { angle: s.angle, alpha: show * smooth(0.12, 0.35, s.inPlane) };
    }
    // Grit sliding toward the new down around the camera while it turns.
    const sink = this.env.particles;
    if (sink && t > 0 && t < turn + 0.5) {
      this.gritAt += i.dt * 90;
      const down = v3.scale(i.up, -1);
      while (this.gritAt >= 1) {
        this.gritAt -= 1;
        const k = Math.floor(i.time * 1000) + Math.floor(this.gritAt * 100);
        const off = { x: (hash01(k, 1) - 0.5) * 16, y: (hash01(k, 2) - 0.5) * 16, z: (hash01(k, 3) - 0.5) * 16 };
        const pebble = hash01(k, 4) < 0.45;
        sink.spawn(
          pebble
            ? { pos: v3.add(i.camera.position, off), vel: v3.scale(down, 3 + 4 * hash01(k, 5)), life: 1.6, sprite: SPRITE.spark, blend: "alpha", orient: "velocity", stretch: 0.06, color: lin([0.55, 0.5, 0.43], 0.45), alpha: 0.85, alpha1: 0, size: 0.06 + 0.06 * hash01(k, 6) }
            : { pos: v3.add(i.camera.position, off), vel: v3.scale(down, 1 + 1.5 * hash01(k, 7)), life: 1.8, sprite: SPRITE.smoke3, blend: "alpha", color: lin([0.6, 0.56, 0.5], 0.4), alpha: 0.28, alpha1: 0, size: 0.3, size1: 1.2, drag: 0.2, fadeIn: 0.2 },
        );
      }
    }
    return clock.over === null || clock.over < 2.5;
  }

  dispose(): void {
    this.creak?.stop(0.2);
    this.creak = null;
  }
}

export const boredLevel = (env: EventEnv): EventVisual => new BoredLevel(env);
