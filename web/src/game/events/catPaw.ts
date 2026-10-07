// Cat paw (M4 C3; milestone-4-plan.md §3.2): a giant cat reaches into the
// level (sim: B3's events/cat_paw.rs, one `ReplicatedKind::CatPaw` prop,
// posed every tick: fingers along local -Z, sweep direction +X, claws toward
// local -Y / +Y / +Z by claw mode; state [phase 0 tell, 1 reach, 2 sweep,
// 3 withdraw, progress 0-255, face, claw mode]). The client draws:
//
// - the tell (the 3 s announcement): the paw's shadow print darkening on
//   the boundary wall where it will come through, three claw slits glowing
//   red-hot, an uneasy rumble and a "mrrp";
// - the paw: a furry ginger mitten with pink toe beans facing the way it
//   swipes, the foreleg reaching back out of the level, claws that spring
//   out for the sweep and retract after;
// - motion blur (fading ghost copies) and speed lines during the swipe, a
//   burst of rock where it breaks through, rock spray and sparks where the
//   claws rake rock, camera shake nearby, a crash and a whoosh.

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { PointLight } from "../../renderer/lights.ts";
import type { WorldVolume } from "../../renderer/wells.ts";
import type { Rgb, SceneInstances } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { DUST, lin, smokeSprite, type VfxSink } from "../vfxLibrary.ts";
import { PAW_TOES } from "../../renderer/models/spectacleProps.ts";
import type { EntityState } from "../views.ts";
import { clamp01, hash01, LoopVoice, phaseSecsLeft, pushInstance, smoothstep, type SpectacleInput, type SpectacleSoundEngine } from "./spectacleCommon.ts";

/** Ginger tabby (display). */
export const PAW_FUR: Rgb = [0.86, 0.5, 0.22];
/** data/events/cat_paw.ron `claw_gap` (u). */
export const CLAW_GAP = 7;
/** Paw phases (state[0]). */
export const PAW_PHASE = { tell: 0, reach: 1, sweep: 2, withdraw: 3 } as const;

/** Axis code `2 * axis + positive` as a unit vector (ls-sim `cat_paw::axis_dir`). */
export function axisDir(code: number): Vec3 {
  const s = code & 1 ? 1 : -1;
  const a = Math.floor(code / 2) % 3;
  return a === 0 ? { x: s, y: 0, z: 0 } : a === 1 ? { x: 0, y: s, z: 0 } : { x: 0, y: 0, z: s };
}

/**
 * The paw's frame for claw mode `mode`: the drawn paw (mesh and rotation:
 * palm toward the sweep, toes toward the claws), the claws' direction, the
 * claw row, the sweep direction, and where the foreleg leaves the wrist.
 */
export function pawFrame(rot: Quat, mode: number): { mesh: "cat_paw" | "cat_paw_up"; paw: Quat; dir: Vec3; spread: Vec3; along: Vec3; arm: Quat; wrist: Vec3 } {
  const along = q.rotate(rot, { x: 1, y: 0, z: 0 });
  const ly = q.rotate(rot, { x: 0, y: 1, z: 0 });
  const lz = q.rotate(rot, { x: 0, y: 0, z: 1 });
  if (mode === 1) return { mesh: "cat_paw_up", paw: rot, dir: ly, spread: lz, along, arm: rot, wrist: lz };
  if (mode === 2) {
    // Raking the wall it came through: toes to the wall, the wrist bent up,
    // the foreleg running diagonally back into the rock.
    const paw = q.mul(rot, q.fromAxisAngle({ x: 1, y: 0, z: 0 }, -Math.PI / 2));
    const armDir = v3.normalize(v3.add(ly, lz));
    return { mesh: "cat_paw", paw, dir: lz, spread: v3.scale(ly, -1), along, arm: q.lookRotation(v3.scale(armDir, -1), along), wrist: ly };
  }
  return { mesh: "cat_paw", paw: rot, dir: v3.scale(ly, -1), spread: lz, along, arm: rot, wrist: lz };
}

