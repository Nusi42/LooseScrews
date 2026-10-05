// Visual effects driven by the game (visual-quality-plan.md VQ5): turns what
// happens each frame into particles through the effect library
// (vfxLibrary.ts) and the renderer's ParticleSystem.
//
// Sources, all client-side and cosmetic:
// - projectiles (no ids on the wire): matched frame to frame by owner,
//   visual and predicted position. A new one near its owner is a shot (muzzle
//   flash); one that disappears hit something or burnt out (impact,
//   explosion); every one in flight draws a glowing core and a trail;
// - ships: engine exhaust per nozzle (from the VQ4 hull meshes), a death
//   explosion when `alive` drops, a respawn shimmer, damage smoke from the
//   hits each ship took since it spawned (hit cues);
// - cues (game/mechanics.ts CUE plus the client-only HIT cue): shockwaves
//   explode, the Railspike flashes, blinding, swaps, rewinds, overcharge;
// - the timed glows and beams of EffectsState (mechanics.ts) as soft sprites
//   instead of translucent spheres;
// - entities: pickups sparkle and burst when taken, fresh wrecks smoulder.
//
// Data-driven like SHAPE_STYLES (scene.ts): VFX_SHAPES maps a visual shape
// name (data/weapons `visual.shape`) to its core, trail, muzzle flash and
// impact, so a new weapon with an existing shape needs no client code.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { MeshData } from "../renderer/mesh.ts";
import { MESH_BUILDERS, shipMeshKind } from "../renderer/models/index.ts";
import { PART_ENGINE } from "../renderer/models/kit.ts";
import type { Rgb3 } from "../renderer/particles.ts";
import { SPRITE } from "../renderer/spriteAtlas.ts";
import type { LightPresetName } from "./lightFx.ts";
import { CUE, NO_SHIP, PAINT_COLORS, type Beam, type Cue, type Flash } from "./mechanics.ts";
import {
  add,
  beamNow,
  count,
  damageSmoke,
  dist,
  explosion,
  FIRE_COOL,
  FIRE_HOT,
  glint,
  glowNow,
  impact,
  len,
  lin,
  madd,
  mul,
  muzzleFlash,
  norm,
  randDir,
  range,
  ring,
  scale3,
  shieldRipple,
  smokeSprite,
  SMOKE_DARK,
  sparkleBurst,
  sparks,
  sub,
  v,
  whiten,
  type ImpactKind,
  type MuzzleKind,
  type Rng,
  type VfxSink,
} from "./vfxLibrary.ts";
import { PROJ_ARMED, PROJ_STUCK, projectilePaintColor, TINKER_RADIUS, type EntityState, type LocalPose, type ProjectileState, type ShipState, type VisualDef } from "./views.ts";

/** Client-only cue kind for a hit on a ship (ls-wasm turns `NetEvent::Hit` / `Event::Damaged` into it): source attacker, target victim, position, value damage. */
export const CUE_HIT = 250;

/** How one visual shape looks in flight and when it is fired or hits. */
export interface ShapeVfx {
  /** Glowing core sprite drawn instead of the mesh (energy shots). */
  core?: { size: number; length: number; intensity: number };
  /** Trail behind it. */
  trail?: { kind: "ribbon" | "smoke" | "sparkle"; width: number; life: number; intensity: number };
  /** Rocket motor flame at the tail. */
  motor?: boolean;
  muzzle: MuzzleKind;
  impact: ImpactKind;
  /** Explosion radius (u) at visual scale 1 (grows with scale^1.5). */
  radius?: number;
  /** The mesh is not drawn (scene.ts asks `hidesMesh`): the core is the projectile. */
  hideMesh?: boolean;
}

/** Effects by visual shape name (ls-sim `visual::Shape`, lowercase). */
export const VFX_SHAPES: Readonly<Record<string, ShapeVfx>> = {
  bolt: { core: { size: 0.4, length: 1.6, intensity: 7 }, trail: { kind: "ribbon", width: 0.3, life: 0.2, intensity: 3 }, muzzle: "energy", impact: "sparks", hideMesh: true },
  pellet: { core: { size: 0.16, length: 0.5, intensity: 6 }, trail: { kind: "ribbon", width: 0.08, life: 0.07, intensity: 2.5 }, muzzle: "kinetic", impact: "sparks", hideMesh: true },
  spike: { core: { size: 0.22, length: 1.8, intensity: 10 }, trail: { kind: "ribbon", width: 0.16, life: 0.45, intensity: 3.5 }, muzzle: "kinetic", impact: "sparks", hideMesh: true },
  rocket: { motor: true, trail: { kind: "smoke", width: 0.3, life: 1.4, intensity: 1 }, muzzle: "rocket", impact: "explosion", radius: 1.9 },
  missile: { motor: true, trail: { kind: "smoke", width: 0.24, life: 1.1, intensity: 1 }, muzzle: "rocket", impact: "explosion", radius: 1.6 },
  mine: { muzzle: "lob", impact: "explosion", radius: 2.0 },
  orb: { core: { size: 0.75, length: 0.75, intensity: 6 }, trail: { kind: "ribbon", width: 0.4, life: 0.14, intensity: 2 }, muzzle: "energy", impact: "plasma", hideMesh: true },
  blob: { core: { size: 0.7, length: 0.7, intensity: 3.5 }, trail: { kind: "sparkle", width: 0.25, life: 0.35, intensity: 2 }, muzzle: "energy", impact: "splat", hideMesh: true },
  arc: { core: { size: 0.5, length: 0.9, intensity: 8 }, trail: { kind: "sparkle", width: 0.2, life: 0.2, intensity: 4 }, muzzle: "energy", impact: "plasma", hideMesh: true },
  grenade: { trail: { kind: "smoke", width: 0.14, life: 0.7, intensity: 1 }, muzzle: "lob", impact: "explosion", radius: 1.6 },
  harpoon: { trail: { kind: "ribbon", width: 0.05, life: 0.25, intensity: 1.2 }, muzzle: "kinetic", impact: "thud" },
  paint: { core: { size: 0.5, length: 0.5, intensity: 2.5 }, trail: { kind: "sparkle", width: 0.2, life: 0.3, intensity: 1.5 }, muzzle: "lob", impact: "splat", hideMesh: true },
  // The Tectonic Charge's blast comes as a shockwave cue.
  charge: { muzzle: "lob", impact: "none" },
};

