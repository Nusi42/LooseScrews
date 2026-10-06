// Disco inferno (M4 C3; milestone-4-plan.md §3.2): the lights go down to
// 8 %, a mirror ball descends over the players and every ship leaves a light
// trail in its side's colour (sim: B3's events/disco_inferno.rs, one
// `ReplicatedKind::DiscoBall` prop spinning about the level's up, state
// [light percent, descent 0-255]). The client draws:
//
// - the level's light (RenderSettings.lightLevel: sun, ambient, sky and
//   haze) eased to the ball's light percent;
// - the mirror ball (spectacleProps.ts, mirror-tile material) on a chain up
//   to the ceiling;
// - eight coloured spot beams sweeping with the spin: a visible shaft
//   through the dusty dark and a coloured pool of light where each meets
//   rock (a VQ6 point light just off the rock);
// - the ball's reflections: a few dozen white specks wheeling over the rock;
// - light trails: glowing ribbons behind every ship (opaque emissive
//   segments with the keep mask, so they stay coloured in ink levels) and a
//   light riding each ship;
// - a four-on-the-floor loop on the music bus.

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { PointLight } from "../../renderer/lights.ts";
import type { Rgb, SceneInstances } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { lin, type VfxSink } from "../vfxLibrary.ts";
import type { EntityState } from "../views.ts";
import { approach, clamp01, LoopVoice, pushInstance, type SpectacleInput, type SpectacleSoundEngine } from "./spectacleCommon.ts";

/** The eight spot colours (display). */
export const DISCO_COLORS: readonly Rgb[] = [
  [1.0, 0.15, 0.55],
  [0.1, 0.85, 1.0],
  [1.0, 0.85, 0.1],
  [0.35, 1.0, 0.3],
  [1.0, 0.45, 0.05],
  [0.35, 0.35, 1.0],
  [0.95, 0.25, 1.0],
  [1.0, 1.0, 1.0],
];
/** Beam reach (u). */
export const BEAM_RANGE = 70;
/** Reflection specks. */
export const SPECKS = 36;
/** Seconds of trail behind a ship. */
export const TRAIL_SECS = 1.6;
/** Trail sample spacing (s). */
const TRAIL_STEP = 1 / 30;

/** Ball-local direction of beam `k` (0-7): around the up axis, tilted down by turns. */
export function beamDirection(k: number, time: number): Vec3 {
  const az = (k / 8) * Math.PI * 2;
  // Each beam nods up and down at its own pace, so the pools wander over the walls and floor.
  const el = -0.35 - 0.45 * (0.5 + 0.5 * Math.sin(time * (0.7 + 0.13 * k) + k * 1.9));
  const c = Math.cos(el);
  return { x: Math.cos(az) * c, y: Math.sin(el), z: Math.sin(az) * c };
}

/** Ball-local directions of the reflection specks (fixed, spread over the sphere's lower two thirds). */
export function speckDirections(n = SPECKS): Vec3[] {
  const out: Vec3[] = [];
  const golden = Math.PI * (3 - Math.sqrt(5));
  for (let i = 0; i < n; i++) {
    const y = 0.35 - ((i + 0.5) / n) * 1.3;
    const r = Math.sqrt(Math.max(0, 1 - y * y));
    out.push({ x: Math.cos(i * golden) * r, y, z: Math.sin(i * golden) * r });
  }
  return out;
}

interface Trail {
  points: Array<{ p: Vec3; t: number }>;
  color: Rgb;
  last: number;
}

export class DiscoInfernoFx {
  private level = 1;
  private specks = speckDirections();
  private speckDist: number[] = new Array<number>(SPECKS).fill(-1);
  private speckCursor = 0;
  private trails = new Map<number, Trail>();
  private readonly music = new LoopVoice("disco.loop");
  private ball: EntityState | null = null;
  private beams: Array<{ from: Vec3; dir: Vec3; len: number; color: Rgb; hit: boolean }> = [];
  private chainLen = 30;
  private announced = false;
  sink: VfxSink | null = null;
  engine: SpectacleSoundEngine | null = null;
  lights: PointLight[] = [];
  /** Sun and ambient factor for the renderer (1 = normal). */
  get lightLevel(): number {
    return this.level;
  }