/** Claw tips at the pad's rim (ls-sim `Sweep::claws`): centre `p`, pad radius `pad`. */
export function clawTips(p: Vec3, dir: Vec3, spread: Vec3, pad: number, gap = CLAW_GAP): Vec3[] {
  const base = v3.addScaled(p, dir, pad);
  return [-1, 0, 1].map((k) => v3.addScaled(base, spread, k * gap));
}

/** How far the claws are out (0..1) in a phase at a progress (0..1). */
export function clawExtension(phase: number, progress: number): number {
  if (phase === PAW_PHASE.sweep) return smoothstep(0, 0.12, progress);
  if (phase === PAW_PHASE.withdraw) return 1 - smoothstep(0, 0.5, progress);
  return 0;
}

/**
 * Where the tell's shadow lands: from points inside the level along -n
 * (n the face's outward normal), the first ray toward the face that runs
 * through open space and meets rock. Null in open levels (no wall drawn).
 */
export function findWall(pos: Vec3, n: Vec3, pad: number, ray: ((o: Vec3, d: Vec3, max: number) => number) | undefined): Vec3 | null {
  if (!ray) return null;
  for (const k of [pad + 10, pad + 16, pad + 24, pad + 34, pad + 48, pad + 66, pad + 90]) {
    const start = v3.addScaled(pos, n, -k);
    // The start must be in open space: nothing right behind it either.
    const behind = ray(start, v3.scale(n, -1), 1.5);
    if (behind >= 0) continue;
    const t = ray(start, n, k + 4);
    if (t > 1.5) return v3.addScaled(start, n, t);
  }
  return null;
}

interface PawTrack {
  id: number;
  phase: number;
  wall: Vec3 | null;
  wallDone: boolean;
  burst: boolean;
  history: Array<{ pos: Vec3; paw: Quat }>;
  sprayClock: number;
}

export class CatPawFx {
  private paws = new Map<number, PawTrack>();
  private readonly scrape = new LoopVoice("paw.scrape");
  sink: VfxSink | null = null;
  engine: SpectacleSoundEngine | null = null;
  lights: PointLight[] = [];
  /** World effect volumes (renderer/wells.ts): the wall cracking where the paw pushes in the tell. */
  volumes: WorldVolume[] = [];
  shake = 0;
  /** Paws of the last update (for addToScene). */
  private drawn: Array<{ e: EntityState; t: PawTrack }> = [];

  reset(): void {
    this.paws.clear();
    this.drawn = [];
    this.scrape.stop(0.2);
  }

  update(i: SpectacleInput): void {
    this.lights = [];
    this.volumes = [];
    this.shake = 0;
    this.drawn = [];
    let scraping = 0;
    let scrapeAt: Vec3 | null = null;
    const seen = new Set<number>();
    for (const e of i.entities) {
      if (e.kind !== "catPaw") continue;
      seen.add(e.id);
      const pad = e.radius > 0 ? e.radius : 10;
      const [phase, prog, face, mode] = e.state;
      const progress = prog / 255;
      let t = this.paws.get(e.id);
      if (!t) {
        t = { id: e.id, phase: -1, wall: null, wallDone: false, burst: false, history: [], sprayClock: 0 };
        this.paws.set(e.id, t);
      }
      const n = axisDir(face);
      if (!t.wallDone) {
        t.wall = findWall(e.position, n, pad, i.raycast);
        t.wallDone = true;
      }
      const f = pawFrame(e.rotation, mode);
      if (phase !== t.phase) this.enterPhase(t, phase, e, f);
      t.phase = phase;
      t.history.unshift({ pos: e.position, paw: f.paw });
      if (t.history.length > 8) t.history.length = 8;
      this.drawn.push({ e, t });

      if (phase === PAW_PHASE.tell) {
        this.tell(t, e, n, f, pad, i);
        continue;
      }
      // The paw lights itself (a cave is dark; the cat is the show).
      this.lights.push({ position: v3.addScaled(e.position, f.along, pad * 1.4), color: [1, 0.9, 0.78], intensity: 16, range: 34, priority: 3 });
      const d = v3.length(v3.sub(e.position, i.camera));
      this.shake = Math.max(this.shake, (phase === PAW_PHASE.sweep ? 0.9 : 0.5) * clamp01(1 - (d - pad) / 45));
      if (phase === PAW_PHASE.sweep) {
        const hits = this.spray(t, e, f, pad, progress, i);
        if (hits.length > 0) {
          scraping += hits.length;
          scrapeAt = hits[0]!;
        }
        this.speedLines(e, f, pad);
        this.shed(e, pad, i);
      }
    }
    for (const id of [...this.paws.keys()]) if (!seen.has(id)) this.paws.delete(id);
    this.scrape.update(this.engine, scraping > 0 && !i.quiet ? Math.min(1, 0.5 + 0.2 * scraping) : 0, scrapeAt);
  }

