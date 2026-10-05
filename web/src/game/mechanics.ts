// Client mechanics UI (M2 wave C) as plain logic: game cues from ls-wasm
// (`Game.takeCues()`, crates/ls-wasm/src/mechanics.rs) turn into timed
// effects (Flash Pod whiteout, screen shake by Tectonic zone, "excuse me"
// and crack-warning banners, Leech Arc beams, muzzle flashes, decoy returns,
// overcharge), plus the charge meter, gadget line, paint colour, eyeball
// camera and the ship selection panel's model. game/scene.ts and the ui/
// modules draw what this produces; Node tests cover it.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { Rgb } from "../renderer/renderer.ts";
import { CHARGE_LEN, CUE_STRIDE } from "../wasm/api.ts";
import type { LocalPose, ProjectileState, ShipState } from "./views.ts";

/** `ls_sim::CueKind` bytes. */
export const CUE = {
  politeWarning: 0,
  blinded: 1,
  chargeStart: 2,
  tectonicLaunch: 3,
  tectonicArmed: 4,
  shockwave: 5,
  beam: 6,
  swapped: 7,
  rewound: 8,
  muzzleFlash: 9,
  gadget: 10,
  decoyReturn: 11,
  overcharge: 12,
  paintColor: 13,
} as const;

/** Ship id meaning "none" in cues. */
export const NO_SHIP = 255;
/** Simulation ticks per second. */
export const TICK_HZ = 60;

export interface Cue {
  kind: number;
  source: number;
  target: number;
  position: Vec3;
  value: number;
  tick: number;
}

/** Decodes `Game.takeCues()` ([kind, source, target, px, py, pz, value, tick] each). */
export function parseCues(a: ArrayLike<number> | undefined): Cue[] {
  const out: Cue[] = [];
  if (!a) return out;
  for (let i = 0; i + CUE_STRIDE <= a.length; i += CUE_STRIDE) {
    out.push({
      kind: a[i] ?? 0,
      source: a[i + 1] ?? NO_SHIP,
      target: a[i + 2] ?? NO_SHIP,
      position: { x: a[i + 3] ?? 0, y: a[i + 4] ?? 0, z: a[i + 5] ?? 0 },
      value: a[i + 6] ?? 0,
      tick: a[i + 7] ?? 0,
    });
  }
  return out;
}

// ----- effects ----------------------------------------------------------------

/** A timed banner line ("Excuse me…", crack warning). */
export interface Banner {
  id: string;
  text: string;
  /** Visual class: "warning" (red), "info", "danger". */
  tone: "warning" | "info" | "danger";
  /** Seconds left of a countdown to show, or null. */
  countdown: number | null;
}

/** A beam to draw this frame (Leech Arc). */
export interface Beam {
  from: Vec3;
  to: Vec3;
  color: Rgb;
  /** 0..1 fade. */
  strength: number;
}

/** A glow flash or ring to draw this frame. */
export interface Flash {
  position: Vec3;
  color: Rgb;
  /** World radius (u). */
  radius: number;
  /** 0..1. */
  strength: number;
  /** Expanding shell (shockwave) rather than a glow ball. */
  ring: boolean;
}

export interface EffectsFrame {
  /** Flash Pod whiteout 0..1 over the whole screen. */
  whiteout: number;
  /** Camera shake amplitude (u of camera offset). */
  shake: number;
  banners: Banner[];
  beams: Beam[];
  flashes: Flash[];
  /** Seconds until the weapons are back after a Borrowed Time rewind (0 = online). */
  weaponsOffline: number;
  /** The own Fusion Charger burns hull (overcharge cue since the last charge start). */
  overcharging: boolean;
}

interface Timed {
  start: number;
  end: number;
}

interface TimedBeam extends Timed {
  source: number;
  /** Locked ship (NO_SHIP = none): the beam follows it. */
  target: number;
  to: Vec3;
}

