// Shared types of the comedy event visuals (M4 C1, game/events/). One module
// per event implements `EventVisual`; the hub (index.ts) runs the module of
// the event that runs (and lets it linger after the end until it says it is
// done), merges their outputs and puts their props into the scene.

import type { Quat } from "../../math/quat.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { LoopHandle, PlayOptions } from "../../audio/engine.ts";
import type { PointLight } from "../../renderer/lights.ts";
import type { ParticleSpec } from "../../renderer/particles.ts";
import type { PixelLook } from "../../renderer/pixelPost.ts";
import type { CakeWave, CameraView, Instance, Rgb } from "../../renderer/renderer.ts";
import type { MeshKind } from "../../renderer/models/index.ts";
import type { Cue } from "../mechanics.ts";
import type { MoodState } from "../modeState.ts";
import type { EntityState, ProjectileState, ShipState, VisualDef } from "../views.ts";
import type { EventClock, EventFx } from "./fx.ts";

/** Where particles go (the renderer's ParticleSystem, or a test recorder). */
export interface ParticleSink {
  spawn(p: ParticleSpec): void;
  immediate(p: ParticleSpec): void;
}

/** What plays sounds (the audio engine, or a test recorder). */
export interface SoundSink {
  play(id: string, o?: PlayOptions): boolean;
  loop(id: string, o?: PlayOptions): LoopHandle | null;
}

/** The own ship this frame (alive and flying), else null in the input. */
export interface OwnShip {
  id: number;
  team: number;
  position: Vec3;
  orientation: Quat;
  velocity: Vec3;
  shipType: number;
  radius: number;
}

/** A gravity well (ls-wasm `Game.wells`). */
export interface WellPoint {
  center: Vec3;
  core: number;
  range: number;
  dynamic: boolean;
}

export interface EventInput {
  /** Wall time (s; particle clock). */
  time: number;
  dt: number;
  /** Fractional server tick (Game.nowTick). */
  tick: number;
  fx: EventFx;
  mood: MoodState;
  /** The camera as the game computed it (before any event override). */
  camera: CameraView;
  own: OwnShip | null;
  ships: readonly ShipState[];
  shots: readonly ProjectileState[];
  entities: readonly EntityState[];
  visuals: readonly VisualDef[];
  cues: readonly Cue[];
  wells: readonly WellPoint[];
  /** The level's up now. */
  up: Vec3;
  /** Side colour of a ship (display). */
  colorOf: (id: number) => Rgb;
  /** First rock along a ray (distance, < 0 for none). */
  raycast?: ((o: Vec3, d: Vec3, max: number) => number) | undefined;
  /** Results or lobby showing: no sounds or HUD. */
  quiet: boolean;
  /** Drawing buffer height (px; the pixel look's cell size). */
  screenHeight: number;
  /** What the visitor events (C2, visitors.ts) need beyond this; absent = the hub runs no visitors this frame. */
  visitors?: VisitorExtras | undefined;
}

/** The visitor events' extra input (main.ts; the hub hands it to `VisitorVisuals`). */
export interface VisitorExtras {
  /** The local player's id (also while spectating or guiding a missile). */
  own: number;
  /** The own ship's position while it flies (gossip directions), else null. */
  ownPosition: Vec3 | null;
  /** Remote ships plus the own one (pizza recipient, mimic prey, the leader's touches). */
  ships: readonly { id: number; alive: boolean; position: Vec3 }[];
  names: ReadonlyMap<number, string>;
  /** ls-wasm `Game.pickupPoints()`. */
  pickupPoints: ArrayLike<number> | undefined;
}

/** Everything the event visuals change this frame (merged over the running modules). */
export interface EventOutput {
  /** Camera to render instead of the game's (side view, hiccup bump), or null. */
  view: CameraView | null;
  /** Extra screen shake (mechanics.ts shakeOffset amplitude). */
  shake: number;
  /** The own ship is seen from outside: draw it, hide the cockpit frame and the crosshair. */
  thirdPerson: boolean;
  lights: PointLight[];
  pixel: PixelLook | null;
  cake: CakeWave | null;
  /** Terrain draws its back faces as the cut fill (side view). */
  cutaway: boolean;
  /** Projectiles drawn by an event (rubber chickens): the scene and effects leave them out. */
  hideShots: Set<ProjectileState>;
  /** Entity ids drawn by an event (pickups in boxes). */
  hideEntities: Set<number>;
  /** Ships to draw instead of the game's (flattened into the side view's plane), or null. */
  ships: ShipState[] | null;
  /** Projectiles to draw instead of the game's, or null. */
  shots: ProjectileState[] | null;
  /** Instances to add, by mesh kind and pass. */
  opaque: Array<[MeshKind, Instance]>;
  translucent: Array<[MeshKind, Instance]>;
  /** Debris drawn in another colour (cake), or null. */
  debrisColor: Rgb | null;
  /** Ships drawn bigger, brighter and rolled toward the camera by `roll` rad (the side view's sprites), or null. */
  shipBoost: { scale: number; emissive: number; roll: number } | null;
  /** HUD hints (ui/eventBanner.ts). */
  hud: EventHudHints;
}

/** What the event HUD shows besides the banner. */
export interface EventHudHints {
  /** Bored level: the new down as a screen angle (rad, 0 = down the screen, clockwise) and the arrow's opacity. */
  down: { angle: number; alpha: number } | null;
  /** Wrong genre: a retro title card. */
  retro: { title: string; sub: string; alpha: number } | null;
  /** Bored level's announcement: sleepy letters drifting over the screen (0..1). */
  zzz: number;
  /** Comic words popping on the screen ("HIC!"): position in screen fractions, scale, tilt (deg), opacity. */
  pops: Array<{ text: string; x: number; y: number; scale: number; tilt: number; alpha: number; color: string }>;
}

export function emptyOutput(): EventOutput {
  return {
    view: null,
    shake: 0,
    thirdPerson: false,
    lights: [],
    pixel: null,
    cake: null,
    cutaway: false,
    hideShots: new Set(),
    hideEntities: new Set(),
    ships: null,
    shots: null,
    opaque: [],
    translucent: [],
    debrisColor: null,
    shipBoost: null,
    hud: { down: null, retro: null, zzz: 0, pops: [] },
  };
}

/** Context the hub hands every module. */
export interface EventEnv {
  particles: ParticleSink | null;
  sound: SoundSink | null;
}

/** One event's visuals. */
export interface EventVisual {
  /**
   * One frame while its event runs (announce, active, ending) and after it
   * is over (`clock.over` = seconds since); returns false once it has nothing
   * more to show after the end.
   */
  update(input: EventInput, clock: EventClock, out: EventOutput): boolean;
  /** Stops loops and timers (a new match, the page closing). */
  dispose?(): void;
}

/** Makes a module for one run of its event. */
export type EventVisualFactory = (env: EventEnv) => EventVisual;
