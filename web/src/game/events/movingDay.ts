// Moving day (M4 C1, milestone-4-plan.md §3.2): every pickup lying in the
// level moves to another pickup point; nothing can be taken during the 2 s
// move. The sim moves a pickup's content at once when the move starts
// (`Command::MovePickup`); the client packs each one into a cardboard box
// and carries it there:
// - announcement (the tell): at -2 s a box pops up around every pickup with
//   its flaps open, at -1.2 s the flaps fold shut, then a strip of packing
//   tape runs over the seam (with the tape's screech);
// - the 2 s move: the box hops along an arc to the new point, turning over
//   once, and lands with a squash and a thump;
// - after it: the flaps pop open, the box shrinks away and the pickup is
//   back. The announcer apologizes for the mess.
// The pickups are hidden while they are boxed (`hideEntities`).

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { lin } from "../vfxLibrary.ts";
import { elasticOut, hash01, smooth, type EventClock } from "./fx.ts";
import type { EventEnv, EventInput, EventOutput, EventVisual } from "./types.ts";

/** Entity kinds Moving day moves (ls-sim: every pickup but scrap). */
export const MOVED_KINDS: ReadonlySet<string> = new Set(["weaponPickup", "upgradePickup", "gadgetPickup"]);

/** Box edge (u). */
export const BOX_SIZE = 1.35;
/** Timeline (s from the move's start; the move itself takes MOVE_SECS). */
const BOX_IN = -2.0;
const FLAPS_SHUT = -1.2;
const TAPE = -0.75;
export const MOVE_SECS = 2;
const UNPACK_SECS = 0.7;

export interface BoxPose {
  position: Vec3;
  rotation: Quat;
  scale: number;
  /** Lid flap angle (rad): 0 shut, up to ~2 open. */
  flaps: number;
  /** Tape laid 0..1. */
  tape: number;
  /** Squash along up on landing (1 = none). */
  squash: number;
}

/**
 * The box of a pickup moving `from` -> `to`, `t` seconds after the move
 * started (negative while announced; `after` seconds after the move, null
 * during it). null when no box shows.
 */
export function boxPose(from: Vec3, to: Vec3, up: Vec3, t: number, seed: number): BoxPose | null {
  if (t < BOX_IN || t > MOVE_SECS + UNPACK_SECS) return null;
  const side = v3.normalize(v3.cross(up, Math.abs(up.x) > 0.9 ? { x: 0, y: 0, z: 1 } : { x: 1, y: 0, z: 0 }));
  const yaw = q.fromAxisAngle(up, hash01(seed, 2) * 6.28);
  const base = q.normalize(q.mul(yaw, q.lookRotation(v3.cross(side, up), up)));
  if (t < 0) {
    const grow = elasticOut((t - BOX_IN) / 0.45);
    const shut = smooth(FLAPS_SHUT, FLAPS_SHUT + 0.35, t);
    return { position: from, rotation: base, scale: BOX_SIZE * grow, flaps: 1.9 * (1 - shut), tape: smooth(TAPE, TAPE + 0.45, t), squash: 1 + 0.08 * Math.sin(t * 9) * (1 - shut) };
  }
  if (t <= MOVE_SECS) {
    const u = t / MOVE_SECS;
    const e = u * u * (3 - 2 * u);
    const dist = v3.length(v3.sub(to, from));
    const height = 2.5 + 0.35 * dist;
    const pos = v3.addScaled(v3.lerp(from, to, e), up, 4 * height * e * (1 - e));
    // One flip about the axis across the flight, and a little wobble.
    const dir = dist > 1e-3 ? v3.normalize(v3.sub(to, from)) : side;
    const across = v3.normalize(v3.cross(up, dir));
    const flip = q.fromAxisAngle(v3.length(across) > 0.1 ? across : side, e * Math.PI * 2);
    const squash = u < 0.08 ? 1 - 0.25 * Math.sin((u / 0.08) * Math.PI) : 1;
    return { position: pos, rotation: q.normalize(q.mul(flip, base)), scale: BOX_SIZE, flaps: 0, tape: 1, squash };
  }
  const a = t - MOVE_SECS;
  const land = a < 0.25 ? 1 - 0.3 * Math.sin((a / 0.25) * Math.PI) : 1;
  const open = smooth(0.12, 0.3, a);
  const shrink = 1 - smooth(0.35, UNPACK_SECS, a);
  return { position: to, rotation: base, scale: BOX_SIZE * shrink, flaps: 2.2 * open, tape: 1 - open, squash: land };
}

interface Moving {
  from: Vec3;
  to: Vec3 | null;
}

class MovingDay implements EventVisual {
  private readonly env: EventEnv;
  private readonly moving = new Map<number, Moving>();
  private taped = false;
  private moved = false;
  private landed = false;
  private opened = false;

  constructor(env: EventEnv) {
    this.env = env;
  }