/** The own Leech Arc beam drawn locally (online; `Game.localBeam()`, crates/ls-wasm/src/beam.rs). */
export interface LocalBeam {
  from: Vec3;
  to: Vec3;
  /** Locked ship id, NO_SHIP for none. */
  target: number;
}

/** Decodes `Game.localBeam()` (`[from x, y, z, to x, y, z, target]`; empty = no beam). */
export function parseLocalBeam(a: ArrayLike<number> | undefined): LocalBeam | null {
  if (!a || a.length < 7) return null;
  return {
    from: { x: a[0] ?? 0, y: a[1] ?? 0, z: a[2] ?? 0 },
    to: { x: a[3] ?? 0, y: a[4] ?? 0, z: a[5] ?? 0 },
    target: a[6] ?? NO_SHIP,
  };
}

interface TimedFlash extends Timed {
  position: Vec3;
  color: Rgb;
  radius: number;
  ring: boolean;
}

interface TimedShake extends Timed {
  amplitude: number;
}

interface TimedBanner extends Timed {
  banner: Omit<Banner, "countdown">;
  countdownEnd: number | null;
}

/** Whiteout fades out over this many seconds at the end of a blinding. */
export const WHITEOUT_FADE = 0.6;
/** Leech Arc beam colour. */
export const BEAM_COLOR: Rgb = [0.45, 1, 1];

/** Tectonic Charge design zones (u): core kill, damage, collapse, shockwave. */
export const TECTONIC_ZONES = { core: 4, damage: 25, collapse: 35, shockwave: 60 } as const;

/**
 * Screen shake (camera offset amplitude, u) from a shockwave of `radius` at
 * `distance` from the camera, by the Tectonic Charge zones (scaled to the
 * radius, so smaller shockwaves shake the same way nearer in): core 0.6,
 * damage zone 0.4, collapse zone 0.25, shockwave zone 0.12; a level-wide
 * jostle (0.04) beyond it for a Tectonic-sized (>= 40 u) shockwave.
 */
export function shockwaveShake(distance: number, radius: number): number {
  if (!(radius > 0)) return 0;
  const k = radius / TECTONIC_ZONES.shockwave;
  if (distance <= TECTONIC_ZONES.core * k) return 0.6;
  if (distance <= TECTONIC_ZONES.damage * k) return 0.4;
  if (distance <= TECTONIC_ZONES.collapse * k) return 0.25;
  if (distance <= radius) return 0.12;
  return radius >= 40 ? 0.04 : 0;
}

/** Which zone of a Tectonic Charge a distance falls in (for the crack warning). */
export function tectonicZone(distance: number): "core" | "damage" | "collapse" | "shockwave" | "outside" {
  if (distance <= TECTONIC_ZONES.core) return "core";
  if (distance <= TECTONIC_ZONES.damage) return "damage";
  if (distance <= TECTONIC_ZONES.collapse) return "collapse";
  if (distance <= TECTONIC_ZONES.shockwave) return "shockwave";
  return "outside";
}

function dist(a: Vec3, b: Vec3): number {
  return Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z);
}

/** Where ships are, for beams and distance checks. */
export interface EffectsContext {
  /** Local player's ship id. */
  player: number;
  /** Camera (or own ship) position. */
  camera: Vec3;
  /** Position of a ship id (own ship included), if known. */
  shipPos: (id: number) => Vec3 | undefined;
  /** Seconds the own weapons stay offline (Borrowed Time, from ls-wasm); missing or 0 = the design's 3 s. */
  weaponsOfflineSecs?: number;
}

/**
 * Timed effects from cues. `apply` feeds the cues of a frame at wall-clock
 * `now` (seconds); `frame` samples what to draw at `now`.
 */
export class EffectsState {
  private blindEnd = 0;
  private blindStart = 0;
  private offlineEnd = 0;
  private overcharge = false;
  private beams: TimedBeam[] = [];
  private flashes: TimedFlash[] = [];
  private shakes: TimedShake[] = [];
  private banners = new Map<string, TimedBanner>();

