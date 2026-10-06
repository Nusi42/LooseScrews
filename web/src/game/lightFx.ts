// Dynamic lights from the game (VQ6): what gives off light this frame and
// for how long. The renderer draws them (renderer/lights.ts, at most
// RenderQuality.maxLights, best first); this file decides where they are.
//
// Two kinds:
// - Attached lights, rebuilt every frame from the state (`sceneLights`):
//   glowing projectiles by shape (SHAPE_LIGHTS), ship engines (orange idle,
//   blue-white boost; none while cloaked), pickups, wreck embers, Leech Arc
//   beams, the mechanics flashes (muzzle flashes, shockwaves, swaps, ...)
//   and mode objectives (reactor, core, extraction).
// - Timed pulses (`LightFx`): `emit(preset or pulse, position, now)` starts
//   a light with an attack, a decay curve and a lifetime; `frame(now)`
//   returns the live ones. `observe(shots, ...)` emits a muzzle pulse for a
//   new projectile and an impact or explosion pulse for one that vanished
//   (by shape), so every weapon lights the cave with no per-weapon code.
//
// The emitter API for effect systems (VQ5 particles, C1 mode visuals): call
// `lightFx.emit("explosion", position, now)` (or a LightPulse of your own)
// next to the particle burst. If an effect system emits impacts itself, set
// `lightFx.watchProjectiles = false` so they are not doubled.
//
// Cosmetic only (may use Math.random); nothing here feeds back into the sim.

import * as q from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { PointLight } from "../renderer/lights.ts";
import type { Rgb } from "../renderer/renderer.ts";
import type { Beam, Flash } from "./mechanics.ts";
import { OBJECTIVE, type Objective } from "./modeState.ts";
import { PROJ_ARMED, PROJ_CUSTOM_SHIFT, PROJ_STUCK, TINKER_RADIUS, type EntityState, type ProjectileState, type ShipState, type VisualDef } from "./views.ts";

/** Projectile state bit of a landed Paperweight (game/scene.ts PAPERWEIGHT_LANDED). */
const PROJ_PAPERWEIGHT_LANDED = 1 << PROJ_CUSTOM_SHIFT;

/** A timed light: rises over `attack`, then decays to 0 at `duration` (s). */
export interface LightPulse {
  /** Display (sRGB) colour. */
  color: Rgb;
  /** Peak HDR intensity (renderer/lights.ts PointLight.intensity). */
  intensity: number;
  /** Range at the peak (u); default from the intensity. */
  range?: number;
  /** Lifetime (s). */
  duration: number;
  /** Rise time (s), default 0. */
  attack?: number;
  /**
   * Decay: "fade" (smooth, quadratic), "flash" (bright and quick, then a long
   * tail: fireballs), "flicker" (fade with random flicker: fires, sparks).
   */
  decay?: "fade" | "flash" | "flicker";
  /** Range factor at the end of the pulse (expanding fireballs), default 1. */
  grow?: number;
  /** Colour at the end of the pulse (cooling fireballs), default `color`. */
  endColor?: Rgb;
}

/** Pulse presets by name (weapons and effects refer to these). */
export const LIGHT_PRESETS = {
  /** Gun muzzle flash (white-gold; tinted by the shot colour when emitted by `observe`). */
  muzzle: { color: [1, 0.86, 0.6], intensity: 12, duration: 0.09, decay: "flash" },
  /** Bolt or pellet hitting something. */
  spark: { color: [1, 0.8, 0.5], intensity: 8, duration: 0.16, decay: "flicker" },
  /** Plasma orb, arc or blob splash. */
  plasma: { color: [0.5, 0.8, 1], intensity: 18, duration: 0.3, decay: "flash", grow: 1.3 },
  /** Rocket, missile or grenade explosion: white-hot flash cooling to orange. */
  explosion: {
    color: [1, 0.85, 0.6],
    endColor: [1, 0.4, 0.1],
    intensity: 90,
    range: 26,
    duration: 0.75,
    attack: 0.02,
    decay: "flash",
    grow: 1.4,
  },
  /** Mines and charges: a bigger blast. */
  bigExplosion: {
    color: [1, 0.9, 0.7],
    endColor: [1, 0.35, 0.08],
    intensity: 160,
    range: 36,
    duration: 1.2,
    attack: 0.03,
    decay: "flash",
    grow: 1.3,
  },
  /** Ship destroyed. */
  shipDeath: {
    color: [1, 0.8, 0.5],
    endColor: [1, 0.3, 0.05],
    intensity: 130,
    range: 32,
    duration: 1.4,
    attack: 0.03,
    decay: "flicker",
    grow: 1.2,
  },
  /** Pickup collected. */
  pickup: { color: [0.7, 1, 0.8], intensity: 8, duration: 0.35, decay: "fade" },
} as const satisfies Record<string, LightPulse>;

