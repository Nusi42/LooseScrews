// Tax drone (M4 C2; sim: crates/ls-sim/src/events/tax_drone.rs): a boxy
// bureaucrat drone in government beige with a dark visor, two narrow
// glowing eyes under a frowning brow, a red necktie that swings with its
// flight, a red-and-blue siren and a receipt printer. It waits at its entry
// cavern with the siren going during the announcement (the in-world tell),
// then chases the leader. Every touch ("ka-ching") sucks a stream of energy
// motes out of the leader into the printer and prints another stretch of
// receipt; five touches and it leaves with a long receipt flapping. Shot
// down it bursts into shredded paper, sparks and smoke.
//
// Entity (`ReplicatedKind::Drone`, role 0): state `[role, touches, phase,
// hp percent]`. Outcome cues: 1-5 a touch (target = the leader), 0xF0 shot
// down (source = the shooter).

import * as q from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { Rgb } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { TAX_SLOT } from "../../renderer/models/visitors.ts";
import { lin } from "../vfxLibrary.ts";
import type { EntityState } from "../views.ts";
import {
  attach,
  burst,
  damageSmoke,
  DRONE_PHASE,
  hoverPose,
  integrity,
  OUTCOME_SHOT_DOWN,
  put,
  rng,
  ROLE,
  rotorSpin,
  shipOf,
  type DrawCtx,
  type Pose,
  type VisitorInput,
  type VisitorOut,
} from "./visitorKit.ts";

export const TAX_EVENT = "tax_drone";

/** Colours (display sRGB). */
export const TAX_BODY: Rgb = [0.66, 0.6, 0.47];
export const TAX_EYES: Rgb = [1.0, 0.32, 0.12];
export const TAX_TIE: Rgb = [0.78, 0.08, 0.1];
export const TAX_PAPER: Rgb = [0.97, 0.96, 0.9];
export const SIREN_RED: Rgb = [1.0, 0.12, 0.08];
export const SIREN_BLUE: Rgb = [0.15, 0.35, 1.0];

/** Receipt length (u) after `touches` touches (a stub before the first). */
export function receiptLength(touches: number): number {
  return 0.1 + 0.24 * Math.max(0, Math.min(5, touches));
}

/** Brightness (0.12..1) of the red and blue siren domes at `time`: they swap twice a second, one always near full. */
export function sirenPhase(time: number): { red: number; blue: number } {
  const w = 0.5 + 0.5 * Math.sin(time * Math.PI * 4);
  const lit = (x: number): number => 0.12 + 0.88 * x;
  return { red: lit(w), blue: lit(1 - w) };
}

/** Energy mote stream from the leader into the drone's printer after a touch. */
function moteStream(i: VisitorInput, ctx: DrawCtx, from: Vec3, to: Vec3): void {
  const sink = ctx.sink;
  if (!sink) return;
  const r = rng(Math.floor(i.time * 1000));
  const n = Math.round(60 * sink.density);
  for (let k = 0; k < n; k++) {
    const start = v3.add(from, { x: (r() - 0.5) * 0.9, y: (r() - 0.5) * 0.9, z: (r() - 0.5) * 0.9 });
    const life = 0.35 + r() * 0.35;
    const delay = r() * 0.25;
    sink.spawn({
      pos: start,
      vel: v3.scale(v3.sub(to, start), 1 / life),
      life,
      t0: sink.now + delay,
      sprite: SPRITE.glow,
      color: lin([0.45, 0.9, 1.0], 7),
      color1: lin([1.0, 0.85, 0.35], 6),
      size: 0.09,
      size1: 0.05,
      orient: "velocity",
      stretch: 0.06,
      fadeIn: 0.15,
    });
  }
  // The "ka-ching": a gold flare at the printer.
  sink.spawn({ pos: to, life: 0.35, t0: sink.now + 0.45, sprite: SPRITE.flare, color: lin([1, 0.85, 0.3], 9), size: 0.9, size1: 0.2, spin: 3 });
}

