// Comedy event visuals (M4 C1, milestone-4-plan.md §3.1-§3.2 and wave C):
// the hub main.ts and the gallery call. It reads what the running event does
// (`Game.eventFx`, fx.ts), runs the event's module (one file per event in
// this folder, registered in EVENT_VISUALS) and lets it linger after the end
// until it has nothing more to show, merges the modules' outputs (camera,
// props, particles, lights, the pixel look, the cake wave, HUD hints) and
// owns the telegraph every event shares: the event banner
// (ui/eventBanner.ts) and the announcer's line (`event.<id>`, with its
// fanfare).
//
// For C2 and C3: add `<event id>: <factory>` to EVENT_VISUALS, the props to
// renderer/models, the sounds to audio/sounds/events*.ts and the gallery
// strip to gallery/eventScenes.ts.

import type { Trigger } from "../../audio/announcer/lines.ts";
import { NO_PIXEL } from "../../renderer/pixelPost.ts";
import type { PointLight } from "../../renderer/lights.ts";
import type { Renderer, SceneInstances } from "../../renderer/renderer.ts";
import { SHIP_MESH_KINDS } from "../../renderer/models/index.ts";
import * as q from "../../math/quat.ts";
import type { EventHud, EventBannerModel } from "../../ui/eventBanner.ts";
import type { ProjectileState, ShipState } from "../views.ts";
import { parseWells } from "../worldFx.ts";
import { bigHeadDay } from "./bigHeadDay.ts";
import { boredLevel } from "./boredLevel.ts";
import { everythingIsCake } from "./cake.ts";
import { coffeeBreak } from "./coffeeBreak.ts";
import { ANNOUNCE_SECS, clockOf, TICK_HZ, type EventClock, type EventFx } from "./fx.ts";
import { levelHiccups } from "./levelHiccups.ts";
import { movingDay } from "./movingDay.ts";
import { rubberChickens } from "./rubberChickens.ts";
import { emptyOutput, type EventEnv, type EventInput, type EventOutput, type EventVisual, type EventVisualFactory, type ParticleSink, type SoundSink, type WellPoint } from "./types.ts";
import { wrongGenre } from "./wrongGenre.ts";

export { parseEventFx, fxFromMood, NO_EVENT_FX } from "./fx.ts";
export type { EventFx } from "./fx.ts";
export type { EventInput, EventOutput, WellPoint, OwnShip } from "./types.ts";
export { emptyOutput } from "./types.ts";

/** The visuals of each event id (C1's eight; C2 and C3 add theirs). */
export const EVENT_VISUALS: Readonly<Record<string, EventVisualFactory>> = {
  bored_level: boredLevel,
  level_hiccups: levelHiccups,
  gravity_coffee_break: coffeeBreak,
  wrong_genre: wrongGenre,
  big_head_day: bigHeadDay,
  rubber_chickens: rubberChickens,
  moving_day: movingDay,
  everything_is_cake: everythingIsCake,
};

/** The gravity wells of `Game.wells()` for the event visuals. */
export function wellPoints(a: ArrayLike<number> | undefined | null): WellPoint[] {
  return parseWells(a).map((w) => ({ center: w.position, core: w.core, range: w.range, dynamic: w.endTick >= 0 }));
}

/** `?eventfx=off`: the game without event visuals (before shots); the banner and the announcer stay. */
export function wantsEventFx(loc: { search: string }): boolean {
  return new URLSearchParams(loc.search).get("eventfx") !== "off";
}

interface Run {
  key: string;
  event: string;
  visual: EventVisual | null;
  fx: EventFx;
  /** Wall time the event was last seen running. */
  seen: number;
  /** Wall time it was first missed (over), else null. */
  overAt: number | null;
  announced: boolean;
}

/** Something that speaks event lines (MatchAudio.announcer). */
export interface EventAnnouncer {
  say(trigger: Trigger, vars?: Readonly<Record<string, string | number | undefined>>): void;
}

export interface EventVisualsOptions {
  particles: ParticleSink | null;
  sound: SoundSink | null;
  hud: EventHud | null;
  announcer: EventAnnouncer | null;
  /** False: no event visuals (banner, line and sting still). */
  enabled: boolean;
}

/** The run key of an event (stable from its announcement to its end). */
export function runKey(fx: EventFx): string {
  return fx.event ? `${fx.event}@${fx.activeTick}` : "";
}

/** The banner of the running event at `tick`, or null. */
export function bannerModel(fx: EventFx, tick: number, target: string): EventBannerModel | null {
  if (!fx.event) return null;
  if (fx.phase === "announce") {
    return { id: fx.event, stage: "announce", secsLeft: Math.max(0, (fx.activeTick - tick) / TICK_HZ), total: ANNOUNCE_SECS, target };
  }
  const total = Math.max(0.1, (fx.phaseEndTick - fx.activeTick) / TICK_HZ);
  const left = fx.phase === "active" ? Math.max(0, (fx.phaseEndTick - tick) / TICK_HZ) : 0;
  return { id: fx.event, stage: "active", secsLeft: left, total, target };
}

