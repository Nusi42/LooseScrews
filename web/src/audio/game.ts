// In-game sound (M4 A6): turns what happens each frame into sounds and
// announcer lines. main.ts creates one `GameAudio` per match and calls
// `update` once per frame with the frame's state; everything here is
// cosmetic and client-side.
//
// Sources (like game/vfx.ts, whose shape table decides impact kinds):
// - projectiles (no ids on the wire) matched frame to frame by owner, visual
//   and predicted position: a new one near its owner is a shot (fire sound
//   of its shape, lower for bigger shots), one that disappears hit rock
//   (impact or explosion of its shape) or a ship (the hit cue's clang);
// - ships: deaths (explosion; your own with a power-down), your respawn,
//   other ships kicking in their boost; your boost as a loop;
// - cues: hits (clang, your own heavier), Tectonic launch, rumble and
//   blast, shockwaves, the Polite chime, charge-ups, swaps, rewinds, Flash
//   Pods, gadgets, decoys, overcharge, the Leech Arc;
// - pickups that vanish next to your ship;
// - kills (deaths with the last attacker) for first blood and multi-kills,
//   and the match start, spoken by the announcer (ui/announcer.ts).

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import { CUE, NO_SHIP, TICK_HZ, type Cue } from "../game/mechanics.ts";
import { CUE_HIT, explosionRadius, shapeVfx } from "../game/vfx.ts";
import type { EntityState, LocalPose, ProjectileState, ShipState, VisualDef } from "../game/views.ts";
import type { AudioEngine, LoopHandle } from "./engine.ts";
import type { Trigger, LineVars } from "./announcer/lines.ts";

/** Shots and impacts farther than this from the camera make no sound (u). */
export const HEAR_RANGE = 240;
/** Seconds between kills that still chain into a multi-kill. */
export const MULTI_KILL_SECS = 4;
/** A death counts as a kill of the last attacker within this time (s). */
const KILL_CREDIT_SECS = 3;

/** Fire sound and pitch of a projectile shape. */
export function fireSound(shape: string, scale: number): { id: string; rate: number } | null {
  // Bigger shots sound lower: rate falls with the visual scale.
  const rate = Math.pow(Math.max(0.4, scale), -0.35);
  switch (shape) {
    case "bolt":
      return { id: "fire.bolt", rate };
    case "orb":
      return { id: "fire.orb", rate };
    case "arc":
      return { id: "fire.arc", rate };
    case "pellet":
      return { id: "fire.pellet", rate };
    case "harpoon":
      return { id: "fire.harpoon", rate: 1 };
    case "rocket":
      return { id: "fire.rocket", rate };
    case "missile":
      return { id: "fire.rocket", rate: rate * 1.18 };
    case "mine":
    case "grenade":
      return { id: "fire.lob", rate };
    case "charge":
      return null; // the Tectonic launch cue plays it
    case "paint":
      return { id: "fire.paint", rate };
    case "blob":
      return { id: "fire.sneeze", rate: 1 };
    case "spike":
      return null; // the Railspike's muzzle-flash cue plays it
    default:
      return { id: "fire.bolt", rate };
  }
}

/** Impact sound of a projectile shape hitting rock (null = silent). */
export function impactSound(visual: VisualDef | undefined): string | null {
  const fx = shapeVfx(visual);
  switch (fx.impact) {
    case "explosion":
      return explosionRadius(fx, visual?.scale ?? 1) >= 3.5 ? "explosion.big" : "explosion.small";
    case "sparks":
      return "impact.sparks";
    case "plasma":
      return "impact.plasma";
    case "splat":
      return "impact.splat";
    case "thud":
      return "impact.thud";
    default:
      return null;
  }
}

/** What the announcer needs from the game audio. */
export interface AnnouncerSink {
  say(trigger: Trigger, vars?: LineVars): void;
}

export interface AudioFrame {
  time: number;
  dt: number;
  player: number;
  online: boolean;
  /** Match flow stage (game/matchFlow.ts): warmup, countdown, match, results. */
  stage: string;
  camera: { position: Vec3; orientation: Quat };
  pose: LocalPose | null;
  ships: readonly ShipState[];
  projectiles: readonly ProjectileState[];
  entities: readonly EntityState[];
  cues: readonly Cue[];
  visuals: readonly VisualDef[];
  /** The own ship boosts this frame (input held and boost left). */
  boosting: boolean;
  /** Gadget key of a ship ("shield", "cloak", "" for none). */
  gadgetOf: (id: number) => string;
  /** Gadget string ids by gadget id (cue values). */
  gadgetIds: readonly string[];
  /** Player names (announcer lines). */
  names: ReadonlyMap<number, string>;
}

