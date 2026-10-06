// Belt Up's world presentation in the client (M4 C7, docs/milestone-4-plan.md
// §3.8 "Visuals" and "HUD"): everything a belt level shows besides its
// rock, for any mode that runs on a belt.
//
// - The belt look for the renderer (renderer/belt.ts `BeltLook`): dust band,
//   aurora, the gates in the own racer's states (next bright, later faint,
//   passed dimmed in the own colour; numbered in the own lap order), the
//   exit portal (brighter once the own lap is done) and sign, ice glints,
//   the screen-edge radiation from the own ship's dose.
// - Instances: the drifting boulders (ls-wasm `boulderPoses`, the
//   simulation's orbit functions, tumbling), thousands of client-only
//   pebbles drifting in the belt near the camera, and the start-light
//   gantries over every racer's slot with their five lamps.
// - Particles: irradiated hulls sizzle (sparks and a green crackle shell),
//   motes swirl into the exit portal.
// - Lights: the portal, the sign, the lit start lamps.
// - Sounds (A6 engine): the start beeps and the go tone, the announcer's
//   "Belt up!", a chime per gate, the lap-done sting, the wrong-way buzzer,
//   the finish fanfare, the Geiger counter by dose and the portal's hum.

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { BeltGateLook, BeltLook } from "../../renderer/belt.ts";
import { SIGN_WORLD_HEIGHT } from "../../renderer/belt.ts";
import type { PointLight } from "../../renderer/lights.ts";
import { GANTRY_LAMP_RADIUS, GANTRY_LAMPS } from "../../renderer/models/raceProps.ts";
import type { ParticleSpec } from "../../renderer/particles.ts";
import type { Instance, Rgb, SceneInstances } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import type { RaceState, Racer } from "../modeState.ts";
import type { ShipState } from "../views.ts";
import { azimuth, type Belt, gateAzimuth, LAP_GATES, prograde, TICK_HZ, WEDGES, wrapAngle } from "./course.ts";

/** Where particles go (the VQ5 particle system or a test fake). */
export interface RaceParticleSink {
  spawn(p: ParticleSpec): void;
  immediate(p: ParticleSpec): void;
}

/** The sound engine calls the race uses (audio/engine.ts AudioEngine). */
export interface RaceSoundEngine {
  play(id: string, o?: { position?: Vec3 | null; gain?: number; rate?: number; delay?: number }): boolean;
  loop(id: string, o?: { position?: Vec3 | null; gain?: number }): { readonly playing: boolean; setGain(g: number, ramp?: number): void; setPosition(p: Vec3): void; stop(fade?: number): void } | null;
}

/** Seconds of start lights the client assumes when it did not see them begin (`belt_up.ron` `lights_secs`). */
export const LIGHTS_SECS = 3;
/** Start lamps per gantry. */
export const START_LAMPS = 5;
/** Seconds the gantries stay (green) after the lights went out. */
export const GO_SECS = 1.6;

/** Start lights at `nowTick`: lamps lit (0-5), the green "go" after they went out, and whether the gantries show. */
export interface StartLights {
  show: boolean;
  lit: number;
  go: boolean;
  /** Seconds since the lights went out (go), else 0. */
  sinceGo: number;
}

/**
 * The start lights: during the lights phase five lamps come on one by one
 * (at 1/6 .. 5/6 of the phase), then all go out at its end ("Belt up!") and
 * the lamps flash green for GO_SECS.
 */
