// Rubber chickens (M4 C1, milestone-4-plan.md §3.2): for 12 s every new shot
// is a rubber chicken (ls-sim `proj_flags::CHICKEN`, replicated; ls-wasm
// adds 256 to the projectile state). Flight, damage and craters are
// unchanged, so only the look changes:
// - the shot is drawn as a rubber chicken scaled to it (energy shots a small
//   one, ordnance a big one, the Tectonic Charge a giant speckled egg),
//   beak first, rolling and flapping, with a soft glow in the weapon's
//   colour (so you still read the weapon) and a feather now and then;
// - a squeak when one is fired, a squawk and a burst of feathers where one
//   hits;
// - the announcement squeaks once (the tell).
// The normal shot visuals are hidden (`hideShots`); the effect library still
// sparks at the impact.

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import type { MeshKind } from "../../renderer/models/index.ts";
import { alongVelocity } from "../scene.ts";
import { lin } from "../vfxLibrary.ts";
import { PROJ_CHICKEN, PROJ_STUCK, type ProjectileState, type VisualDef } from "../views.ts";
import { hash01, type EventClock } from "./fx.ts";
import { ShotTracker } from "./shots.ts";
import type { EventEnv, EventInput, EventOutput, EventVisual, ParticleSink } from "./types.ts";

/** Chicken length (u) per unit of the shot's visual scale, by shape; the Charge is an egg. */
const SHAPE_SIZE: Readonly<Record<string, number>> = {
  bolt: 0.8,
  pellet: 0.6,
  spike: 0.85,
  arc: 0.8,
  paint: 0.7,
  blob: 0.75,
  orb: 0.95,
  rocket: 1.25,
  missile: 1.25,
  grenade: 1.1,
  harpoon: 1.15,
  mine: 1.05,
  paperweight: 1.0,
  charge: 1.4,
};

export interface ChickenLook {
  kind: MeshKind;
  scale: number;
}

/** The chicken (or egg) a shot of `visual` becomes. */
export function chickenLook(visual: VisualDef | undefined): ChickenLook {
  const shape = visual?.shape ?? "bolt";
  const s = Math.min(2.6, (SHAPE_SIZE[shape] ?? 0.7) * Math.max(0.6, visual?.scale ?? 1));
  return { kind: shape === "charge" ? "giant_egg" : "chicken", scale: s };
}

/** Whether a projectile is a rubber chicken. */
export function isChicken(p: ProjectileState): boolean {
  return (p.state & PROJ_CHICKEN) !== 0;
}

/** Orientation of a flying chicken: beak along the velocity, rolling and flapping. */
export function chickenPose(vel: Vec3, id: number, age: number, time: number): Quat {
  const along = alongVelocity(vel);
  const rollRate = (hash01(id, 1) < 0.5 ? -1 : 1) * (2.5 + 3 * hash01(id, 2));
  const roll = q.fromAxisAngle({ x: 0, y: 0, z: 1 }, hash01(id, 3) * 6.283 + age * rollRate);
  const flap = q.fromAxisAngle({ x: 0, y: 1, z: 0 }, Math.sin(time * 17 + id) * 0.18);
  return q.normalize(q.mul(along, q.mul(roll, flap)));
}

const FEATHER_WHITE = [0.5, 0.47, 0.42] as const;
const FEATHER_YELLOW = [0.55, 0.38, 0.05] as const;

/** A burst of feathers at `at` (an impact), `n` of them. */
export function featherBurst(sink: ParticleSink, at: Vec3, n: number, size: number, seed: number): void {
  for (let i = 0; i < n; i++) {
    const a = hash01(seed, i * 3 + 1) * Math.PI * 2;
    const b = Math.acos(2 * hash01(seed, i * 3 + 2) - 1);
    const d = { x: Math.sin(b) * Math.cos(a), y: Math.cos(b), z: Math.sin(b) * Math.sin(a) };
    const speed = 3 + 5 * hash01(seed, i * 3 + 3);
    sink.spawn({
      pos: v3.addScaled(at, d, 0.15),
      vel: v3.scale(d, speed),
      life: 2.2 + 1.2 * hash01(seed, i + 40),
      sprite: SPRITE.feather,
      blend: "alpha",
      color: i % 3 === 0 ? FEATHER_YELLOW : FEATHER_WHITE,
      alpha: 1,
      alpha1: 0,
      size: size * (0.28 + 0.2 * hash01(seed, i + 70)),
      drag: 1.8,
      spin: (hash01(seed, i + 90) - 0.5) * 9,
      rot: hash01(seed, i + 110) * 6.28,
      fadeIn: 0.02,
    });
  }
  sink.spawn({ pos: at, life: 0.5, sprite: SPRITE.smoke1, blend: "alpha", color: [0.32, 0.3, 0.27], alpha: 0.5, alpha1: 0, size: size * 0.4, size1: size * 1.6, drag: 1 });
}