export class EventVisuals {
  private readonly env: EventEnv;
  private readonly hud: EventHud | null;
  private readonly announcer: EventAnnouncer | null;
  private readonly enabled: boolean;
  private runs: Run[] = [];
  private last: EventOutput = emptyOutput();

  constructor(o: EventVisualsOptions) {
    this.env = { particles: o.particles, sound: o.sound };
    this.hud = o.hud;
    this.announcer = o.announcer;
    this.enabled = o.enabled;
  }

  /** One frame: runs the modules and returns what changes (also kept for `apply` and `addToScene`). */
  update(i: EventInput, nameOf: (id: number) => string = () => ""): EventOutput {
    const out = emptyOutput();
    const fx = i.fx;
    const key = runKey(fx);
    if (key && !this.runs.some((r) => r.key === key)) {
      const factory = this.enabled ? EVENT_VISUALS[fx.event] : undefined;
      this.runs.push({ key, event: fx.event, visual: factory ? factory(this.env) : null, fx, seen: i.time, overAt: null, announced: false });
    }
    const target = i.mood.target >= 0 ? nameOf(i.mood.target) : "";
    for (const r of this.runs) {
      const running = r.key === key;
      if (running) {
        r.fx = fx;
        r.seen = i.time;
        r.overAt = null;
        if (!r.announced) {
          r.announced = true;
          // A late join into a running event gets no telegraph.
          // (The announcer plays its fanfare, `sting.event`, before the line.)
          if (fx.phase === "announce" && !i.quiet) this.announcer?.say(`event.${fx.event}` as Trigger, { target });
        }
      } else if (r.overAt === null) {
        r.overAt = i.time;
      }
      if (!r.visual) continue;
      const clock: EventClock = running ? clockOf(fx, i.tick) : { ...clockOf(r.fx, i.tick), over: i.time - (r.overAt ?? i.time) };
      const alive = r.visual.update(running ? i : { ...i, fx: r.fx }, clock, out);
      if (!alive && !running) {
        r.visual.dispose?.();
        r.visual = null;
      }
    }
    this.runs = this.runs.filter((r) => r.visual !== null || r.key === key);
    this.hud?.update({ banner: i.quiet ? null : bannerModel(fx, i.tick, target), hints: out.hud, sideView: out.thirdPerson });
    this.last = out;
    return out;
  }

  /** Puts the frame's renderer settings in place: the pixel look, the cake wave, the cutaway terrain. */
  apply(renderer: Renderer, out: EventOutput = this.last): void {
    renderer.settings.post.pixel = out.pixel ?? NO_PIXEL;
    renderer.settings.cake = out.cake;
    renderer.terrainCutaway = out.cutaway;
  }

  /** Adds the frame's props to the scene (and recolours debris for the cake). */
  addToScene(scene: SceneInstances, lights: PointLight[], out: EventOutput = this.last): void {
    for (const [kind, inst] of out.opaque) (scene.opaque[kind] ??= []).push(inst);
    for (const [kind, inst] of out.translucent) (scene.translucent[kind] ??= []).push(inst);
    const debris = out.debrisColor;
    if (debris) for (const inst of scene.opaque.debris ?? []) inst.color = debris;
    const boost = out.shipBoost;
    if (boost) {
      for (const kind of SHIP_MESH_KINDS) {
        for (const inst of scene.opaque[kind] ?? []) {
          inst.scale *= boost.scale;
          inst.emissive = Math.max(inst.emissive, boost.emissive);
          // Roll about the hull's long axis, top toward the viewer (+z, the side view's camera).
          const f = q.forward(inst.rotation);
          const sign = f.x >= 0 ? -1 : 1;
          inst.rotation = q.normalize(q.mul(inst.rotation, q.fromAxisAngle({ x: 0, y: 0, z: 1 }, sign * boost.roll)));
        }
      }
    }
    lights.push(...out.lights);
  }

  /** The ships to draw (the side view flattens them into its plane). */
  ships(ships: readonly ShipState[], out: EventOutput = this.last): readonly ShipState[] {
    return out.ships ?? ships;
  }

  /** The projectiles to draw. */
  shots(shots: readonly ProjectileState[], out: EventOutput = this.last): readonly ProjectileState[] {
    return out.shots ?? shots;
  }

  /** A shot the scene and effects leave out: an event draws it, or `other` says so (the guided Eyeball). */
  hideShot(other: ((p: ProjectileState) => boolean) | undefined, out: EventOutput = this.last): ((p: ProjectileState) => boolean) | undefined {
    const hidden = out.hideShots;
    if (hidden.size === 0) return other;
    return (p) => hidden.has(p) || (other?.(p) ?? false);
  }

  /** Stops every module (a new match). */
  reset(): void {
    for (const r of this.runs) r.visual?.dispose?.();
    this.runs = [];
    this.last = emptyOutput();
  }

  dispose(): void {
    this.reset();
  }
}
