// What The Architect looks like in the world (M4 C4): the exit gate, every
// stroke's telegraph (a glowing sphere in the stroke's colour whose ring
// fills as the 2 s run out, pulsing faster, with a beam so it is found from
// far away and sparks drawn in toward its centre), the stroke acting (rock
// dust bursting for the rock strokes, a confetti pop for the event strokes,
// a squid-ink cloud for the Palette blackout, a violet field for the well
// next to B5's well look), and for the architect the placement preview
// (a wire sphere, red when the stroke cannot go there) and a light pillar
// over every runner. Volumes go to the mode effects pass
// (renderer/modeFx.ts), lights to VQ6, particles to VQ5.

import type { Vec3 } from "../../math/vec3.ts";
import type { PointLight } from "../../renderer/lights.ts";
import type { ModeVolume } from "../../renderer/modeFx.ts";
import type { Rgb } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import type { ParticleSpec } from "../../renderer/particles.ts";
import { sideColor } from "../sides.ts";
import { add, count, dist, lin, madd, mul, norm, randDir, range, rockBurst, ring, smokeSprite, sparkleBurst, sparks, type Rng, type VfxSink } from "../vfxLibrary.ts";
import { secsTo, telegraphProgress, type ArchitectEvent, type ArchitectRules, type ArchitectView, type StrokeDef, type StrokeMarker } from "./model.ts";

/** The exit gate's green. */
export const EXIT_COLOR: Rgb = [0.38, 1.0, 0.56];
/** A preview that cannot be placed. */
export const BAD_COLOR: Rgb = [1.0, 0.24, 0.2];
/** Width of the light pillar over a runner (architect only). */
export const RUNNER_PILLAR_WIDTH = 0.16;
/** Volumes the mode effects pass draws at most (shaders/modeFx.ts). */
const MAX_VOLUMES = 12;

/** Where the architect is about to place a stroke. */
export interface StrokePreview {
  def: StrokeDef;
  center: Vec3;
  radius: number;
  /** False: red (the stroke would be refused there). */
  ok: boolean;
}

export interface ArchitectSceneInput {
  view: ArchitectView;
  rules: ArchitectRules;
  nowTick: number;
  /** Animation time (s). */
  time: number;
  /** The local player is the round's architect. */
  amArchitect: boolean;
  preview: StrokePreview | null;
  /** Runners' rendered positions with their side (architect only: their pillars). */
  runners: readonly { id: number; side: number; pos: Vec3 }[];
  /** The overview (map) view is on: a work light over the target. */
  overviewTarget: Vec3 | null;
  camera: Vec3;
}

function zone(center: Vec3, radius: number, color: Rgb, intensity: number, o: Partial<Extract<ModeVolume, { kind: "zone" }>> = {}): ModeVolume {
  return { kind: "zone", center, radius, color, intensity, film: 0.1, grid: 0.2, contour: 1.4, contourWidth: 0.3, ...o };
}

/** The telegraph's look at progress `p` (0 = placed, 1 = acting now). */
export function telegraphVolume(s: StrokeMarker, p: number): ModeVolume {
  const k = Math.max(0, Math.min(1, p));
  return zone(s.pos, s.radius, s.def.color, 1.2 + 1.6 * k, {
    film: 0.1 + 0.18 * k,
    grid: 0.3 + 0.35 * k,
    contour: 1.8 + 1.6 * k,
    contourWidth: 0.4,
    ring: k,
    pulse: 1.2 + 4.8 * k * k,
    dashSpeed: 3 + 6 * k,
  });
}

