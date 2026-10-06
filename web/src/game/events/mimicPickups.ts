// Mimic pickups (M4 C2; sim: crates/ls-sim/src/events/mimic_pickups.rs):
// during the announcement the picked pickups tremble and twitch; then each
// grows into a mimic: a supply crate in the pickup's colour whose lid opens
// on a row of teeth, with a lolling tongue and googly eyes that follow its
// prey. It hops (squash on landing, stretch in the air), snaps its lid on
// every bite and shatters into splinters when killed (its content drops
// where it died). The real pickups stay hidden while their mimics are out.
//
// Entity (`ReplicatedKind::Mimic`): visual = the pickup's visual, state
// `[weapon id, ammo, flags, hp percent]`, flags bit 0 upgrade, bit 1
// hunting, bit 2 biting, bit 3 going home. Mood params: the picked pickups'
// spawn indices (255 unused). Outcome cues: 2 a bite (target = the bitten
// ship), 0xF0 a mimic killed (source = the shooter).

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { Rgb } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { phase, styleFor } from "../scene.ts";
import { lin } from "../vfxLibrary.ts";
import type { EntityState } from "../views.ts";
import { burst, damageSmoke, fromZUp, integrity, NONE, OUTCOME_SHOT_DOWN, put, rng, type DrawCtx, type VisitorInput, type VisitorOut } from "./visitorKit.ts";

export const MIMIC_EVENT = "mimic_pickups";
/** Outcome byte of a bite. */
export const OUTCOME_BITE = 2;
/** Mimic flags (`state[2]`). */
export const MIMIC_FLAG = { upgrade: 1, hunting: 2, biting: 4, home: 8 } as const;
/** Unused mimic slot in the mood params. */
export const NO_SPAWN = 255;
/** A pickup within this distance (u) of a picked point is the picked one. */
export const PICK_MATCH = 1.2;
/** Length of one hop (u along the ground) and its height (u). */
export const HOP_LENGTH = 2.4;
export const HOP_HEIGHT = 0.55;
export const IRIS: Rgb = [0.72, 0.88, 0.12];

/** Pickup kinds a mimic can imitate. */
const MIMICABLE: ReadonlySet<string> = new Set(["weaponPickup", "upgradePickup"]);

/** The picked spawn points of a running Mimic pickups event (empty otherwise). */
export function pickedPoints(i: Pick<VisitorInput, "mood" | "pickupPoints">): Vec3[] {
  if (i.mood.eventId !== MIMIC_EVENT || i.mood.phase === "") return [];
  const out: Vec3[] = [];
  for (const idx of i.mood.params) {
    if (idx === NO_SPAWN) continue;
    const p = i.pickupPoints[idx];
    if (p) out.push(p);
  }
  return out;
}

/** The pickup entities at the picked points (weapon and upgrade pickups only). */
export function pickedPickups(entities: readonly EntityState[], points: readonly Vec3[]): EntityState[] {
  if (points.length === 0) return [];
  return entities.filter((e) => MIMICABLE.has(e.kind) && points.some((p) => v3.length(v3.sub(p, e.position)) < PICK_MATCH));
}

/**
 * Squash and stretch of a hop at `hop` (radians of the hop cycle; 0, π, 2π
 * are landings): vertical stretch (below 1 squashes) and the lift (u).
 */
export function hopShape(hop: number, moving: number): { stretch: number; lift: number } {
  const s = Math.abs(Math.sin(hop));
  const m = Math.max(0, Math.min(1, moving));
  // Squashed flat on the ground, stretched tall at take-off and landing speed.
  const stretch = 1 + m * (s < 0.35 ? -0.24 * (1 - s / 0.35) : 0.16 * Math.cos(hop * 2) ** 2);
  return { stretch, lift: m * HOP_HEIGHT * s };
}

/** Lid opening (rad) for a mimic's flags at `time`. */
export function lidAngle(flags: number, time: number, id: number): number {
  if (flags & MIMIC_FLAG.biting) return 1.05;
  if (flags & MIMIC_FLAG.hunting) return 0.4 + 0.3 * Math.abs(Math.sin(time * 9 + id));
  // Idle or going home: chattering teeth.
  return 0.08 + 0.06 * Math.abs(Math.sin(time * 14 + id));
}