export function startLights(race: RaceState | null, nowTick: number, lightsStartTick: number, raceStartTick: number): StartLights {
  if (!race) return { show: false, lit: 0, go: false, sinceGo: 0 };
  if (race.phase === "lights") {
    const end = race.phaseEndTick;
    const start = lightsStartTick > 0 && lightsStartTick < end ? lightsStartTick : end - LIGHTS_SECS * TICK_HZ;
    const total = Math.max(1, end - start);
    const step = total / (START_LAMPS + 1);
    const lit = Math.max(0, Math.min(START_LAMPS, Math.floor((nowTick - start) / step)));
    return { show: true, lit, go: false, sinceGo: 0 };
  }
  if (race.phase === "racing" && raceStartTick > 0) {
    const since = (nowTick - raceStartTick) / TICK_HZ;
    if (since >= 0 && since < GO_SECS) return { show: true, lit: 0, go: true, sinceGo: since };
  }
  return { show: false, lit: 0, go: false, sinceGo: 0 };
}

/** Lap order number (1-8) of gate `k` for a racer starting at `slot`: its own gate is 8. */
export function lapNumber(k: number, slot: number): number {
  return ((((k - slot - 1) % WEDGES) + WEDGES) % WEDGES) + 1;
}

/** Gate colours. */
export const GATE_NEXT: Rgb = [0.62, 0.97, 1.0];
export const GATE_LATER: Rgb = [0.3, 0.72, 1.0];
export const GATE_NEUTRAL: Rgb = [0.45, 0.8, 1.0];

/**
 * The 8 gates as the own racer sees them: the next one bright and pulsing,
 * later ones faint, passed ones dim in the own colour, numbered in the
 * own lap order. Without an own racer (spectators, other modes on a belt):
 * all alike, numbered 1-8.
 */
export function gateLooks(own: Pick<Racer, "gates" | "slot"> | null, ownColor: Rgb): BeltGateLook[] {
  const out: BeltGateLook[] = [];
  for (let k = 0; k < WEDGES; k++) {
    if (!own) {
      out.push({ color: GATE_NEUTRAL, intensity: 0.5, number: k + 1, pulse: 0, state: 0 });
      continue;
    }
    const n = lapNumber(k, own.slot);
    if (n <= own.gates) out.push({ color: ownColor, intensity: 0.2, number: n, pulse: 0, state: 2 });
    else if (n === own.gates + 1) out.push({ color: GATE_NEXT, intensity: 1.15, number: n, pulse: 1, state: 1 });
    else out.push({ color: GATE_LATER, intensity: 0.42, number: n, pulse: 0, state: 0 });
  }
  return out;
}

/** Turns the integer server tick into a smooth fractional one between updates. */
export class TickClock {
  private tick = -1;
  private at = 0;

  /** Fractional tick at `time` (s) given the latest integer `nowTick`. */
  update(nowTick: number, time: number): number {
    if (nowTick !== this.tick) {
      this.tick = nowTick;
      this.at = time;
    }
    return nowTick + Math.min(1.5, Math.max(0, (time - this.at) * TICK_HZ));
  }

  reset(): void {
    this.tick = -1;
  }
}

