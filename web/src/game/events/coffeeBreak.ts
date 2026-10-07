// Gravity coffee break (M4 C1, milestone-4-plan.md §3.2): every gravity well
// switches off for 20 s (fading over 1 s). B5 dims the wells and stops their
// swirl (game/worldFx.ts `coffeeDim`); debris drifts free in the sim. C1
// gives every well its break:
// - the tell: a steaming coffee mug drops onto each well's core stone with
//   a bounce and a clink while the event is announced;
// - during the break the mug steams, takes a sip now and then (a slurp from
//   the nearest one) and lounge music plays;
// - when the break is over the mug lifts off and shrinks away.
// Static wells only (dynamic wells are short-lived and have no core stone).

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { lin } from "../vfxLibrary.ts";
import { hash01, smooth, type EventClock } from "./fx.ts";
import type { EventEnv, EventInput, EventOutput, EventVisual, WellPoint } from "./types.ts";

/** Mug height per unit of core radius. */
const MUG_PER_CORE = 1.15;
/** Seconds of the drop (announce time -2.6 .. -1.8). */
const DROP_FROM = -2.6;
const DROP_SECS = 0.8;
/** Steam puffs per second per mug in view. */
const STEAM_RATE = 7;
/** Seconds between sips. */
const SIP_EVERY = 4.2;

export interface MugPose {
  position: Vec3;
  rotation: Quat;
  scale: number;
  /** 0..1 how much the mug is there (drop-in, lift-off). */
  present: number;
}

/**
 * Where a well's mug is `t` seconds into the event (negative while
 * announced) or `over` seconds after it: dropped onto the top of the core
 * along `up`, bobbing and sipping during the break, lifting off at the end.
 */
export function mugPose(w: WellPoint, up: Vec3, t: number, over: number | null, seed: number): MugPose | null {
  const size = Math.max(1.2, w.core * MUG_PER_CORE);
  const rest = v3.addScaled(w.center, up, w.core + size * 0.5 - 0.05);
  const side = v3.normalize(v3.cross(up, Math.abs(up.x) > 0.9 ? { x: 0, y: 0, z: 1 } : { x: 1, y: 0, z: 0 }));
  const base = q.lookRotation(v3.cross(side, up), up);
  const spin = q.fromAxisAngle(up, hash01(seed, 1) * 6.28 + (t > 0 ? t * 0.08 : 0));
  if (over !== null) {
    if (over > 1.2) return null;
    const k = smooth(0, 1.2, over);
    return { position: v3.addScaled(rest, up, k * k * 9), rotation: q.normalize(q.mul(q.fromAxisAngle(side, k * 0.8), q.mul(spin, base))), scale: size * (1 - k), present: 1 - k };
  }
  const d = (t - DROP_FROM) / DROP_SECS;
  if (d < 0) return null;
  // Falls like a dropped mug for 0.6 of the drop, then two small bounces.
  const fall = d < 0.6 ? 1 - (d / 0.6) * (d / 0.6) : 0.08 * Math.abs(Math.sin(((d - 0.6) / 0.4) * Math.PI * 2)) * Math.max(0, 1 - (d - 0.6) / 0.4);
  // A sip: the mug tips toward the core for a moment.
  const s = t > 1 ? ((t - 1) % SIP_EVERY) / SIP_EVERY : 1;
  const sip = t > 1 ? Math.sin(Math.min(1, s * 6) * Math.PI) * 0.35 : 0;
  const bob = t > 0 ? Math.sin(t * 1.7 + seed) * 0.06 * size : 0;
  return {
    position: v3.addScaled(rest, up, fall * 10 + bob),
    rotation: q.normalize(q.mul(q.fromAxisAngle(side, sip + fall * 0.6), q.mul(spin, base))),
    scale: size,
    present: Math.min(1, d * 3),
  };
}