interface MimicTrack {
  hop: number;
  last: Vec3;
  landed: boolean;
}

/** The mimics' looks; tracks each mimic's hop phase. */
export class MimicFx {
  private tracks = new Map<number, MimicTrack>();

  reset(): void {
    this.tracks.clear();
  }

  /** Pickup entity ids to leave out of the scene: the picked ones while their event runs. */
  hidden(i: VisitorInput): Set<number> {
    const out = new Set<number>();
    for (const e of pickedPickups(i.entities, pickedPoints(i))) out.add(e.id);
    return out;
  }

  update(i: VisitorInput, ctx: DrawCtx, out: VisitorOut): void {
    for (const o of i.outcomes) {
      if (o.event !== MIMIC_EVENT) continue;
      if (o.byte === OUTCOME_BITE) {
        out.sounds.push({ id: "visitor.chomp", position: o.position });
        ctx.sink?.spawn({ pos: o.position, life: 0.18, sprite: SPRITE.flare, color: lin([1, 1, 0.95], 6), size: 1.2, size1: 0.3 });
        if (o.target === i.own) out.toasts.push({ event: MIMIC_EVENT, text: "CHOMP! That crate bit you.", own: true });
      } else if (o.byte === OUTCOME_SHOT_DOWN) {
        const colour = this.colourNear(i, o.position);
        splinters(ctx, o.position, colour, Math.floor(i.time * 211));
        out.sounds.push({ id: "visitor.splinter", position: o.position });
        const by = o.source !== NONE ? i.name(o.source) : "Someone";
        out.toasts.push({ event: MIMIC_EVENT, text: `${by} killed a mimic. Its loot is real again.`, own: o.source === i.own });
      }
    }
    // The announcement: the picked pickups tremble (drawn here; the scene leaves them out).
    if (i.mood.eventId === MIMIC_EVENT && i.mood.phase === "announce") {
      for (const p of pickedPickups(i.entities, pickedPoints(i))) this.tremble(i, ctx, p);
    }
    const live = new Set<number>();
    for (const m of i.entities) {
      if (m.kind !== "mimic") continue;
      live.add(m.id);
      this.draw(i, ctx, m, out);
    }
    for (const id of [...this.tracks.keys()]) if (!live.has(id)) this.tracks.delete(id);
  }

  private colourNear(i: VisitorInput, at: Vec3): Rgb {
    let best: EntityState | null = null;
    let d = Infinity;
    for (const m of i.entities) {
      if (m.kind !== "mimic") continue;
      const dd = v3.length(v3.sub(m.position, at));
      if (dd < d) {
        d = dd;
        best = m;
      }
    }
    return (best && i.visuals[best.visual]?.color) || [0.8, 0.6, 0.3];
  }

  private tremble(i: VisitorInput, ctx: DrawCtx, p: EntityState): void {
    const visual = i.visuals[p.visual];
    const style = styleFor(visual);
    const ph = phase(p.id);
    const shake = 0.06;
    const pos = v3.add(p.position, {
      x: Math.sin(i.time * 47 + ph) * shake,
      y: Math.sin(i.time * 2 + ph) * (style.bob ?? 0) + Math.abs(Math.sin(i.time * 13)) * 0.08,
      z: Math.cos(i.time * 53 + ph) * shake,
    });
    const rot = q.mul(q.fromAxisAngle({ x: 0, y: 1, z: 0 }, i.time * (style.spin ?? 0) + ph), q.fromAxisAngle({ x: 0, y: 0, z: 1 }, Math.sin(i.time * 31 + ph) * 0.12));
    const scale = style.size * (visual?.scale ?? 1) * (1 + 0.06 * Math.sin(i.time * 9));
    if (style.mesh === "crate") {
      // A supply crate peeks: its lid cracks open on a row of teeth now and then.
      drawCrate(ctx, pos, q.normalize(rot), scale * 0.62, 1, visual?.color ?? [1, 1, 1], peekAngle(i.time, p.id), null, i.time + p.id);
      return;
    }
    put(ctx, style.mesh, { position: pos, rotation: q.normalize(rot), scale }, visual?.color ?? [1, 1, 1], { emissive: style.emissive + 0.3 * Math.abs(Math.sin(i.time * 6)) });
  }