  update(i: EventInput, clock: EventClock, out: EventOutput): boolean {
    const sound = i.quiet ? null : this.env.sound;
    const t = clock.over === null ? clock.t : MOVE_SECS + clock.over;
    // Remember where every pickup was before the move, then where it went.
    for (const e of i.entities) {
      if (!MOVED_KINDS.has(e.kind)) continue;
      const m = this.moving.get(e.id);
      if (!m) {
        if (t < 0) this.moving.set(e.id, { from: e.position, to: null });
      } else if (t < 0) {
        m.from = e.position;
      } else if (!m.to) {
        m.to = e.position;
      }
    }
    const cam = i.camera.position;
    let nearest = Infinity;
    let nearestAt: Vec3 | null = null;
    for (const [id, m] of this.moving) {
      const to = m.to ?? m.from;
      const pose = boxPose(m.from, to, i.up, t, id);
      if (!pose) continue;
      if (t > BOX_IN + 0.2 && t < MOVE_SECS + 0.45) out.hideEntities.add(id);
      const d = v3.length(v3.sub(pose.position, cam));
      if (d < nearest) {
        nearest = d;
        nearestAt = pose.position;
      }
      this.addBox(out, pose, i.up);
      // A puff trail behind every box in flight.
      if (this.env.particles && t > 0.05 && t < MOVE_SECS - 0.05 && d < 90 && hash01(id, Math.floor(i.time * 14)) < 0.5) {
        this.env.particles.spawn({ pos: pose.position, vel: v3.scale(i.up, -0.4), life: 0.9, sprite: SPRITE.smoke3, blend: "alpha", color: lin([0.72, 0.62, 0.48], 0.4), alpha: 0.45, alpha1: 0, size: 0.4, size1: 1.3, drag: 1.5, spin: 1 });
      }
    }
    if (nearestAt && nearest < 80) {
      if (!this.taped && t >= TAPE) {
        this.taped = true;
        sound?.play("moving.tape", { position: nearestAt });
      }
      if (!this.moved && t >= 0) {
        this.moved = true;
        sound?.play("moving.hop", { position: nearestAt });
      }
      if (!this.landed && t >= MOVE_SECS) {
        this.landed = true;
        sound?.play("moving.thump", { position: nearestAt });
        if (this.env.particles) for (const [id, m] of this.moving) if (m.to && v3.length(v3.sub(m.to, cam)) < 80) this.dust(m.to, i.up, id);
      }
      if (!this.opened && t >= MOVE_SECS + 0.12) {
        this.opened = true;
        sound?.play("moving.pop", { position: nearestAt });
      }
    }
    return clock.over === null || t < MOVE_SECS + UNPACK_SECS;
  }

  private addBox(out: EventOutput, b: BoxPose, up: Vec3): void {
    if (b.scale <= 0.01) return;
    const s = b.scale;
    const lu = q.rotate(b.rotation, { x: 0, y: 1, z: 0 });
    // Squash: shorter along the box's up, kept on its base.
    const centre = v3.addScaled(b.position, lu, (b.squash - 1) * s * 0.5);
    out.opaque.push(["moving_box", { position: centre, rotation: b.rotation, scale: s, stretch: 1, color: [1, 1, 1], emissive: 0 }]);
    void up;
    // Two flaps hinged on the top edges at local z = -0.5 and +0.5, folding over the top.
    const top = v3.addScaled(centre, lu, s * 0.5 * b.squash);
    for (const sideSign of [-1, 1]) {
      const hinge = v3.add(top, q.rotate(b.rotation, { x: 0, y: 0, z: sideSign * 0.5 * s }));
      // Shut: the flap lies along -sideSign z toward the centre; open: it swings up and out.
      const face = q.fromAxisAngle({ x: 0, y: 1, z: 0 }, sideSign > 0 ? Math.PI : 0);
      const swing = q.fromAxisAngle({ x: 1, y: 0, z: 0 }, -b.flaps);
      out.opaque.push(["box_flap", { position: hinge, rotation: q.normalize(q.mul(b.rotation, q.mul(face, swing))), scale: s, color: [1, 1, 1], emissive: 0 }]);
    }
    if (b.tape > 0.01) {
      const start = v3.add(top, q.rotate(b.rotation, { x: 0, y: 0.03 * s, z: -0.5 * s }));
      out.opaque.push(["box_tape", { position: start, rotation: b.rotation, scale: s, stretch: b.tape, color: [1, 1, 1], emissive: 0 }]);
    }
  }

  private dust(at: Vec3, up: Vec3, seed: number): void {
    const sink = this.env.particles;
    if (!sink) return;
    const side = v3.normalize(v3.cross(up, Math.abs(up.x) > 0.9 ? { x: 0, y: 0, z: 1 } : { x: 1, y: 0, z: 0 }));
    const side2 = v3.cross(up, side);
    for (let k = 0; k < 10; k++) {
      const a = (k / 10) * Math.PI * 2 + hash01(seed, k);
      const d = v3.add(v3.scale(side, Math.cos(a)), v3.scale(side2, Math.sin(a)));
      sink.spawn({ pos: v3.addScaled(at, up, -BOX_SIZE * 0.4), vel: v3.scale(d, 3 + 2 * hash01(seed, k + 20)), life: 1.1, sprite: SPRITE.smoke1, blend: "alpha", color: lin([0.72, 0.6, 0.45], 0.45), alpha: 0.5, alpha1: 0, size: 0.35, size1: 1.4, drag: 2.5 });
    }
  }
}

export const movingDay = (env: EventEnv): EventVisual => new MovingDay(env);
