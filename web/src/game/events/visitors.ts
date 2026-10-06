// The visitor events' looks, sounds and HUD (M4 C2, docs/milestone-4-plan.md
// §3.2): Tax drone, Pizza delivery, Customer survey, Mimic pickups and
// Gossip wall, one file each (taxDrone.ts, pizzaDelivery.ts,
// customerSurvey.ts, mimicPickups.ts, gossipWall.ts). main.ts makes one
// `VisitorVisuals` per game and calls `update` once a frame (before the
// scene is built), `addToScene` after, and filters the entities it hides
// (mimicked pickups). The scene builder leaves the visitors' entity kinds
// (drones, mimics, the gossip face) to this module (`skipKinds`).
// `?visitors=off` draws them as the plain fallback (the before shots).

import type { Quat } from "../../math/quat.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { PointLight } from "../../renderer/lights.ts";
import type { MeshKind } from "../../renderer/models/index.ts";
import { emptyScene, type SceneInstances } from "../../renderer/renderer.ts";
import { VisitorHud, type Project, type VisitorHudModel } from "../../ui/visitorHud.ts";
import type { Cue } from "../mechanics.ts";
import type { MoodState } from "../modeState.ts";
import type { VfxSink } from "../vfxLibrary.ts";
import type { EntityState, VisualDef } from "../views.ts";
import { SurveyFx } from "./customerSurvey.ts";
import { GossipFx } from "./gossipWall.ts";
import { MimicFx } from "./mimicPickups.ts";
import { PizzaFx } from "./pizzaDelivery.ts";
import { TaxDroneFx } from "./taxDrone.ts";
import { outcomes, type DrawCtx, type SoundCall, type VisitorInput, type VisitorOut, type VisitorShip } from "./visitorKit.ts";

/** Entity kinds this module draws (the scene builder skips them). */
export const VISITOR_KINDS: ReadonlySet<string> = new Set(["drone", "mimic", "gossipFace"]);

/** The audio engine's part the visitors use (audio/engine.ts). */
export interface VisitorSoundEngine {
  play(id: string, o?: { position?: Vec3 | null; gain?: number }): boolean;
  loop(id: string, o?: { position?: Vec3 | null; gain?: number }): { readonly playing: boolean; setGain(g: number, ramp?: number): void; setPosition(p: Vec3): void; stop(fade?: number): void } | null;
}

/** What main.ts hands the visitors each frame (raw views). */
export interface VisitorFrameInput {
  time: number;
  dt: number;
  nowTick: number;
  camera: Vec3;
  orientation: Quat;
  /** The level's up. */
  up: Vec3;
  mood: MoodState;
  cues: readonly Cue[];
  entities: readonly EntityState[];
  ships: readonly VisitorShip[];
  own: number;
  /** The own ship's position while it flies (gossip directions), else null. */
  ownPosition: Vec3 | null;
  names: ReadonlyMap<number, string>;
  visuals: readonly VisualDef[];
  /** ls-wasm `Game.pickupPoints()` (x, y, z each). */
  pickupPoints: ArrayLike<number> | undefined;
  /** Results or lobby: loops fade out, the HUD hides. */
  quiet: boolean;
}

export interface VisitorFrame {
  /** Pickup entity ids the scene should not draw (mimicked pickups). */
  hidden: ReadonlySet<number>;
  lights: PointLight[];
  hud: VisitorHudModel;
}

/** `?visitors=off`: the visitors as plain fallback shapes (before shots). */
export function wantsVisitors(loc: { search: string } | undefined): boolean {
  return !loc || new URLSearchParams(loc.search).get("visitors") !== "off";
}

/** Parses `Game.pickupPoints()`. */
export function parsePoints(a: ArrayLike<number> | undefined): Vec3[] {
  const out: Vec3[] = [];
  if (!a) return out;
  for (let i = 0; i + 3 <= a.length; i += 3) out.push({ x: a[i] ?? 0, y: a[i + 1] ?? 0, z: a[i + 2] ?? 0 });
  return out;
}

type Loop = NonNullable<ReturnType<VisitorSoundEngine["loop"]>>;

/** Everything of C2 behind one object (main.ts, the gallery). */
export class VisitorVisuals {
  readonly tax = new TaxDroneFx();
  readonly pizza = new PizzaFx();
  readonly survey = new SurveyFx();
  readonly mimics = new MimicFx();
  readonly gossip = new GossipFx();
  readonly enabled: boolean;
  hud: VisitorHud | null = null;
  private readonly sink: VfxSink | null;
  private readonly engine: VisitorSoundEngine | null;
  private scene: SceneInstances = emptyScene();
  private loops = new Map<string, Loop>();
  private points: Vec3[] = [];
  private pointsKey = "";