class RubberChickens implements EventVisual {
  private readonly env: EventEnv;
  private readonly tracker = new ShotTracker();
  private readonly lastFeather = new Map<number, number>();
  private announced = false;

  constructor(env: EventEnv) {
    this.env = env;
  }

  update(i: EventInput, clock: EventClock, out: EventOutput): boolean {
    const sink = this.env.particles;
    const sound = i.quiet ? null : this.env.sound;
    if (clock.phase === "announce" && !this.announced && clock.t > -2.2) {
      this.announced = true;
      sound?.play("chicken.squeak", { gain: 0.8 });
    }
    const frame = this.tracker.update(i.shots, i.dt, i.time);
    let chickens = 0;
    i.shots.forEach((p, k) => {
      if (!isChicken(p)) return;
      chickens++;
      const id = frame.ids[k] ?? 0;
      const look = chickenLook(i.visuals[p.visual]);
      out.hideShots.add(p);
      const stuck = (p.state & PROJ_STUCK) !== 0;
      const born = frame.born.find((b) => b.id === id)?.born ?? i.time;
      const rot = stuck
        ? q.normalize(q.mul(q.fromAxisAngle(v3.normalize({ x: hash01(id, 5) - 0.5, y: 1, z: hash01(id, 6) - 0.5 }), hash01(id, 7) * 6.28), q.fromAxisAngle({ x: 1, y: 0, z: 0 }, 1.4)))
        : chickenPose(p.velocity, id, i.time - born, i.time);
      out.opaque.push([look.kind, { position: p.position, rotation: rot, scale: look.scale, color: [1, 1, 1], emissive: 0 }]);
      const color = i.visuals[p.visual]?.color ?? [1, 0.8, 0.3];
      // A soft glow of the weapon's colour behind it, and a feather now and then.
      sink?.immediate({ pos: p.position, life: 1, sprite: SPRITE.glow, color: lin(color, 0.6), size: look.scale * 0.9 });
      const tick = Math.floor(i.time / 0.14);
      if (!stuck && sink && this.lastFeather.get(id) !== tick && hash01(id, tick) < 0.55) {
        this.lastFeather.set(id, tick);
        const side = v3.normalize({ x: hash01(id, tick + 1) - 0.5, y: hash01(id, tick + 2) - 0.3, z: hash01(id, tick + 3) - 0.5 });
        sink.spawn({ pos: p.position, vel: v3.add(v3.scale(p.velocity, 0.08), v3.scale(side, 1.5)), life: 1.6, sprite: SPRITE.feather, blend: "alpha", color: FEATHER_WHITE, alpha: 0.9, alpha1: 0, size: look.scale * 0.32, drag: 1.2, spin: (hash01(id, tick + 4) - 0.5) * 8, rot: hash01(id, tick) * 6.28 });
      }
    });
    // Squeaks on fire (a few per frame at most), squawks and feathers where they hit.
    let squeaks = 0;
    for (const b of frame.born) {
      if (!isChicken({ kind: b.kind, owner: b.owner, position: b.position, velocity: b.velocity, visual: b.visual, state: b.state })) continue;
      if (squeaks++ < 3) sound?.play("chicken.squeak", { position: b.position });
    }
    let squawks = 0;
    for (const g of frame.gone) {
      this.lastFeather.delete(g.id);
      if ((g.state & PROJ_CHICKEN) === 0) continue;
      const look = chickenLook(i.visuals[g.visual]);
      if (sink) featherBurst(sink, g.position, look.kind === "giant_egg" ? 34 : 14, look.scale, g.id);
      if (squawks++ < 2) sound?.play(look.kind === "giant_egg" ? "chicken.egg" : "chicken.squawk", { position: g.position });
    }
    if (this.lastFeather.size > 512) this.lastFeather.clear();
    return clock.over === null || chickens > 0;
  }
}

export const rubberChickens = (env: EventEnv): EventVisual => new RubberChickens(env);
