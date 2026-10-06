// Big head day (M4 C1, milestone-4-plan.md §3.2): for 15 s every ship's hit
// sphere against projectiles is 3x larger. Each ship wears it as a cartoon
// balloon head of exactly that size: a glossy translucent balloon in its
// side's colour around the hull with a face on its nose side (raised brows,
// a wide grin, cheeks) and googly pupils that jiggle with the ship's moves
// and look at you. During the announcement the balloons are blown up in
// three squeaky puffs; at the start they pop to full size; every hit honks
// and makes the balloon wobble; at the end they burst in a pop of rubber
// shreds and confetti. The own ship wears one too, but the cockpit view
// does not draw it (it would tint the whole view).

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { Rgb } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { CUE_HIT } from "../vfx.ts";
import { lin } from "../vfxLibrary.ts";
import { elasticOut, hash01, smooth, type EventClock } from "./fx.ts";
import type { EventEnv, EventInput, EventOutput, EventVisual, ParticleSink } from "./types.ts";

/** Hit sphere factor (data/events/big_head_day.ron `hit_scale`) when the core does not say. */
export const HIT_SCALE = 3;
/** The balloon's opacity. */
const BALLOON_OPACITY = 0.34;

/** Eye centres on the unit balloon (balloonFaceModel: yaw +-0.34, pitch 0.24, at 0.96 plus the white's depth). */
function eyeCentre(side: number): Vec3 {
  const a = side * 0.34;
  const b = 0.24;
  const r = 1.06;
  return { x: Math.sin(a) * Math.cos(b) * r, y: Math.sin(b) * r, z: -Math.cos(a) * Math.cos(b) * r };
}

/**
 * Balloon size (0..1 of the hit sphere) `t` seconds into the event (negative
 * while announced): three puffs during the announcement, a pop to full size
 * at the start, then full; `over` seconds after the end it bursts.
 */
export function inflation(t: number, over: number | null): number {
  if (over !== null) return over < 0.07 ? 1 + over * 2 : 0;
  if (t >= 0) return 0.62 + 0.38 * elasticOut(Math.min(1, t / 0.6));
  const puffs = [-1.9, -1.2, -0.5];
  let s = 0.12;
  for (const p of puffs) s += 0.165 * smooth(p, p + 0.22, t);
  return s;
}

interface Pupil {
  o: { x: number; y: number };
  v: { x: number; y: number };
}

interface Head {
  pupils: [Pupil, Pupil];
  lastPos: Vec3 | null;
  lastVel: Vec3 | null;
  wobbleAt: number;
  seen: number;
}

/** Pupil travel limit inside the eye white (unit balloon). */
const PUPIL_TRAVEL = 0.1;

/**
 * One step of a googly pupil: a spring toward `target` (looking at the
 * viewer) kicked by the ship's acceleration (`kick`, in the eye's plane),
 * underdamped so it jiggles; it bounces off the eye's rim.
 */
export function stepPupil(p: Pupil, target: { x: number; y: number }, kick: { x: number; y: number }, dt: number): void {
  const k = 70;
  const c = 3.2;
  const steps = Math.max(1, Math.ceil(dt / (1 / 120)));
  const h = dt / steps;
  for (let s = 0; s < steps; s++) {
    p.v.x += (-(p.o.x - target.x) * k - p.v.x * c - kick.x) * h;
    p.v.y += (-(p.o.y - target.y) * k - p.v.y * c - kick.y) * h;
    p.o.x += p.v.x * h;
    p.o.y += p.v.y * h;
    const l = Math.hypot(p.o.x, p.o.y);
    if (l > PUPIL_TRAVEL) {
      const nx = p.o.x / l, ny = p.o.y / l;
      p.o.x = nx * PUPIL_TRAVEL;
      p.o.y = ny * PUPIL_TRAVEL;
      const vn = p.v.x * nx + p.v.y * ny;
      if (vn > 0) {
        p.v.x -= 1.6 * vn * nx;
        p.v.y -= 1.6 * vn * ny;
      }
    }
  }
}