export type LightPresetName = keyof typeof LIGHT_PRESETS;

/** Light a projectile shape carries, and the pulse when it hits (or vanishes). */
export interface ShapeLight {
  /** Carried light (0 = none); colour = the visual's colour unless `color`. */
  intensity: number;
  range?: number;
  /** Fixed colour (rocket motors burn orange whatever the visual colour). */
  color?: Rgb;
  /** Light sits this far behind the head along the flight (u, rockets' motors). */
  behind?: number;
  /** Pulse when the projectile vanishes; null = none. */
  impact: LightPresetName | null;
  /** Tint the impact pulse with the visual's colour. */
  tintImpact?: boolean;
  /** Muzzle pulse when it appears (guns; launchers flash bigger). */
  muzzle?: number;
}

/** By visual shape name (ls-sim `visual::Shape`, as game/scene.ts SHAPE_STYLES). */
export const SHAPE_LIGHTS: Readonly<Record<string, ShapeLight>> = {
  bolt: { intensity: 7, range: 9, impact: "spark", tintImpact: true, muzzle: 1 },
  pellet: { intensity: 2.5, range: 5, impact: "spark", tintImpact: true, muzzle: 0.8 },
  spike: { intensity: 8, range: 10, impact: "spark", tintImpact: true, muzzle: 1.4 },
  rocket: { intensity: 9, range: 11, color: [1, 0.6, 0.25], behind: 0.6, impact: "explosion", muzzle: 1.2 },
  missile: { intensity: 8, range: 10, color: [1, 0.6, 0.25], behind: 0.5, impact: "explosion", muzzle: 1 },
  mine: { intensity: 0, impact: "bigExplosion" },
  orb: { intensity: 12, range: 11, impact: "plasma", tintImpact: true, muzzle: 1 },
  blob: { intensity: 6, range: 8, impact: "plasma", tintImpact: true, muzzle: 0.8 },
  arc: { intensity: 12, range: 11, impact: "plasma", tintImpact: true, muzzle: 1 },
  grenade: { intensity: 1.5, range: 6, color: [1, 0.5, 0.2], impact: "explosion", muzzle: 0.8 },
  harpoon: { intensity: 0, impact: null, muzzle: 0.6 },
  paint: { intensity: 4, range: 7, impact: "spark", tintImpact: true, muzzle: 0.6 },
  charge: { intensity: 14, range: 17, impact: "bigExplosion", muzzle: 1.2 },
  // M4 B7: a faint glow in flight, a pulsing violet lamp once landed (sceneLights).
  paperweight: { intensity: 2, range: 6, impact: null, muzzle: 0.4 },
};

/** A landed Paperweight (a gravity well) glows this bright, pulsing. */
export const PAPERWEIGHT_GLOW = 6;

/** Armed mines blink this bright (red-orange). */
export const MINE_BLINK = 4;

/** Engine light: idle and full boost. */
export const ENGINE_LIGHT = {
  idle: { color: [1, 0.55, 0.25] as Rgb, intensity: 4, range: 10 },
  boost: { color: [0.6, 0.78, 1] as Rgb, intensity: 12, range: 16 },
} as const;

/** A live pulse. */
interface LivePulse {
  pulse: LightPulse;
  position: Vec3;
  start: number;
  scale: number;
  seed: number;
}

const mixRgb = (a: Rgb, b: Rgb, t: number): Rgb => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];

/** Brightness 0..1 of a pulse `age` seconds in (0 before and after its life); `noise` 0..1 for flicker. */
export function pulseLevel(p: LightPulse, age: number, noise = 0.5): number {
  if (age < 0 || age >= p.duration) return 0;
  const attack = Math.min(p.attack ?? 0, p.duration * 0.5);
  if (age < attack) return age / attack;
  const t = (age - attack) / Math.max(1e-6, p.duration - attack);
  switch (p.decay ?? "fade") {
    case "flash":
      // Fast initial drop, long glowing tail, reaching 0 at the end.
      return Math.exp(-5 * t) * (1 - t);
    case "flicker":
      return (1 - t) * (1 - t) * (0.65 + 0.7 * noise);
    default:
      return (1 - t) * (1 - t);
  }
}