/** Shredded paper, sparks and smoke where a tax drone was shot down. */
function shredded(ctx: DrawCtx, at: Vec3, seed: number): void {
  const r = rng(seed);
  burst(ctx.sink, 16, (k, rnd) => ({
    pos: at,
    vel: { x: (rnd() - 0.5) * 9, y: rnd() * 6 - 1, z: (rnd() - 0.5) * 9 },
    life: 1.4 + rnd() * 1.2,
    sprite: SPRITE.shard,
    color: k % 5 === 0 ? [0.75, 0.08, 0.1] : [0.92, 0.91, 0.86],
    alpha: 1,
    alpha1: 0,
    size: 0.12 + rnd() * 0.08,
    drag: 2.2,
    blend: "alpha",
    spin: (rnd() - 0.5) * 14,
    rot: rnd() * 6,
  }), r);
  burst(ctx.sink, 24, (_, rnd) => ({
    pos: at,
    vel: { x: (rnd() - 0.5) * 14, y: (rnd() - 0.5) * 14, z: (rnd() - 0.5) * 14 },
    life: 0.3 + rnd() * 0.3,
    sprite: SPRITE.spark,
    color: lin([1, 0.75, 0.35], 8),
    size: 0.07,
    orient: "velocity",
    stretch: 0.03,
    drag: 3,
  }), r);
  burst(ctx.sink, 6, (_, rnd) => ({
    pos: v3.add(at, { x: (rnd() - 0.5) * 0.6, y: (rnd() - 0.5) * 0.6, z: (rnd() - 0.5) * 0.6 }),
    vel: { x: (rnd() - 0.5) * 2, y: rnd() * 1.5, z: (rnd() - 0.5) * 2 },
    life: 1.6,
    sprite: SPRITE.smoke1,
    color: [0.12, 0.11, 0.1],
    alpha: 0.7,
    alpha1: 0,
    size: 0.6,
    size1: 2.2,
    drag: 1,
    blend: "alpha",
  }), r);
  ctx.sink?.spawn({ pos: at, life: 0.25, sprite: SPRITE.flare, color: lin([1, 0.8, 0.5], 12), size: 2.4, size1: 0.6 });
}

/** Seconds the wreckage of a shot-down drone (receipt, tie, siren domes) tumbles before it is gone. */
export const WRECK_SECS = 2.4;

/** A loose piece of a shot-down tax drone. */
interface Piece {
  mesh: "tax_receipt" | "tax_tie" | "visitor_siren";
  color: Rgb;
  from: Pose;
  vel: Vec3;
  spin: Vec3;
  stretch: number;
  born: number;
}

/** The Tax drone's looks and sounds; keeps each drone's printed receipt length and last pose. */
export class TaxDroneFx {
  private receipt = new Map<number, number>();
  private last = new Map<number, Pose>();
  private pieces: Piece[] = [];

  reset(): void {
    this.receipt.clear();
    this.last.clear();
    this.pieces = [];
  }

  update(i: VisitorInput, ctx: DrawCtx, out: VisitorOut): void {
    const drones = i.entities.filter((e) => e.kind === "drone" && e.state[0] === ROLE.tax);
    for (const o of i.outcomes) {
      if (o.event !== TAX_EVENT) continue;
      if (o.byte === OUTCOME_SHOT_DOWN) {
        shredded(ctx, o.position, Math.floor(i.time * 977));
        this.breakUp(i, ctx, o.position);
        ctx.lights.push({ position: o.position, color: [1, 0.7, 0.4], intensity: 9, range: 10 });
        out.sounds.push({ id: "visitor.burst", position: o.position });
        const by = o.source !== 255 ? i.name(o.source) : "Someone";
        out.toasts.push({ event: TAX_EVENT, text: `${by} shot down the tax drone. Tax refund!`, own: o.source === i.own });
      } else if (o.byte >= 1 && o.byte <= 5) {
        const drone = drones[0];
        const leader = shipOf(i, o.target);
        if (drone && leader) moteStream(i, ctx, leader.position, slotOf(hoverPose(drone, i.time, ctx.up)));
        out.sounds.push({ id: "visitor.kaching", position: drone?.position ?? o.position });
        out.sounds.push({ id: "visitor.printer", position: drone?.position ?? o.position });
        const left = 5 - o.byte;
        out.toasts.push({
          event: TAX_EVENT,
          text: o.target === i.own
            ? `Taxed! 10 % of your energy. ${left > 0 ? `${left} more to go.` : "Thank you for your contribution."}`
            : `${i.name(o.target)} was taxed (${o.byte} of 5).`,
          own: o.target === i.own,
        });
      }
    }
    const seen = new Set<number>();
    for (const d of drones) {
      seen.add(d.id);
      this.draw(i, ctx, d, out);
    }
    for (const id of [...this.receipt.keys()]) if (!seen.has(id)) this.receipt.delete(id);
    for (const id of [...this.last.keys()]) if (!seen.has(id)) this.last.delete(id);
    this.drawPieces(i, ctx);
  }

