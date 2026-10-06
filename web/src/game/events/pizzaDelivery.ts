// Pizza delivery (M4 C2; sim: crates/ls-sim/src/events/pizza_delivery.rs):
// a cheerful red quadcopter with one big eye and a propeller beanie carries
// a steaming pizza box in its claws (toppings drip off it now and then). It
// waits at its entry cavern during the announcement, flies to the recipient
// and rings the doorbell. While the recipient holds still the pizza
// assembles slice by slice under its ship (the progress ring everyone
// sees; the recipient also gets a HUD ring); delivered, it pops in a burst
// of cheese and green repair sparkles. Shot down, the pizza splats.
//
// Entity (`ReplicatedKind::Drone`, role 1): state `[role, hold progress
// 0-255, phase, hp percent]`, owner = the recipient. Outcome cues: 1
// delivered (target = the recipient), 0xF0 shot down (source = the shooter).

import * as q from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { Rgb } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { lin } from "../vfxLibrary.ts";
import type { EntityState } from "../views.ts";
import {
  attach,
  burst,
  damageSmoke,
  DRONE_PHASE,
  hoverPose,
  integrity,
  NONE,
  OUTCOME_SHOT_DOWN,
  put,
  rng,
  ROLE,
  rotorSpin,
  shipOf,
  type DrawCtx,
  type VisitorInput,
  type VisitorOut,
} from "./visitorKit.ts";

export const PIZZA_EVENT = "pizza_delivery";

export const PIZZA_RED: Rgb = [0.86, 0.13, 0.1];
export const CARDBOARD: Rgb = [0.74, 0.57, 0.38];
export const LOGO: Rgb = [0.95, 0.32, 0.08];
export const CHEESE: Rgb = [1.0, 0.8, 0.32];
export const CRUST: Rgb = [0.78, 0.5, 0.24];
/** Outcome byte of a delivered pizza. */
export const OUTCOME_DELIVERED = 1;
/** Slices of the progress pizza (data/events/pizza_delivery.ron: 3 s hold). */
export const SLICES = 12;
/** The doorbell rings when the drone first comes this close to the recipient (u). */
export const DOORBELL_RANGE = 5;

/** Slices shown for a hold progress byte (0-255): none at 0, all 12 once full. */
export function slicesFor(progress: number): number {
  return Math.max(0, Math.min(SLICES, Math.ceil((progress / 255) * SLICES - 1e-6)));
}

/** The pizza's looks; remembers which drones rang the bell. */
export class PizzaFx {
  private rang = new Set<number>();
  private steam = 0;

  reset(): void {
    this.rang.clear();
  }

  update(i: VisitorInput, ctx: DrawCtx, out: VisitorOut): void {
    for (const o of i.outcomes) {
      if (o.event !== PIZZA_EVENT) continue;
      if (o.byte === OUTCOME_DELIVERED) {
        const s = shipOf(i, o.target);
        delivered(ctx, s?.position ?? o.position, Math.floor(i.time * 131));
        ctx.lights.push({ position: s?.position ?? o.position, color: [0.5, 1, 0.55], intensity: 6, range: 9 });
        out.sounds.push({ id: "visitor.delivered", position: s?.position ?? o.position });
        out.toasts.push({ event: PIZZA_EVENT, text: o.target === i.own ? "Pizza delivered. Full hull, enjoy!" : `${i.name(o.target)} got their pizza. Full hull.`, own: o.target === i.own });
      } else if (o.byte === OUTCOME_SHOT_DOWN) {
        splat(ctx, o.position, Math.floor(i.time * 173));
        out.sounds.push({ id: "visitor.splat", position: o.position });
        const by = o.source !== NONE ? i.name(o.source) : "Someone";
        out.toasts.push({ event: PIZZA_EVENT, text: `${by} shot down the pizza. 50 scrap on the floor!`, own: o.source === i.own });
      }
    }
    const seen = new Set<number>();
    for (const d of i.entities) {
      if (d.kind !== "drone" || d.state[0] !== ROLE.pizza) continue;
      seen.add(d.id);
      this.draw(i, ctx, d, out);
    }
    for (const id of [...this.rang]) if (!seen.has(id)) this.rang.delete(id);
  }

  /** Hold progress 0..1 of the own pizza (the HUD ring), or null. */
  ownHold(i: VisitorInput): number | null {
    const d = i.entities.find((e) => e.kind === "drone" && e.state[0] === ROLE.pizza && e.owner === i.own && e.state[2] === DRONE_PHASE.working);
    return d ? d.state[1] / 255 : null;
  }