/** Effects of shapes this client does not know: an energy shot. */
export const DEFAULT_SHAPE_VFX: ShapeVfx = { core: { size: 0.3, length: 1, intensity: 6 }, trail: { kind: "ribbon", width: 0.2, life: 0.12, intensity: 2.5 }, muzzle: "energy", impact: "sparks", hideMesh: true };

export function shapeVfx(visual: VisualDef | undefined): ShapeVfx {
  return (visual && VFX_SHAPES[visual.shape]) || DEFAULT_SHAPE_VFX;
}

/** True when the particle core replaces the projectile's mesh (scene.ts skips the mesh, trail and glow shell). */
export function hidesMesh(visual: VisualDef | undefined): boolean {
  return shapeVfx(visual).hideMesh === true;
}

// ----- engine mounts --------------------------------------------------------------

/** One engine nozzle in hull space (instance scale 1, the ship looks along -Z). */
export interface EngineMount {
  /** Centre of the nozzle mouth. */
  pos: Vec3;
  /** Mouth radius (u). */
  radius: number;
}

/**
 * Engine nozzles of a hull mesh: the PART_ENGINE vertices clustered by
 * proximity, each cluster's mouth at its rearmost (largest z) extent.
 */
export function engineMounts(mesh: MeshData): EngineMount[] {
  const p = mesh.positions;
  const pts: Vec3[] = [];
  for (let i = 0; i < mesh.team.length; i++) {
    if (mesh.team[i] === PART_ENGINE) pts.push({ x: p[i * 3] ?? 0, y: p[i * 3 + 1] ?? 0, z: p[i * 3 + 2] ?? 0 });
  }
  // Single-linkage clusters seen from behind (x, y): the bell and the glowing
  // core of one nozzle are separate pieces at different depths.
  const LINK = 0.03;
  const parent = pts.map((_, i) => i);
  const find = (i: number): number => {
    while (parent[i] !== i) i = parent[i] = parent[parent[i]!]!;
    return i;
  };
  for (let i = 0; i < pts.length; i++) {
    for (let j = i + 1; j < pts.length; j++) {
      const a = pts[i]!;
      const b = pts[j]!;
      if (Math.abs(a.x - b.x) < LINK && Math.abs(a.y - b.y) < LINK && Math.hypot(a.x - b.x, a.y - b.y) < LINK) parent[find(i)] = find(j);
    }
  }
  const groups = new Map<number, Vec3[]>();
  pts.forEach((pt, i) => {
    const r = find(i);
    const g = groups.get(r);
    if (g) g.push(pt);
    else groups.set(r, [pt]);
  });
  const mounts: EngineMount[] = [];
  for (const g of groups.values()) {
    let sx = 0;
    let sy = 0;
    let maxZ = -Infinity;
    for (const pt of g) {
      sx += pt.x;
      sy += pt.y;
      maxZ = Math.max(maxZ, pt.z);
    }
    const cx = sx / g.length;
    const cy = sy / g.length;
    let r = 0;
    for (const pt of g) if (pt.z > maxZ - 0.02) r = Math.max(r, Math.hypot(pt.x - cx, pt.y - cy));
    mounts.push({ pos: { x: cx, y: cy, z: maxZ }, radius: Math.max(0.015, r) });
  }
  // A small piece inside a bigger nozzle (a recessed core) belongs to it.
  const merged: EngineMount[] = [];
  for (const m of mounts.sort((a, b) => b.radius - a.radius)) {
    const host = merged.find((h) => Math.hypot(h.pos.x - m.pos.x, h.pos.y - m.pos.y) < h.radius);
    if (host) host.pos = { ...host.pos, z: Math.max(host.pos.z, m.pos.z) };
    else merged.push({ pos: { ...m.pos }, radius: m.radius });
  }
  // Stable order (left to right, bottom to top).
  return merged.sort((a, b) => a.pos.x - b.pos.x || a.pos.y - b.pos.y);
}

const mountCache = new Map<number, EngineMount[]>();

/** Engine nozzles of a ship type (cached; built from the VQ4 hull mesh once). */
export function engineMountsOf(shipType: number): EngineMount[] {
  let m = mountCache.get(shipType);
  if (!m) {
    m = engineMounts(MESH_BUILDERS[shipMeshKind(shipType)]());
    mountCache.set(shipType, m);
  }
  return m;
}

// ----- per-frame input --------------------------------------------------------------

export interface VfxFrame {
  /** Wall clock (s): the particle clock follows it. */
  time: number;
  /** Seconds since the last frame. */
  dt: number;
  /** Camera position (effects far away are skipped). */
  camera: Vec3;
  /** Camera orientation (the own muzzle flash sits below the view). */
  cameraOrientation?: Quat;
  /** Local player's ship id and pose (the own ship is not in `ships`). */
  player: number;
  pose: LocalPose | null;
  ships: readonly ShipState[];
  shots: readonly ProjectileState[];
  entities: readonly EntityState[];
  visuals: readonly VisualDef[];
  /** Cues of this frame (mechanics.ts parseCues), including CUE_HIT. */
  cues: readonly Cue[];
  /** Beams and timed glows of EffectsState.frame (mechanics.ts). */
  beams: readonly Beam[];
  flashes: readonly Flash[];
  /** Gadget kind of a ship id ("shield", "cloak", "decoy", ""). */
  gadgetOf?: (id: number) => string;
  /** Hull points of a ship type (damage smoke below 30 %); default 100. */
  hullOf?: (shipType: number) => number;
  /** Projectiles not drawn (the Eyeball the camera rides). */
  hideShot?: ((p: ProjectileState) => boolean) | undefined;
  /**
   * Side colour (display) of a ship id (M3 C1: 4 teams): plain paint shots
   * and respawn rings take it; undefined = the visual's own colour.
   */
  teamColorOf?: ((id: number) => Rgb3 | undefined) | undefined;
}

