// Level hiccups (M4 C1, milestone-4-plan.md §3.2): for 15 s, every 3 s (plus
// or minus 1, seeded) the level hiccups and lifts everything about 2 u along
// its up. The hiccup ticks come from the core (`Game.eventFx` -> ls-sim
// `level_hiccups::hiccup_ticks`), so the look lands on the server's tick:
// - a camera bump (a sharp dip and a nod that settles; the sim's lift moves
//   the ship itself);
// - a "hic" (pitched per hiccup);
// - dust and pebbles shaken off the ceilings above the camera;
// - hiccup bubbles rising from every ship nearby;
// - a comic "HIC!" popping on the screen.
// The announcement has one small hic of its own (the tell, a small "hic?").

import * as q from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { lin } from "../vfxLibrary.ts";
import { elasticOut, hash01, smooth, TICK_HZ, type EventClock } from "./fx.ts";
import type { EventEnv, EventInput, EventOutput, EventVisual, ParticleSink } from "./types.ts";

/** How long the camera bump lasts (s). */
export const BUMP_SECS = 0.6;

/** Camera bump `t` seconds after a hiccup: offset along up (u) and a nod (rad). */
export function bump(t: number): { lift: number; nod: number } {
  if (t < 0 || t > BUMP_SECS) return { lift: 0, nod: 0 };
  const e = Math.exp(-t * 8);
  return { lift: -0.32 * e * Math.sin(t * 34), nod: 0.045 * e * Math.sin(t * 34 + 0.6) };
}

/** Hiccup ticks crossed between two ticks (`from` exclusive, `to` inclusive). */
export function crossed(ticks: readonly number[], from: number, to: number): number[] {
  return ticks.filter((h) => h > from && h <= to);
}

/** Dust and pebbles shaken off a ceiling point (falling along -up). */
export function shakeDust(sink: ParticleSink, at: Vec3, up: Vec3, seed: number, amount = 1): void {
  const side = v3.normalize(v3.cross(up, Math.abs(up.x) > 0.9 ? { x: 0, y: 0, z: 1 } : { x: 1, y: 0, z: 0 }));
  const side2 = v3.cross(up, side);
  const n = Math.round(14 * amount);
  for (let i = 0; i < n; i++) {
    const a = hash01(seed, i) * Math.PI * 2;
    const r = 2.5 * Math.sqrt(hash01(seed, i + 30));
    const from = v3.add(at, v3.add(v3.scale(side, Math.cos(a) * r), v3.add(v3.scale(side2, Math.sin(a) * r), v3.scale(up, -0.25))));
    const pebble = i % 3 === 0;
    sink.spawn(
      pebble
        ? { pos: from, vel: v3.scale(up, -(3 + 4 * hash01(seed, i + 60))), life: 1.3, sprite: SPRITE.shard, blend: "alpha", color: [0.07, 0.06, 0.05], alpha: 1, alpha1: 0.8, size: 0.08 + 0.1 * hash01(seed, i + 90), spin: 7 }
        : { pos: from, vel: v3.scale(up, -(2 + 3 * hash01(seed, i + 120))), life: 2.4, sprite: SPRITE.smoke2, blend: "alpha", color: lin([0.66, 0.6, 0.52], 0.55), alpha: 0.62, alpha1: 0, size: 0.6, size1: 2.4, drag: 0.5, fadeIn: 0.03 },
    );
  }
}

/** Seconds a "HIC!" stays on the screen. */
export const POP_SECS = 0.95;

/** The "HIC!" of a hiccup `age` seconds ago: size (pops with an overshoot) and opacity. */
export function popLook(age: number): { scale: number; alpha: number } {
  if (age < 0 || age > POP_SECS) return { scale: 0, alpha: 0 };
  return { scale: 0.35 + 0.65 * elasticOut(age / 0.4), alpha: 1 - smooth(POP_SECS * 0.6, POP_SECS, age) };
}

/** Hiccup bubbles rising from a ship. */
export function bubbles(sink: ParticleSink, at: Vec3, up: Vec3, seed: number): void {
  for (let i = 0; i < 5; i++) {
    const off = { x: hash01(seed, i) - 0.5, y: hash01(seed, i + 7) - 0.5, z: hash01(seed, i + 13) - 0.5 };
    sink.spawn({
      pos: v3.add(at, v3.add(v3.scale(off, 0.9), v3.scale(up, 0.5))),
      vel: v3.add(v3.scale(up, 1.6 + 1.2 * hash01(seed, i + 3)), v3.scale(off, 0.6)),
      life: 0.9 + 0.5 * hash01(seed, i + 21),
      sprite: SPRITE.ring,
      color: lin([0.75, 0.92, 1.0], 1.3),
      size: 0.22 + 0.2 * hash01(seed, i + 5),
      size1: 0.45 + 0.3 * hash01(seed, i + 9),
      drag: 0.6,
    });
  }
}