class CoffeeBreak implements EventVisual {
  private readonly env: EventEnv;
  private clinked = false;
  private music: { stop(f?: number): void } | null = null;
  private musicDone = false;
  private steamAt = 0;
  private lastSip = -1;

  constructor(env: EventEnv) {
    this.env = env;
  }

  update(i: EventInput, clock: EventClock, out: EventOutput): boolean {
    const sound = i.quiet ? null : this.env.sound;
    const sink = this.env.particles;
    const wells = i.wells.filter((w) => !w.dynamic && w.core > 0);
    const cam = i.camera.position;
    let nearest: { d: number; pos: Vec3 } | null = null;
    let any = false;
    this.steamAt += i.dt * STEAM_RATE;
    const puffs = Math.floor(this.steamAt);
    this.steamAt -= puffs;
    wells.forEach((w, k) => {
      const m = mugPose(w, i.up, clock.t, clock.over, k + 1);
      if (!m) return;
      any = true;
      out.opaque.push(["coffee_mug", { position: m.position, rotation: m.rotation, scale: m.scale, color: [1, 1, 1], emissive: 0 }]);
      const d = v3.length(v3.sub(m.position, cam));
      if (!nearest || d < nearest.d) nearest = { d, pos: m.position };
      // A warm lamp so the mug reads with the well's glow off.
      out.lights.push({ position: v3.addScaled(m.position, i.up, m.scale * 0.9), color: [1, 0.78, 0.5], intensity: 2.2 * m.present, range: m.scale * 6 });
      // Steam from the coffee.
      if (sink && clock.t > -1.6 && clock.over === null && d < 90) {
        const top = v3.addScaled(m.position, q.up(m.rotation), m.scale * 0.42);
        for (let p = 0; p < puffs; p++) {
          const s = Math.floor(i.time * 97) + p * 13 + k * 101;
          const drift = { x: (hash01(s, 1) - 0.5) * 0.4, y: 0, z: (hash01(s, 2) - 0.5) * 0.4 };
          sink.spawn({
            pos: v3.add(top, v3.scale(drift, m.scale * 0.5)),
            vel: v3.add(v3.scale(i.up, m.scale * (0.55 + 0.3 * hash01(s, 3))), drift),
            life: 2.4,
            sprite: SPRITE.smoke0,
            blend: "alpha",
            color: lin([0.92, 0.9, 0.88], 0.55),
            alpha: 0.32,
            alpha1: 0,
            size: m.scale * 0.18,
            size1: m.scale * 0.75,
            spin: (hash01(s, 4) - 0.5) * 1.5,
            drag: 0.25,
            fadeIn: 0.15,
          });
        }
      }
    });
    const near = nearest as { d: number; pos: Vec3 } | null;
    if (!this.clinked && clock.t > DROP_FROM + DROP_SECS * 0.3 && clock.over === null && near) {
      this.clinked = true;
      sound?.play("coffee.clink", { position: near.pos });
    }
    // Once, for the active phase: the ending stops it (a start there would restart it every frame).
    if (clock.t >= 0 && clock.over === null && clock.phase === "active" && !this.music && !this.musicDone) {
      this.music = sound?.loop("coffee.lounge", { gain: 0.9 }) ?? null;
    }
    if ((clock.over !== null || clock.phase === "ending") && this.music) {
      this.music.stop(1.2);
      this.music = null;
    }
    if (clock.over !== null || clock.phase === "ending") this.musicDone = true;
    const sip = clock.t > 1 ? Math.floor((clock.t - 1) / SIP_EVERY) : -1;
    if (sip !== this.lastSip && sip >= 0 && clock.over === null && near && near.d < 70) sound?.play("coffee.slurp", { position: near.pos });
    this.lastSip = sip;
    return clock.over === null || any;
  }

  dispose(): void {
    this.music?.stop(0.2);
    this.music = null;
  }
}

export const coffeeBreak = (env: EventEnv): EventVisual => new CoffeeBreak(env);