  private enterPhase(t: PawTrack, phase: number, e: EntityState, f: ReturnType<typeof pawFrame>): void {
    const where = t.wall ?? e.position;
    if (phase === PAW_PHASE.tell) {
      this.engine?.play("paw.mrrp", { position: where });
      this.engine?.play("paw.rumble", { position: where });
    } else if (phase === PAW_PHASE.reach) {
      this.engine?.play("paw.crash", { position: where });
      if (!t.burst) this.breakThrough(t, e);
    } else if (phase === PAW_PHASE.sweep) {
      this.engine?.play("paw.whoosh", { position: v3.addScaled(e.position, f.along, 10) });
    } else if (phase === PAW_PHASE.withdraw) {
      this.engine?.play("paw.mrrp", { position: e.position, rate: 1.25 });
    }
  }

  /** The tell: a shadow print on the wall, darkening, and three red-hot claw slits. */
  private tell(t: PawTrack, e: EntityState, n: Vec3, f: ReturnType<typeof pawFrame>, pad: number, i: SpectacleInput): void {
    const sink = this.sink;
    if (!t.wall) return;
    const left = phaseSecsLeft(i.mood, i.nowTick);
    const grow = i.mood.eventId === "cat_paw" && i.mood.phase === "announce" ? clamp01(1 - left / 3) : 0.6;
    const inward = v3.scale(n, -1);
    const at = v3.addScaled(t.wall, inward, 0.15);
    // Toes of the print toward the claws, in the wall's plane.
    let toe = v3.sub(f.dir, v3.scale(n, v3.dot(f.dir, n)));
    if (v3.length(toe) < 0.3) toe = f.along;
    toe = v3.normalize(toe);
    const row = v3.normalize(v3.cross(n, toe));
    const knead = 1 + 0.05 * Math.sin(i.time * 5);
    // The paw presses on the rock from outside: its print glows through,
    // hot like a branding iron (pad and three toes), and darkens at its
    // heart as the push grows.
    if (sink) {
      const blob = (p: Vec3, size: number): void => {
        const q0 = v3.addScaled(p, inward, 0.06);
        sink.immediate({ pos: q0, vel: inward, life: 1, sprite: SPRITE.glow, color: lin([1, 0.4, 0.12], 0.8 + 1.6 * grow), size: size * 2.1, blend: "add", orient: "plane" });
        sink.immediate({ pos: q0, vel: inward, life: 1, sprite: SPRITE.glow, color: lin([1, 0.55, 0.2], 3 + 7 * grow), size: size * 0.95, blend: "add", orient: "plane" });
        sink.immediate({ pos: q0, vel: inward, life: 1, sprite: SPRITE.glow, color: lin([1, 0.75, 0.4], 3 + 6 * grow), size: size * 0.55, blend: "add", orient: "plane" });
        sink.immediate({ pos: v3.addScaled(p, inward, 0.1), vel: inward, life: 1, sprite: SPRITE.glow, color: [0.03, 0.01, 0.005], alpha: 0.75 * grow, alpha1: 0.75 * grow, size: size * 0.55, blend: "alpha", orient: "plane" });
      };
      blob(v3.addScaled(at, toe, -pad * 0.15), pad * 1.35 * knead);
      for (const k of [-1, 0, 1]) blob(v3.addScaled(v3.addScaled(at, toe, pad * (k === 0 ? 0.98 : 0.84)), row, k * pad * 0.66), pad * 0.5 * knead);
    }
    if (sink) {
      for (const k of [-1, 0, 1]) {
        const flick = 0.6 + 0.4 * Math.sin(i.time * 23 + k * 2.1) * Math.sin(i.time * 9.3 + k);
        const c = v3.addScaled(v3.addScaled(at, toe, pad * (k === 0 ? 1.3 : 1.16)), row, k * pad * 0.7);
        sink.immediate({ pos: v3.addScaled(c, inward, 0.1), vel: toe, life: 1, sprite: SPRITE.streak, color: lin([1, 0.55, 0.2], 12 * grow * flick), size: 0.7, stretch: pad * 0.32, blend: "add", orient: "fixed" });
        sink.immediate({ pos: v3.addScaled(c, inward, 0.2), vel: inward, life: 1, sprite: SPRITE.glow, color: lin([1, 0.3, 0.05], 1.6 * grow * flick), size: pad * 0.45, blend: "add", orient: "plane" });
      }
      // Grit trickling off the wall.
      if (hash01(i.nowTick, e.id) < 0.7 * grow) {
        const p = v3.addScaled(v3.addScaled(at, toe, (hash01(i.nowTick, 3) - 0.3) * pad * 1.6), row, (hash01(i.nowTick, 4) - 0.5) * pad * 1.6);
        sink.spawn({ pos: p, vel: v3.add(v3.scale(inward, 1.5), v3.scale(i.up, -3)), life: 1.2, sprite: SPRITE.shard, color: [0.3, 0.27, 0.24], alpha: 1, alpha1: 0.2, size: 0.25, drag: 0.3, blend: "alpha", spin: 6 });
      }
    }
    this.lights.push({ position: v3.addScaled(at, inward, pad * 0.9), color: [1, 0.55, 0.25], intensity: 5 + 14 * grow, range: pad * 3, priority: 2 });
    const d = v3.length(v3.sub(at, i.camera));
    this.shake = Math.max(this.shake, 0.18 * grow * clamp01(1 - d / 60));
  }