interface ShotTrack {
  owner: number;
  visual: number;
  pos: Vec3;
  vel: Vec3;
  matched: boolean;
}

interface ShipTrack {
  pos: Vec3;
  alive: boolean;
  boosting: boolean;
  radius: number;
}

const dist = (a: Vec3, b: Vec3): number => Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z);
const madd = (a: Vec3, b: Vec3, k: number): Vec3 => ({ x: a.x + b.x * k, y: a.y + b.y * k, z: a.z + b.z * k });
const len = (a: Vec3): number => Math.hypot(a.x, a.y, a.z);

export class GameAudio {
  private readonly engine: AudioEngine;
  private readonly announcer: AnnouncerSink | null;
  private shots: ShotTrack[] = [];
  private ships = new Map<number, ShipTrack>();
  private pickups = new Map<number, EntityState>();
  private primed = false;
  private boostLoop: LoopHandle | null = null;
  private boostLevel = 0;
  private rumble: { handle: LoopHandle; start: number; end: number } | null = null;
  private lastHit = new Map<number, { source: number; time: number }>();
  private killChains = new Map<number, { count: number; time: number }>();
  private kills = 0;
  private prevStage: string | null = null;

  constructor(engine: AudioEngine, announcer: AnnouncerSink | null = null) {
    this.engine = engine;
    this.announcer = announcer;
  }

  /** Forgets everything tracked (a new match on the same connection). */
  reset(): void {
    this.shots = [];
    this.ships.clear();
    this.pickups.clear();
    this.primed = false;
    this.lastHit.clear();
    this.killChains.clear();
    this.kills = 0;
    this.prevStage = null;
    this.rumble?.handle.stop(0.3);
    this.rumble = null;
  }

  update(f: AudioFrame): void {
    const e = this.engine;
    e.setListener(f.camera.position, q.forward(f.camera.orientation), q.up(f.camera.orientation));
    this.stage(f);
    const shipPos = (id: number): Vec3 | undefined => (id === f.player ? f.pose?.position : this.ships.get(id)?.pos);
    this.updateShips(f);
    this.applyCues(f, shipPos);
    this.updateShots(f, shipPos);
    this.updatePickups(f);
    this.updateBoost(f);
    this.updateRumble(f);
    this.primed = true;
  }

  private at(f: AudioFrame, p: Vec3): boolean {
    return dist(p, f.camera.position) <= HEAR_RANGE;
  }

  private stage(f: AudioFrame): void {
    if (f.stage === "match" && this.prevStage !== "match" && (this.prevStage === "countdown" || this.prevStage === "warmup" || (!f.online && this.prevStage === null))) {
      this.announcer?.say("matchStart");
    }
    if (f.stage === "results" && this.prevStage === "match") {
      this.engine.stopAll(0.6, "effects");
      this.boostLoop = null;
      this.rumble = null;
    }
    this.prevStage = f.stage;
  }

  // ----- ships --------------------------------------------------------------------------

  private updateShips(f: AudioFrame): void {
    const all: ShipState[] = [...f.ships];
    if (f.pose) all.push({ id: f.player, team: 0, alive: f.pose.alive, boosting: f.boosting, position: f.pose.position, orientation: f.pose.orientation, shipType: 1, radius: 0, gadgetActive: false });
    for (const s of all) {
      const own = s.id === f.player;
      const t = this.ships.get(s.id);
      if (t && this.primed) {
        if (t.alive && !s.alive) this.died(f, s.id, t.pos, own);
        if (!t.alive && s.alive && own) this.engine.play("ship.respawn");
        if (!own && s.alive && s.boosting && !t.boosting && this.at(f, s.position)) this.engine.play("boost.start", { position: s.position });
      }
      this.ships.set(s.id, { pos: s.position, alive: s.alive, boosting: s.boosting, radius: s.radius || 1 });
    }
  }