  apply(cues: readonly Cue[], now: number, ctx: EffectsContext): void {
    for (const c of cues) this.applyOne(c, now, ctx);
  }

  private banner(id: string, text: string, tone: Banner["tone"], now: number, secs: number, countdown = false): void {
    this.banners.set(id, {
      start: now,
      end: now + secs,
      banner: { id, text, tone },
      countdownEnd: countdown ? now + secs : null,
    });
  }

  private flash(position: Vec3, color: Rgb, radius: number, now: number, secs: number, ring = false): void {
    this.flashes.push({ start: now, end: now + secs, position, color, radius, ring });
  }

  private shake(amplitude: number, now: number, secs: number): void {
    if (amplitude > 0) this.shakes.push({ start: now, end: now + secs, amplitude });
  }

  private applyOne(c: Cue, now: number, ctx: EffectsContext): void {
    const me = ctx.player;
    const secs = c.value / TICK_HZ;
    const near = dist(c.position, ctx.camera);
    switch (c.kind) {
      case CUE.politeWarning:
        if (c.target === me) this.banner("polite", "Excuse me… (Polite Laser locked on you)", "warning", now, Math.max(secs, 0.8), true);
        break;
      case CUE.blinded:
        if (c.target === me) {
          if (now >= this.blindEnd) this.blindStart = now;
          this.blindEnd = Math.max(this.blindEnd, now + secs);
        }
        this.flash(c.position, [1, 1, 1], 3, now, 0.25);
        break;
      case CUE.chargeStart:
        if (c.source === me) this.overcharge = false;
        break;
      case CUE.tectonicLaunch:
        this.banner("tectonic-launch", "Tectonic Charge launched", "danger", now, 3);
        this.shake(0.05, now, 1.5);
        break;
      case CUE.tectonicArmed: {
        // The zone of the own ship (the camera may be riding an Eyeball).
        const own = ctx.shipPos(me);
        const zone = tectonicZone(own ? dist(c.position, own) : near);
        const where =
          zone === "outside" ? "" : zone === "core" ? " (you are at the core!)" : ` (you are in the ${zone} zone)`;
        this.banner("tectonic-armed", `Ground cracking: Tectonic Charge${where}`, "danger", now, Math.max(secs, 0.5), true);
        this.flash(c.position, [1, 0.35, 0.1], 4, now, Math.max(secs, 0.5));
        this.shake(zone === "outside" ? 0.03 : 0.08, now, Math.max(secs, 0.5));
        break;
      }
      case CUE.shockwave:
        this.shake(shockwaveShake(near, c.value), now, c.value >= 40 ? 1.2 : 0.5);
        this.flash(c.position, [1, 0.85, 0.6], c.value, now, c.value >= 40 ? 1.0 : 0.4, true);
        break;
      case CUE.beam:
        this.beams.push({ start: now, end: now + Math.max(secs, 0.05), source: c.source, target: c.target, to: c.position });
        break;
      case CUE.swapped:
        if (c.source === me || c.target === me) this.banner("swap", "Swapped places!", "info", now, 1.5);
        this.flash(c.position, [0.8, 0.5, 1], 2, now, 0.4);
        {
          const other = ctx.shipPos(c.target);
          if (other) this.flash(other, [0.8, 0.5, 1], 2, now, 0.4);
        }
        break;
      case CUE.rewound:
        if (c.target === me) {
          // The cue's value is the rewind; the offline time is the weapon's.
          const offline = ctx.weaponsOfflineSecs && ctx.weaponsOfflineSecs > 0 ? ctx.weaponsOfflineSecs : 3;
          this.offlineEnd = now + offline;
          this.banner("rewound", `Rewound ${Math.round(secs)} s · weapons offline`, "info", now, offline, true);
        }
        this.flash(c.position, [0.6, 0.9, 1], 2.5, now, 0.6);
        break;
      case CUE.muzzleFlash:
        this.flash(c.position, [1, 0.95, 0.75], 1.4, now, 0.12);
        if (c.source !== me) this.shake(near < 20 ? 0.02 : 0, now, 0.1);
        break;
      case CUE.gadget:
        break;
      case CUE.decoyReturn:
        if (c.source === me) this.banner("decoy", "Decoy snapping back", "info", now, 1.5);
        this.flash(c.position, [1, 0.8, 0.3], 2.5, now, 0.5);
        break;
      case CUE.overcharge:
        if (c.source === me) {
          this.overcharge = true;
          this.banner("overcharge", "Overcharge! The Fusion Charger burns your hull", "danger", now, 2);
        }
        {
          const at = ctx.shipPos(c.source) ?? c.position;
          this.flash(at, [1, 0.3, 0.1], 1.6, now, 0.5);
        }
        break;
      default:
        break;
    }
  }