/** Light of a pulse at `age` (null when dark). */
export function pulseLight(p: LightPulse, position: Vec3, age: number, scale = 1, noise = 0.5): PointLight | null {
  const level = pulseLevel(p, age, noise);
  if (level <= 0.001) return null;
  const t = Math.min(1, Math.max(0, age / p.duration));
  const grow = 1 + ((p.grow ?? 1) - 1) * t;
  const color = p.endColor ? mixRgb(p.color, p.endColor, Math.min(1, t * 1.6)) : p.color;
  const light: PointLight = { position, color, intensity: p.intensity * scale * level };
  if (p.range !== undefined) light.range = p.range * grow * Math.sqrt(scale);
  else if (grow !== 1) light.range = Math.min(40, Math.sqrt((p.intensity * scale) / 0.02)) * grow;
  return light;
}

/** Previous-frame projectile for vanish / appear detection. */
interface TrackedShot {
  owner: number;
  visual: number;
  position: Vec3;
  velocity: Vec3;
}

/** Most pulses alive at once (oldest dropped first). */
export const MAX_PULSES = 96;

/** Timed lights: the emitter API (see the file comment). */
export class LightFx {
  private pulses: LivePulse[] = [];
  private prevShots: TrackedShot[] = [];
  private lastObserve: number | null = null;
  private aliveShips = new Set<number>();
  /** Emit muzzle and impact pulses from projectile appear / vanish (`observe`). */
  watchProjectiles = true;
  /** Random source for flicker (tests pass a fixed one). */
  random: () => number = Math.random;

  /** Starts a pulse (a preset name or a pulse of your own) at `position`; `scale` multiplies the intensity. */
  emit(pulse: LightPresetName | LightPulse, position: Vec3, now: number, options: { color?: Rgb; scale?: number } = {}): void {
    let p: LightPulse = typeof pulse === "string" ? LIGHT_PRESETS[pulse] : pulse;
    if (options.color) p = { ...p, color: options.color, endColor: p.endColor ? mixRgb(options.color, p.endColor, 0.5) : options.color };
    this.pulses.push({ pulse: p, position, start: now, scale: options.scale ?? 1, seed: this.random() * 100 });
    if (this.pulses.length > MAX_PULSES) this.pulses.splice(0, this.pulses.length - MAX_PULSES);
  }

  /** Number of live pulses (tests, debug). */
  get live(): number {
    return this.pulses.length;
  }

  /** The live pulses' lights at `now` (expired pulses are dropped). */
  frame(now: number, out: PointLight[] = []): PointLight[] {
    this.pulses = this.pulses.filter((p) => now - p.start < p.pulse.duration);
    for (const p of this.pulses) {
      const noise = p.pulse.decay === "flicker" ? this.random() : 0.5;
      const light = pulseLight(p.pulse, p.position, now - p.start, p.scale, noise);
      if (light) out.push(light);
    }
    return out;
  }