  private draw(i: VisitorInput, ctx: DrawCtx, m: EntityState, out: VisitorOut): void {
    const colour: Rgb = i.visuals[m.visual]?.color ?? [0.8, 0.6, 0.3];
    const flags = m.state[2] ?? 0;
    const up = ctx.up;
    // Hop phase from the ground speed (across the level's up).
    const t = this.tracks.get(m.id) ?? { hop: 0, last: m.position, landed: true };
    if (!this.tracks.has(m.id)) {
      // Just grown: a puff and a growl.
      poof(ctx, m.position, colour, m.id);
      out.sounds.push({ id: "visitor.growl", position: m.position });
    }
    const moved = v3.sub(m.position, t.last);
    const ground = v3.length(v3.sub(moved, v3.scale(up, v3.dot(moved, up))));
    const speed = i.dt > 0 ? ground / i.dt : 0;
    const moving = Math.max(0, Math.min(1, (speed - 0.8) / 3));
    const before = t.hop;
    t.hop += moving > 0 ? (ground / HOP_LENGTH) * Math.PI : 0;
    // Settle onto the ground when it stops.
    if (moving === 0) t.hop = Math.round(t.hop / Math.PI) * Math.PI;
    if (Math.floor(t.hop / Math.PI) > Math.floor(before / Math.PI) && moving > 0.2) {
      out.sounds.push({ id: "visitor.hop", position: m.position, gain: 0.6 });
      ctx.sink?.spawn({ pos: v3.addScaled(m.position, up, -0.5), vel: v3.scale(up, 0.4), life: 0.5, sprite: SPRITE.smoke1, color: [0.45, 0.42, 0.38], alpha: 0.35, alpha1: 0, size: 0.4, size1: 1.2, blend: "alpha" });
    }
    t.last = m.position;
    this.tracks.set(m.id, t);
    const { stretch, lift } = hopShape(t.hop, moving);
    // Breathing while idle.
    const breathe = 1 + (moving > 0 ? 0 : 0.03 * Math.sin(i.time * 3 + m.id));
    const s = Math.pow(stretch * breathe, -1 / 3);
    const base = v3.addScaled(m.position, up, lift);
    const rot = m.rotation;
    const open = lidAngle(flags, i.time, m.id);
    const prey = nearestShip(i, m.position);
    drawCrate(ctx, base, rot, s, stretch * breathe, colour, open, prey ?? i.camera, i.time + m.id);
    damageSmoke(ctx.sink, base, integrity(m), i.dt, rng(Math.floor(i.time * 60) + m.id));
    if (flags & MIMIC_FLAG.hunting) out.sounds.push({ id: "visitor.snarl", position: base, loop: `mimic-${m.id}` });
  }
}

/** The lid of a picked crate during the announcement: shut, with a quick peek about every 1.3 s. */
export function peekAngle(time: number, id: number): number {
  const t = (time + (id % 7) * 0.37) % 1.3;
  return t < 0.32 ? 0.42 * Math.sin((t / 0.32) * Math.PI) : 0.02;
}

/**
 * A mimic crate: the body (Z-up, squashed by `stretch`), the lid on its
 * hinge opened by `open`, googly eyes on the lid's front looking at
 * `lookAt` (none when null) and, while open, the tongue.
 */
