// Gossip wall (M4 C2; sim: crates/ls-sim/src/events/gossip_wall.rs): a
// stone face pushes out of the rock nearest the pilots during the
// announcement (dust and pebbles crumbling off), with arched brows, sly
// half-shut lids, eyes that dart from pilot to pilot, a big nose and
// lipsticked lips. For 15 s it whispers everyone's whereabouts: through-
// wall markers with names show every ship (cloaked ones too), and the face
// gossips in a speech bubble ("Psst. Ana is 40 u below you. Again.").
// Then it sinks back into the rock.
//
// Entity (`ReplicatedKind::GossipFace`): position on the rock, forward =
// the rock's outward normal (the face looks out), radius = face size,
// state `[talking (1 while the reveal runs), 0, 0, 0]`.

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { Rgb } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { attach, burst, put, rng, secsLeft, smooth01, type DrawCtx, type Pose, type VisitorInput, type VisitorOut, type VisitorShip } from "./visitorKit.ts";

export const GOSSIP_EVENT = "gossip_wall";
export const STONE: Rgb = [0.56, 0.5, 0.45];
/** Rock chunks around the face's rim (they bed it into the wall). */
export const RIM_ROCK: Rgb = [0.4, 0.37, 0.34];
export const GOSSIP_IRIS: Rgb = [0.25, 0.55, 0.85];
/** Seconds the face takes to push out (the 3 s announcement) and to sink back (the 1 s ending). */
export const GROW_SECS = 3;
export const SINK_SECS = 1;
/** Seconds each piece of gossip stays up. */
export const LINE_SECS = 3.2;

/** How far the face has pushed out of the rock (0..1) in the event's phase. */
export function faceGrowth(phase: string, phaseEndTick: number, nowTick: number): number {
  if (phase === "announce") return smooth01(1 - secsLeft(phaseEndTick, nowTick) / GROW_SECS);
  if (phase === "ending") return smooth01(secsLeft(phaseEndTick, nowTick) / SINK_SECS);
  return phase === "active" ? 1 : 0;
}

/** Where `p` is from a viewer at `from` looking along `orientation`: "above you", "behind you", ... */
export function whereFrom(p: Vec3, from: Vec3, orientation: Quat, up: Vec3): string {
  const d = v3.sub(p, from);
  const dist = Math.round(v3.length(d));
  const vert = v3.dot(d, up);
  const fwd = v3.dot(d, q.forward(orientation));
  const side = v3.dot(d, q.right(orientation));
  const a = Math.abs;
  if (a(vert) >= a(fwd) && a(vert) >= a(side)) return `${dist} u ${vert > 0 ? "above" : "below"} you`;
  if (a(fwd) >= a(side)) return fwd > 0 ? `${dist} u ahead of you` : `right behind you, ${dist} u`;
  return `${dist} u to your ${side > 0 ? "right" : "left"}`;
}

const LINES: readonly ((name: string, where: string) => string)[] = [
  (n, w) => `Psst. ${n} is ${w}. Again.`,
  (n, w) => `Don't tell anyone, but ${n} is ${w}.`,
  (n, w) => `${n}? Oh, ${w}. Everyone knows.`,
  (n, w) => `I heard ${n} is ${w}. You didn't hear it from me.`,
  (n, w) => `Between us: ${n}, ${w}. Hiding. Badly.`,
  (n, w) => `You'll never guess who's ${w}. It's ${n}.`,
];

/**
 * The face's line at `time`: a new piece of gossip every LINE_SECS about
 * the next ship in turn (by id), where it is from the own view; null when
 * nobody else is alive. Pure, for the tests.
 */
export function gossipLine(time: number, ships: readonly VisitorShip[], own: number, ownPos: Vec3 | null, orientation: Quat, up: Vec3, name: (id: number) => string): { text: string; about: number } | null {
  const others = ships.filter((s) => s.alive && s.id !== own).sort((a, b) => a.id - b.id);
  if (others.length === 0) return null;
  const slot = Math.max(0, Math.floor(time / LINE_SECS));
  const s = others[slot % others.length]!;
  const line = LINES[(slot * 7 + s.id) % LINES.length]!;
  const where = ownPos ? whereFrom(s.position, ownPos, orientation, up) : "somewhere you can't see";
  return { text: line(name(s.id), where), about: s.id };
}