  constructor(sink: VfxSink | null, enabled: boolean, engine: VisitorSoundEngine | null = null, ui: HTMLElement | null = null) {
    this.sink = sink;
    this.enabled = enabled;
    this.engine = enabled ? engine : null;
    if (enabled && ui) this.hud = new VisitorHud(ui);
  }

  /** Entity kinds the scene builder leaves to this module. */
  get skipKinds(): ReadonlySet<string> | undefined {
    return this.enabled ? VISITOR_KINDS : undefined;
  }

  reset(): void {
    this.tax.reset();
    this.pizza.reset();
    this.survey.reset();
    this.mimics.reset();
    this.gossip.reset();
    this.stopLoops();
    this.hud?.clear();
    this.points = [];
    this.pointsKey = "";
    this.scene = emptyScene();
  }

  update(f: VisitorFrameInput): VisitorFrame {
    this.scene = emptyScene();
    const lights: PointLight[] = [];
    if (!this.enabled) return { hidden: new Set(), lights, hud: { markers: [], bubble: null, survey: false, pizza: null, toasts: [] } };
    if (f.pickupPoints && f.pickupPoints.length !== this.points.length * 3) this.pointsKey = "";
    if (!this.pointsKey && f.pickupPoints) {
      this.points = parsePoints(f.pickupPoints);
      this.pointsKey = String(this.points.length);
    }
    const i: VisitorInput = {
      time: f.time,
      dt: f.dt,
      nowTick: f.nowTick,
      camera: f.camera,
      orientation: f.orientation,
      mood: f.mood,
      outcomes: outcomes(f.cues),
      entities: f.entities,
      ships: f.ships,
      own: f.own,
      name: (id) => f.names.get(id) || `Pilot ${id}`,
      visuals: f.visuals,
      pickupPoints: this.points,
    };
    const ctx: DrawCtx = { scene: this.scene, time: f.time, lights, sink: this.sink, up: f.up };
    const out: VisitorOut = { toasts: [], sounds: [] };
    this.tax.update(i, ctx, out);
    this.pizza.update(i, ctx, out);
    this.survey.update(i, ctx, out);
    this.mimics.update(i, ctx, out);
    const gossip = this.gossip.update(i, ctx, out, f.ownPosition);
    this.sounds(out.sounds, f.quiet);
    const hud: VisitorHudModel = f.quiet
      ? { markers: [], bubble: null, survey: false, pizza: null, toasts: [] }
      : {
          markers: gossip?.markers ?? [],
          bubble: gossip?.bubble ?? null,
          survey: this.survey.ownSurveyed(i),
          pizza: this.pizza.ownHold(i),
          toasts: out.toasts,
        };
    return { hidden: this.mimics.hidden(i), lights, hud };
  }

  /** The HUD for this frame (`project` maps world points to clip space). */
  updateHud(model: VisitorHudModel, project: Project | null, width: number, height: number, now: number): void {
    this.hud?.update(model, project, width, height, now);
  }

  /** The visitors' instances of the last update into `scene`. */
  addToScene(scene: SceneInstances): void {
    for (const pass of ["opaque", "translucent"] as const) {
      for (const [kind, list] of Object.entries(this.scene[pass])) {
        if (!list || list.length === 0) continue;
        (scene[pass][kind as MeshKind] ??= []).push(...list);
      }
    }
  }

  dispose(): void {
    this.stopLoops();
    this.hud?.dispose();
    this.hud = null;
  }

  private sounds(calls: readonly SoundCall[], quiet: boolean): void {
    const e = this.engine;
    const wanted = new Map<string, SoundCall>();
    for (const c of calls) {
      if (c.loop) wanted.set(c.loop, c);
      else if (e && !quiet) e.play(c.id, { position: c.position, gain: c.gain ?? 1 });
    }
    for (const [key, h] of this.loops) {
      if (!wanted.has(key) || quiet || !h.playing) {
        h.stop(0.4);
        this.loops.delete(key);
      }
    }
    if (!e || quiet) return;
    for (const [key, c] of wanted) {
      let h = this.loops.get(key);
      if (!h) {
        h = e.loop(c.id, { position: c.position, gain: c.gain ?? 1 }) ?? undefined;
        if (!h) continue;
        this.loops.set(key, h);
      }
      if (c.position) h.setPosition(c.position);
    }
  }

  private stopLoops(): void {
    for (const h of this.loops.values()) h.stop(0.2);
    this.loops.clear();
  }
}