  /** Stops the overcharge warning (the charge ended). */
  clearOvercharge(): void {
    this.overcharge = false;
  }

  /**
   * What to draw at `now` (drops finished effects). `local` is the own
   * Leech Arc beam predicted by ls-wasm (online), drawn at full strength
   * from the own ship to its lock.
   */
  frame(now: number, ctx: EffectsContext, local: LocalBeam | null = null): EffectsFrame {
    const alive = <T extends Timed>(xs: T[]): T[] => xs.filter((x) => x.end > now);
    this.beams = alive(this.beams);
    this.flashes = alive(this.flashes);
    this.shakes = alive(this.shakes);
    for (const [id, b] of this.banners) if (b.end <= now) this.banners.delete(id);

    let whiteout = 0;
    if (now < this.blindEnd) {
      const left = this.blindEnd - now;
      whiteout = left < WHITEOUT_FADE ? left / WHITEOUT_FADE : 1;
      // A quick flash-in over the first 60 ms.
      whiteout *= Math.min(1, (now - this.blindStart) / 0.06 + 0.3);
      whiteout = Math.max(0, Math.min(1, whiteout));
    }
    let shake = 0;
    for (const s of this.shakes) {
      const f = (s.end - now) / Math.max(1e-6, s.end - s.start);
      shake = Math.max(shake, s.amplitude * f);
    }
    const beams: Beam[] = [];
    for (const b of this.beams) {
      const from = ctx.shipPos(b.source);
      if (!from) continue;
      // A locked beam stays on its target as it moves.
      const to = (b.target !== NO_SHIP ? ctx.shipPos(b.target) : undefined) ?? b.to;
      beams.push({ from, to, color: BEAM_COLOR, strength: (b.end - now) / Math.max(1e-6, b.end - b.start) });
    }
    if (local) {
      const from = ctx.shipPos(ctx.player) ?? local.from;
      const to = (local.target !== NO_SHIP ? ctx.shipPos(local.target) : undefined) ?? local.to;
      beams.push({ from, to, color: BEAM_COLOR, strength: 1 });
    }
    const flashes: Flash[] = this.flashes.map((f) => {
      const t = (now - f.start) / Math.max(1e-6, f.end - f.start);
      return {
        position: f.position,
        color: f.color,
        radius: f.ring ? f.radius * Math.min(1, 0.15 + t) : f.radius,
        strength: 1 - t,
        ring: f.ring,
      };
    });
    const banners: Banner[] = [...this.banners.values()].map((b) => ({
      ...b.banner,
      countdown: b.countdownEnd === null ? null : Math.max(0, b.countdownEnd - now),
    }));
    return {
      whiteout,
      shake,
      banners,
      beams,
      flashes,
      weaponsOffline: Math.max(0, this.offlineEnd - now),
      overcharging: this.overcharge,
    };
  }
}