/** This frame's volumes, most important first (at most the pass's 12). */
export function architectVolumes(i: ArchitectSceneInput): ModeVolume[] {
  const out: ModeVolume[] = [];
  const v = i.view;
  if (i.preview) {
    const c = i.preview.ok ? i.preview.def.color : BAD_COLOR;
    out.push(zone(i.preview.center, i.preview.radius, c, i.preview.ok ? 1.0 : 1.3, { film: 0.04, grid: 0.75, contour: 2.2, contourWidth: 0.25, dashSpeed: 8 }));
  }
  const telegraphs = v.strokes.filter((s) => s.phase === "telegraph");
  for (const s of telegraphs) out.push(telegraphVolume(s, telegraphProgress(s, i.nowTick, i.rules.telegraphSecs)));
  for (const s of v.strokes) {
    if (s.phase !== "active") continue;
    const left = secsTo(s.timerEnd, i.nowTick);
    if (s.def.index === 3) {
      // The temporary well's reach: a faint violet field (B5 draws the well itself).
      out.push(zone(s.pos, s.radius, s.def.color, 0.7, { film: 0.03, grid: 0.12, contour: 0.9, contourWidth: 0.2, dashSpeed: -2, pulse: 0.4 }));
    } else if (s.def.index === 6) {
      // The ink blackout's edge: a thin pale outline (the ink cloud darkens inside).
      out.push(zone(s.pos, s.radius, [0.62, 0.6, 0.85], 0.3 * Math.min(1, left), { film: 0.0, grid: 0.0, contour: 1.0, contourWidth: 0.16, dashSpeed: 0.6 }));
    } else {
      // The rock strokes and events act at once: a flash that fades.
      const age = Math.max(0, 1 - left / 1.0);
      out.push(zone(s.pos, s.radius, s.def.color, 2.6 * (1 - age), { film: 0.25 * (1 - age), grid: 0.4, contour: 2.4, contourWidth: 0.5 }));
    }
  }
  if (v.exit) {
    const gate = v.roundOver ? 0.6 : 1.4;
    out.push(zone(v.exit.pos, v.exit.radius, EXIT_COLOR, gate, { film: 0.2, grid: 0.38, contour: 1.4, contourWidth: 0.35, ring: 1, pulse: 0.6, dashSpeed: 1.5 }));
    out.push({ kind: "beam", center: v.exit.pos, width: 0.9, height: 90, color: EXIT_COLOR, intensity: 1.2 * gate, core: 0.9 });
  }
  for (const s of telegraphs) out.push({ kind: "beam", center: s.pos, width: 0.45, height: 50, color: s.def.color, intensity: 1.0, core: 0.7 });
  if (i.amArchitect) {
    // The pillar starts above the hull, so the ship itself stays visible.
    for (const r of i.runners) out.push({ kind: "beam", center: { x: r.pos.x, y: r.pos.y + 1.2, z: r.pos.z }, width: RUNNER_PILLAR_WIDTH, height: 22, color: sideColor(r.side, 0), intensity: 0.9, core: 1 });
  }
  return out.slice(0, MAX_VOLUMES);
}

/** Lights of the exit, the telegraphs and the overview's work light. */
export function architectLights(i: ArchitectSceneInput, out: PointLight[] = []): PointLight[] {
  const v = i.view;
  if (v.exit) {
    const flick = 0.85 + 0.15 * Math.sin(i.time * 2.3);
    out.push({ position: v.exit.pos, color: EXIT_COLOR, intensity: 14 * flick, range: Math.min(40, v.exit.radius * 3.5), priority: 2 });
  }
  for (const s of v.strokes) {
    if (s.phase !== "telegraph") continue;
    const p = telegraphProgress(s, i.nowTick, i.rules.telegraphSecs);
    const beat = 0.6 + 0.4 * Math.sin(i.time * Math.PI * 2 * (1.2 + 4.8 * p * p));
    out.push({ position: s.pos, color: s.def.color, intensity: (6 + 18 * p) * beat, range: Math.min(40, s.radius * 1.4), priority: 3 });
  }
  if (i.overviewTarget) {
    // A warm work lamp over the cut, so the floor plan reads from up high.
    const t = i.overviewTarget;
    out.push({ position: { x: t.x, y: t.y + 14, z: t.z }, color: [1.0, 0.93, 0.82], intensity: 60, range: 40, priority: 4 });
  }
  return out;
}

/** How many ink puffs fill a blackout sphere (radius 16) per second. */
const INK_RATE = 140;
/** Squid ink: blue-black with a violet sheen. */
const INK: [number, number, number] = [0.006, 0.004, 0.016];
const INK_EDGE: [number, number, number] = [0.05, 0.02, 0.09];

/**
 * Particles of the strokes and the exit: sparks drawn into a telegraph,
 * bursts when a stroke acts, the ink cloud, motes rising in the exit. Call
 * `update` once per frame after `Vfx.update` (immediate sprites).
 */
export class ArchitectFx {
  private readonly sink: VfxSink;
  private readonly rng: Rng;
  private inkCarry = 0;

  constructor(sink: VfxSink, rng: Rng = Math.random) {
    this.sink = sink;
    this.rng = rng;
  }