  private died(f: AudioFrame, id: number, pos: Vec3, own: boolean): void {
    if (own) {
      this.engine.play("explosion.ship", { gain: 0.9 });
      this.engine.play("ship.powerdown");
    } else if (this.at(f, pos)) {
      this.engine.play("explosion.ship", { position: pos });
    }
    // Kill credit: the last attacker within a few seconds.
    const hit = this.lastHit.get(id);
    this.lastHit.delete(id);
    if (!hit || f.time - hit.time > KILL_CREDIT_SECS || hit.source === id || hit.source === NO_SHIP) return;
    const killer = f.names.get(hit.source) ?? "";
    const victim = f.names.get(id) ?? "";
    this.kills++;
    const chain = this.killChains.get(hit.source);
    const count = chain && f.time - chain.time <= MULTI_KILL_SECS ? chain.count + 1 : 1;
    this.killChains.set(hit.source, { count, time: f.time });
    if (this.kills === 1) this.announcer?.say("firstBlood", { killer, victim });
    else if (count >= 2) this.announcer?.say(count >= 4 ? "multiKill4" : count === 3 ? "multiKill3" : "multiKill2", { killer, victim });
  }

  // ----- cues ---------------------------------------------------------------------------

  private applyCues(f: AudioFrame, shipPos: (id: number) => Vec3 | undefined): void {
    const e = this.engine;
    const me = f.player;
    for (const c of f.cues) {
      const pos = c.position;
      const near = this.at(f, pos);
      switch (c.kind) {
        case CUE_HIT:
          this.lastHit.set(c.target, { source: c.source, time: f.time });
          if (c.target === me) e.play("impact.hullOwn", { gain: Math.min(1.2, 0.6 + c.value / 40) });
          else if (near) e.play(f.gadgetOf(c.target) === "shield" ? "impact.shield" : "impact.hull", { position: pos, gain: Math.min(1.2, 0.6 + c.value / 40) });
          break;
        case CUE.politeWarning:
          if (c.target === me) e.play("polite.chime", { gain: 1.1 });
          else if (near) e.play("polite.chime", { position: shipPos(c.target) ?? pos, gain: 0.7 });
          break;
        case CUE.blinded:
          if (near) e.play("flash.bang", { position: pos });
          if (c.target === me) e.play("flash.ring", { gain: Math.min(1, 0.4 + c.value / TICK_HZ / 3) });
          break;
        case CUE.chargeStart:
          if (c.source === me) e.play("charge.start");
          else if (near) e.play("charge.start", { position: shipPos(c.source) ?? pos, gain: 0.7 });
          break;
        case CUE.tectonicLaunch:
          if (c.source === me) e.play("tectonic.launch");
          else e.play("tectonic.launch", { position: shipPos(c.source) ?? pos });
          break;
        case CUE.tectonicArmed: {
          const secs = Math.max(0.5, c.value / TICK_HZ);
          this.rumble?.handle.stop(0.2);
          const handle = e.loop("tectonic.rumble", { position: pos, gain: 0.35 });
          this.rumble = handle ? { handle, start: f.time, end: f.time + secs } : null;
          break;
        }
        case CUE.shockwave:
          if (c.value >= 40) {
            this.rumble?.handle.stop(0.15);
            this.rumble = null;
            e.play("tectonic.blast", { position: pos });
          } else if (near) {
            e.play("explosion.big", { position: pos });
          }
          break;
        case CUE.beam:
          if (c.source === me) e.play("beam.leech");
          else if (near) e.play("beam.leech", { position: shipPos(c.source) ?? pos });
          break;
        case CUE.swapped:
          if (c.source === me || c.target === me) e.play("swap");
          else if (near) e.play("swap", { position: pos });
          break;
        case CUE.rewound:
          if (c.target === me) e.play("rewind");
          else if (near) e.play("rewind", { position: pos });
          break;
        case CUE.muzzleFlash:
          // Railspike: heard far away (it reveals the shooter).
          if (c.source === me) e.play("fire.spike", { gain: 0.8 });
          else e.play("fire.spike", { position: shipPos(c.source) ?? pos });
          break;
        case CUE.gadget: {
          if (c.value >= 253) break;
          const key = f.gadgetIds[c.value] ?? "";
          const id = key === "shield" ? "gadget.shield" : key === "cloak" ? "gadget.cloak" : "gadget.generic";
          const at = shipPos(c.source) ?? pos;
          if (c.source === me) e.play(id);
          else if (this.at(f, at)) e.play(id, { position: at });
          break;
        }
        case CUE.decoyReturn:
          if (near) e.play("decoy.return", { position: pos });
          break;
        case CUE.overcharge:
          if (c.source === me) e.play("overcharge");
          else if (near) e.play("overcharge", { position: shipPos(c.source) ?? pos, gain: 0.6 });
          break;
        case CUE.paintColor:
          if (c.source === me) e.play("paint.select");
          break;
        default:
          break;
      }
    }
  }

  // ----- projectiles ----------------------------------------------------------------------