interface ShotTrack {
  owner: number;
  visual: number;
  pos: Vec3;
  vel: Vec3;
  state: number;
  /** Distance carried over to the next trail section or puff (u). */
  carry: number;
  matched: boolean;
}

interface ShipTrack {
  pos: Vec3;
  vel: Vec3;
  alive: boolean;
  shipType: number;
  radius: number;
  /** Damage taken since the last spawn (hit cues). */
  damage: number;
  /** Fractional exhaust and smoke particles carried to the next frame. */
  exhaustCarry: number;
  smokeCarry: number;
  seen: number;
}

interface Emitter {
  kind: "tectonic" | "overcharge" | "wreck";
  start: number;
  end: number;
  pos: Vec3;
  ship: number;
  carry: number;
}

/**
 * A short-lived light an effect gives off (explosion, muzzle flash, impact),
 * for VQ6's dynamic lights: `Vfx.lights(now)` lists the live ones.
 */
export interface EffectLight {
  position: Vec3;
  /** Linear radiance at full strength. */
  color: Rgb3;
  /** Reach (u). */
  radius: number;
  /** 0..1 at `now` (1 at birth, fading to 0 at the end of its life). */
  strength: number;
}

/**
 * Where effects send their dynamic-light pulses (VQ6's `LightFx`, which
 * `main.ts` sets with `watchProjectiles = false` so impacts are not doubled).
 */
export interface LightEmitter {
  emit(pulse: LightPresetName, position: Vec3, now: number, options?: { color?: Rgb3; scale?: number }): void;
}

/** Display colour halfway to white (light tints from a weapon's colour). */
const lightTint = (c: Rgb3): Rgb3 => [(c[0] + 1) * 0.5, (c[1] + 1) * 0.5, (c[2] + 1) * 0.5];

interface TimedLight {
  position: Vec3;
  color: Rgb3;
  radius: number;
  start: number;
  end: number;
}

/** Projectiles farther than this from the camera spawn no impact (u). */
export const IMPACT_RANGE = 220;
/** Ships farther than this get no exhaust particles (u). */
export const EXHAUST_RANGE = 140;
/** Damage smoke below this fraction of the hull. */
export const SMOKE_BELOW = 0.3;

const ZERO: Vec3 = v(0, 0, 0);
const WHITE: Rgb3 = [1, 1, 1];

/**
 * Explosion radius (u) of a shape at visual scale `scale`: the preset's
 * radius times scale^1.5 (the Bolt Rocket, scale 1, 1.9 u; the Heavy Rocket, scale 1.8, 4.6 u:
 * between its crater and its splash damage radius).
 */
export function explosionRadius(fx: ShapeVfx, scale: number): number {
  return (fx.radius ?? 1.9) * Math.pow(Math.max(0.3, scale), 1.5);
}

/** The effect state of one match; `update` once per frame before rendering. */
export class Vfx {
  private readonly sink: VfxSink;
  private readonly rng: Rng;
  private shots: ShotTrack[] = [];
  private ships = new Map<number, ShipTrack>();
  private entities = new Map<number, EntityState>();
  private emitters: Emitter[] = [];
  private shakes: Array<{ start: number; end: number; amplitude: number }> = [];
  private flashLights: TimedLight[] = [];
  private primed = false;
  private now = 0;
  /** Dynamic-light pulses of muzzle flashes, impacts, explosions and deaths (VQ6); null = none. */
  lightSink: LightEmitter | null = null;
  /** Impacts, muzzle flashes and explosions spawned so far (tests, debug). */
  readonly counts = { muzzles: 0, impacts: 0, explosions: 0, deaths: 0, ripples: 0 };

  constructor(sink: VfxSink, rng: Rng = Math.random) {
    this.sink = sink;
    this.rng = rng;
  }

  /** Forgets every tracked object (a new match); particles in flight are the renderer's to clear. */
  reset(): void {
    this.shots = [];
    this.ships.clear();
    this.entities.clear();
    this.emitters = [];
    this.shakes = [];
    this.flashLights = [];
    this.primed = false;
  }

  /** Camera shake amplitude (u) from nearby explosions at `now`. */
  shake(now = this.now): number {
    let a = 0;
    for (const s of this.shakes) {
      if (now >= s.end || now < s.start) continue;
      a = Math.max(a, s.amplitude * (1 - (now - s.start) / (s.end - s.start)));
    }
    return a;
  }

  /**
   * Lights of the effects alive at `now` (VQ6 dynamic lights; the engine
   * glow and projectile cores are not listed: VQ6 takes those from ships and
   * projectiles directly). Strength fades quadratically over each light's life.
   */
  lights(now = this.now): EffectLight[] {
    const out: EffectLight[] = [];
    for (const l of this.flashLights) {
      if (now < l.start || now >= l.end) continue;
      const k = 1 - (now - l.start) / (l.end - l.start);
      out.push({ position: l.position, color: l.color, radius: l.radius, strength: k * k });
    }
    return out;
  }

  private light(position: Vec3, color: Rgb3, radius: number, life: number): void {
    this.flashLights.push({ position, color, radius, start: this.now, end: this.now + life });
  }