  update(i: ArchitectSceneInput, events: readonly ArchitectEvent[], dt: number): void {
    const sink = this.sink;
    const rng = this.rng;
    for (const e of events) {
      if (e.kind === "telegraph") this.placed(e.stroke);
      else if (e.kind === "act") this.acted(e.stroke);
      else if (e.kind === "escaped" && i.view.exit) sparkleBurst(sink, rng, i.view.exit.pos, EXIT_COLOR, 60, 9);
    }
    for (const s of i.view.strokes) {
      if (s.phase === "telegraph") {
        // Sparks drawn in from the shell toward the centre, faster as it nears.
        const p = telegraphProgress(s, i.nowTick, i.rules.telegraphSecs);
        const n = count(sink, rng, (14 + 50 * p) * Math.max(0, dt) * 4);
        const c = lin(s.def.color, 6);
        for (let k = 0; k < n; k++) {
          const d = randDir(rng);
          const from = madd(s.pos, d, s.radius * range(rng, 0.85, 1.0));
          const life = range(rng, 0.5, 0.9);
          sink.spawn({ pos: from, vel: mul(d, (-s.radius * 0.8) / life), life, sprite: SPRITE.spark, orient: "velocity", stretch: 0.06, color: c, color1: lin(s.def.color, 0.5), size: 0.16, size1: 0.05, fadeIn: 0.2 });
        }
      } else if (s.def.index === 6) {
        this.ink(s, dt);
        this.squidEyes(s, i);
      }
    }
    const exit = i.view.exit;
    if (exit && !i.view.roundOver) {
      // Motes rising through the gate.
      const n = count(sink, rng, 10 * Math.max(0, dt) * 6);
      for (let k = 0; k < n; k++) {
        const d = randDir(rng);
        const pos = madd(exit.pos, { x: d.x, y: -Math.abs(d.y), z: d.z }, exit.radius * Math.sqrt(rng()));
        sink.spawn({ pos, vel: { x: 0, y: range(rng, 1.5, 4), z: 0 }, life: range(rng, 1.2, 2.2), sprite: SPRITE.glow, color: lin(EXIT_COLOR, 2.2), color1: lin(EXIT_COLOR, 0), size: 0.25, size1: 0.1, fadeIn: 0.3 });
      }
    }
  }

  /** A stroke appeared: a ring and a puff of its colour at the centre. */
  private placed(s: StrokeMarker): void {
    ring(this.sink, s.pos, s.def.color, 6, 1, s.radius * 2, 0.6);
    sparkleBurst(this.sink, this.rng, s.pos, s.def.color, 30, 6);
  }

  /** A stroke acts: rock dust for rock strokes, confetti for events, a violet thump for the well. */
  private acted(s: StrokeMarker): void {
    const sink = this.sink;
    const rng = this.rng;
    ring(sink, s.pos, s.def.color, 8, s.radius * 0.4, s.radius * 2.4, 0.7);
    if (s.def.changesRock) {
      const n = count(sink, rng, 12);
      for (let k = 0; k < n; k++) {
        const at = madd(s.pos, randDir(rng), s.radius * Math.cbrt(rng()) * 0.9);
        const t0 = sink.now + range(rng, 0, 0.14);
        rockBurst(sink, rng, at, null, range(rng, 1.5, 3.2), t0);
        // Pale stone dust that hangs in the air for a few seconds (the aftermath).
        for (let j = 0; j < 3; j++) {
          const d = randDir(rng);
          sink.spawn({ pos: madd(at, d, 1.2), vel: mul(d, range(rng, 0.6, 2.2)), t0, life: range(rng, 3.2, 5.2), drag: 0.9, sprite: smokeSprite(rng), blend: "alpha", color: STONE_DUST, color1: STONE_DUST_DARK, alpha: 0.55, alpha1: 0, size: range(rng, 1.4, 2.6), size1: range(rng, 3.5, 5.5), spin: range(rng, -0.6, 0.6), rot: rng() * 6.28 });
        }
        // Hot chips flung out where the rock grows.
        sparks(sink, rng, at, randDir(rng), lin(s.def.color, 1), 6, 1.2, 2.2, t0);
      }
    } else if (s.def.levelWide) {
      // Confetti: every colour, tumbling down slowly.
      const n = count(sink, rng, 140);
      for (let k = 0; k < n; k++) {
        const d = randDir(rng);
        const hue = rng();
        const c: Rgb = hueRgb(hue);
        sink.spawn({ pos: madd(s.pos, d, 1), vel: add(mul(d, range(rng, 6, 16)), { x: 0, y: 4, z: 0 }), life: range(rng, 1.6, 2.8), drag: 1.4, sprite: SPRITE.shard, color: lin(c, 3), size: 0.35, size1: 0.3, spin: range(rng, -9, 9), rot: rng() * 6.28 });
      }
    } else if (s.def.index === 3) {
      sparkleBurst(sink, rng, s.pos, s.def.color, 80, 14, true);
    }
  }

