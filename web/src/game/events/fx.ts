// What the running comedy event does now (M4 C1): ls-wasm `Game.eventFx`
// (crates/ls-wasm/src/events.rs) decoded, plus the event's clock. Every
// event visual reads its timing from here, so it starts and ends on the
// server's ticks. A core without `eventFx` (older builds) falls back to the
// replicated mood view: the phase and the parameter bytes (Bored level's
// axis, the cake's origin) decode in TS; hiccup ticks need the core.

import type { Vec3 } from "../../math/vec3.ts";
import type { MoodState } from "../modeState.ts";

/** Ticks per second of the simulation. */
export const TICK_HZ = 60;
/** The announce phase (data/events/director.ron `announce_secs`). */
export const ANNOUNCE_SECS = 3;

export type EventPhaseId = "" | "announce" | "active" | "ending";

export interface EventFx {
  /** Running event id (`data/events/<id>.ron`), "" for none. */
  event: string;
  phase: EventPhaseId;
  /** The tick the values are for. */
  now: number;
  /** Announce: the tick the active phase starts; later: the tick it started. */
  activeTick: number;
  phaseEndTick: number;
  wellScale: number;
  hitScale: number;
  chicken: boolean;
  craterScale: number;
  scavengeScale: number;
  /** The level's up the event sets this tick (Bored level), else null. */
  up: Vec3 | null;
  hitPlane: Vec3 | null;
  bored: { from: Vec3; axis: Vec3; turnDegrees: number; turnSecs: number } | null;
  hiccups: { ticks: number[]; liftTicks: number; up: Vec3 } | null;
  cake: { origin: Vec3 } | null;
}

export const NO_EVENT_FX: EventFx = {
  event: "",
  phase: "",
  now: 0,
  activeTick: 0,
  phaseEndTick: 0,
  wellScale: 1,
  hitScale: 1,
  chicken: false,
  craterScale: 1,
  scavengeScale: 1,
  up: null,
  hitPlane: null,
  bored: null,
  hiccups: null,
  cake: null,
};

type Json = Record<string, unknown>;
const obj = (v: unknown): Json => (v && typeof v === "object" ? (v as Json) : {});
const num = (v: unknown, d = 0): number => (typeof v === "number" && Number.isFinite(v) ? v : d);
const vec = (v: unknown): Vec3 | null => {
  if (!Array.isArray(v) || v.length < 3) return null;
  return { x: num(v[0]), y: num(v[1]), z: num(v[2]) };
};
const phaseOf = (v: unknown): EventPhaseId => (v === "announce" || v === "active" || v === "ending" ? v : "");

/** Parses `Game.eventFx` (broken or missing: no event). */
export function parseEventFx(json: string | undefined | null): EventFx {
  if (!json) return NO_EVENT_FX;
  let j: Json;
  try {
    j = obj(JSON.parse(json));
  } catch {
    return NO_EVENT_FX;
  }
  const event = typeof j["event"] === "string" ? j["event"] : "";
  if (!event) return { ...NO_EVENT_FX, now: num(j["now"]) };
  const b = j["bored"] ? obj(j["bored"]) : null;
  const h = j["hiccups"] ? obj(j["hiccups"]) : null;
  const c = j["cake"] ? obj(j["cake"]) : null;
  return {
    event,
    phase: phaseOf(j["phase"]),
    now: num(j["now"]),
    activeTick: num(j["activeTick"]),
    phaseEndTick: num(j["phaseEndTick"]),
    wellScale: num(j["wellScale"], 1),
    hitScale: num(j["hitScale"], 1),
    chicken: j["chicken"] === true,
    craterScale: num(j["craterScale"], 1),
    scavengeScale: num(j["scavengeScale"], 1),
    up: vec(j["up"]),
    hitPlane: vec(j["hitPlane"]),
    bored: b ? { from: vec(b["from"]) ?? { x: 0, y: 1, z: 0 }, axis: vec(b["axis"]) ?? { x: 1, y: 0, z: 0 }, turnDegrees: num(b["turnDegrees"], 90), turnSecs: num(b["turnSecs"], 5) } : null,
    hiccups: h ? { ticks: Array.isArray(h["ticks"]) ? h["ticks"].map((t) => num(t)) : [], liftTicks: num(h["liftTicks"], 15), up: vec(h["up"]) ?? { x: 0, y: 1, z: 0 } } : null,
    cake: c ? { origin: vec(c["origin"]) ?? { x: 0, y: 0, z: 0 } } : null,
  };
}