  update(f: VfxFrame): void {
    const sink = this.sink;
    this.now = f.time;
    sink.now = f.time;
    sink.beginFrame?.();
    this.teamColorOf = f.teamColorOf;
    this.shakes = this.shakes.filter((s) => s.end > f.time);
    this.flashLights = this.flashLights.filter((l) => l.end > f.time);
    const shipPos = (id: number): Vec3 | undefined => {
      if (id === f.player) return f.pose?.alive ? f.pose.position : undefined;
      const t = this.ships.get(id);
      return t?.alive ? t.pos : undefined;
    };
    this.updateShips(f);
    this.applyCues(f, shipPos);
    this.updateShots(f, shipPos);
    this.drawFlashes(f);
    this.updateEntities(f);
    this.updateEmitters(f);
    this.primed = true;
  }

  // ----- ships --------------------------------------------------------------------

  private updateShips(f: VfxFrame): void {
    const sink = this.sink;
    const rng = this.rng;
    const dt = Math.max(1e-4, f.dt);
    const all: ShipState[] = [...f.ships];
    if (f.pose) {
      all.push({
        id: f.player,
        team: 0,
        alive: f.pose.alive,
        boosting: false,
        position: f.pose.position,
        orientation: f.pose.orientation,
        shipType: this.ships.get(f.player)?.shipType ?? 1,
        radius: this.ships.get(f.player)?.radius ?? TINKER_RADIUS,
        gadgetActive: false,
      });
    }
    for (const s of all) {
      let t = this.ships.get(s.id);
      const own = s.id === f.player;
      if (!t) {
        t = { pos: s.position, vel: own && f.pose ? f.pose.velocity : ZERO, alive: s.alive, shipType: s.shipType, radius: s.radius, damage: 0, exhaustCarry: 0, smokeCarry: 0, seen: f.time };
        this.ships.set(s.id, t);
      } else {
        const raw = own && f.pose ? f.pose.velocity : mul(sub(s.position, t.pos), 1 / dt);
        // A jump (respawn, snap) is not velocity.
        t.vel = len(raw) > 120 ? ZERO : add(mul(t.vel, 0.7), mul(raw, 0.3));
        if (t.alive && !s.alive && this.primed) {
          this.counts.deaths++;
          const r = Math.max(1.6, (s.radius || TINKER_RADIUS) * 4.4);
          explosion(sink, rng, t.pos, r, { vel: mul(t.vel, 0.5), hull: true });
          this.light(t.pos, FIRE_HOT, r * 6, 0.6);
          this.lightSink?.emit("shipDeath", t.pos, this.now, { scale: (s.radius || TINKER_RADIUS) / TINKER_RADIUS });
          this.counts.explosions++;
          this.shakeFrom(t.pos, r, f.camera);
        }
        if (!t.alive && s.alive) {
          t.damage = 0;
          if (this.primed && !own) {
            // Respawn: an implosion in the ship's side colour.
            const c = f.teamColorOf?.(s.id) ?? [0.6, 0.9, 1];
            ring(sink, s.position, c, 3, 0.4, 3.2, 0.5);
            sparkleBurst(sink, rng, s.position, c, 18, 4, true);
          }
        }
      }
      t.pos = s.position;
      t.alive = s.alive;
      t.shipType = s.shipType;
      t.radius = s.radius || TINKER_RADIUS;
      t.seen = f.time;
      if (own || !s.alive) continue;
      const gadget = s.gadgetActive ? (f.gadgetOf?.(s.id) ?? "") : "";
      if (gadget === "cloak") {
        // A cloaked ship gives itself away only by a faint shimmer.
        if (rng() < dt * 6 * sink.density) glint(sink, rng, madd(s.position, randDir(rng), t.radius * 0.8), [0.8, 0.9, 1], 0.12, 2, t.vel);
        continue;
      }
      this.exhaust(f, s, t, dt);
      const hull = f.hullOf?.(s.shipType) ?? 100;
      const frac = 1 - t.damage / Math.max(1, hull);
      if (frac < SMOKE_BELOW && dist(s.position, f.camera) < EXHAUST_RANGE) {
        const heavy = frac < SMOKE_BELOW / 2;
        t.smokeCarry += dt * (heavy ? 22 : 12) * sink.density;
        while (t.smokeCarry >= 1) {
          t.smokeCarry -= 1;
          const local = randDir(rng);
          damageSmoke(sink, rng, madd(s.position, local, t.radius * 0.5), t.vel, heavy);
        }
        if (rng() < dt * 3) sparks(sink, rng, madd(s.position, randDir(rng), t.radius * 0.6), randDir(rng), [1, 0.7, 0.4], 4, 1, 0.5);
      }
    }
    for (const [id, t] of this.ships) if (f.time - t.seen > 2) this.ships.delete(id);
  }