/** Burst of a balloon: rubber shreds in its colour and confetti. */
export function balloonBurst(sink: ParticleSink, at: Vec3, radius: number, color: Rgb, seed: number): void {
  const confetti: Rgb[] = [[1, 0.3, 0.4], [1, 0.85, 0.2], [0.3, 0.7, 1], [0.4, 0.95, 0.5]];
  for (let i = 0; i < 26; i++) {
    const a = hash01(seed, i) * Math.PI * 2;
    const b = Math.acos(2 * hash01(seed, i + 50) - 1);
    const d = { x: Math.sin(b) * Math.cos(a), y: Math.cos(b), z: Math.sin(b) * Math.sin(a) };
    const shred = i < 14;
    sink.spawn({
      pos: v3.addScaled(at, d, radius * 0.9),
      vel: v3.scale(d, (shred ? 5 : 3) + 5 * hash01(seed, i + 9)),
      life: shred ? 1.2 : 2.2,
      sprite: shred ? SPRITE.shard : SPRITE.feather,
      blend: "alpha",
      color: shred ? lin(color, 0.55) : lin(confetti[i % 4]!, 0.6),
      alpha: 1,
      alpha1: 0,
      size: shred ? radius * 0.22 : radius * 0.12,
      drag: 2.2,
      spin: (hash01(seed, i + 77) - 0.5) * 14,
    });
  }
  sink.spawn({ pos: at, life: 0.12, sprite: SPRITE.ring, color: lin(color, 0.5), size: radius * 0.9, size1: radius * 1.8 });
}

class BigHeadDay implements EventVisual {
  private readonly env: EventEnv;
  private readonly heads = new Map<number, Head>();
  private puffs = 0;
  private burstAt = -1;
  private started = false;
  private burst = false;
  private lastRadius = new Map<number, { pos: Vec3; r: number; color: Rgb }>();

  constructor(env: EventEnv) {
    this.env = env;
  }