  /** The paw bursts through the wall: rock chunks, dust and a flash of daylight. */
  private breakThrough(t: PawTrack, e: EntityState): void {
    t.burst = true;
    const sink = this.sink;
    if (!sink || !t.wall) return;
    const n = axisDir(e.state[2]);
    const inward = v3.scale(n, -1);
    for (let k = 0; k < 70 * sink.density; k++) {
      const a = hash01(k, 1) * Math.PI * 2;
      const r = hash01(k, 2);
      const side = v3.add(v3.scale({ x: Math.cos(a), y: Math.sin(a), z: Math.cos(a * 1.7) }, 0.9), inward);
      const dir = v3.normalize(v3.add(v3.scale(inward, 1.4), v3.scale(side, 0.8 * r)));
      sink.spawn({ pos: v3.addScaled(t.wall, inward, 0.5), vel: v3.scale(dir, 8 + 16 * hash01(k, 3)), life: 1.2 + hash01(k, 4), sprite: SPRITE.shard, color: [0.34, 0.31, 0.28], alpha: 1, alpha1: 0.4, size: 0.4 + 0.9 * hash01(k, 5), drag: 0.9, blend: "alpha", spin: 9 * (hash01(k, 6) - 0.5) });
    }
    for (let k = 0; k < 18; k++) {
      const off = v3.scale({ x: hash01(k, 7) - 0.5, y: hash01(k, 8) - 0.5, z: hash01(k, 9) - 0.5 }, 12);
      sink.spawn({ pos: v3.add(t.wall, off), vel: v3.scale(inward, 3 + 4 * hash01(k, 10)), life: 2.6, sprite: smokeSprite(() => hash01(k, 11)), color: DUST, alpha: 0.7, alpha1: 0, size: 5, size1: 11, drag: 0.8, blend: "alpha", spin: 0.4 });
    }
  }