  /** Engine flames (immediate, glued to the nozzles) and exhaust particles. */
  private exhaust(f: VfxFrame, s: ShipState, t: ShipTrack, dt: number): void {
    const sink = this.sink;
    const rng = this.rng;
    const mounts = engineMountsOf(s.shipType);
    if (mounts.length === 0) return;
    const scale = t.radius / TINKER_RADIUS;
    const back = q.rotate(s.orientation, v(0, 0, 1));
    const boost = s.boosting ? 1 : 0;
    // Idle: a warm orange glow; boost: a long blue-white flame (VQ4's engine colours).
    const core: Rgb3 = boost ? [5, 7.5, 12] : [7, 3.4, 1.1];
    const flame: Rgb3 = boost ? [2.4, 4.2, 9] : [4.5, 1.6, 0.4];
    const flicker = 0.85 + 0.15 * Math.sin(f.time * 47 + s.id * 1.7) + 0.08 * Math.sin(f.time * 83 + s.id);
    const far = dist(s.position, f.camera) > EXHAUST_RANGE;
    for (const m of mounts) {
      const mouth = add(s.position, q.rotate(s.orientation, mul(m.pos, scale)));
      const r = m.radius * scale;
      const flameLen = (boost ? 9 : 3.2) * r * flicker;
      sink.immediate({ pos: madd(mouth, back, r * 0.2), life: 1, sprite: SPRITE.glow, color: scale3(core, flicker), size: r * (boost ? 5.5 : 4) });
      sink.immediate({ pos: madd(mouth, back, flameLen * 0.45), vel: back, life: 1, sprite: SPRITE.flame, hot: true, orient: "fixed", stretch: flameLen, color: scale3(flame, flicker), size: r * 2.3 });
      if (boost) sink.immediate({ pos: madd(mouth, back, flameLen * 0.25), vel: back, life: 1, sprite: SPRITE.streak, orient: "fixed", stretch: flameLen * 0.6, color: [6, 8, 12], size: r * 0.9 });
    }
    if (far) return;
    // Exhaust particles: hot gas leaving the nozzles, cooling to dull red.
    t.exhaustCarry += dt * (boost ? 240 : 70) * sink.density;
    while (t.exhaustCarry >= 1) {
      t.exhaustCarry -= 1;
      const m = mounts[Math.floor(rng() * mounts.length) % mounts.length]!;
      const mouth = add(s.position, q.rotate(s.orientation, mul(m.pos, scale)));
      const r = m.radius * scale;
      const speed = boost ? range(rng, 7, 11) : range(rng, 2.5, 4);
      // Born at some moment of the last frame, where the nozzle was then, so
      // the plume is continuous rather than one clump per frame.
      const delay = rng() * dt;
      // Soft streaks of glowing gas that overlap into a continuous plume.
      sink.spawn({
        pos: madd(madd(mouth, back, r * range(rng, 0.3, 1.2)), t.vel, -delay),
        vel: add(mul(t.vel, 0.85), madd(mul(back, speed), randDir(rng), 0.15)),
        t0: sink.now - delay,
        life: boost ? range(rng, 0.22, 0.36) : range(rng, 0.16, 0.28),
        drag: 2.5,
        sprite: SPRITE.glow,
        orient: "velocity",
        stretch: 0.1,
        color: boost ? [0.45, 0.7, 1.5] : [0.7, 0.25, 0.06],
        color1: boost ? [0.02, 0.02, 0.06] : [0.04, 0.008, 0.002],
        size: r * 4,
        size1: r * (boost ? 9 : 6),
      });
    }
  }

  private shakeFrom(at: Vec3, r: number, camera: Vec3): void {
    const d = dist(at, camera);
    if (d > r * 10) return;
    const amplitude = Math.min(0.45, (0.09 * r) / Math.max(1, d / (r * 1.5)));
    if (amplitude > 0.005) this.shakes.push({ start: this.now, end: this.now + 0.25 + 0.06 * r, amplitude });
  }

  // ----- cues -------------------------------------------------------------------------

  private applyCues(f: VfxFrame, shipPos: (id: number) => Vec3 | undefined): void {
    const sink = this.sink;
    const rng = this.rng;
    for (const c of f.cues) {
      switch (c.kind) {
        case CUE_HIT: {
          const t = this.ships.get(c.target);
          if (t) t.damage += c.value;
          break;
        }
        case CUE.shockwave: {
          // Tectonic Charge, Hot Reactor: a blast sized to the shockwave (its visible core is smaller).
          const r = Math.max(2.5, Math.min(18, c.value * 0.3));
          explosion(sink, rng, c.position, r, { terrain: true });
          this.light(c.position, FIRE_HOT, r * 6, 0.7);
          if (c.value >= 40) ring(sink, c.position, [1, 0.8, 0.55], 2.5, r, c.value * 2, 1.2);
          this.counts.explosions++;
          this.shakeFrom(c.position, r, f.camera);
          break;
        }
        case CUE.muzzleFlash: {
          // Railspike: a flash that reveals the shooter.
          const from = shipPos(c.source) ?? c.position;
          const dir = norm(sub(c.position, from), v(0, 0, -1));
          muzzleFlash(sink, rng, "energy", c.position, dir, [0.85, 0.85, 1], c.source === f.player ? 0.6 : 1.8);
          sink.spawn({ pos: c.position, life: 0.2, sprite: SPRITE.glow, color: [10, 10, 14], color1: [0, 0, 0], size: 3, size1: 4.5 });
          break;
        }
        case CUE.blinded:
          sink.spawn({ pos: c.position, life: 0.3, sprite: SPRITE.flare, color: [40, 40, 40], color1: [0, 0, 0], size: 6, size1: 9, rot: rng() * 6.28 });
          sparkleBurst(sink, rng, c.position, WHITE, 30, 14);
          break;
        case CUE.swapped: {
          const other = shipPos(c.target);
          for (const at of other ? [c.position, other] : [c.position]) {
            sparkleBurst(sink, rng, at, [0.8, 0.5, 1], 22, 5, true);
            ring(sink, at, [0.8, 0.5, 1], 4, 2.5, 0.3, 0.4);
          }
          break;
        }
        case CUE.rewound:
          sparkleBurst(sink, rng, c.position, [0.6, 0.9, 1], 28, 6, true);
          ring(sink, c.position, [0.6, 0.9, 1], 3, 3.5, 0.3, 0.5);
          break;
        case CUE.decoyReturn:
          sparkleBurst(sink, rng, c.position, [1, 0.8, 0.3], 18, 5);
          ring(sink, c.position, [1, 0.8, 0.3], 3, 0.4, 3, 0.4);
          break;
        case CUE.overcharge:
          this.emitters.push({ kind: "overcharge", start: f.time, end: f.time + 0.6, pos: c.position, ship: c.source, carry: 0 });
          break;
        case CUE.tectonicArmed:
          this.emitters.push({ kind: "tectonic", start: f.time, end: f.time + Math.max(0.5, c.value / 60), pos: c.position, ship: NO_SHIP, carry: 0 });
          break;
        case CUE.gadget: {
          // A gadget switched on (values below 253): a ring around the ship.
          const at = shipPos(c.source);
          if (at && c.value < 253) ring(sink, at, [0.55, 0.85, 1], 2.5, 0.5, 2.4, 0.35);
          break;
        }
        default:
          break;
      }
    }
  }