  update(i: EventInput, clock: EventClock, out: EventOutput): boolean {
    const sound = i.quiet ? null : this.env.sound;
    const scale = clock.phase === "active" || clock.phase === "ending" ? Math.max(1, i.fx.hitScale) : HIT_SCALE;
    const size = inflation(clock.t, clock.over);
    // The tell: a squeaky puff per inflation step, then the pop to size.
    const puffAt = [-1.9, -1.2, -0.5].filter((p) => clock.t >= p && clock.over === null).length;
    while (this.puffs < puffAt) {
      this.puffs++;
      sound?.play("bighead.inflate", { rate: 0.9 + this.puffs * 0.12 });
    }
    if (clock.t >= 0 && clock.over === null && !this.started) {
      this.started = true;
      sound?.play("bighead.inflate", { rate: 1.45 });
    }
    if (clock.over !== null) {
      if (!this.burst) {
        this.burst = true;
        this.burstAt = i.time;
        let n = 0;
        for (const [id, b] of this.lastRadius) {
          if (this.env.particles) balloonBurst(this.env.particles, b.pos, b.r, b.color, id);
          if (n++ < 4) sound?.play("bighead.pop", { position: b.pos });
        }
      }
      // A comic "POP!" over the nearest balloon.
      const age = i.time - this.burstAt;
      if (age < 0.8) out.hud.pops.push({ text: "POP!", x: 0.5, y: 0.4, scale: 0.4 + 0.6 * Math.min(1, age / 0.12), tilt: -8, alpha: 1 - smooth(0.45, 0.8, age), color: "#ff7ab6" });
      return clock.over < 0.8;
    }
    // Honks and wobbles on hits.
    let honks = 0;
    for (const c of i.cues) {
      if (c.kind !== CUE_HIT || clock.t < 0) continue;
      const h = this.heads.get(c.target);
      if (h) h.wobbleAt = i.time;
      if (honks++ < 3) sound?.play("bighead.honk", { position: c.position, rate: 0.85 + 0.35 * hash01(c.tick, c.target) });
    }
    this.lastRadius.clear();
    for (const s of i.ships) {
      if (!s.alive) continue;
      const head = this.heads.get(s.id) ?? { pupils: [{ o: { x: 0, y: 0 }, v: { x: 0, y: 0 } }, { o: { x: 0, y: 0 }, v: { x: 0, y: 0 } }], lastPos: null, lastVel: null, wobbleAt: -9, seen: i.time };
      head.seen = i.time;
      this.heads.set(s.id, head);
      const r = s.radius * scale * size;
      if (r <= 0.01) continue;
      const color = i.colorOf(s.id);
      this.lastRadius.set(s.id, { pos: s.position, r, color });
      // Wobble: the balloon squashes along its face axis after a hit.
      const wt = i.time - head.wobbleAt;
      const wob = wt >= 0 && wt < 1.2 ? 0.16 * Math.exp(-wt * 5) * Math.sin(wt * 28) : 0;
      const breathe = 0.015 * Math.sin(i.time * 2.3 + s.id);
      const rot = s.orientation;
      out.translucent.push(["balloon", { position: s.position, rotation: rot, scale: r * (1 + breathe), stretch: 1 + wob, color: lighten(color), emissive: 0, opacity: BALLOON_OPACITY }]);
      out.opaque.push(["balloon_face", { position: s.position, rotation: rot, scale: r * (1 + breathe), stretch: 1 + wob, color: [1, 1, 1], emissive: 0 }]);
      // Googly pupils: jiggle with the ship's acceleration, look at the viewer.
      // Ships carry no velocity on the client: the change of position stands in.
      const vel = head.lastPos && i.dt > 0 ? v3.scale(v3.sub(s.position, head.lastPos), 1 / i.dt) : { x: 0, y: 0, z: 0 };
      const rawAccel = head.lastVel && i.dt > 0 ? v3.scale(v3.sub(vel, head.lastVel), 1 / i.dt) : { x: 0, y: 0, z: 0 };
      // Snapshot steps make spikes: clamp to a hard turn's acceleration.
      const al = v3.length(rawAccel);
      const accel = al > 80 ? v3.scale(rawAccel, 80 / al) : rawAccel;
      head.lastPos = s.position;
      head.lastVel = vel;
      const toCam = v3.normalize(v3.sub(i.camera.position, s.position));
      for (let e = 0; e < 2; e++) {
        const side = e === 0 ? -1 : 1;
        const c = eyeCentre(side);
        const frame = eyeFrame(rot, c);
        const look = { x: v3.dot(toCam, frame.right) * 0.07, y: v3.dot(toCam, frame.up) * 0.07 };
        const kick = { x: v3.dot(accel, frame.right) * 0.004 + wob * 9, y: v3.dot(accel, frame.up) * 0.004 + v3.dot(i.up, frame.up) * 0.8 };
        const p = head.pupils[e]!;
        stepPupil(p, look, kick, Math.min(i.dt, 0.05));
        const local = v3.add(c, v3.add(v3.scale(frame.rightLocal, p.o.x), v3.scale(frame.upLocal, p.o.y)));
        const pos = v3.add(s.position, v3.scale(q.rotate(rot, { x: local.x, y: local.y, z: local.z * (1 + wob) }), r * (1 + breathe)));
        out.opaque.push(["pupil", { position: pos, rotation: frame.rotation, scale: r, color: [1, 1, 1], emissive: 0 }]);
      }
    }
    for (const [id, h] of this.heads) if (i.time - h.seen > 30) this.heads.delete(id);
    return true;
  }
}

/** A side colour lifted toward a bright balloon latex. */
function lighten(c: Rgb): Rgb {
  return [c[0] * 0.75 + 0.25, c[1] * 0.75 + 0.25, c[2] * 0.75 + 0.25];
}

/** The tangent frame of an eye at `c` on the balloon turned by `rot`: world right/up, local right/up and the pupil's rotation. */
function eyeFrame(rot: Quat, c: Vec3): { right: Vec3; up: Vec3; rightLocal: Vec3; upLocal: Vec3; rotation: Quat } {
  const n = v3.normalize(c);
  const rightLocal = v3.normalize(v3.cross({ x: 0, y: 1, z: 0 }, n));
  const upLocal = v3.cross(n, rightLocal);
  // The pupil mesh is flattened along z: its z axis along the eye's normal.
  const local = q.lookRotation(v3.scale(n, -1), upLocal);
  return { right: q.rotate(rot, rightLocal), up: q.rotate(rot, upLocal), rightLocal, upLocal, rotation: q.normalize(q.mul(rot, local)) };
}

export const bigHeadDay = (env: EventEnv): EventVisual => new BigHeadDay(env);