/** Deterministic camera shake offset (u) for amplitude `a` at time `t` (s), along the camera axes. */
export function shakeOffset(a: number, t: number): { x: number; y: number } {
  if (!(a > 0)) return { x: 0, y: 0 };
  return {
    x: a * (Math.sin(t * 71.3) * 0.6 + Math.sin(t * 37.1 + 1.3) * 0.4),
    y: a * (Math.sin(t * 63.7 + 0.7) * 0.6 + Math.sin(t * 29.3 + 2.1) * 0.4),
  };
}

// ----- charge meter, gadget, paint colour -------------------------------------------

export interface ChargeMeter {
  weapon: number;
  /** 0..1 of full charge. */
  fraction: number;
  heldSecs: number;
  fullSecs: number;
  /** Held long enough to fire on release. */
  ready: boolean;
  autoRelease: boolean;
  overcharge: boolean;
}

/** Decodes `Game.charge()` (empty = not charging). */
export function parseCharge(a: ArrayLike<number> | undefined): ChargeMeter | null {
  if (!a || a.length < CHARGE_LEN || !a[7]) return null;
  const heldSecs = a[2] ?? 0;
  return {
    weapon: a[0] ?? 0,
    fraction: Math.max(0, Math.min(1, a[1] ?? 0)),
    heldSecs,
    fullSecs: a[3] ?? 0,
    ready: heldSecs + 1e-6 >= (a[4] ?? 0),
    autoRelease: (a[5] ?? 0) !== 0,
    overcharge: (a[6] ?? 0) !== 0,
  };
}

/** Text next to the charge meter. */
export function chargeLabel(name: string, c: ChargeMeter): string {
  if (c.overcharge) return `${name}: OVERCHARGE, release!`;
  if (c.fraction >= 1) return c.autoRelease ? `${name}: firing` : `${name}: full, release to fire`;
  if (!c.ready) return `${name}: charging…`;
  return `${name}: ${Math.round(c.fraction * 100)}%`;
}

export interface GadgetState {
  id: number;
  key: string;
  name: string;
  active: boolean;
  toggle: boolean;
  cooldownLeft: number;
  cooldownSecs: number;
  energyCost: number;
  drainPerSec: number;
}

export function emptyGadget(): GadgetState {
  return { id: 255, key: "", name: "", active: false, toggle: false, cooldownLeft: 0, cooldownSecs: 0, energyCost: 0, drainPerSec: 0 };
}

/** HUD text for the gadget: "on", "off", "ready", "3.2 s". */
export function gadgetDetail(g: GadgetState): string {
  if (!g.name) return "";
  if (g.active) return "on";
  if (g.cooldownLeft > 0.05) return `${g.cooldownLeft.toFixed(1)} s`;
  return g.toggle ? "off" : "ready";
}

/** Paint effect colour ids (ls_math::terrain::PAINT_*). */
export const PAINT = { plain: 0, red: 1, blue: 2, yellow: 3 } as const;

/** Paint colours for the terrain shader and HUD, by id. */
export const PAINT_COLORS: readonly Rgb[] = [
  [1, 1, 1],
  [0.95, 0.18, 0.15],
  [0.2, 0.45, 1.0],
  [1.0, 0.85, 0.15],
];

/** HUD name and effect of a paint colour id. */
export function paintColorLabel(id: number): string {
  switch (id) {
    case PAINT.red:
      return "Red · bouncy (stripes)";
    case PAINT.blue:
      return "Blue · slippery (waves)";
    case PAINT.yellow:
      return "Yellow · sticky (dots)";
    default:
      return "Plain";
  }
}

// ----- eyeball camera ------------------------------------------------------------

/** ls-sim `proj_flags::CONTROLS_OWNER` (replicated state bit of a guided projectile). */
export const PROJ_GUIDED = 1 << 3;

/** The local player's guided projectile (Eyeball), if one is in flight. */
export function guidedProjectile(shots: readonly ProjectileState[], player: number): ProjectileState | null {
  return shots.find((p) => p.owner === player && (p.state & PROJ_GUIDED) !== 0) ?? null;
}