  /**
   * Muzzle pulses for projectiles that appeared and impact pulses for ones
   * that vanished since the last call (by SHAPE_LIGHTS). A projectile counts
   * as the same one when owner and visual match and it is where its last
   * velocity puts it. After a gap of more than 0.5 s nothing is emitted.
   */
  observe(shots: readonly ProjectileState[], visuals: readonly VisualDef[], now: number): void {
    const dt = this.lastObserve === null ? -1 : now - this.lastObserve;
    this.lastObserve = now;
    const prev = this.prevShots;
    this.prevShots = shots.map((s) => ({ owner: s.owner, visual: s.visual, position: s.position, velocity: s.velocity }));
    if (!this.watchProjectiles || dt < 0 || dt > 0.5) return;
    const matched = new Uint8Array(shots.length);
    for (const p of prev) {
      const px = p.position.x + p.velocity.x * dt, py = p.position.y + p.velocity.y * dt, pz = p.position.z + p.velocity.z * dt;
      const speed = Math.hypot(p.velocity.x, p.velocity.y, p.velocity.z);
      const tol = 1.5 + speed * dt * 0.5;
      let best = -1;
      let bestD = tol * tol;
      shots.forEach((s, i) => {
        if (matched[i] || s.owner !== p.owner || s.visual !== p.visual) return;
        const d2 = (s.position.x - px) ** 2 + (s.position.y - py) ** 2 + (s.position.z - pz) ** 2;
        if (d2 <= bestD) {
          bestD = d2;
          best = i;
        }
      });
      if (best >= 0) {
        matched[best] = 1;
        continue;
      }
      const visual = visuals[p.visual];
      const shape = visual ? SHAPE_LIGHTS[visual.shape] : undefined;
      if (!shape?.impact) continue;
      // It hit something between its last position and where it would be now.
      const at = { x: p.position.x + p.velocity.x * dt * 0.5, y: p.position.y + p.velocity.y * dt * 0.5, z: p.position.z + p.velocity.z * dt * 0.5 };
      const scale = Math.max(0.6, Math.min(2, visual?.scale ?? 1));
      this.emit(shape.impact, at, now, { scale, ...(shape.tintImpact && visual ? { color: brighten(visual.color) } : {}) });
    }
    shots.forEach((s, i) => {
      if (matched[i]) return;
      const visual = visuals[s.visual];
      const shape = visual ? SHAPE_LIGHTS[visual.shape] : undefined;
      if (!shape?.muzzle || (s.state & PROJ_STUCK) !== 0) return;
      this.emit("muzzle", s.position, now, { scale: shape.muzzle, color: mixRgb(LIGHT_PRESETS.muzzle.color, brighten(visual!.color), 0.5) });
    });
  }

  /** A shipDeath pulse where a ship that was alive in the last call is now dead. */
  observeShips(ships: readonly ShipState[], now: number): void {
    const alive = new Set<number>();
    for (const s of ships) {
      if (s.alive) alive.add(s.id);
      else if (this.aliveShips.has(s.id)) this.emit("shipDeath", s.position, now, { scale: (s.radius || TINKER_RADIUS) / TINKER_RADIUS });
    }
    this.aliveShips = alive;
  }

  /** Forgets every pulse and tracked projectile (new match). */
  clear(): void {
    this.pulses = [];
    this.prevShots = [];
    this.lastObserve = null;
    this.aliveShips = new Set();
  }
}

/** A colour pushed toward full brightness (light colours read better saturated and bright). */
export function brighten(c: Rgb): Rgb {
  const m = Math.max(c[0], c[1], c[2], 1e-3);
  return [c[0] / m, c[1] / m, c[2] / m];
}

/** What `sceneLights` reads (the same state game/scene.ts draws). */
export interface SceneLightInput {
  ships?: readonly ShipState[];
  shots?: readonly ProjectileState[];
  entities?: readonly EntityState[];
  visuals?: readonly VisualDef[];
  flashes?: readonly Flash[];
  beams?: readonly Beam[];
  objectives?: readonly Objective[];
  /** Gadget kind of a ship id ("cloak" hides its engine light). */
  gadgetOf?: (id: number) => string;
  /** Current sim tick (reactor countdown pulse). */
  nowTick?: number;
  /** Seconds (animation). */
  time: number;
}

/** Light per mechanics flash: intensity per unit of flash radius at full strength. */
export const FLASH_LIGHT_PER_RADIUS = 5;

