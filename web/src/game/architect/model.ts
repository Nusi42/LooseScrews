// The Architect on the client (M4 C4, milestone-4-plan.md §3.5): the eight
// strokes, the mode's numbers (`Game.architectRules()`), the round and
// stroke state read from the replicated mode state, the client-side checks
// of a stroke before it is sent (the server checks again and refuses
// silently), and a tracker that turns snapshots into events (a stroke
// telegraphed, acting, gone; a stroke of ours accepted or refused; a round
// over). Pure (no DOM, no GPU), so Node tests cover all of it.
//
// Wire facts (ls_sim::modes::the_architect): objective EXIT (kind 5) is the
// exit gate, `holder` = the round's architect, `state` = ROUND_OVER (bit 0)
// | RUNNERS_WON (bit 1) | rounds << 4, `timerEnd` = the round's end (or
// the next round's start while over); the round is `ModeState.stage`
// (0-based). Every stroke in flight is an objective STROKE (kind 6) with id
// 1 + stroke id % 128, `state` = kind | phase << 4 (0 telegraph, 1 acting),
// `timerEnd` = when it acts (telegraph) or ends (acting), `holder` = the
// architect. The architect's `meter` is the budget (points); everyone
// else's is their architect points so far.

import type { Vec3 } from "../../math/vec3.ts";
import type { Rgb } from "../../renderer/renderer.ts";
import { OBJECTIVE, PLAYER_FLAG, type ModeState, type MoodState } from "../modeState.ts";

/** Mode id of The Architect. */
export const ARCHITECT_MODE = "the_architect";

/** Exit objective state bits (`the_architect::exit_state`). */
export const ARCHITECT_EXIT = { roundOver: 1, runnersWon: 2, roundsShift: 4 } as const;

/** Sim ticks per second. */
const HZ = 60;

/** The slider a stroke moves (ui/workshop.ts `SliderKey`). */
export type StrokeSlider = "enclosure" | "density" | "verticality" | "gravityChaos" | "palette" | "absurdity" | "instability";

export interface StrokeDef {
  /** Wire index (`ArchitectAction::stroke`, `StrokeKind`; append only). */
  index: number;
  /** `StrokeKind::id`. */
  id: string;
  /** "Enclosure+". */
  name: string;
  /** What it does, two or three words ("Grow rock"). */
  verb: string;
  /** One line with a wink, for the stroke card and the tooltip. */
  blurb: string;
  slider: StrokeSlider;
  /** Which way the slider moves: +1, -1, or 0 (Palette and Verticality are switches). */
  sign: 1 | -1 | 0;
  /** Display colour of its telegraph, card and preview. */
  color: Rgb;
  /** Cost in `data/rulesets/the_architect.ron` (the match's rules replace it). */
  defaultCost: number;
  /** Changes rock: keeps clear of the exit and the spawn points. */
  changesRock: boolean;
  /** Acts on the whole level (an event); the centre only places the telegraph. */
  levelWide: boolean;
}

/** The eight strokes in wire order (keys 1-8). */
export const STROKES: readonly StrokeDef[] = [
  {
    index: 0,
    id: "enclosure_plus",
    name: "Enclosure+",
    verb: "Grow rock",
    blurb: "Walls where they least expect them",
    slider: "enclosure",
    sign: 1,
    color: [1.0, 0.62, 0.26],
    defaultCost: 25,
    changesRock: true,
    levelWide: false,
  },
  {
    index: 1,
    id: "enclosure_minus",
    name: "Enclosure−",
    verb: "Open a hole",
    blurb: "A shortcut. For you, not for them",
    slider: "enclosure",
    sign: -1,
    color: [0.36, 0.86, 1.0],
    defaultCost: 15,
    changesRock: true,
    levelWide: false,
  },
  {
    index: 2,
    id: "density_plus",
    name: "Density+",
    verb: "Pillars and rocks",
    blurb: "Slalom practice, compulsory",
    slider: "density",
    sign: 1,
    color: [0.62, 0.95, 0.32],
    defaultCost: 20,
    changesRock: true,
    levelWide: false,
  },
  {
    index: 3,
    id: "gravity_chaos_plus",
    name: "Gravity+",
    verb: "A gravity well",
    blurb: "30 seconds of everything falling toward one spot",
    slider: "gravityChaos",
    sign: 1,
    color: [0.74, 0.5, 1.0],
    defaultCost: 30,
    changesRock: false,
    levelWide: false,
  },
  {
    index: 4,
    id: "instability_plus",
    name: "Instability+",
    verb: "A cave-in",
    blurb: "The ceiling had it coming",
    slider: "instability",
    sign: 1,
    color: [1.0, 0.36, 0.22],
    defaultCost: 35,
    changesRock: true,
    levelWide: false,
  },
  {
    index: 5,
    id: "absurdity_plus",
    name: "Absurdity+",
    verb: "A comedy event",
    blurb: "Roll the dice. The level picks the joke",
    slider: "absurdity",
    sign: 1,
    color: [1.0, 0.42, 0.82],
    defaultCost: 60,
    changesRock: false,
    levelWide: true,
  },
  {
    index: 6,
    id: "palette",
    name: "Palette",
    verb: "Ink blackout",
    blurb: "20 seconds of squid ink. Good luck reading the map",
    slider: "palette",
    sign: 0,
    color: [0.86, 0.88, 0.96],
    defaultCost: 15,
    changesRock: false,
    levelWide: false,
  },
  {
    index: 7,
    id: "verticality",
    name: "Verticality",
    verb: "Bored level",
    blurb: "Tip the whole world on its side. Once a round",
    slider: "verticality",
    sign: 0,
    color: [1.0, 0.84, 0.32],
    defaultCost: 80,
    changesRock: false,
    levelWide: true,
  },
];