  reset(): void {
    this.level = 1;
    this.trails.clear();
    this.speckDist.fill(-1);
    this.ball = null;
    this.beams = [];
    this.announced = false;
    this.music.stop(0.2);
  }

  update(i: SpectacleInput): void {
    this.lights = [];
    this.beams = [];
    const ball = i.entities.find((e) => e.kind === "discoBall") ?? null;
    const running = i.mood.eventId === "disco_inferno" && (i.mood.phase === "active" || i.mood.phase === "announce");
    if (ball && !this.ball) {
      this.engine?.play("disco.drop", { position: ball.position });
      this.speckDist.fill(-1);
    }
    this.ball = ball;
    // The light follows the ball's percent (eased; the sim's step is instant).
    const want = ball ? clamp01(ball.state[0] / 100) : 1;
    this.level = approach(this.level, want, i.dt, want < this.level ? 0.35 : 0.6);
    if (i.mood.eventId === "disco_inferno" && i.mood.phase === "announce" && !this.announced) {
      this.announced = true;
      this.engine?.play("disco.scratch", {});
    }
    if (i.mood.eventId !== "disco_inferno") this.announced = false;
    this.music.update(this.engine, ball && !i.quiet ? 0.9 : 0, null);

    const dark = 1 - this.level;
    if (ball) this.spots(ball, i, dark);
    this.updateTrails(i, ball !== null || running);
  }

  private spots(ball: EntityState, i: SpectacleInput, dark: number): void {
    const sink = this.sink;
    const centre = ball.position;
    const r = ball.radius > 0 ? ball.radius : 1.5;
    // The chain to the ceiling (once a second is plenty).
    if (i.raycast && (i.nowTick % 30 === 0 || this.chainLen <= 0)) {
      const t = i.raycast(v3.addScaled(centre, i.up, r * 1.1), i.up, 60);
      this.chainLen = t >= 0 ? t : 45;
    }
    const descent = ball.state[1] / 255;
    const on = clamp01(descent * 1.4 - 0.2) * clamp01(dark * 1.6);
    for (let k = 0; k < 8; k++) {
      const local = beamDirection(k, i.time);
      const dir = v3.normalize(q.rotate(ball.rotation, local));
      const from = v3.addScaled(centre, dir, r * 1.05);
      const t = i.raycast ? i.raycast(from, dir, BEAM_RANGE) : -1;
      const len = t >= 0 ? t : BEAM_RANGE;
      const color = DISCO_COLORS[k]!;
      this.beams.push({ from, dir, len, color, hit: t >= 0 });
      if (on <= 0.01) continue;
      if (sink) {
        // The shaft: a cone of light through the dusty dark, widening with distance, in sections.
        const sections = 4;
        for (let s = 0; s < sections; s++) {
          const a = s / sections;
          const b = (s + 1) / sections;
          const mid = v3.addScaled(from, dir, len * (a + b) * 0.5);
          const width = 0.25 + len * 0.09 * (a + b) * 0.5;
          sink.immediate({ pos: mid, vel: dir, life: 1, sprite: SPRITE.streak, color: lin(color, 0.6 * on * (1 - 0.35 * a)), size: width, stretch: len / sections + width * 0.6, blend: "add", orient: "fixed" });
        }
        if (t >= 0) {
          const at = v3.addScaled(from, dir, len - 0.3);
          sink.immediate({ pos: at, life: 1, sprite: SPRITE.glow, color: lin(color, 1.6 * on), size: 1.4 + len * 0.09, blend: "add" });
        }
      }
      if (t >= 0) this.lights.push({ position: v3.addScaled(from, dir, Math.max(0.5, len - 1.6)), color, intensity: 12 * on, range: 6 + len * 0.06, priority: 2 });
    }
    // The ball has its own pin spot (as in a real club): its tiles flash in it.
    const toCam = v3.normalize(v3.sub(i.camera, centre));
    this.lights.push({ position: v3.add(v3.addScaled(centre, toCam, r * 2.2), v3.scale(i.up, -r)), color: [1, 0.95, 0.9], intensity: 7 * on + 1.5, range: r * 3.2, priority: 3 });
    // Reflection specks wheeling over the rock: a few rays per frame, round robin.
    const per = 12;
    for (let n = 0; n < per && i.raycast; n++) {
      const k = this.speckCursor;
      this.speckCursor = (this.speckCursor + 1) % SPECKS;
      const dir = v3.normalize(q.rotate(ball.rotation, this.specks[k]!));
      this.speckDist[k] = i.raycast(centre, dir, BEAM_RANGE);
    }
    if (!sink) return;
    for (let k = 0; k < SPECKS; k++) {
      const d = this.speckDist[k] ?? -1;
      if (d < 0) continue;
      const dir = v3.normalize(q.rotate(ball.rotation, this.specks[k]!));
      const twinkle = 0.65 + 0.35 * Math.sin(i.time * 11 + k * 1.7);
      const p = v3.addScaled(centre, dir, d - 0.2);
      sink.immediate({ pos: p, life: 1, sprite: SPRITE.flare, color: lin(k % 5 === 0 ? DISCO_COLORS[k % 8]! : [1, 1, 1], 6 * on * twinkle), size: 0.35 + 0.012 * d, blend: "add", rot: k * 0.7 });
    }
  }