  /** The receipt, the tie and the siren domes fly off the drone shot down at `at`. */
  private breakUp(i: VisitorInput, ctx: DrawCtx, at: Vec3): void {
    let near: [number, Pose] | null = null;
    for (const [id, p] of this.last) if (!near || v3.length(v3.sub(p.position, at)) < v3.length(v3.sub(near[1].position, at))) near = [id, p];
    const pose = near?.[1] ?? { position: at, rotation: q.identity(), scale: 1 };
    const len = near ? (this.receipt.get(near[0]) ?? receiptLength(1)) : receiptLength(2);
    const r = rng(Math.floor(i.time * 613));
    const out = (x: number, y: number, z: number): Vec3 => q.rotate(pose.rotation, { x, y, z });
    const piece = (mesh: Piece["mesh"], color: Rgb, offset: Vec3, vel: Vec3, stretch = 1): void => {
      this.pieces.push({ mesh, color, from: attach(pose, offset), vel: v3.add(vel, v3.scale(ctx.up, 1 + r() * 1.5)), spin: { x: (r() - 0.5) * 9, y: (r() - 0.5) * 5, z: (r() - 0.5) * 9 }, stretch, born: i.time });
    };
    piece("tax_receipt", TAX_PAPER, { x: TAX_SLOT[0], y: TAX_SLOT[1], z: TAX_SLOT[2] }, out(1.5, 0, -2.5), Math.max(0.5, len));
    piece("tax_tie", TAX_TIE, { x: 0, y: -0.06, z: -0.545 }, out(-1.5, 0, -2.2));
    piece("visitor_siren", SIREN_RED, { x: -0.15, y: 0.48, z: 0.04 }, out(-1.4, 0.2, 0.3));
    piece("visitor_siren", SIREN_BLUE, { x: 0.15, y: 0.48, z: 0.04 }, out(1.5, 0.4, 0.3));
    // Shredded bits of receipt.
    for (let k = 0; k < 12; k++) {
      const a = (k / 12) * Math.PI * 2;
      piece("tax_receipt", TAX_PAPER, { x: (r() - 0.5) * 0.6, y: (r() - 0.5) * 0.5, z: (r() - 0.5) * 0.5 }, out(Math.cos(a) * (2 + r() * 2.5), Math.sin(a) * 2, (r() - 0.5) * 3), 0.12 + r() * 0.1);
    }
  }

  private drawPieces(i: VisitorInput, ctx: DrawCtx): void {
    this.pieces = this.pieces.filter((p) => i.time - p.born < WRECK_SECS);
    for (const p of this.pieces) {
      const age = Math.max(0, i.time - p.born);
      // Paper drifts down slowly with a flutter; the rest falls.
      const paper = p.mesh !== "visitor_siren";
      const fall = paper ? 1.2 : 5;
      const drag = paper ? 1.6 : 0.4;
      const k = (1 - Math.exp(-drag * age)) / drag;
      const pos = v3.add(v3.add(p.from.position, v3.scale(p.vel, k)), v3.scale(ctx.up, -0.5 * fall * age * age));
      const flutter = paper ? Math.sin(age * 9) * 0.5 : 0;
      const spin = q.mul(q.fromAxisAngle({ x: 1, y: 0, z: 0 }, p.spin.x * age + flutter), q.fromAxisAngle({ x: 0, y: 0, z: 1 }, p.spin.z * age));
      const shrink = age > WRECK_SECS - 0.35 ? Math.max(0.01, (WRECK_SECS - age) / 0.35) : 1;
      put(ctx, p.mesh, { position: pos, rotation: q.normalize(q.mul(p.from.rotation, spin)), scale: p.from.scale * shrink }, p.color, {
        stretch: p.mesh === "tax_receipt" ? p.stretch : 1,
        emissive: p.mesh === "visitor_siren" ? 0.6 * Math.max(0, 1 - age) : 0,
      });
    }
  }