/** The stroke of a wire index (null when unknown). */
export function strokeDef(index: number): StrokeDef | null {
  return STROKES[index] ?? null;
}

/** CSS colour of a stroke (`rgb(...)`). */
export function strokeCss(def: StrokeDef, alpha = 1): string {
  const [r, g, b] = def.color.map((c) => Math.round(c * 255));
  return alpha >= 1 ? `rgb(${r}, ${g}, ${b})` : `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** The stroke a slider moved in `direction` (> 0 up, < 0 down) picks; null when it has none. */
export function strokeForSlider(slider: StrokeSlider, direction: number): StrokeDef | null {
  const down = direction < 0;
  for (const s of STROKES) {
    if (s.slider !== slider) continue;
    if (s.sign === 0 || (down ? s.sign < 0 : s.sign > 0)) return s;
  }
  return null;
}

// ---------------------------------------------------------------------------
// Rules

/** The mode's numbers (`Game.architectRules()`, ls-wasm `architect_rules_json`). */
export interface ArchitectRules {
  /** Cost per stroke index. */
  costs: number[];
  minRadius: number;
  maxRadius: number;
  cooldownSecs: number;
  telegraphSecs: number;
  maxStrokes: number;
  budgetStart: number;
  budgetPerSec: number;
  budgetMax: number;
  keepClear: number;
  wellSecs: number;
  paletteSecs: number;
  roundSecs: number;
  roundResultsSecs: number;
  runnerLives: number;
  /** The rule set's Absurdity cap: 0 refuses Absurdity+ and Verticality. */
  absurdity: number;
  /** Spawn points rock strokes keep clear of. */
  spawns: Vec3[];
}

/** `data/rulesets/the_architect.ron`. */
export const DEFAULT_ARCHITECT_RULES: Readonly<ArchitectRules> = Object.freeze({
  costs: STROKES.map((s) => s.defaultCost),
  minRadius: 12,
  maxRadius: 24,
  cooldownSecs: 2,
  telegraphSecs: 2,
  maxStrokes: 4,
  budgetStart: 100,
  budgetPerSec: 4,
  budgetMax: 250,
  keepClear: 6,
  wellSecs: 30,
  paletteSecs: 20,
  roundSecs: 240,
  roundResultsSecs: 6,
  runnerLives: 2,
  absurdity: 100,
  spawns: [],
});

function num(v: unknown, fallback: number): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

/** Parses `Game.architectRules()`; missing or broken fields take the defaults, no text gives null. */
export function parseArchitectRules(text: string | undefined | null): ArchitectRules | null {
  if (typeof text !== "string" || text === "") return null;
  let j: Record<string, unknown>;
  try {
    const v: unknown = JSON.parse(text);
    if (v === null || typeof v !== "object" || Array.isArray(v)) return null;
    j = v as Record<string, unknown>;
  } catch {
    return null;
  }
  const d = DEFAULT_ARCHITECT_RULES;
  const costs = Array.isArray(j["costs"]) ? (j["costs"] as unknown[]) : [];
  const spawns = Array.isArray(j["spawns"]) ? (j["spawns"] as unknown[]) : [];
  return {
    costs: STROKES.map((s, i) => num(costs[i], s.defaultCost)),
    minRadius: num(j["minRadius"], d.minRadius),
    maxRadius: num(j["maxRadius"], d.maxRadius),
    cooldownSecs: num(j["cooldownSecs"], d.cooldownSecs),
    telegraphSecs: num(j["telegraphSecs"], d.telegraphSecs),
    maxStrokes: num(j["maxStrokes"], d.maxStrokes),
    budgetStart: num(j["budgetStart"], d.budgetStart),
    budgetPerSec: num(j["budgetPerSec"], d.budgetPerSec),
    budgetMax: num(j["budgetMax"], d.budgetMax),
    keepClear: num(j["keepClear"], d.keepClear),
    wellSecs: num(j["wellSecs"], d.wellSecs),
    paletteSecs: num(j["paletteSecs"], d.paletteSecs),
    roundSecs: num(j["roundSecs"], d.roundSecs),
    roundResultsSecs: num(j["roundResultsSecs"], d.roundResultsSecs),
    runnerLives: num(j["runnerLives"], d.runnerLives),
    absurdity: num(j["absurdity"], d.absurdity),
    spawns: spawns.flatMap((p) => (Array.isArray(p) && p.length >= 3 ? [{ x: num(p[0], 0), y: num(p[1], 0), z: num(p[2], 0) }] : [])),
  };
}

/** Cost of a stroke under `rules`. */
export function strokeCost(rules: ArchitectRules, index: number): number {
  return rules.costs[index] ?? STROKES[index]?.defaultCost ?? 0;
}

// ---------------------------------------------------------------------------
// State from the mode state

export type StrokePhase = "telegraph" | "active";

/** One stroke in flight (objective STROKE). */
export interface StrokeMarker {
  /** Objective id (1 + stroke id % 128). */
  id: number;
  def: StrokeDef;
  pos: Vec3;
  radius: number;
  phase: StrokePhase;
  /** Tick it acts (telegraph) or ends (active). */
  timerEnd: number;
  /** The architect who placed it. */
  architect: number;
}

export interface RunnerState {
  id: number;
  /** Through the exit this round. */
  finished: boolean;
  /** Out of lives this round. */
  out: boolean;
  lives: number;
  /** Mode score (runner points + architect points). */
  score: number;
  /** Architect points so far (the `meter` of a non-architect). */
  architectPoints: number;
}

export interface ArchitectView {
  /** Round (0-based) and rounds of the match. */
  round: number;
  rounds: number;
  /** Round results (between rounds). */
  roundOver: boolean;
  /** The round that just ended went to the runners. */
  runnersWon: boolean;
  /** The round's end, or (while over) the next round's start. */
  phaseEndTick: number;
  /** The round's architect, -1 = none (a lone runner practises). */
  architect: number;
  /** The architect's budget (points). */
  budget: number;
  exit: { pos: Vec3; radius: number } | null;
  strokes: StrokeMarker[];
  runners: RunnerState[];
}

/** The Architect's state from the mode state; null in other modes. */
export function architectView(m: ModeState): ArchitectView | null {
  if (m.mode !== ARCHITECT_MODE) return null;
  const exitObj = m.objectives.find((o) => o.kind === OBJECTIVE.exit);
  const flagged = m.players.find((p) => (p.flags & PLAYER_FLAG.architect) !== 0);
  const architect = flagged?.id ?? (exitObj && exitObj.holder >= 0 && m.running ? exitObj.holder : -1);
  const archRow = m.players.find((p) => p.id === architect);
  const state = exitObj?.state ?? 0;
  const strokes: StrokeMarker[] = [];
  for (const o of m.objectives) {
    if (o.kind !== OBJECTIVE.stroke) continue;
    const def = strokeDef(o.state & 15);
    if (!def) continue;
    strokes.push({ id: o.id, def, pos: o.pos, radius: o.radius, phase: o.state >> 4 === 1 ? "active" : "telegraph", timerEnd: o.timerEnd, architect: o.holder });
  }
  const runners: RunnerState[] = m.players
    .filter((p) => p.id !== architect && (p.flags & PLAYER_FLAG.spectator) === 0)
    .map((p) => ({
      id: p.id,
      finished: (p.flags & PLAYER_FLAG.finished) !== 0,
      out: (p.flags & PLAYER_FLAG.eliminated) !== 0,
      lives: p.lives,
      score: p.score,
      architectPoints: p.meter,
    }));
  return {
    round: Math.max(0, m.stage),
    rounds: Math.max(1, state >> ARCHITECT_EXIT.roundsShift),
    roundOver: (state & ARCHITECT_EXIT.roundOver) !== 0,
    runnersWon: (state & ARCHITECT_EXIT.runnersWon) !== 0,
    phaseEndTick: exitObj?.timerEnd ?? m.stageEndTick,
    architect,
    budget: flagged ? archRow?.meter ?? 0 : 0,
    exit: exitObj ? { pos: exitObj.pos, radius: exitObj.radius > 0 ? exitObj.radius : 6 } : null,
    strokes,
    runners,
  };
}

/** Telegraph progress 0..1 of a stroke (1 = it acts now), from its act tick. */
export function telegraphProgress(s: StrokeMarker, nowTick: number, telegraphSecs: number): number {
  if (s.phase === "active") return 1;
  const left = (s.timerEnd - nowTick) / HZ;
  return Math.max(0, Math.min(1, 1 - left / Math.max(0.1, telegraphSecs)));
}

/** Seconds until a tick (0 when past). */
export function secsTo(tick: number, nowTick: number): number {
  return Math.max(0, (tick - nowTick) / HZ);
}

// ---------------------------------------------------------------------------
// Client-side checks

export interface StrokeCheck {
  view: ArchitectView;
  rules: ArchitectRules;
  own: number;
  nowTick: number;
  stroke: number;
  center: Vec3;
  radius: number;
  /** Level box [minX, minY, minZ, maxX, maxY, maxZ]. */
  bounds: ArrayLike<number>;
  /** Our next stroke is allowed from this tick (the cooldown). */
  cooldownUntil: number;
  /** A Verticality stroke was placed this round. */
  verticalityUsed: boolean;
  mood: MoodState;
}

/** Why a stroke cannot be placed (the server would refuse it), or null when it can. */
export function strokeProblem(c: StrokeCheck): string | null {
  const v = c.view;
  const def = strokeDef(c.stroke);
  if (!def) return "Unknown stroke";
  if (v.architect !== c.own) return "Only the architect places strokes";
  if (v.roundOver) return "The round is over";
  const cost = strokeCost(c.rules, c.stroke);
  if (!(c.radius >= c.rules.minRadius - 1e-3 && c.radius <= c.rules.maxRadius + 1e-3)) return `Size ${Math.round(c.rules.minRadius)}-${Math.round(c.rules.maxRadius)} u`;
  const b = (i: number): number => c.bounds[i] ?? 0;
  const p = c.center;
  if (!(p.x >= b(0) && p.x <= b(3) && p.y >= b(1) && p.y <= b(4) && p.z >= b(2) && p.z <= b(5))) return "Outside the level";
  if (def.changesRock) {
    const clear = c.rules.keepClear + c.radius;
    if (v.exit && dist(v.exit.pos, p) < clear + v.exit.radius) return "Too close to the exit";
    if (c.rules.spawns.some((s) => dist(s, p) < clear)) return "Too close to the runners' start";
  }
  if (v.strokes.length >= c.rules.maxStrokes) return `${c.rules.maxStrokes} strokes at once at most`;
  if (c.nowTick < c.cooldownUntil) return `Recharging · ${secsTo(c.cooldownUntil, c.nowTick).toFixed(1)} s`;
  if (v.budget < cost) return `Needs ${cost} · you have ${v.budget}`;
  if (def.index === 7 && c.verticalityUsed) return "Verticality: once a round";
  if (def.levelWide) {
    if (c.rules.absurdity <= 0) return "Comedy events are off in these rules";
    const pendingEvent = v.strokes.some((s) => s.def.levelWide && s.phase === "telegraph");
    if (c.mood.eventId !== "" || pendingEvent) return "An event is already running";
  }
  return null;
}

/** The short reason a refused stroke most likely had (after the fact, from a check at send time). */
export function refusalText(problem: string | null, def: StrokeDef): string {
  if (problem) return problem;
  // The server refuses a reshape that would change nothing there (all rock, or all open).
  if (def.index === 0 || def.index === 2) return "Nothing to grow there: aim at open space near rock";
  if (def.index === 1) return "Nothing to open there: aim into rock";
  if (def.index === 4) return "No ceiling to drop there";
  return "The level said no";
}

function dist(a: Vec3, b: Vec3): number {
  return Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z);
}

// ---------------------------------------------------------------------------
// Events between snapshots

export type ArchitectEvent =
  /** A stroke appeared (telegraph begins). `ours`: placed by this client's player. */
  | { kind: "telegraph"; stroke: StrokeMarker; ours: boolean }
  /** A stroke's telegraph ended: it acts now. */
  | { kind: "act"; stroke: StrokeMarker }
  /** A stroke ended (gone from the state). */
  | { kind: "end"; stroke: StrokeMarker }
  /** Our stroke request was refused (no stroke appeared in time). */
  | { kind: "refused"; stroke: number; reason: string }
  /** A runner got through the exit. */
  | { kind: "escaped"; runner: number }
  /** A round ended. */
  | { kind: "roundOver"; result: RoundRecord }
  /** A round started (`architect` is its architect). */
  | { kind: "roundStart"; round: number; architect: number };

export interface RoundRecord {
  /** 0-based. */
  round: number;
  architect: number;
  runners: number;
  escaped: number;
  runnersWon: boolean;
}

/** Ticks a sent stroke may take to show up before it counts as refused. */
export const REFUSE_TICKS = 50;

/**
 * Follows the architect state from frame to frame: emits events, keeps our
 * cooldown and the once-per-round Verticality, and the rounds played.
 */
export class ArchitectTracker {
  private strokes = new Map<number, StrokeMarker>();
  private finished = new Set<number>();
  private round = -1;
  private over = false;
  private pending: { stroke: number; sentTick: number; problem: string | null; before: Set<number> } | null = null;
  /** Our next stroke from this tick on. */
  cooldownUntil = 0;
  verticalityUsed = false;
  /** Rounds that ended this match, in order. */
  readonly history: RoundRecord[] = [];

  /** A stroke request went out at `nowTick` (`problem`: the client check then, for the refusal text). */
  sent(stroke: number, nowTick: number, problem: string | null = null): void {
    this.pending = { stroke, sentTick: nowTick, problem, before: new Set(this.strokes.keys()) };
  }

  /** Waiting for the server to accept a stroke. */
  get waiting(): boolean {
    return this.pending !== null;
  }

  /** Compares `v` with the last frame; returns what happened. */
  update(v: ArchitectView | null, own: number, nowTick: number, rules: ArchitectRules): ArchitectEvent[] {
    const out: ArchitectEvent[] = [];
    if (!v) {
      this.strokes.clear();
      return out;
    }
    if (v.round !== this.round) {
      out.push({ kind: "roundStart", round: v.round, architect: v.architect });
      this.round = v.round;
      this.verticalityUsed = false;
      this.finished.clear();
      this.over = false;
    }
    if (v.roundOver && !this.over) {
      const record: RoundRecord = {
        round: v.round,
        architect: v.architect,
        runners: v.runners.length,
        escaped: v.runners.filter((r) => r.finished).length,
        runnersWon: v.runnersWon,
      };
      if (!this.history.some((h) => h.round === record.round)) this.history.push(record);
      out.push({ kind: "roundOver", result: record });
    }
    this.over = v.roundOver;
    for (const r of v.runners) {
      if (r.finished && !this.finished.has(r.id)) {
        this.finished.add(r.id);
        out.push({ kind: "escaped", runner: r.id });
      }
    }
    const seen = new Set<number>();
    for (const s of v.strokes) {
      seen.add(s.id);
      const old = this.strokes.get(s.id);
      if (!old || old.def.index !== s.def.index) {
        const ours = s.architect === own && own >= 0;
        out.push({ kind: "telegraph", stroke: s, ours });
        if (ours) {
          this.cooldownUntil = Math.max(this.cooldownUntil, nowTick + Math.round(rules.cooldownSecs * HZ));
          if (s.def.index === 7) this.verticalityUsed = true;
          if (this.pending && !this.pending.before.has(s.id)) this.pending = null;
        }
        if (s.phase === "active") out.push({ kind: "act", stroke: s });
      } else if (old.phase === "telegraph" && s.phase === "active") {
        out.push({ kind: "act", stroke: s });
      }
      this.strokes.set(s.id, s);
    }
    for (const [id, s] of [...this.strokes]) {
      if (!seen.has(id)) {
        this.strokes.delete(id);
        out.push({ kind: "end", stroke: s });
      }
    }
    if (this.pending && nowTick - this.pending.sentTick > REFUSE_TICKS) {
      const def = strokeDef(this.pending.stroke);
      out.push({ kind: "refused", stroke: this.pending.stroke, reason: def ? refusalText(this.pending.problem, def) : "Refused" });
      this.pending = null;
    }
    return out;
  }

  /** Forget everything (a new match). */
  reset(): void {
    this.strokes.clear();
    this.finished.clear();
    this.round = -1;
    this.over = false;
    this.pending = null;
    this.cooldownUntil = 0;
    this.verticalityUsed = false;
    this.history.length = 0;
  }
}