  private squidEyes(s: StrokeMarker, i: ArchitectSceneInput): void {
    // Once the ink is thick (after the splat), gone again before it thins.
    const left = secsTo(s.timerEnd, i.nowTick);
    if (i.rules.paletteSecs - left < 0.8 || left < 1.2) return;
    for (const p of squidEyeSprites(s.pos, i.camera, i.time)) this.sink.immediate(p);
  }

  /** The Palette blackout: puffs of squid ink filling the sphere, slowly churning. */
  private ink(s: StrokeMarker, dt: number): void {
    const sink = this.sink;
    const rng = this.rng;
    this.inkCarry += INK_RATE * sink.density * Math.max(0, dt) * Math.max(0.5, s.radius / 16);
    while (this.inkCarry >= 1) {
      this.inkCarry -= 1;
      // Thickest at the centre (the architect places it in the open; the shell often cuts rock).
      const k = rng();
      const at = madd(s.pos, randDir(rng), s.radius * 0.7 * k * k);
      const drift = mul(norm({ x: at.z - s.pos.z, y: 0, z: s.pos.x - at.x }), 0.8);
      sink.spawn({
        pos: at,
        vel: drift,
        life: range(rng, 2.6, 3.8),
        sprite: smokeSprite(rng),
        blend: "alpha",
        color: INK,
        color1: INK_EDGE,
        alpha: 1.6,
        alpha1: 0.2,
        size: range(rng, 2.6, 4.4),
        size1: range(rng, 5, 7.5),
        fadeIn: 0.2,
        spin: range(rng, -0.5, 0.5),
        rot: rng() * 6.28,
      });
    }
  }
}

/** The pale stone dust a rock stroke leaves hanging (alpha-blended albedo). */
const STONE_DUST: [number, number, number] = [0.32, 0.27, 0.21];
const STONE_DUST_DARK: [number, number, number] = [0.16, 0.13, 0.1];

/**
 * Two cartoon eyes peering out of the ink at the viewer, blinking now and
 * then: the squid is still in there.
 */
export function squidEyeSprites(center: Vec3, camera: Vec3, time: number): ParticleSpec[] {
  const to = norm({ x: camera.x - center.x, y: camera.y - center.y, z: camera.z - center.z });
  const right = norm({ x: to.z, y: 0, z: -to.x }, { x: 1, y: 0, z: 0 });
  // Blink 0.14 s every 3.1 s; the eyes drift a little.
  if (time % 3.1 < 0.14) return [];
  const bob = { x: 0, y: 0.25 * Math.sin(time * 1.3), z: 0 };
  const front = madd(add(center, bob), to, 1.5);
  const out: ParticleSpec[] = [];
  for (const side of [-1, 1]) {
    const eye = madd(front, right, side * 0.75);
    // Opacity far above 1 turns the soft glow sprite into a crisp cartoon disc.
    out.push({ pos: eye, life: 1, sprite: SPRITE.glow, blend: "alpha", color: [1.5, 1.5, 1.4], alpha: 7, alpha1: 7, size: 1.15 });
    // The pupils look at the viewer, a little cross-eyed.
    const pupil = madd(madd(eye, to, 0.3), right, -side * 0.13);
    out.push({ pos: pupil, life: 1, sprite: SPRITE.glow, blend: "alpha", color: [0, 0, 0], alpha: 9, alpha1: 9, size: 0.5 });
  }
  return out;
}

/** A saturated colour of hue 0..1. */
function hueRgb(h: number): Rgb {
  const k = (n: number): number => {
    const x = (n + h * 6) % 6;
    return Math.max(0, Math.min(1, Math.abs(x - 3) - 1));
  };
  return [k(5), k(3), k(1)];
}

/** Distance from the camera to a stroke's shell (negative inside). */
export function shellDistance(camera: Vec3, s: { pos: Vec3; radius: number }): number {
  return dist(camera, s.pos) - s.radius;
}