  private updateShots(f: AudioFrame, shipPos: (id: number) => Vec3 | undefined): void {
    const dt = Math.max(1e-4, f.dt);
    for (const t of this.shots) t.matched = false;
    const next: ShotTrack[] = [];
    const fired = new Set<string>();
    for (const p of f.projectiles) {
      let best: ShotTrack | null = null;
      let bestD = Infinity;
      for (const t of this.shots) {
        if (t.matched || t.owner !== p.owner || t.visual !== p.visual) continue;
        const d = dist(madd(t.pos, t.vel, dt), p.position);
        if (d < 1.5 + len(t.vel) * dt * 0.75 && d < bestD) {
          best = t;
          bestD = d;
        }
      }
      if (best) {
        best.matched = true;
        best.pos = p.position;
        best.vel = p.velocity;
        next.push(best);
        continue;
      }
      next.push({ owner: p.owner, visual: p.visual, pos: p.position, vel: p.velocity, matched: true });
      const owner = shipPos(p.owner);
      const key = `${p.owner}/${p.visual}`;
      if (!this.primed || !owner || fired.has(key) || dist(owner, p.position) >= 3 + len(p.velocity) * 0.12) continue;
      fired.add(key);
      const visual = f.visuals[p.visual];
      const s = fireSound(visual?.shape ?? "", visual?.scale ?? 1);
      if (!s) continue;
      if (p.owner === f.player) this.engine.play(s.id, { rate: s.rate, gain: 0.75 });
      else if (this.at(f, owner)) this.engine.play(s.id, { rate: s.rate, position: owner });
    }
    // Tracks without a match hit something or burnt out.
    if (this.primed) {
      for (const t of this.shots) {
        if (t.matched) continue;
        const hit = madd(t.pos, t.vel, dt * 0.5);
        if (!this.at(f, hit)) continue;
        const visual = f.visuals[t.visual];
        const id = impactSound(visual);
        if (!id) continue;
        const explosion = id.startsWith("explosion");
        // A ship hit is the hit cue's clang; explosions always sound.
        if (!explosion && this.nearShip(hit, t.owner, f.player)) continue;
        this.engine.play(id, { position: hit });
      }
    }
    this.shots = next;
  }

  private nearShip(p: Vec3, owner: number, player: number): boolean {
    for (const [id, s] of this.ships) {
      if (!s.alive || id === owner) continue;
      const r = id === player ? 1.5 : s.radius;
      if (dist(s.pos, p) < r * 1.9 + 0.6) return true;
    }
    return false;
  }

  // ----- pickups, boost, rumble -------------------------------------------------------------

  private updatePickups(f: AudioFrame): void {
    const seen = new Map<number, EntityState>();
    for (const e of f.entities) if (e.kind.endsWith("Pickup")) seen.set(e.id, e);
    if (this.primed && f.pose?.alive) {
      for (const [id, e] of this.pickups) {
        if (seen.has(id)) continue;
        if (dist(e.position, f.pose.position) < 5) this.engine.play("pickup");
      }
    }
    this.pickups = seen;
  }

  private updateBoost(f: AudioFrame): void {
    const on = f.boosting && f.pose?.alive === true;
    // Smooth level so a tap is a short swell, not a click.
    this.boostLevel += ((on ? 1 : 0) - this.boostLevel) * Math.min(1, f.dt * (on ? 10 : 5));
    if (this.boostLevel > 0.02) {
      if (!this.boostLoop?.playing) {
        this.boostLoop = this.engine.loop("boost.loop", { gain: 0 });
        if (on) this.engine.play("boost.start", { gain: 0.8 });
      }
      this.boostLoop?.setGain(this.boostLevel);
      this.boostLoop?.setRate(0.9 + 0.2 * this.boostLevel);
    } else if (this.boostLoop) {
      this.boostLoop.stop(0.1);
      this.boostLoop = null;
    }
  }

  private updateRumble(f: AudioFrame): void {
    const r = this.rumble;
    if (!r) return;
    if (f.time >= r.end + 0.6 || !r.handle.playing) {
      r.handle.stop(0.4);
      this.rumble = null;
      return;
    }
    // Swells toward the blast.
    const k = Math.min(1, Math.max(0, (f.time - r.start) / Math.max(0.1, r.end - r.start)));
    r.handle.setGain(0.35 + 0.65 * k * k);
    r.handle.setRate(0.85 + 0.3 * k);
  }

  dispose(): void {
    this.boostLoop?.stop(0.05);
    this.boostLoop = null;
    this.rumble?.handle.stop(0.1);
    this.rumble = null;
    this.engine.stopAll(0.2);
  }
}