  /** Rock spray and sparks where the claw tips rake rock; returns the scraped points. */
  private spray(t: PawTrack, e: EntityState, f: ReturnType<typeof pawFrame>, pad: number, progress: number, i: SpectacleInput): Vec3[] {
    const out: Vec3[] = [];
    if (!i.raycast || clawExtension(PAW_PHASE.sweep, progress) < 0.5) return out;
    const sink = this.sink;
    t.sprayClock += i.dt * 40 * (sink?.density ?? 1);
    const emit = Math.floor(t.sprayClock);
    t.sprayClock -= emit;
    const back = v3.scale(f.along, -1);
    for (const tip of clawTips(e.position, f.dir, f.spread, pad)) {
      // Rock within reach of the claw: along its pointing direction from just inside the pad.
      const from = v3.addScaled(tip, f.dir, -2);
      const hit = i.raycast(from, f.dir, 5);
      if (hit < 0 || hit > 4.5) continue;
      const p = v3.addScaled(from, f.dir, hit);
      out.push(p);
      if (!sink) continue;
      for (let k = 0; k < Math.max(1, emit); k++) {
        const r1 = hash01(i.nowTick * 3 + k, e.id + out.length);
        const r2 = hash01(i.nowTick * 5 + k, e.id + 7);
        const up = v3.scale(f.dir, -1);
        const dir = v3.normalize(v3.add(v3.add(v3.scale(back, 0.7), v3.scale(up, 1.1 + r1)), v3.scale(f.spread, (r2 - 0.5) * 2.2)));
        sink.spawn({ pos: p, vel: v3.scale(dir, 10 + 14 * r2), life: 0.9 + 0.6 * r1, sprite: SPRITE.shard, color: [0.36, 0.33, 0.3], alpha: 1, alpha1: 0.3, size: 0.25 + 0.5 * r1, drag: 1.2, blend: "alpha", spin: 12 * (r2 - 0.5) });
        sink.spawn({ pos: p, vel: v3.scale(dir, 18 + 10 * r1), life: 0.25 + 0.2 * r2, sprite: SPRITE.spark, color: lin([1, 0.6, 0.25], 8), color1: lin([0.8, 0.15, 0.02], 2), size: 0.12, stretch: 0.04, drag: 2, blend: "add", orient: "velocity" });
      }
      sink.spawn({ pos: p, vel: v3.add(v3.scale(back, 3), v3.scale(f.dir, -2)), life: 1.8, sprite: smokeSprite(() => hash01(i.nowTick, 21)), color: DUST, alpha: 0.55, alpha1: 0, size: 2.5, size1: 6, drag: 1, blend: "alpha", spin: 0.5 });
    }
    return out;
  }