  // ----- projectiles ------------------------------------------------------------------

  private updateShots(f: VfxFrame, shipPos: (id: number) => Vec3 | undefined): void {
    const sink = this.sink;
    const rng = this.rng;
    const dt = Math.max(1e-4, f.dt);
    for (const t of this.shots) t.matched = false;
    const next: ShotTrack[] = [];
    const muzzled = new Set<string>();
    for (const p of f.shots) {
      // Match the nearest track of the same owner and visual to where it should be now.
      let best: ShotTrack | null = null;
      let bestD = Infinity;
      for (const t of this.shots) {
        if (t.matched || t.owner !== p.owner || t.visual !== p.visual) continue;
        const pred = madd(t.pos, t.vel, dt);
        const d = dist(pred, p.position);
        const tol = 1.5 + len(t.vel) * dt * 0.75;
        if (d < tol && d < bestD) {
          best = t;
          bestD = d;
        }
      }
      const visual = f.visuals[p.visual];
      const fx = shapeVfx(visual);
      const color = this.shotColor(p, visual);
      let track: ShotTrack;
      if (best) {
        best.matched = true;
        track = best;
        // No trail behind the Eyeball the camera rides (it would fill the view).
        if (!f.hideShot?.(p)) this.trail(f, track, p, fx, color, visual?.scale ?? 1);
        track.pos = p.position;
        track.vel = p.velocity;
        track.state = p.state;
      } else {
        track = { owner: p.owner, visual: p.visual, pos: p.position, vel: p.velocity, state: p.state, carry: 0, matched: true };
        // A new shot near its owner was just fired.
        const owner = shipPos(p.owner);
        const key = `${p.owner}/${p.visual}`;
        if (this.primed && owner && !muzzled.has(key) && dist(owner, p.position) < 3 + len(p.velocity) * 0.12) {
          muzzled.add(key);
          this.counts.muzzles++;
          const dir = norm(p.velocity, v(0, 0, -1));
          const own = p.owner === f.player;
          const ownerTrack = this.ships.get(p.owner);
          let at = madd(owner, dir, (ownerTrack?.radius ?? TINKER_RADIUS) * 1.3);
          if (own && f.cameraOrientation) at = madd(at, q.up(f.cameraOrientation), -0.16);
          muzzleFlash(sink, rng, fx.muzzle, at, dir, color, own ? 0.35 : 1, ownerTrack?.vel ?? ZERO);
          if (fx.muzzle !== "none") {
            this.light(at, lin(color, 3), 6, 0.08);
            this.lightSink?.emit("muzzle", at, this.now, { color: lightTint(color), scale: own ? 0.6 : 1 });
          }
        }
      }
      next.push(track);
      if (!f.hideShot?.(p)) this.drawShot(f, p, fx, color, visual?.scale ?? 1);
    }
    // Tracks without a match this frame hit something or burnt out.
    for (const t of this.shots) {
      if (t.matched || !this.primed) continue;
      const hit = madd(t.pos, t.vel, dt * 0.5);
      // Too far to see, or the own ship was hit (the camera is inside it).
      const fromCam = dist(hit, f.camera);
      if (fromCam > IMPACT_RANGE || fromCam < 0.8) continue;
      const visual = f.visuals[t.visual];
      const fx = shapeVfx(visual);
      const scale = visual?.scale ?? 1;
      const color = this.shotColor({ owner: t.owner, state: t.state }, visual);
      // Near a ship: a hull hit (or a shield ripple), else rock.
      let hitShip: { pos: Vec3; radius: number; vel: Vec3; shield: boolean } | null = null;
      for (const [id, s] of this.ships) {
        if (!s.alive || id === t.owner) continue;
        const shield = f.gadgetOf?.(id) === "shield";
        const reach = s.radius * (shield ? 1.9 : 1.2) + 0.6;
        if (dist(s.pos, hit) < reach) hitShip = { pos: s.pos, radius: s.radius, vel: s.vel, shield };
      }
      this.counts.impacts++;
      if (hitShip?.shield && fx.impact !== "explosion") {
        shieldRipple(sink, rng, hitShip.pos, hitShip.radius * 1.7, hit, hitShip.vel);
        this.counts.ripples++;
        continue;
      }
      const dir = norm(t.vel, v(0, 0, -1));
      const radius = explosionRadius(fx, scale);
      // On rock, pull the effect back out of the wall along the flight, so
      // the fireball is not half buried (the soft particles hide what is inside).
      const onRock = hitShip === null && !(t.state & PROJ_STUCK);
      const at = onRock ? madd(hit, dir, -(fx.impact === "explosion" ? radius * 0.45 : 0.15)) : hit;
      impact(sink, rng, at, { kind: fx.impact, color, size: scale, dir, terrain: hitShip === null && !(t.state & PROJ_STUCK && fx.impact !== "explosion"), radius });
      if (fx.impact === "explosion") {
        this.light(at, FIRE_HOT, radius * 6, 0.5);
        this.lightSink?.emit(radius >= 3.5 ? "bigExplosion" : "explosion", at, this.now);
      } else if (fx.impact !== "none") {
        this.light(at, lin(color, 3), 4 * Math.max(0.5, scale), 0.12);
        const plasma = fx.impact === "plasma" || fx.impact === "splat";
        this.lightSink?.emit(plasma ? "plasma" : "spark", at, this.now, { color: lightTint(color), scale: Math.max(0.5, Math.min(2, scale)) });
      }
      if (hitShip?.shield) {
        shieldRipple(sink, rng, hitShip.pos, hitShip.radius * 1.7, at, hitShip.vel);
        this.counts.ripples++;
      }
      if (fx.impact === "explosion") {
        this.counts.explosions++;
        this.shakeFrom(at, radius, f.camera);
      }
    }
    this.shots = next;
  }