  private draw(i: VisitorInput, ctx: DrawCtx, d: EntityState, out: VisitorOut): void {
    const pose = hoverPose(d, i.time, ctx.up);
    this.last.set(d.id, pose);
    put(ctx, "tax_drone", pose, TAX_BODY);
    // Eyes: a blink every few seconds, a squint while it waits.
    const blink = (i.time + d.id * 0.7) % 3.4 < 0.12;
    if (!blink) put(ctx, "tax_eyes", attach(pose, { x: 0, y: 0.1, z: -0.535 }, q.identity(), d.state[2] === DRONE_PHASE.waiting ? 0.85 : 1), TAX_EYES, { emissive: 1 });
    // Rotors.
    let k = 0;
    for (const sx of [-1, 1]) for (const sz of [-1, 1]) put(ctx, "visitor_rotor", attach(pose, { x: sx * 0.78, y: 0.25, z: sz * 0.46 }, rotorSpin(i.time, k++), 0.42), [0.1, 0.1, 0.11]);
    // Siren: red and blue domes taking turns, each with its light.
    const s = sirenPhase(i.time + d.id * 0.13);
    const red = attach(pose, { x: -0.15, y: 0.48, z: 0.04 });
    const blue = attach(pose, { x: 0.15, y: 0.48, z: 0.04 });
    put(ctx, "visitor_siren", red, SIREN_RED, { emissive: s.red });
    put(ctx, "visitor_siren", blue, SIREN_BLUE, { emissive: s.blue });
    ctx.lights.push({ position: v3.addScaled(red.position, ctx.up, 0.3), color: SIREN_RED, intensity: 10 * s.red, range: 14 });
    ctx.lights.push({ position: v3.addScaled(blue.position, ctx.up, 0.3), color: SIREN_BLUE, intensity: 12 * s.blue, range: 14 });
    // Necktie: hangs along the level's down and swings back as it flies.
    const local = q.rotate(q.conjugate(pose.rotation), d.velocity);
    const swing = q.mul(
      q.fromAxisAngle({ x: 1, y: 0, z: 0 }, Math.max(-0.9, Math.min(0.9, -local.z * 0.05)) + Math.sin(i.time * 3.1 + d.id) * 0.12),
      q.fromAxisAngle({ x: 0, y: 0, z: 1 }, Math.max(-0.6, Math.min(0.6, local.x * 0.04)) + Math.sin(i.time * 2.3) * 0.08),
    );
    put(ctx, "tax_tie", attach(pose, { x: 0, y: -0.06, z: -0.545 }, swing), TAX_TIE);
    // Receipt: prints toward its length for the touches so far, flutters.
    const want = receiptLength(d.state[1]);
    const have = this.receipt.get(d.id) ?? want;
    const len = have < want ? Math.min(want, have + i.dt * 0.9) : want;
    this.receipt.set(d.id, len);
    const flutter = q.mul(
      q.fromAxisAngle({ x: 1, y: 0, z: 0 }, Math.PI / 2 + Math.max(-0.7, Math.min(0.5, -local.z * 0.04)) + Math.sin(i.time * 7 + d.id) * 0.06 * len),
      q.fromAxisAngle({ x: 0, y: 0, z: 1 }, Math.sin(i.time * 5.3) * 0.08),
    );
    put(ctx, "tax_receipt", attach(pose, { x: TAX_SLOT[0], y: TAX_SLOT[1], z: TAX_SLOT[2] }, flutter), TAX_PAPER, { stretch: len });
    damageSmoke(ctx.sink, pose.position, integrity(d), i.dt, rng(Math.floor(i.time * 60) + d.id));
    if (d.state[2] !== DRONE_PHASE.leaving) out.sounds.push({ id: "visitor.siren", position: pose.position, loop: `tax-siren-${d.id}` });
  }
}

/** World position of the printer slot of a drone drawn at `pose`. */
export function slotOf(pose: Pose): Vec3 {
  return attach(pose, { x: TAX_SLOT[0], y: TAX_SLOT[1], z: TAX_SLOT[2] }).position;
}