  private draw(i: VisitorInput, ctx: DrawCtx, d: EntityState, out: VisitorOut): void {
    const pose = hoverPose(d, i.time, ctx.up);
    put(ctx, "pizza_drone", pose, PIZZA_RED);
    for (let k = 0; k < 4; k++) {
      const a = Math.PI / 4 + (k * Math.PI) / 2;
      put(ctx, "visitor_rotor", attach(pose, { x: Math.cos(a) * 0.72, y: 0.15, z: Math.sin(a) * 0.72 }, rotorSpin(i.time, k), 0.38), [0.1, 0.1, 0.11]);
    }
    // The beanie's propeller turns lazily.
    put(ctx, "visitor_rotor", attach(pose, { x: 0, y: 0.5, z: 0 }, q.fromAxisAngle({ x: 0, y: 1, z: 0 }, i.time * 9), 0.3), [0.95, 0.8, 0.15]);
    // The box swings a little under the claws; logos on top and bottom.
    const sway = q.fromAxisAngle({ x: 1, y: 0, z: 0 }, Math.sin(i.time * 2.3 + d.id) * 0.06);
    const box = attach(pose, { x: 0, y: -0.43, z: 0 }, sway);
    put(ctx, "pizza_box", box, CARDBOARD);
    put(ctx, "pizza_logo", attach(box, { x: 0, y: 0.056, z: 0 }), LOGO, { emissive: 0.25 });
    put(ctx, "pizza_logo", attach(box, { x: 0, y: -0.056, z: 0 }, q.fromAxisAngle({ x: 1, y: 0, z: 0 }, Math.PI)), LOGO, { emissive: 0.25 });
    // A warm glow under it (fresh from the oven).
    ctx.lights.push({ position: v3.addScaled(box.position, ctx.up, -0.6), color: [1, 0.66, 0.35], intensity: 2.6, range: 6 });
    this.steamAndToppings(i, ctx, box.position);
    damageSmoke(ctx.sink, pose.position, integrity(d), i.dt, rng(Math.floor(i.time * 60) + d.id));
    if (d.state[2] !== DRONE_PHASE.leaving) out.sounds.push({ id: "visitor.rotor", position: pose.position, loop: `pizza-rotor-${d.id}` });
    // The recipient: the doorbell when it arrives, the pizza assembling under its ship.
    const target = d.owner !== NONE ? shipOf(i, d.owner) : undefined;
    if (!target || d.state[2] !== DRONE_PHASE.working) return;
    if (!this.rang.has(d.id) && v3.length(v3.sub(target.position, d.position)) < DOORBELL_RANGE) {
      this.rang.add(d.id);
      out.sounds.push({ id: "visitor.doorbell", position: pose.position });
      out.toasts.push({ event: PIZZA_EVENT, text: d.owner === i.own ? "Pizza! Hold still for 3 seconds." : `Pizza for ${i.name(d.owner)}!`, own: d.owner === i.own });
    }
    const n = slicesFor(d.state[1]);
    if (n <= 0) return;
    const spin = q.fromAxisAngle(ctx.up, i.time * 0.6);
    const base = q.mul(spin, q.lookRotation(perp(ctx.up), ctx.up));
    const centre = v3.addScaled(target.position, ctx.up, -0.62);
    for (let k = 0; k < n; k++) {
      const rot = q.mul(q.fromAxisAngle(ctx.up, (k * Math.PI * 2) / SLICES), base);
      const fresh = k === n - 1 ? 0.85 + 0.15 * Math.sin(i.time * 12) : 1;
      const slice = { position: centre, rotation: q.normalize(rot), scale: 1.25 * fresh };
      put(ctx, "pizza_slice", slice, CHEESE, { emissive: 0.12 });
      put(ctx, "pizza_crust", slice, CRUST, { emissive: 0.05 });
    }
    ctx.lights.push({ position: v3.addScaled(centre, ctx.up, 0.4), color: [1, 0.78, 0.4], intensity: 1.2 + 0.25 * n, range: 5 });
  }

  private steamAndToppings(i: VisitorInput, ctx: DrawCtx, box: Vec3): void {
    const sink = ctx.sink;
    if (!sink || i.dt <= 0) return;
    this.steam += i.dt * 7 * sink.density;
    const r = rng(Math.floor(i.time * 997));
    while (this.steam >= 1) {
      this.steam -= 1;
      sink.spawn({
        pos: v3.add(box, { x: (r() - 0.5) * 0.7, y: 0.08, z: (r() - 0.5) * 0.7 }),
        vel: v3.add(v3.scale(ctx.up, 0.5 + r() * 0.4), { x: (r() - 0.5) * 0.2, y: 0, z: (r() - 0.5) * 0.2 }),
        life: 1.4 + r() * 0.6,
        sprite: SPRITE.smoke2,
        color: [0.95, 0.93, 0.9],
        alpha: 0.28,
        alpha1: 0,
        size: 0.15,
        size1: 0.6,
        drag: 0.4,
        blend: "alpha",
        spin: (r() - 0.5) * 2,
        fadeIn: 0.2,
      });
      if (r() < 0.12) {
        // A topping falls off.
        const colours: Rgb[] = [[0.8, 0.1, 0.08], [1, 0.82, 0.25], [0.2, 0.55, 0.15]];
        sink.spawn({
          pos: v3.add(box, { x: (r() - 0.5) * 0.8, y: -0.1, z: (r() - 0.5) * 0.8 }),
          vel: v3.scale(ctx.up, -1.5 - r()),
          life: 1.6,
          sprite: SPRITE.shard,
          color: colours[Math.floor(r() * 3)] ?? CHEESE,
          alpha: 1,
          alpha1: 1,
          size: 0.08,
          blend: "alpha",
          spin: (r() - 0.5) * 8,
        });
      }
    }
  }
}