  /** Loose fur: ginger tufts shed by the swipe, drifting slowly down after it. */
  private shed(e: EntityState, pad: number, i: SpectacleInput): void {
    const sink = this.sink;
    if (!sink) return;
    const n = Math.floor(hash01(i.nowTick, 77) * 3 * sink.density + 0.5);
    for (let k = 0; k < n; k++) {
      const r1 = hash01(i.nowTick * 7 + k, 5);
      const r2 = hash01(i.nowTick * 3 + k, 9);
      const r3 = hash01(i.nowTick * 5 + k, 13);
      const off = v3.scale(v3.normalize({ x: r1 - 0.5, y: r2 - 0.5, z: r3 - 0.5 }), pad * 0.95);
      const vel = v3.add(v3.scale(e.velocity, 0.15), v3.add(v3.scale(i.up, -0.5), v3.scale(off, 0.08)));
      const c = lin(PAW_FUR, 0.55);
      sink.spawn({ pos: v3.add(e.position, off), vel, life: 3.5 + 2 * r1, sprite: SPRITE.shard, color: c, alpha: 1, alpha1: 0, size: 0.35 + 0.35 * r2, drag: 1.6, blend: "alpha", spin: 2.5 * (r3 - 0.5), rot: r1 * 6 });
    }
  }

  /** Streaks of air behind the swiping paw. */
  private speedLines(e: EntityState, f: ReturnType<typeof pawFrame>, pad: number): void {
    const sink = this.sink;
    if (!sink) return;
    const speed = v3.length(e.velocity);
    if (speed < 5) return;
    const dir = v3.scale(e.velocity, 1 / speed);
    for (let k = 0; k < 7; k++) {
      const a = hash01(k, 31) * Math.PI * 2;
      const r = pad * (0.75 + 0.35 * hash01(k, 32));
      const off = v3.add(v3.scale(f.spread, Math.cos(a) * r), v3.scale(v3.cross(dir, f.spread), Math.sin(a) * r));
      const p = v3.add(v3.addScaled(e.position, dir, -pad * (0.8 + 0.8 * hash01(k, 33))), off);
      sink.immediate({ pos: p, vel: dir, life: 1, sprite: SPRITE.streak, color: lin([0.85, 0.9, 1.0], 0.9), size: 0.25, stretch: pad * (1.2 + hash01(k, 34)), blend: "add", orient: "fixed" });
    }
  }

  addToScene(scene: SceneInstances): void {
    for (const { e, t } of this.drawn) {
      const pad = e.radius > 0 ? e.radius : 10;
      const [phase, prog, , mode] = e.state;
      const f = pawFrame(e.rotation, mode);
      // In the tell the paw waits outside the wall (only visible in open levels).
      pushInstance(scene, "opaque", f.mesh, { position: e.position, rotation: f.paw, scale: pad, color: PAW_FUR, emissive: 0 });
      pushInstance(scene, "opaque", "paw_arm", { position: v3.addScaled(e.position, f.wrist, pad * 0.5), rotation: f.arm, scale: pad, color: PAW_FUR, emissive: 0 });
      const ext = clawExtension(phase, prog / 255);
      if (ext > 0.01) {
        const rot = q.lookRotation(v3.cross(f.along, f.dir), v3.scale(f.dir, -1));
        const flip = f.mesh === "cat_paw_up" ? -1 : 1;
        for (const toe of PAW_TOES) {
          // Roots in the toes' tips; the claws slide out along their direction.
          const local = { x: toe[0] + 0.14, y: (toe[1] - 0.2 + 0.12 * ext) * flip, z: toe[2] };
          const root = v3.add(e.position, v3.scale(q.rotate(f.paw, local), pad));
          pushInstance(scene, "opaque", "paw_claw", { position: root, rotation: rot, scale: pad * 1.05 * (0.4 + 0.6 * ext), color: [1, 1, 1], emissive: 0 });
        }
      }
      // Motion blur: fading copies along the last frames of the swipe.
      if (phase !== PAW_PHASE.tell && v3.length(e.velocity) > 8) {
        const ghosts = [
          [2, 0.3],
          [4, 0.17],
          [6, 0.08],
        ] as const;
        for (const [k, opacity] of ghosts) {
          const h = t.history[k];
          if (!h) continue;
          pushInstance(scene, "translucent", f.mesh, { position: h.pos, rotation: h.paw, scale: pad * 1.02, color: PAW_FUR, emissive: 0.1, opacity });
        }
      }
    }
  }
}