  private teamColorOf: ((id: number) => Rgb3 | undefined) | undefined;

  private shotColor(p: { owner: number; state: number }, visual: VisualDef | undefined): Rgb3 {
    if (visual?.shape === "paint") {
      const id = projectilePaintColor(p.state);
      if (id !== 0) return PAINT_COLORS[id] ?? WHITE;
      // Plain paint flies in the shooter's team colour (as the mesh path draws it).
      const team = this.teamColorOf?.(p.owner);
      if (team) return team;
    }
    return visual?.color ?? WHITE;
  }

  /** The glowing core (and rocket motor, armed-mine blink) of a projectile in flight: immediate sprites. */
  private drawShot(f: VfxFrame, p: ProjectileState, fx: ShapeVfx, color: Rgb3, scale: number): void {
    const sink = this.sink;
    const c = lin(color, 1);
    const dir = norm(p.velocity, v(0, 0, -1));
    if (fx.core) {
      const k = fx.core;
      const pulse = p.state & PROJ_ARMED ? 0.6 + 0.4 * Math.sin(f.time * 10) : 1;
      // Coloured halo and a white-hot centre, stretched along the flight
      // (velocity mode: no dimming end-on, unlike trail sections).
      const speed = Math.max(1, len(p.velocity));
      sink.immediate({ pos: p.position, vel: p.velocity, life: 1, sprite: SPRITE.glow, orient: "velocity", stretch: (k.length * scale * 1.15) / speed, color: scale3(c, k.intensity * pulse * 0.35), size: k.size * scale * 1.8 });
      sink.immediate({ pos: p.position, vel: p.velocity, life: 1, sprite: SPRITE.glow, orient: "velocity", stretch: (k.length * scale) / speed, color: scale3(whiten(c, 0.75), k.intensity * pulse * 1.6), size: k.size * scale * 0.6 });
    }
    if (fx.motor) {
      const tail = madd(p.position, dir, -0.45 * scale);
      const flick = 0.85 + 0.15 * Math.sin(f.time * 60 + p.position.x);
      sink.immediate({ pos: tail, life: 1, sprite: SPRITE.glow, color: scale3(FIRE_HOT, 0.9 * flick), size: 0.9 * scale });
      sink.immediate({ pos: madd(tail, dir, -0.35 * scale), vel: mul(dir, -1), life: 1, sprite: SPRITE.flame, hot: true, orient: "fixed", stretch: 0.9 * scale * flick, color: [7, 3.5, 1.1], size: 0.32 * scale });
    }
    const shape = f.visuals[p.visual]?.shape ?? "";
    if (shape === "mine" && p.state & PROJ_ARMED && Math.sin(f.time * 10) > 0) {
      sink.immediate({ pos: p.position, life: 1, sprite: SPRITE.flare, color: scale3(c, 6), size: 0.9 * scale });
    }
    if (shape === "charge") {
      const pulse = 0.7 + 0.3 * Math.sin(f.time * 6);
      sink.immediate({ pos: p.position, life: 1, sprite: SPRITE.glow, color: scale3(c, 5 * pulse), size: 2.4 * scale });
    }
  }

  /** Trail sections from the track's last position to `p` (spawned: they stay behind and fade). */
  private trail(f: VfxFrame, t: ShotTrack, p: ProjectileState, fx: ShapeVfx, color: Rgb3, scale: number): void {
    const tr = fx.trail;
    if (!tr || p.state & PROJ_STUCK) return;
    const sink = this.sink;
    const rng = this.rng;
    const seg = sub(p.position, t.pos);
    const segLen = len(seg);
    if (segLen < 1e-4 || segLen > 30) return;
    const dir = mul(seg, 1 / segLen);
    const c = lin(color, 1);
    const dt = Math.max(1e-4, f.dt);
    const width = tr.width * Math.max(0.5, scale);
    if (tr.kind === "ribbon") {
      // Overlapping streak sections with birth times along the segment, so the
      // ribbon fades and narrows smoothly from the head back.
      const spacing = Math.max(width * 0.7, segLen / 16);
      let d = t.carry;
      while (d < segLen) {
        const frac = d / segLen;
        sink.spawn({
          pos: madd(t.pos, dir, d),
          vel: mul(dir, 1e-3),
          t0: sink.now - dt * (1 - frac),
          life: tr.life,
          sprite: SPRITE.streak,
          orient: "fixed",
          stretch: spacing * 2.2,
          color: scale3(whiten(c, 0.3), tr.intensity),
          color1: scale3(c, tr.intensity * 0.1),
          size: width,
          size1: width * 0.25,
        });
        d += spacing;
      }
      t.carry = d - segLen;
    } else if (tr.kind === "smoke") {
      const spacing = Math.max(0.18, width * 0.8);
      let d = t.carry;
      while (d < segLen) {
        const frac = d / segLen;
        const at = madd(t.pos, dir, d);
        sink.spawn({
          pos: at,
          vel: madd(mul(dir, -0.8), randDir(rng), 0.35),
          t0: sink.now - dt * (1 - frac),
          life: range(rng, 0.7, 1.0) * tr.life,
          drag: 1.2,
          sprite: smokeSprite(rng),
          blend: "alpha",
          color: [0.075, 0.07, 0.065],
          color1: SMOKE_DARK,
          alpha: 0.42,
          alpha1: 0,
          fadeIn: 0.04,
          size: width,
          size1: width * range(rng, 3, 4.5),
          spin: range(rng, -1, 1),
          rot: rng() * 6.28,
        });
        if (rng() < 0.4) {
          sink.spawn({ pos: at, vel: madd(mul(dir, -2), randDir(rng), 0.6), t0: sink.now - dt * (1 - frac), life: 0.07, drag: 4, sprite: SPRITE.flame, hot: true, color: scale3(FIRE_HOT, 0.6), color1: FIRE_COOL, size: width * 0.6, size1: width * 0.9, rot: rng() * 6.28 });
        }
        d += spacing;
      }
      t.carry = d - segLen;
    } else {
      // Sparkle: glittering bits shed along the path.
      const n = count(sink, rng, (segLen / Math.max(0.15, width)) * 0.8);
      for (let i = 0; i < n; i++) {
        const frac = rng();
        sink.spawn({
          pos: madd(madd(t.pos, dir, segLen * frac), randDir(rng), width * 0.4),
          vel: madd(mul(dir, -1), randDir(rng), 0.8),
          t0: sink.now - dt * (1 - frac),
          life: tr.life * range(rng, 0.6, 1),
          drag: 2,
          sprite: SPRITE.glow,
          color: scale3(whiten(c, 0.3), tr.intensity),
          color1: [0, 0, 0],
          size: width,
          size1: width * 0.3,
        });
      }
    }
  }