/** ls_sim::events::params::unpack_dir: three signed bytes in 1/127 steps, normalised (+Y for zeros). */
export function unpackDir(b0: number, b1: number, b2: number): Vec3 {
  const s = (b: number): number => ((b & 0xff) > 127 ? (b & 0xff) - 256 : b & 0xff) / 127;
  const v = { x: s(b0), y: s(b1), z: s(b2) };
  const l = Math.hypot(v.x, v.y, v.z);
  return l > 0 ? { x: v.x / l, y: v.y / l, z: v.z / l } : { x: 0, y: 1, z: 0 };
}

/** ls_sim::events::params::unpack_pos: three little-endian i16 in 1/64 u. */
export function unpackPos(p: readonly number[], at = 0): Vec3 {
  const i16 = (lo: number, hi: number): number => {
    const v = ((hi & 0xff) << 8) | (lo & 0xff);
    return (v > 32767 ? v - 65536 : v) / 64;
  };
  return { x: i16(p[at] ?? 0, p[at + 1] ?? 0), y: i16(p[at + 2] ?? 0, p[at + 3] ?? 0), z: i16(p[at + 4] ?? 0, p[at + 5] ?? 0) };
}

/** The effects a core without `eventFx` lets the client know: phase, Bored level's frame, the cake's origin. */
export function fxFromMood(m: MoodState | null | undefined, now: number): EventFx {
  if (!m || !m.eventId) return { ...NO_EVENT_FX, now };
  const p = m.params;
  const phase = phaseOf(m.phase);
  const fx: EventFx = { ...NO_EVENT_FX, event: m.eventId, phase, now, activeTick: phase === "announce" ? m.phaseEndTick : m.activeTick, phaseEndTick: m.phaseEndTick };
  if (m.eventId === "bored_level") fx.bored = { from: unpackDir(p[0] ?? 0, p[1] ?? 0, p[2] ?? 0), axis: unpackDir(p[3] ?? 0, p[4] ?? 0, p[5] ?? 0), turnDegrees: 90, turnSecs: 5 };
  if (m.eventId === "everything_is_cake") fx.cake = { origin: unpackPos(p) };
  if (m.eventId === "rubber_chickens" && phase !== "announce") fx.chicken = true;
  if (m.eventId === "big_head_day" && phase !== "announce") fx.hitScale = 3;
  return fx;
}

/**
 * An event's clock in seconds, relative to its active phase: negative
 * during the announcement (-3 .. 0), 0 .. `activeSecs` while it acts.
 */
export interface EventClock {
  event: string;
  phase: EventPhaseId;
  /** Seconds since the active phase started (negative while announcing). */
  t: number;
  /** Seconds the active phase lasts (known once it started; the announce phase does not know it). */
  activeSecs: number | null;
  /** The event is over (seconds since it was last seen running), else null. */
  over: number | null;
}

/** The clock of `fx` at fractional tick `tick` (interpolated between snapshots). */
export function clockOf(fx: EventFx, tick: number): EventClock {
  const t = (tick - fx.activeTick) / TICK_HZ;
  const activeSecs = fx.phase === "active" ? (fx.phaseEndTick - fx.activeTick) / TICK_HZ : null;
  return { event: fx.event, phase: fx.phase, t: fx.phase === "announce" ? Math.min(0, t) : t, activeSecs, over: null };
}

/** Smoothstep 0..1 of x between a and b. */
export function smooth(a: number, b: number, x: number): number {
  const t = Math.min(1, Math.max(0, (x - a) / (b - a)));
  return t * t * (3 - 2 * t);
}

/** Elastic ease-out (overshoots, settles at 1): a pop. */
export function elasticOut(x: number): number {
  if (x <= 0) return 0;
  if (x >= 1) return 1;
  return Math.pow(2, -10 * x) * Math.sin(((x * 10 - 0.75) * 2 * Math.PI) / 3) + 1;
}

/** A small deterministic hash in [0, 1) of two numbers. */
export function hash01(a: number, b = 0): number {
  const x = Math.sin(a * 12.9898 + b * 78.233) * 43758.5453;
  return x - Math.floor(x);
}
