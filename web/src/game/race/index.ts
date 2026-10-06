// Belt Up in the client (M4 C7): one object the game loop (main.ts) calls
// once a frame on any level. On a belt level it draws the belt's look
// (renderer.setBelt, the sooty rock colour, boulders, pebbles, start-light
// gantries, lights, particles and sounds: raceFx.ts); in a Belt Up match it
// also runs the race HUD (ui/raceHud.ts: lights, position, gaps, distance,
// minimap, wrong way, radiation meter, finish banner, heat results) and
// points the spectator camera of a finished racer at the race leader.
// Everything else stays untouched on classic levels.

import type { Vec3 } from "../../math/vec3.ts";
import type { PointLight } from "../../renderer/lights.ts";
import { DEFAULT_SETTINGS, type CameraView, type Renderer, type Rgb, type SceneInstances } from "../../renderer/renderer.ts";
import { RaceHud, raceHudModel } from "../../ui/raceHud.ts";
import type { GameHandle } from "../../wasm/api.ts";
import { VIEW_FLAG, type ModeState } from "../modeState.ts";
import type { ShipState } from "../views.ts";
import { parseBelt, WrongWay, type Belt } from "./course.ts";
import { BELT_ROCK, RaceVisuals, type RaceParticleSink, type RaceSoundEngine } from "./raceFx.ts";

export { BELT_ROCK } from "./raceFx.ts";

/** What the race needs of a frame. */
export interface RaceClientInput {
  game: GameHandle;
  time: number;
  dt: number;
  view: CameraView;
  mode: ModeState;
  /** Own ship pose (null when there is none). */
  pose: { position: Vec3; velocity: Vec3; alive: boolean } | null;
  ships: readonly ShipState[];
  names: ReadonlyMap<number, string>;
  /** Own colour (passed gates, the minimap). */
  ownColor: Rgb;
  hull: number;
  hullMax: number;
  spectating: boolean;
  /** Overlays up (lobby, match results): the HUD and race sounds rest. */
  quiet: boolean;
  width: number;
  height: number;
  /** Announcer (A6) for "Belt up!". */
  say?: ((trigger: "beltUp") => void) | undefined;
}

/** Stable 32-bit seed of a share code (the pebble field). */
function seedOf(code: string): number {
  let h = 2166136261;
  for (let i = 0; i < code.length; i++) h = Math.imul(h ^ code.charCodeAt(i), 16777619);
  return h >>> 0;
}

export class RaceClient {
  readonly visuals: RaceVisuals;
  readonly hud: RaceHud;
  private readonly wrong = new WrongWay();
  private belt: Belt | null = null;
  private beltCode = "";
  private autoFollow = true;
  private followHeat = -1;

  constructor(ui: HTMLElement, sink: RaceParticleSink | null, engine: RaceSoundEngine | null) {
    this.visuals = new RaceVisuals(sink, engine);
    this.hud = new RaceHud(ui);
    this.hud.root.classList.add("hidden");
  }

  /** The level's belt (parsed once per share code); null on classic levels. */
  beltOf(game: GameHandle): Belt | null {
    if (game.shareCode !== this.beltCode) {
      this.beltCode = game.shareCode;
      this.belt = parseBelt(game.beltSite?.());
    }
    return this.belt;
  }

  /** A new match on the same connection. */
  reset(): void {
    this.visuals.reset();
    this.beltCode = "";
    this.belt = null;
    this.autoFollow = true;
    this.followHeat = -1;
  }

  /**
   * One frame: sets the renderer's belt look and rock colour, adds the
   * race's instances to `scene` and its lights to `lights`, updates the HUD.
   */
  frame(i: RaceClientInput, renderer: Renderer, scene: SceneInstances, lights: PointLight[]): void {
    const game = i.game;
    const belt = this.beltOf(game);
    const race = i.mode.race;
    const nowTick = game.nowTick?.() ?? 0;
    const own = game.player;
    const ownPose = i.pose && i.pose.alive && !i.spectating ? { position: i.pose.position, velocity: i.pose.velocity } : null;
    const racer = race?.racers.find((r) => r.id === own) ?? null;
    this.visuals.wrongWay = belt !== null && race?.phase === "racing" && this.wrong.update(belt, racer, ownPose?.position ?? null, ownPose?.velocity ?? null, i.time);
    const rf = this.visuals.update({
      time: i.time,
      dt: i.dt,
      nowTick,
      camera: i.view.position,
      orientation: i.view.orientation,
      belt,
      race,
      own,
      ownPose,
      ownColor: i.ownColor,
      ships: i.ships,
      boulderPoses: game.boulderPoses ? (t) => game.boulderPoses?.(t) ?? new Float32Array(0) : undefined,
      doseAt: game.doseRateAt ? (p) => game.doseRateAt?.(p.x, p.y, p.z) ?? 0 : undefined,
      quiet: i.quiet,
      seed: seedOf(game.shareCode),
      say: i.say,
    });
    renderer.setBelt(rf.look);
    renderer.settings.rockColor = belt ? BELT_ROCK : DEFAULT_SETTINGS.rockColor;
    if (belt) {
      this.visuals.addToScene(scene);
      lights.push(...rf.lights);
    }
    // The race HUD (Belt Up only).
    const positions = new Map<number, Vec3>();
    for (const s of i.ships) if (s.alive) positions.set(s.id, s.position);
    if (ownPose) positions.set(own, ownPose.position);
    const points = new Map(i.mode.players.map((p) => [p.id, p.score]));
    const sides = new Map(i.mode.players.map((p) => [p.id, p.side]));
    this.hud.update(
      raceHudModel({
        belt: race && !i.quiet ? belt : null,
        race,
        own,
        names: i.names,
        sides,
        points,
        positions,
        ownPos: ownPose?.position ?? null,
        ownVel: ownPose?.velocity ?? null,
        nowTick,
        raceStart: rf.raceStart,
        dose: rf.dose,
        hull: i.hull,
        hullMax: i.hullMax,
        instagib: (i.mode.flags & VIEW_FLAG.instagib) !== 0,
        lights: rf.lights5,
        wrongWay: this.visuals.wrongWay,
        viewProj: renderer.viewProjection(i.view),
        width: i.width,
        height: i.height,
        time: i.time,
        spectating: i.spectating,
      }),
    );
  }

  /**
   * Spectator camera of a racer who finished (or waits for the next heat):
   * the id of the best racer still racing to follow, while the player has
   * not taken the camera over (right click: free camera); null otherwise.
   */
  followTarget(mode: ModeState, ships: readonly ShipState[], following: number | null, freePressed: boolean): number | null {
    const race = mode.race;
    if (!race || race.phase !== "racing") return null;
    if (race.heat !== this.followHeat) {
      this.followHeat = race.heat;
      this.autoFollow = true;
    }
    if (freePressed) this.autoFollow = false;
    if (!this.autoFollow || following !== null) return null;
    const alive = new Set(ships.filter((s) => s.alive).map((s) => s.id));
    const leader = [...race.racers].filter((r) => r.finishTick === 0 && alive.has(r.id)).sort((a, b) => (a.place || 99) - (b.place || 99))[0];
    return leader ? leader.id : null;
  }

  dispose(): void {
    this.visuals.dispose();
    this.hud.dispose();
  }
}