/** One revealed ship for the through-wall markers. */
export interface GossipMarker {
  id: number;
  name: string;
  position: Vec3;
  distance: number;
}

/** What the HUD shows of the gossip wall this frame. */
export interface GossipHud {
  markers: GossipMarker[];
  bubble: { text: string; anchor: Vec3 } | null;
}

/** The gossip face's looks and its HUD model. */
export class GossipFx {
  private lineSlot = -1;
  private crumble = 0;

  reset(): void {
    this.lineSlot = -1;
  }

  update(i: VisitorInput, ctx: DrawCtx, out: VisitorOut, ownPos: Vec3 | null): GossipHud | null {
    const face = i.entities.find((e) => e.kind === "gossipFace");
    if (!face || i.mood.eventId !== GOSSIP_EVENT) return null;
    const g = faceGrowth(i.mood.phase, i.mood.phaseEndTick, i.nowTick);
    const talking = (face.state[0] ?? 0) === 1;
    const line = talking ? gossipLine(i.time, i.ships, i.own, ownPos, i.orientation, ctx.up, i.name) : null;
    const about = line ? i.ships.find((s) => s.id === line.about)?.position ?? null : null;
    const pose = this.draw(i, ctx, face, g, talking, about);
    if (g > 0.02 && g < 0.98) this.crumbles(i, ctx, pose, face.radius || 3);
    if (talking) {
      out.sounds.push({ id: "visitor.mumble", position: pose.position, loop: "gossip-mumble" });
      const slot = Math.floor(i.time / LINE_SECS);
      if (slot !== this.lineSlot) {
        this.lineSlot = slot;
        out.sounds.push({ id: "visitor.psst", position: pose.position });
      }
    }
    const markers: GossipMarker[] = talking
      ? i.ships
          .filter((s) => s.alive && s.id !== i.own)
          .map((s) => ({ id: s.id, name: i.name(s.id), position: s.position, distance: ownPos ? v3.length(v3.sub(s.position, ownPos)) : 0 }))
      : [];
    // The bubble floats beside the face (to the viewer's right of it).
    return { markers, bubble: line ? { text: line.text, anchor: attach(pose, { x: -1.15, y: 0.25, z: -0.4 }).position } : null };
  }