class LevelHiccups implements EventVisual {
  private readonly env: EventEnv;
  private lastTick = -1;
  private bumpAt = -9;
  private tellDone = false;
  private count = 0;
  private readonly pops: Array<{ at: number; text: string; x: number; y: number; tilt: number; small: boolean }> = [];

  constructor(env: EventEnv) {
    this.env = env;
  }

  update(i: EventInput, clock: EventClock, out: EventOutput): boolean {
    const sink = this.env.particles;
    const sound = i.quiet ? null : this.env.sound;
    const up = i.fx.hiccups?.up ?? i.up;
    // The tell: one small hic during the announcement.
    if (clock.phase === "announce" && !this.tellDone && clock.t > -1.1) {
      this.tellDone = true;
      sound?.play("hiccup.hic", { rate: 1.25, gain: 0.7 });
      this.pops.push({ at: i.time, text: "hic?", x: 0.62, y: 0.36, tilt: 8, small: true });
      if (sink && i.own) bubbles(sink, i.own.position, up, 7);
    }
    const ticks = i.fx.hiccups?.ticks ?? [];
    if (this.lastTick >= 0) {
      for (const h of crossed(ticks, this.lastTick, i.tick)) {
        this.count++;
        // The bump starts on the hiccup's own time even if the frame came late.
        this.bumpAt = i.time - (i.tick - h) / TICK_HZ;
        sound?.play("hiccup.hic", { rate: 0.9 + 0.25 * hash01(h, 3) });
        this.pops.push({ at: this.bumpAt, text: "HIC!", x: 0.32 + 0.36 * hash01(h, 5), y: 0.28 + 0.16 * hash01(h, 6), tilt: (hash01(h, 7) - 0.5) * 24, small: false });
        if (sink) this.shake(i, up, h, sink);
      }
    }
    this.lastTick = i.tick;
    for (const p of this.pops) {
      const look = popLook(i.time - p.at);
      if (look.alpha <= 0) continue;
      out.hud.pops.push({ text: p.text, x: p.x, y: p.y, scale: look.scale * (p.small ? 0.55 : 1), tilt: p.tilt, alpha: look.alpha, color: "#8fd3ff" });
    }
    while (this.pops.length > 0 && i.time - (this.pops[0]?.at ?? 0) > POP_SECS) this.pops.shift();
    const b = bump(i.time - this.bumpAt);
    if (b.lift !== 0 || b.nod !== 0) {
      const cam = i.camera;
      const nod = q.fromAxisAngle(q.right(cam.orientation), b.nod);
      out.view = { ...cam, position: v3.addScaled(cam.position, up, b.lift), orientation: q.normalize(q.mul(nod, cam.orientation)) };
      out.shake = Math.max(out.shake, 0.25 * Math.exp(-(i.time - this.bumpAt) * 10));
    }
    return clock.over === null || i.time - this.bumpAt < BUMP_SECS;
  }

  private shake(i: EventInput, up: Vec3, seed: number, sink: ParticleSink): void {
    // Ceilings above the camera: straight up and four tilted rays.
    const cam = i.camera.position;
    const side = v3.normalize(v3.cross(up, Math.abs(up.x) > 0.9 ? { x: 0, y: 0, z: 1 } : { x: 1, y: 0, z: 0 }));
    const side2 = v3.cross(up, side);
    // Straight up, four tilted rays, and three up the view so the dust falls in sight.
    const fwd = q.forward(i.camera.orientation);
    const dirs = [
      up,
      ...[0, 1, 2, 3].map((k) => v3.normalize(v3.add(up, v3.scale(v3.add(v3.scale(side, Math.cos(k * 1.57 + seed)), v3.scale(side2, Math.sin(k * 1.57 + seed))), 0.8)))),
      ...[1.2, 2.5, 4].map((k) => v3.normalize(v3.add(up, v3.scale(fwd, k)))),
    ];
    for (const [k, d] of dirs.entries()) {
      const t = i.raycast?.(cam, d, 28) ?? -1;
      if (t > 0) shakeDust(sink, v3.addScaled(cam, d, t), up, seed * 7 + k, k === 0 ? 1.2 : k >= 5 ? 1.4 : 0.7);
    }
    for (const s of i.ships) if (s.alive && v3.length(v3.sub(s.position, cam)) < 60) bubbles(sink, s.position, up, seed + s.id);
    if (i.own) bubbles(sink, v3.addScaled(i.own.position, q.forward(i.own.orientation), 1.2), up, seed + 99);
  }
}

export const levelHiccups = (env: EventEnv): EventVisual => new LevelHiccups(env);