  // ----- timed glows and beams --------------------------------------------------------

  private drawFlashes(f: VfxFrame): void {
    const sink = this.sink;
    for (const b of f.beams) beamNow(sink, this.rng, b.from, b.to, b.color, Math.max(0.2, b.strength));
    for (const fl of f.flashes) {
      // Shockwave rings explode through the cue; the rest glow softly.
      if (fl.ring || fl.strength <= 0) continue;
      glowNow(sink, fl.position, fl.color, fl.radius * 2.2, 3 * fl.strength * fl.strength);
    }
  }

  // ----- entities ---------------------------------------------------------------------

  private updateEntities(f: VfxFrame): void {
    const sink = this.sink;
    const rng = this.rng;
    const dt = Math.max(1e-4, f.dt);
    const seen = new Map<number, EntityState>();
    for (const e of f.entities) {
      seen.set(e.id, e);
      const color = f.visuals[e.visual]?.color ?? WHITE;
      const near = dist(e.position, f.camera) < 90;
      // Soft glows instead of the translucent shells (scene.ts with vfx): pickups and a returning decoy.
      if (e.kind.endsWith("Pickup") && near) {
        const shape = f.visuals[e.visual]?.shape ?? "";
        const strong = shape === "upgrade" ? 1.6 : 1;
        glowNow(sink, e.position, color, 2.2 * strong, 0.9 * strong * (0.8 + 0.2 * Math.sin(f.time * 4 + e.id)));
      }
      if (e.kind === "decoy" && (e.state[1] & 0x80) !== 0) glowNow(sink, e.position, color, 2.5, 2 + Math.sin(f.time * 20));
      if (e.kind.endsWith("Pickup") && near && rng() < dt * 5 * sink.density) {
        glint(sink, rng, madd(e.position, randDir(rng), range(rng, 0.3, 0.7)), color, range(rng, 0.25, 0.45), 6, v(0, 0.4, 0));
      }
      if (e.kind === "wreck" && !this.entities.has(e.id) && this.primed) {
        this.emitters.push({ kind: "wreck", start: f.time, end: f.time + 6, pos: e.position, ship: e.id, carry: 0 });
      }
    }
    if (this.primed) {
      for (const [id, e] of this.entities) {
        if (seen.has(id) || !e.kind.endsWith("Pickup")) continue;
        if (dist(e.position, f.camera) > 90) continue;
        const color = f.visuals[e.visual]?.color ?? WHITE;
        sparkleBurst(sink, rng, e.position, color, 20, 5);
        ring(sink, e.position, color, 3, 0.3, 2.2, 0.35);
      }
    }
    this.entities = seen;
  }

  private updateEmitters(f: VfxFrame): void {
    const sink = this.sink;
    const rng = this.rng;
    const dt = Math.max(1e-4, f.dt);
    this.emitters = this.emitters.filter((e) => e.end > f.time);
    for (const e of this.emitters) {
      const age = (f.time - e.start) / (e.end - e.start);
      if (e.kind === "tectonic") {
        // The ground cracks: a pulsing red glow, embers and dust shaken loose.
        const pulse = 0.6 + 0.4 * Math.sin(f.time * (8 + 16 * age));
        glowNow(sink, e.pos, [1, 0.35, 0.1], 5 + 4 * age, 2.5 * pulse * (0.5 + age));
        e.carry += dt * 40 * sink.density;
        while (e.carry >= 1) {
          e.carry -= 1;
          const d = randDir(rng);
          sink.spawn({ pos: madd(e.pos, d, range(rng, 0.5, 4)), vel: mul(d, range(rng, 0.5, 2.5)), life: range(rng, 0.5, 1.2), drag: 1, sprite: SPRITE.spark, orient: "velocity", stretch: 0.08, color: [8, 3, 0.8], color1: [0.6, 0.08, 0.01], size: 0.07 });
        }
      } else if (e.kind === "overcharge") {
        const at = this.ships.get(e.ship)?.pos ?? (e.ship === f.player ? f.pose?.position : undefined) ?? e.pos;
        if (e.ship !== f.player) sparks(this.sink, rng, madd(at, randDir(rng), 0.4), randDir(rng), [1, 0.3, 0.1], dt * 60, 1.5, 0.6);
      } else {
        // A fresh wreck smoulders: smoke and embers, dying down.
        if (dist(e.pos, f.camera) > EXHAUST_RANGE) continue;
        e.carry += dt * 14 * (1 - age) * sink.density;
        while (e.carry >= 1) {
          e.carry -= 1;
          damageSmoke(sink, rng, madd(e.pos, randDir(rng), 0.4), ZERO, age < 0.5);
        }
      }
    }
  }
}