  private draw(i: VisitorInput, ctx: DrawCtx, face: { id: number; position: Vec3; rotation: Quat; radius: number }, g: number, talking: boolean, about: Vec3 | null): Pose {
    const r = face.radius > 0 ? face.radius : 3;
    const fwd = q.forward(face.rotation);
    // Pushes out of the rock: from inside along the normal, growing.
    const pose: Pose = { position: v3.addScaled(face.position, fwd, -(1 - g) * r * 0.6), rotation: face.rotation, scale: r * (0.45 + 0.55 * g) };
    if (g <= 0.001) return pose;
    put(ctx, "gossip_face", pose, STONE);
    // A collar of broken rock around the rim beds the face into the wall.
    const rim: Pose = { position: face.position, rotation: face.rotation, scale: r };
    for (let k = 0; k < 14; k++) {
      const a = (k / 14) * Math.PI * 2 + 0.3;
      const jag = 0.15 * Math.sin(k * 2.7);
      const chunk = attach(rim, { x: Math.cos(a) * (1.02 + jag), y: Math.sin(a) * (1.18 + jag), z: -0.05 }, q.fromAxisAngle(v3.normalize({ x: Math.sin(k), y: Math.cos(k * 1.3), z: 0.4 }), k * 1.7), (0.32 + 0.12 * Math.sin(k * 1.9) ** 2) * Math.min(1, g * 1.5));
      put(ctx, "debris", chunk, RIM_ROCK);
    }
    const t = i.time;
    // Brows: arched, waggling while it talks, one raised knowingly.
    for (const s of [-1, 1]) {
      const lift = talking ? 0.04 * Math.sin(t * 5 + s) + (s > 0 ? 0.05 : 0) : 0;
      const tilt = q.fromAxisAngle({ x: 0, y: 0, z: 1 }, s * (0.18 + (talking ? 0.1 * Math.sin(t * 3) : 0)));
      put(ctx, "gossip_brow", attach(pose, { x: s * 0.34, y: 0.47 + lift, z: -0.36 }, tilt, 0.82), STONE);
    }
    // Eyes: look at whoever is being gossiped about, else at the camera, with quick darting glances.
    const dart = Math.sin(t * 1.7) > 0.85 ? { x: Math.sin(t * 13) * 3, y: 0, z: 0 } : { x: 0, y: 0, z: 0 };
    const target = v3.add(about ?? i.camera, dart);
    const blink = (t + face.id) % 4.1 < 0.13;
    for (const s of [-1, 1]) {
      const eye = attach(pose, { x: s * 0.33, y: 0.2, z: -0.31 }, q.identity(), 0.19);
      let dir = v3.normalize(v3.sub(target, eye.position));
      // Eyes cannot look into the rock: keep them in the front half.
      const back = v3.dot(dir, fwd);
      if (back < 0.25) dir = v3.normalize(v3.addScaled(dir, fwd, 0.25 - back));
      const up = q.up(face.rotation);
      put(ctx, "visitor_eye", { position: eye.position, rotation: q.lookRotation(dir, up), scale: eye.scale }, GOSSIP_IRIS);
      // Sly half-shut lids; a blink; a wink with the right eye while talking.
      // Sinking back it winks: a secret kept.
      const wink = s > 0 && ((talking && t % 6.3 < 0.35) || i.mood.phase === "ending");
      const lidOpen = blink || wink ? -1.25 : 0.32;
      put(ctx, "gossip_lid", attach(pose, { x: s * 0.33, y: 0.2, z: -0.31 }, q.fromAxisAngle({ x: 1, y: 0, z: 0 }, lidOpen), 0.19), STONE);
    }
    // The jaw chatters in syllables while it talks.
    const syllable = talking ? Math.max(0, Math.sin(t * 11)) * (0.55 + 0.45 * Math.sin(t * 2.3) ** 2) : 0;
    put(ctx, "gossip_jaw", attach(pose, { x: 0, y: -0.43, z: -0.22 }, q.fromAxisAngle({ x: 1, y: 0, z: 0 }, 0.05 + 0.32 * syllable)), STONE);
    // A soft warm light on the face so it reads in dark caves.
    ctx.lights.push({ position: v3.addScaled(pose.position, fwd, r * 1.4), color: [1, 0.82, 0.74], intensity: 3.2 * g, range: r * 3.2 });
    return pose;
  }

  private crumbles(i: VisitorInput, ctx: DrawCtx, pose: Pose, radius: number): void {
    const sink = ctx.sink;
    if (!sink || i.dt <= 0) return;
    this.crumble += i.dt * 30 * sink.density;
    const r = rng(Math.floor(i.time * 733));
    const right = q.right(pose.rotation);
    const up = q.up(pose.rotation);
    const fwd = q.forward(pose.rotation);
    while (this.crumble >= 1) {
      this.crumble -= 1;
      const a = r() * Math.PI * 2;
      const rim = v3.add(pose.position, v3.add(v3.scale(right, Math.cos(a) * radius * 0.9), v3.scale(up, Math.sin(a) * radius)));
      sink.spawn({
        pos: rim,
        vel: v3.add(v3.scale(fwd, 1 + r() * 2), v3.scale(ctx.up, -1 - r() * 2)),
        life: 1.1,
        sprite: r() < 0.5 ? SPRITE.shard : SPRITE.smoke1,
        color: [0.5, 0.45, 0.4],
        alpha: 0.8,
        alpha1: 0,
        size: r() < 0.5 ? 0.12 : 0.6,
        size1: 0.9,
        drag: 1,
        blend: "alpha",
        spin: (r() - 0.5) * 6,
      });
    }
  }
}

/** A burst of dust when the face first appears (exported for the gallery's history). */
export function faceDust(ctx: DrawCtx, at: Vec3, seed: number): void {
  burst(ctx.sink, 12, (_, rnd) => ({
    pos: v3.add(at, { x: (rnd() - 0.5) * 3, y: (rnd() - 0.5) * 3, z: (rnd() - 0.5) * 3 }),
    vel: { x: (rnd() - 0.5) * 2, y: rnd(), z: (rnd() - 0.5) * 2 },
    life: 1.5,
    sprite: SPRITE.smoke0,
    color: [0.5, 0.46, 0.42],
    alpha: 0.5,
    alpha1: 0,
    size: 1,
    size1: 2.5,
    blend: "alpha",
  }), rng(seed));
}