  private updateTrails(i: SpectacleInput, on: boolean): void {
    const ships: Array<{ id: number; p: Vec3; alive: boolean }> = i.ships.map((s) => ({ id: s.id, p: s.position, alive: s.alive }));
    if (i.own) ships.push({ id: i.own.id, p: i.own.position, alive: true });
    for (const s of ships) {
      let t = this.trails.get(s.id);
      if (!t) {
        if (!on) continue;
        t = { points: [], color: i.colorOf(s.id), last: -1 };
        this.trails.set(s.id, t);
      }
      t.color = i.colorOf(s.id);
      if (on && s.alive && i.time - t.last >= TRAIL_STEP) {
        // A jump (respawn, teleport) starts a new trail.
        const prev = t.points[0];
        if (prev && v3.length(v3.sub(prev.p, s.p)) > 12) t.points = [];
        t.points.unshift({ p: s.p, t: i.time });
        t.last = i.time;
      }
      while (t.points.length > 0 && i.time - t.points[t.points.length - 1]!.t > TRAIL_SECS) t.points.pop();
      if (on && s.alive) this.lights.push({ position: s.p, color: t.color, intensity: 5, range: 8 });
    }
    for (const [id, t] of this.trails) if (t.points.length === 0 && !on) this.trails.delete(id);
  }

  addToScene(scene: SceneInstances, time: number, up: Vec3): void {
    const ball = this.ball;
    if (ball) {
      const r = ball.radius > 0 ? ball.radius : 1.5;
      pushInstance(scene, "opaque", "disco_ball", { position: ball.position, rotation: ball.rotation, scale: r, color: [0.8, 0.8, 0.85], emissive: 0 });
      const top = v3.addScaled(ball.position, up, r * 1.2);
      const len = Math.max(1, this.chainLen);
      pushInstance(scene, "opaque", "disco_chain", { position: top, rotation: q.lookRotation(v3.scale(up, -1), perpendicularTo(up)), scale: r, stretch: len / r, color: [0.6, 0.6, 0.6], emissive: 0 });
    }
    for (const t of this.trails.values()) {
      const pts = t.points;
      for (let k = 0; k + 1 < pts.length; k++) {
        const a = pts[k]!;
        const b = pts[k + 1]!;
        const d = v3.sub(a.p, b.p);
        const len = v3.length(d);
        if (len < 0.02) continue;
        const age = time - a.t;
        const w = 0.32 * (1 - clamp01(age / TRAIL_SECS)) + 0.04;
        const rot: Quat = q.lookRotation(v3.scale(d, -1 / len), perpendicularTo(d));
        pushInstance(scene, "opaque", "light_trail", { position: v3.lerp(a.p, b.p, 0.5), rotation: rot, scale: w, stretch: (len + w) / w, color: t.color, emissive: 1 - 0.6 * clamp01(age / TRAIL_SECS), keepColor: 1 });
      }
    }
  }

  /** The beams of the last update (tests, gallery). */
  get lastBeams(): ReadonlyArray<{ from: Vec3; dir: Vec3; len: number; color: Rgb; hit: boolean }> {
    return this.beams;
  }
}

function perpendicularTo(d: Vec3): Vec3 {
  const n = v3.normalize(d);
  return Math.abs(n.y) > 0.9 ? { x: 1, y: 0, z: 0 } : { x: 0, y: 1, z: 0 };
}