/** A unit vector perpendicular to `u`. */
function perp(u: Vec3): Vec3 {
  const a = Math.abs(u.y) < 0.9 ? { x: 0, y: 1, z: 0 } : { x: 1, y: 0, z: 0 };
  return v3.normalize(v3.cross(u, a));
}

function delivered(ctx: DrawCtx, at: Vec3, seed: number): void {
  const r = rng(seed);
  // Hearts (pink flares) and a big green ring: full hull.
  burst(ctx.sink, 14, (_, rnd) => ({
    pos: v3.add(at, { x: (rnd() - 0.5) * 1.2, y: (rnd() - 0.5) * 0.8, z: (rnd() - 0.5) * 1.2 }),
    vel: { x: (rnd() - 0.5) * 1.5, y: 1.8 + rnd() * 1.6, z: (rnd() - 0.5) * 1.5 },
    life: 1.4 + rnd() * 0.6,
    sprite: SPRITE.glow,
    color: lin([1, 0.35, 0.55], 6),
    size: 0.32,
    size1: 0.1,
    drag: 0.8,
  }), r);
  ctx.sink?.spawn({ pos: at, life: 0.7, sprite: SPRITE.ring, color: lin([0.45, 1, 0.5], 3), size: 0.8, size1: 3.2 });
  ctx.sink?.spawn({ pos: at, life: 0.5, sprite: SPRITE.glow, color: lin([0.5, 1, 0.55], 2.5), size: 1.6, size1: 0.6 });
  burst(ctx.sink, 30, (k, rnd) => ({
    pos: at,
    vel: { x: (rnd() - 0.5) * 7, y: (rnd() - 0.5) * 7, z: (rnd() - 0.5) * 7 },
    life: 0.7 + rnd() * 0.5,
    sprite: SPRITE.shard,
    color: k % 4 === 0 ? [0.8, 0.12, 0.08] : [1, 0.82, 0.3],
    alpha: 1,
    alpha1: 0,
    size: 0.1,
    drag: 2.5,
    blend: "alpha",
    spin: (rnd() - 0.5) * 10,
  }), r);
  burst(ctx.sink, 40, (_, rnd) => ({
    pos: v3.add(at, { x: (rnd() - 0.5) * 1.6, y: (rnd() - 0.5) * 1.0, z: (rnd() - 0.5) * 1.6 }),
    vel: { x: 0, y: 1.5 + rnd() * 2, z: 0 },
    life: 0.9 + rnd() * 0.6,
    sprite: SPRITE.flare,
    color: lin([0.4, 1, 0.5], 6),
    size: 0.18,
    size1: 0.02,
    drag: 0.5,
    spin: 3,
  }), r);
  ctx.sink?.spawn({ pos: at, life: 0.5, sprite: SPRITE.ring, color: lin([0.5, 1, 0.55], 2.5), size: 0.6, size1: 2.6 });
}

function splat(ctx: DrawCtx, at: Vec3, seed: number): void {
  const r = rng(seed);
  const colours: Rgb[] = [[0.82, 0.12, 0.08], [1, 0.8, 0.3], [0.74, 0.57, 0.38], [0.95, 0.9, 0.7]];
  burst(ctx.sink, 46, (k, rnd) => ({
    pos: at,
    vel: { x: (rnd() - 0.5) * 11, y: rnd() * 7 - 2, z: (rnd() - 0.5) * 11 },
    life: 1.0 + rnd() * 0.8,
    sprite: SPRITE.shard,
    color: colours[k % colours.length] ?? CHEESE,
    alpha: 1,
    alpha1: 0,
    size: 0.11 + rnd() * 0.1,
    drag: 1.8,
    blend: "alpha",
    spin: (rnd() - 0.5) * 12,
  }), r);
  burst(ctx.sink, 5, (_, rnd) => ({
    pos: at,
    vel: { x: (rnd() - 0.5) * 2, y: rnd() * 1.5, z: (rnd() - 0.5) * 2 },
    life: 1.3,
    sprite: SPRITE.smoke0,
    color: [0.3, 0.22, 0.15],
    alpha: 0.6,
    alpha1: 0,
    size: 0.5,
    size1: 2,
    drag: 1,
    blend: "alpha",
  }), r);
  ctx.sink?.spawn({ pos: at, life: 0.25, sprite: SPRITE.flare, color: lin([1, 0.7, 0.4], 10), size: 2, size1: 0.5 });
}