function drawCrate(ctx: DrawCtx, base: Vec3, rot: Quat, s: number, stretch: number, colour: Rgb, open: number, lookAt: Vec3 | null, t: number): void {
  put(ctx, "mimic_body", { position: base, rotation: fromZUp(rot), scale: s }, colour, { stretch, emissive: 0.05 });
  const hinge = v3.add(base, q.rotate(rot, { x: 0, y: 0.22 * s * stretch, z: 0.55 * s }));
  const lidRot = q.normalize(q.mul(rot, q.fromAxisAngle({ x: 1, y: 0, z: 0 }, open)));
  put(ctx, "mimic_lid", { position: hinge, rotation: lidRot, scale: s }, colour, { emissive: 0.05 });
  if (lookAt) {
    for (const sx of [-1, 1]) {
      const eye = v3.add(hinge, q.rotate(lidRot, v3.scale({ x: sx * 0.27, y: 0.2, z: -1.16 }, s)));
      const dir = v3.normalize(v3.sub(lookAt, eye));
      const jiggle = q.fromAxisAngle({ x: 0, y: 0, z: 1 }, Math.sin(t * 17 + sx * 3) * 0.4);
      const eyeRot = q.normalize(q.mul(q.lookRotation(v3.length(dir) > 0 ? dir : q.forward(rot), ctx.up), jiggle));
      put(ctx, "visitor_eye", { position: eye, rotation: eyeRot, scale: 0.2 * s }, IRIS);
    }
  }
  if (open > 0.3) {
    // Hangs out over the front rim and down the front, wagging.
    const wag = q.mul(q.fromAxisAngle({ x: 1, y: 0, z: 0 }, -0.95 + 0.2 * Math.sin(t * 8)), q.fromAxisAngle({ x: 0, y: 1, z: 0 }, Math.sin(t * 6) * 0.3));
    const root = v3.add(base, q.rotate(rot, { x: 0.08 * s, y: 0.27 * s * stretch, z: -0.5 * s }));
    put(ctx, "mimic_tongue", { position: root, rotation: q.normalize(q.mul(rot, wag)), scale: s * (0.85 + 0.15 * Math.sin(t * 5)) }, [1, 1, 1]);
  }
}

function nearestShip(i: VisitorInput, at: Vec3): Vec3 | null {
  let best: Vec3 | null = null;
  let d = 30;
  for (const s of i.ships) {
    if (!s.alive) continue;
    const dd = v3.length(v3.sub(s.position, at));
    if (dd < d) {
      d = dd;
      best = s.position;
    }
  }
  return best;
}

function poof(ctx: DrawCtx, at: Vec3, colour: Rgb, seed: number): void {
  const r = rng(seed * 31 + 7);
  burst(ctx.sink, 10, (_, rnd) => ({
    pos: v3.add(at, { x: (rnd() - 0.5) * 0.8, y: (rnd() - 0.5) * 0.8, z: (rnd() - 0.5) * 0.8 }),
    vel: { x: (rnd() - 0.5) * 2, y: rnd() * 1.5, z: (rnd() - 0.5) * 2 },
    life: 0.9,
    sprite: SPRITE.smoke2,
    color: [0.7, 0.68, 0.64],
    alpha: 0.5,
    alpha1: 0,
    size: 0.5,
    size1: 1.6,
    drag: 1.2,
    blend: "alpha",
  }), r);
  ctx.sink?.spawn({ pos: at, life: 0.3, sprite: SPRITE.flare, color: lin(colour, 6), size: 1.6, size1: 0.3 });
}

function splinters(ctx: DrawCtx, at: Vec3, colour: Rgb, seed: number): void {
  const r = rng(seed);
  burst(ctx.sink, 36, (k, rnd) => ({
    pos: at,
    vel: { x: (rnd() - 0.5) * 10, y: rnd() * 7 - 1, z: (rnd() - 0.5) * 10 },
    life: 1 + rnd() * 0.8,
    sprite: SPRITE.shard,
    color: k % 3 === 0 ? [0.15, 0.15, 0.17] : k % 7 === 0 ? [0.95, 0.94, 0.9] : colour,
    alpha: 1,
    alpha1: 0,
    size: 0.12 + rnd() * 0.08,
    drag: 2,
    blend: "alpha",
    spin: (rnd() - 0.5) * 14,
  }), r);
  ctx.sink?.spawn({ pos: at, life: 0.25, sprite: SPRITE.flare, color: lin(colour, 10), size: 2.2, size1: 0.4 });
}