/** Mulberry32 (stable pebble field per level). */
function rng(seed: number): () => number {
  let a = seed >>> 0 || 0x9e3779b9;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Pebbles in the field (client only, never collide). */
export const PEBBLE_COUNT = 9000;
/** Pebbles drawn per frame at most (nearest the camera first is not needed: within PEBBLE_RANGE). */
export const PEBBLE_DRAW_MAX = 2600;
/** Distance (u) within which pebbles are drawn. */
export const PEBBLE_RANGE = 70;

/** Floats per pebble: azimuth, radius, height, size, spin axis xyz, spin rate, orbit rate, shade. */
const PF = 10;

/**
 * Thousands of small rock chips in the belt for depth: denser toward the
 * inner edge and the belt plane like the asteroids, drifting slowly
 * prograde and tumbling. Only the ones near the camera are drawn.
 */
export class PebbleField {
  readonly data: Float32Array;
  readonly count: number;
  private readonly belt: Belt;

  constructor(belt: Belt, seed: number, count = PEBBLE_COUNT) {
    this.belt = belt;
    this.count = count;
    this.data = new Float32Array(count * PF);
    const r = rng(seed ^ 0x5eb1e5);
    const span = belt.outerRadius - belt.innerRadius;
    for (let i = 0; i < count; i++) {
      const o = i * PF;
      // Density falling linearly from 2 at the inner edge to 1 at the outer
      // (pdf 2 - x, inverted CDF), like the asteroids.
      const radial = belt.innerRadius - 6 + (span + 12) * (2 - Math.sqrt(4 - 3 * r()));
      const g = (r() + r() + r() - 1.5) / 1.5;
      const ax = v3.normalize({ x: r() - 0.5, y: r() - 0.5, z: r() - 0.5 });
      this.data[o] = r() * Math.PI * 2;
      this.data[o + 1] = radial;
      this.data[o + 2] = g * belt.halfThickness * 1.15;
      this.data[o + 3] = 0.05 * Math.pow(9, r());
      this.data[o + 4] = ax.x;
      this.data[o + 5] = ax.y;
      this.data[o + 6] = ax.z;
      this.data[o + 7] = 0.2 + 1.6 * r();
      this.data[o + 8] = 0.002 + 0.004 * r();
      this.data[o + 9] = r();
    }
  }

  /** Pebble instances near `cam` at `time` (looking along `fwd`), at most `max`. */
  instances(cam: Vec3, fwd: Vec3, time: number, out: Instance[], max = PEBBLE_DRAW_MAX, range = PEBBLE_RANGE): void {
    const b = this.belt;
    const d = this.data;
    const camAz = azimuth(b, cam);
    const camR = Math.hypot(cam.x - b.center.x, cam.z - b.center.z);
    // Quick reject by angle: a pebble within `range` lies within this azimuth window.
    const window = camR > range ? Math.asin(Math.min(1, range / camR)) + 0.02 : Math.PI;
    let n = 0;
    for (let i = 0; i < this.count && n < max; i++) {
      const o = i * PF;
      const a = (d[o] ?? 0) + (d[o + 8] ?? 0) * time;
      if (Math.abs(wrapAngle(a - camAz)) > window) continue;
      const rad = d[o + 1] ?? 0;
      const p = { x: b.center.x + rad * Math.cos(a), y: b.center.y + (d[o + 2] ?? 0), z: b.center.z - rad * Math.sin(a) };
      const dx = p.x - cam.x, dy = p.y - cam.y, dz = p.z - cam.z;
      const dist2 = dx * dx + dy * dy + dz * dz;
      // Too far, or so close it would fill the view.
      if (dist2 > range * range || dist2 < 12) continue;
      // Behind the camera (with a margin for big ones close by).
      if (dx * fwd.x + dy * fwd.y + dz * fwd.z < -2) continue;
      const size = d[o + 3] ?? 0.1;
      // Tiny far pebbles are sub-pixel: skip.
      if (size * size * 1600 < dist2 * 0.01) continue;
      const axis = { x: d[o + 4] ?? 0, y: d[o + 5] ?? 1, z: d[o + 6] ?? 0 };
      const shade = d[o + 9] ?? 0.5;
      const g = 0.24 + 0.16 * shade;
      out.push({
        position: p,
        rotation: q.fromAxisAngle(axis, (d[o + 7] ?? 1) * time + i),
        scale: 2 * size,
        color: [g * (1.05 + 0.1 * shade), g, g * (0.95 - 0.08 * shade)],
        emissive: 0,
      });
      n++;
    }
  }
}

/** Input of one frame. */
export interface RaceFxInput {
  time: number;
  dt: number;
  /** Server tick (integer; Game.nowTick()). */
  nowTick: number;
  camera: Vec3;
  orientation: Quat;
  belt: Belt | null;
  race: RaceState | null;
  /** Own player id. */
  own: number;
  /** Own ship (alive, in play), null otherwise. */
  ownPose: { position: Vec3; velocity: Vec3 } | null;
  /** Own colour (passed gates, minimap). */
  ownColor: Rgb;
  ships: readonly ShipState[];
  /** Boulder poses at a fractional tick (ls-wasm `boulderPoses`); absent: from the belt's orbits. */
  boulderPoses?: ((tick: number) => Float32Array) | undefined;
  /** Dose rate (hull/s) at a point (ls-wasm `doseRateAt`). */
  doseAt?: ((p: Vec3) => number) | undefined;
  /** Overlays up (lobby, results): no sounds of the race. */
  quiet: boolean;
  /** Level seed for the pebble field (any stable number per level). */
  seed: number;
  /** Announcer line (A6) for "Belt up!". */
  say?: ((trigger: "beltUp") => void) | undefined;
}

/** What the frame hands the renderer and the HUD. */
export interface RaceFrame {
  look: BeltLook | null;
  lights: PointLight[];
  /** Own dose rate (hull/s; 0 outside belts). */
  dose: number;
  /** Start lights state (the HUD's lamps). */
  lights5: StartLights;
  /** Tick the current heat's race started (0 unknown). */
  raceStart: number;
  /** Fractional server tick used this frame. */
  tick: number;
}

/** Rock colour of belt levels: sooty asteroids (RenderSettings.rockColor, the terrain's base albedo). */
export const BELT_ROCK: Rgb = [0.34, 0.33, 0.325];

/** Display colours. */
const PORTAL_GREEN: Rgb = [0.25, 1.0, 0.5];
const LAMP_RED: Rgb = [1.0, 0.06, 0.03];
const LAMP_OFF: Rgb = [0.18, 0.03, 0.03];
const LAMP_GREEN: Rgb = [0.25, 1.0, 0.35];

/** Sizzle colours (linear radiance). */
const SIZZLE: readonly [number, number, number] = [0.6, 3.2, 1.2];

function hash01(i: number, salt: number): number {
  let h = Math.imul(i ^ Math.imul(salt, 0x27d4eb2d), 0x9e3779b1) >>> 0;
  h ^= h >>> 15;
  h = Math.imul(h, 0x85ebca6b) >>> 0;
  h ^= h >>> 13;
  return (h >>> 0) / 4294967296;
}

/** Poisson count with mean `m` (small means; Knuth). */
function poisson(m: number, rnd: () => number): number {
  if (m <= 0) return 0;
  if (m > 30) return Math.round(m + Math.sqrt(m) * (rnd() + rnd() + rnd() - 1.5) * 1.4);
  const l = Math.exp(-m);
  let k = 0;
  let p = 1;
  do {
    k++;
    p *= rnd();
  } while (p > l);
  return k - 1;
}

/** The gantry pose over a slot: ahead of the ship and above, its lamps facing the ship. */
export function gantryPose(belt: Belt, slot: Vec3): { position: Vec3; rotation: Quat } {
  const a = azimuth(belt, slot);
  const ahead = prograde(a);
  const position = { x: slot.x + ahead.x * 7, y: slot.y + 2.4, z: slot.z + ahead.z * 7 };
  // The mesh's lamps face -Z: turn -Z toward the ship (backwards along the course).
  const rotation = q.lookRotation(v3.scale(ahead, -1), { x: 0, y: 1, z: 0 });
  return { position, rotation };
}

/** Scale of the gantry (u across). */
export const GANTRY_SCALE = 3.2;

/** Race presentation state across frames. */
export class RaceVisuals {
  private sink: RaceParticleSink | null;
  private engine: RaceSoundEngine | null;
  private readonly clock = new TickClock();
  private pebbles: PebbleField | null = null;
  private pebbleKey = "";
  private instances: { boulders: Instance[]; pebbles: Instance[]; gantries: Instance[]; lamps: Instance[] } = { boulders: [], pebbles: [], gantries: [], lamps: [] };
  private heat = -1;
  private phase = "";
  private lightsStart = 0;
  private raceStart = 0;
  private lastLit = 0;
  private wentGo = false;
  private ownGates = -1;
  private ownFinished = false;
  private geigerDebt = 0;
  private portalLoop: ReturnType<RaceSoundEngine["loop"]> = null;
  private wrongClock = 0;
  private radiation = 0;
  private readonly rnd: () => number;
  /** Ship sizzle clocks (particles per ship). */
  private readonly sizzle = new Map<number, number>();
  /** The HUD sets this each frame (wrong way warning for the buzzer). */
  wrongWay = false;

  constructor(sink: RaceParticleSink | null, engine: RaceSoundEngine | null = null, random: () => number = Math.random) {
    this.sink = sink;
    this.engine = engine;
    this.rnd = random;
  }

  reset(): void {
    this.clock.reset();
    this.pebbles = null;
    this.pebbleKey = "";
    this.heat = -1;
    this.phase = "";
    this.lightsStart = 0;
    this.raceStart = 0;
    this.lastLit = 0;
    this.wentGo = false;
    this.ownGates = -1;
    this.ownFinished = false;
    this.radiation = 0;
    this.sizzle.clear();
    this.portalLoop?.stop(0.3);
    this.portalLoop = null;
    this.instances = { boulders: [], pebbles: [], gantries: [], lamps: [] };
  }

  dispose(): void {
    this.portalLoop?.stop(0.2);
    this.portalLoop = null;
  }

  /** One frame: the belt look, lights, particles and sounds. */
  update(i: RaceFxInput): RaceFrame {
    const tick = this.clock.update(i.nowTick, i.time);
    const b = i.belt;
    const empty: RaceFrame = { look: null, lights: [], dose: 0, lights5: { show: false, lit: 0, go: false, sinceGo: 0 }, raceStart: 0, tick };
    this.instances = { boulders: [], pebbles: [], gantries: [], lamps: [] };
    if (!b) {
      this.portalLoop?.stop(0.3);
      this.portalLoop = null;
      return empty;
    }
    const race = i.race;
    this.track(race, i.nowTick);
    const own = race?.racers.find((r) => r.id === i.own) ?? null;
    const lights: PointLight[] = [];

    // Boulders, tumbling on their orbits.
    const poses = i.boulderPoses?.(tick);
    if (poses && poses.length >= 8) {
      for (let k = 0; k + 7 < poses.length; k += 8) {
        this.instances.boulders.push({
          position: { x: poses[k] ?? 0, y: poses[k + 1] ?? 0, z: poses[k + 2] ?? 0 },
          rotation: { x: poses[k + 3] ?? 0, y: poses[k + 4] ?? 0, z: poses[k + 5] ?? 0, w: poses[k + 6] ?? 1 },
          // The mesh is 1 u across: scale by the diameter.
          scale: 2 * (poses[k + 7] ?? 2),
          color: boulderColor(k / 8),
          emissive: 0,
        });
      }
    } else {
      b.boulders.forEach((o, k) => {
        const a = o.phase + (o.angularSpeed * tick) / TICK_HZ;
        this.instances.boulders.push({
          position: { x: b.center.x + o.radius * Math.cos(a), y: b.center.y + o.height, z: b.center.z - o.radius * Math.sin(a) },
          rotation: q.fromAxisAngle(o.tumbleAxis.y === 0 && o.tumbleAxis.x === 0 && o.tumbleAxis.z === 0 ? { x: 0, y: 1, z: 0 } : o.tumbleAxis, (o.tumbleSpeed * tick) / TICK_HZ + o.phase),
          scale: 2 * o.size,
          color: boulderColor(k),
          emissive: 0,
        });
      });
    }

    // Pebbles near the camera.
    const key = `${i.seed}:${b.center.x},${b.center.y},${b.center.z}`;
    if (key !== this.pebbleKey) {
      this.pebbleKey = key;
      this.pebbles = new PebbleField(b, i.seed);
    }
    this.pebbles?.instances(i.camera, q.forward(i.orientation), i.time, this.instances.pebbles);

    // Start lights over the racers' slots.
    const s5 = startLights(race, i.nowTick, this.lightsStart, this.raceStart);
    if (s5.show && race) {
      for (const r of race.racers) {
        const slot = b.slots[r.slot];
        if (!slot) continue;
        const g = gantryPose(b, slot.pos);
        this.instances.gantries.push({ position: g.position, rotation: g.rotation, scale: GANTRY_SCALE, color: [0.16, 0.17, 0.19], emissive: 0 });
        const fade = s5.go ? Math.max(0, 1 - s5.sinceGo / GO_SECS) : 1;
        GANTRY_LAMPS.forEach(([lx, ly, lz], n) => {
          const on = s5.go ? fade > 0.02 : n < s5.lit;
          // The gantry faces the racer: its local +x is the racer's left, so lamps fill left to right as seen from the slot.
          const pos = v3.add(g.position, q.rotate(g.rotation, { x: -lx * GANTRY_SCALE, y: ly * GANTRY_SCALE, z: lz * GANTRY_SCALE }));
          this.instances.lamps.push({
            position: pos,
            rotation: g.rotation,
            scale: GANTRY_LAMP_RADIUS * 2 * GANTRY_SCALE,
            color: on ? (s5.go ? LAMP_GREEN : LAMP_RED) : LAMP_OFF,
            emissive: on ? (s5.go ? 0.5 + 0.3 * fade : 0.55) : 0.12,
          });
        });
        const lit = s5.go ? 3 * fade : s5.lit;
        if (lit > 0) lights.push({ position: v3.add(g.position, v3.scale(prograde(azimuth(b, slot.pos)), -1.2)), color: s5.go ? LAMP_GREEN : LAMP_RED, intensity: 1.6 * lit, range: 14 });
      }
    }

    // Gates, exit and sign.
    const lapDone = own !== null && own.gates >= LAP_GATES;
    const portal = race ? (lapDone ? 1.25 : 0.85) + 0.1 * Math.sin(i.time * 3) : 0.85;
    const signHeight = b.exitRadius * 1.25 + 5 + SIGN_WORLD_HEIGHT / 2;
    const look: BeltLook = {
      center: b.center,
      innerRadius: b.innerRadius,
      outerRadius: b.outerRadius,
      halfThickness: b.halfThickness,
      exitRadius: b.exitRadius,
      cocoonRadius: b.cocoonRadius,
      gateInnerRadius: b.gateInnerRadius,
      gateOuterRadius: b.gateOuterRadius,
      dust: 1,
      dustColor: [1.0, 0.8, 0.58],
      aurora: 1,
      glints: 1,
      gates: gateLooks(own && own.finishTick === 0 && race?.phase !== "results" ? own : null, i.ownColor),
      gateStrength: race ? 1 : 0.8,
      portal,
      sign: 1,
      signHeight,
      swirl: lapDone ? 1.6 : 1,
      radiation: 0,
    };
    lights.push({ position: b.center, color: PORTAL_GREEN, intensity: 26 * portal, range: 40, priority: 2 });
    lights.push({ position: { x: b.center.x, y: b.center.y + signHeight, z: b.center.z }, color: PORTAL_GREEN, intensity: 18, range: 36, priority: 2 });

    // Radiation: the own dose (screen edges, Geiger), sizzling hulls.
    const dose = i.ownPose && i.doseAt ? Math.max(0, i.doseAt(i.ownPose.position)) : 0;
    const want = dose > 0.25 ? Math.min(1, 0.25 + dose / 10) : 0;
    this.radiation += (want - this.radiation) * (1 - Math.exp(-6 * i.dt));
    look.radiation = this.radiation < 0.01 ? 0 : this.radiation;
    this.sizzleShips(i);
    this.portalMotes(i, b);
    this.sounds(i, b, race, own, dose, s5);
    return { look, lights, dose, lights5: s5, raceStart: this.raceStart, tick };
  }

  /** Remembers when the lights and the race of each heat began. */
  private track(race: RaceState | null, nowTick: number): void {
    if (!race) return;
    if (race.heat !== this.heat) {
      this.heat = race.heat;
      this.lightsStart = 0;
      this.raceStart = 0;
      this.wentGo = false;
      this.lastLit = 0;
      this.ownGates = -1;
      this.ownFinished = false;
    }
    if (race.phase !== this.phase) {
      if (race.phase === "lights") this.lightsStart = this.phase === "" ? 0 : nowTick;
      if (race.phase === "racing" && this.phase === "lights") this.raceStart = nowTick;
      this.phase = race.phase;
    }
    if (race.phase === "lights") this.raceStart = race.phaseEndTick;
    if (race.phase === "racing" && this.raceStart === 0) this.raceStart = Math.max(1, race.phaseEndTick - 180 * TICK_HZ);
  }

  private sizzleShips(i: RaceFxInput): void {
    const sink = this.sink;
    if (!sink || !i.doseAt) return;
    for (const s of i.ships) {
      if (!s.alive) continue;
      const dose = i.doseAt(s.position);
      if (dose < 0.5) {
        this.sizzle.delete(s.id);
        continue;
      }
      const k = Math.min(1, dose / 12);
      // A crackling green shell.
      const flick = 0.55 + 0.45 * hash01(Math.floor(i.time * 24), s.id);
      sink.immediate({ pos: s.position, life: 1, sprite: SPRITE.glow, color: [SIZZLE[0] * 0.12 * k * flick, SIZZLE[1] * 0.12 * k * flick, SIZZLE[2] * 0.12 * k * flick], size: s.radius * 4.2 });
      // Sparks off the hull, more with the dose.
      let debt = (this.sizzle.get(s.id) ?? 0) + i.dt * (6 + 40 * k);
      while (debt >= 1) {
        debt -= 1;
        const d = v3.normalize({ x: this.rnd() - 0.5, y: this.rnd() - 0.5, z: this.rnd() - 0.5 });
        const p = v3.addScaled(s.position, d, s.radius * (0.7 + 0.5 * this.rnd()));
        sink.spawn({ pos: p, vel: v3.scale(d, 2 + 5 * this.rnd()), life: 0.18 + 0.25 * this.rnd(), sprite: SPRITE.spark, color: SIZZLE, color1: [0.2, 0.8, 0.3], size: 0.05 + 0.05 * k, orient: "velocity", stretch: 0.05, drag: 3 });
        if (this.rnd() < 0.12) sink.spawn({ pos: p, life: 0.1, sprite: SPRITE.flare, color: [1.2, 3.6, 1.6], size: 0.12 + 0.14 * k, spin: 4, rot: this.rnd() * 6 });
      }
      this.sizzle.set(s.id, debt);
    }
  }

  /** Motes spiralling into the exit portal (immediate sprites, cheap). */
  private portalMotes(i: RaceFxInput, b: Belt): void {
    const sink = this.sink;
    if (!sink) return;
    if (v3.length(v3.sub(i.camera, b.center)) > 260) return;
    const n = 70;
    for (let k = 0; k < n; k++) {
      const ph = hash01(k, 91);
      const life = 3.5;
      const age = ((i.time / life + ph) % 1 + 1) % 1;
      const r = b.exitRadius * (1.25 + 4.5 * (1 - age) * (1 - age));
      const a = ph * 40 + age * 9 + i.time * 0.4;
      const tilt = (hash01(k, 92) - 0.5) * 0.9;
      const p = {
        x: b.center.x + r * Math.cos(a),
        y: b.center.y + r * Math.sin(a) * Math.sin(tilt) + (hash01(k, 93) - 0.5) * 2 * (1 - age),
        z: b.center.z + r * Math.sin(a) * Math.cos(tilt),
      };
      const glow = Math.sin(age * Math.PI);
      sink.immediate({ pos: p, life: 1, sprite: SPRITE.glow, color: [0.4 * glow, 2.6 * glow, 1.1 * glow], size: 0.16 + 0.14 * glow });
    }
  }

  private sounds(i: RaceFxInput, b: Belt, race: RaceState | null, own: Racer | null, dose: number, s5: StartLights): void {
    const e = this.engine;
    if (!e) return;
    // The portal hums where it is.
    if (!this.portalLoop || !this.portalLoop.playing) this.portalLoop = e.loop("race.portal", { position: b.center, gain: 0.8 });
    if (i.quiet || !race) return;
    // Start lights: a beep per lamp, the go tone and the announcer.
    if (race.phase === "lights") {
      if (s5.lit > this.lastLit) for (let n = this.lastLit; n < s5.lit; n++) e.play("race.lightOn");
      this.lastLit = s5.lit;
      if (this.wentGo) this.wentGo = false;
    }
    if (s5.go && !this.wentGo && s5.sinceGo < 0.5) {
      this.wentGo = true;
      e.play("race.go");
      i.say?.("beltUp");
    }
    if (race.phase !== "lights" && !s5.go) this.lastLit = 0;
    // Gates and the finish of the own racer.
    if (own) {
      if (this.ownGates >= 0 && own.gates > this.ownGates) {
        if (own.gates >= LAP_GATES) e.play("race.lapDone");
        else e.play("race.gate", { rate: 1 + 0.045 * own.gates });
      }
      this.ownGates = own.gates;
      if (own.finishTick > 0 && !this.ownFinished) {
        this.ownFinished = true;
        e.play("race.finish");
      }
    }
    // Wrong way: a buzzer every 1.2 s.
    if (this.wrongWay) {
      this.wrongClock -= i.dt;
      if (this.wrongClock <= 0) {
        e.play("race.wrongWay");
        this.wrongClock = 1.2;
      }
    } else this.wrongClock = 0;
    // Geiger: clicks at a rate growing with the dose (Poisson, spread over the frame).
    if (dose > 0.2) {
      const rate = Math.min(70, 3 + dose * 5.5);
      this.geigerDebt += rate * i.dt;
      const n = poisson(this.geigerDebt, this.rnd);
      this.geigerDebt = 0;
      for (let k = 0; k < Math.min(n, 6); k++) e.play("race.geiger", { delay: this.rnd() * i.dt, gain: 0.6 + 0.4 * Math.min(1, dose / 10) });
    }
  }

  /** Adds this frame's boulders, pebbles, gantries and lamps to the scene. */
  addToScene(scene: SceneInstances): void {
    const { boulders, pebbles, gantries, lamps } = this.instances;
    if (boulders.length > 0) (scene.opaque.boulder ??= []).push(...boulders);
    if (pebbles.length > 0) (scene.opaque.pebble ??= []).push(...pebbles);
    if (gantries.length > 0) (scene.opaque.gantry ??= []).push(...gantries);
    if (lamps.length > 0) (scene.opaque.orb ??= []).push(...lamps);
  }

  /** Counts of this frame's instances (tests, gallery status). */
  get counts(): { boulders: number; pebbles: number; gantries: number; lamps: number } {
    const { boulders, pebbles, gantries, lamps } = this.instances;
    return { boulders: boulders.length, pebbles: pebbles.length, gantries: gantries.length, lamps: lamps.length };
  }
}

/** Sooty boulder colours with a little variation. */
function boulderColor(k: number): Rgb {
  const s = hash01(k, 61);
  const g = 0.27 + 0.1 * s;
  return [g * 1.04, g, g * 0.93];
}

/** Gate azimuth helper for the HUD (re-exported for one import site). */
export { gateAzimuth };