/** Attached lights of this frame (see the file comment). */
export function sceneLights(input: SceneLightInput, out: PointLight[] = []): PointLight[] {
  const t = input.time;
  const visuals = input.visuals ?? [];
  for (const s of input.ships ?? []) {
    if (!s.alive) continue;
    if (s.gadgetActive && input.gadgetOf?.(s.id) === "cloak") continue;
    const e = s.boosting ? ENGINE_LIGHT.boost : ENGINE_LIGHT.idle;
    const r = s.radius || TINKER_RADIUS;
    const back = q.forward(s.orientation);
    const flicker = 0.9 + 0.1 * Math.sin(t * 31 + s.id * 1.7);
    out.push({
      position: { x: s.position.x - back.x * r * 1.4, y: s.position.y - back.y * r * 1.4, z: s.position.z - back.z * r * 1.4 },
      color: e.color,
      intensity: e.intensity * flicker * (r / TINKER_RADIUS),
      range: e.range,
    });
  }
  for (const p of input.shots ?? []) {
    const visual = visuals[p.visual];
    const shape = visual ? SHAPE_LIGHTS[visual.shape] : undefined;
    if (!shape || !visual) continue;
    let intensity = shape.intensity * Math.max(0.5, Math.min(2, visual.scale));
    let color = shape.color ?? brighten(visual.color);
    if (visual.shape === "mine" && (p.state & PROJ_ARMED) !== 0) {
      intensity = Math.sin(t * 10) > 0 ? MINE_BLINK : 0;
      color = [1, 0.3, 0.1];
    }
    if (visual.shape === "paperweight" && (p.state & PROJ_PAPERWEIGHT_LANDED) !== 0) {
      intensity = PAPERWEIGHT_GLOW * (0.75 + 0.25 * Math.sin(t * 3));
    }
    if (intensity <= 0) continue;
    let position = p.position;
    const speed = Math.hypot(p.velocity.x, p.velocity.y, p.velocity.z);
    if (shape.behind && speed > 1e-3) {
      const k = shape.behind / speed;
      position = { x: p.position.x - p.velocity.x * k, y: p.position.y - p.velocity.y * k, z: p.position.z - p.velocity.z * k };
    }
    const light: PointLight = { position, color, intensity };
    if (shape.range !== undefined) light.range = shape.range;
    out.push(light);
  }
  for (const e of input.entities ?? []) {
    const visual = visuals[e.visual];
    if (e.kind.endsWith("Pickup") && visual) {
      const pulse = 0.8 + 0.2 * Math.sin(t * 4 + e.id);
      out.push({ position: e.position, color: brighten(visual.color), intensity: 1.6 * pulse, range: 6 });
    } else if (e.kind === "wreck") {
      // Smouldering embers.
      const glow = 0.7 + 0.3 * Math.sin(t * 2.3 + e.id) * Math.sin(t * 5.1 + e.id * 0.7);
      out.push({ position: e.position, color: [1, 0.4, 0.1], intensity: 1.4 * glow, range: 6 });
    } else if (e.kind === "decoy" && (e.state[1] & 0x80) !== 0 && visual) {
      out.push({ position: e.position, color: [1, 0.8, 0.3], intensity: 3, range: 8 });
    }
  }
  for (const b of input.beams ?? []) {
    if (b.strength <= 0) continue;
    const c = brighten(b.color);
    out.push({ position: b.to, color: c, intensity: 6 * b.strength, range: 10 });
    out.push({
      position: { x: (b.from.x + b.to.x) / 2, y: (b.from.y + b.to.y) / 2, z: (b.from.z + b.to.z) / 2 },
      color: c,
      intensity: 3 * b.strength,
      range: 9,
    });
  }
  for (const f of input.flashes ?? []) {
    if (f.strength <= 0 || f.radius <= 0) continue;
    const intensity = FLASH_LIGHT_PER_RADIUS * Math.min(f.radius, 20) * f.strength * f.strength * (f.ring ? 1.5 : 1);
    out.push({ position: f.position, color: f.color, intensity, range: Math.min(40, 4 + f.radius * 2.5) });
  }
  for (const o of input.objectives ?? []) {
    const light = objectiveLight(o, t, input.nowTick ?? 0);
    if (light) out.push(light);
  }
  return out;
}

/** Light of a mode objective (reactor, core, extraction); zones give none. */
export function objectiveLight(o: Objective, t: number, nowTick: number): PointLight | null {
  if (o.kind === OBJECTIVE.reactor) {
    // Pulses faster as the detonation nears (60 Hz ticks).
    const left = o.timerEnd > 0 ? Math.max(0, (o.timerEnd - nowTick) / 60) : 30;
    const rate = left < 5 ? 9 : left < 15 ? 5 : 2.2;
    const pulse = 0.6 + 0.4 * Math.sin(t * rate);
    return { position: o.pos, color: [1, 0.3, 0.12], intensity: 14 * pulse, range: 24, priority: 3 };
  }
  if (o.kind === OBJECTIVE.core) {
    return { position: o.pos, color: [1, 0.82, 0.35], intensity: 9 * (0.85 + 0.15 * Math.sin(t * 3)), range: 16, priority: 3 };
  }
  if (o.kind === OBJECTIVE.extraction) {
    return { position: o.pos, color: [0.35, 1, 0.55], intensity: 8, range: 18, priority: 2 };
  }
  return null;
}