/**
 * Camera pose riding a guided projectile: at its position, looking along its
 * velocity, keeping the ship's up (or the previous camera's up) as the roll
 * reference.
 */
export function eyeballPose(p: ProjectileState, upHint: Vec3): { position: Vec3; orientation: Quat } {
  const v = p.velocity;
  const speed = Math.hypot(v.x, v.y, v.z);
  if (speed < 1e-6) return { position: p.position, orientation: q.identity() };
  const f = { x: v.x / speed, y: v.y / speed, z: v.z / speed };
  const d = f.x * upHint.x + f.y * upHint.y + f.z * upHint.z;
  const hint = Math.abs(d) > 0.99 ? { x: 0, y: 0, z: 1 } : upHint;
  return { position: p.position, orientation: q.lookRotation(f, hint) };
}

// ----- cloak and shield ------------------------------------------------------------

/** Which gadget a player's gadget id is ("shield", "cloak", "decoy" or ""), from `gadgetIds()`. */
export function gadgetKey(gadgetIds: readonly string[], id: number): string {
  return gadgetIds[id] ?? "";
}

// ----- ship selection ---------------------------------------------------------------

export interface ShipChoice {
  shipType: number;
  name: string;
  allowed: boolean;
  current: boolean;
  next: boolean;
  hull: number;
  topSpeedPct: number;
  damageMult: number;
  secondarySlots: number;
  radius: number;
  turnRate: number;
  canPassScrap: boolean;
  primaries: string[];
  secondaries: [string, number][];
  gadget: string;
}

/** Parses `Game.shipChoices()` (JSON); invalid input gives an empty list. */
export function parseShipChoices(json: string | undefined): ShipChoice[] {
  if (!json) return [];
  try {
    const v: unknown = JSON.parse(json);
    if (!Array.isArray(v)) return [];
    return v.filter((x): x is ShipChoice => typeof x === "object" && x !== null && typeof (x as ShipChoice).name === "string");
  } catch {
    return [];
  }
}

/** One-line role of each ship type (design doc). */
export const SHIP_ROLES: readonly string[] = [
  "Hit and run: fastest, smallest, fragile",
  "All-rounder",
  "Heavy: tough, slow, most secondaries",
  "Support: passes scrap to teammates",
];

/** Lines for one ship card: stats and loadout preview. */
export function shipCardLines(c: ShipChoice): string[] {
  const secondaries = c.secondaries.map(([n, k]) => `${n} ×${k}`).join(", ") || "none";
  const lines = [
    `Hull ${Math.round(c.hull)} · speed ${Math.round(c.topSpeedPct)}% of laser · damage ×${c.damageMult}`,
    `${c.secondarySlots} secondary slot${c.secondarySlots === 1 ? "" : "s"}${c.canPassScrap ? " · passes scrap (hold R by a teammate)" : ""}`,
    `Primaries: ${c.primaries.join(", ") || "none"}`,
    `Secondaries: ${secondaries}`,
  ];
  if (c.gadget) lines.push(`Gadget: ${c.gadget}`);
  return lines;
}

/** The ship type a number key (1-4) picks, if that type is allowed. */
export function shipForKey(choices: readonly ShipChoice[], key: number): ShipChoice | null {
  const c = choices[key - 1];
  return c && c.allowed ? c : null;
}

// ----- own ship helpers -----------------------------------------------------------

/** Position of a ship id: the local pose for the player, else the remote ship. */
export function shipPositionLookup(
  player: number,
  pose: LocalPose | null,
  ships: readonly ShipState[],
): (id: number) => Vec3 | undefined {
  return (id) => {
    if (id === NO_SHIP) return undefined;
    if (id === player) return pose?.alive ? pose.position : undefined;
    const s = ships.find((x) => x.id === id);
    return s?.alive ? s.position : undefined;
  };
}
