// Live backdrop of the start screen (first-impression pass, FI): a generated
// level flown through by the cinematic camera, with a dogfight ahead of it
// (cinematic.ts), drawn by the game's renderer with every effect on
// (terrain, sky, particles, dynamic lights, post). The renderer is the one
// the game uses afterwards, so its pipelines and its quality pick are warm
// when the player presses Play; `stop()` hands it back empty.
//
// The level is built in offline practice (ls-wasm, no dummies) from one of
// BACKDROP_WORLDS (caverns that open to the sky, picked per visit) or a given
// share code. Chunks stream in under a small per-frame budget so the start
// screen stays responsive; the camera holds still until the chunks around
// it are in, then the page fades the scene in (`body.backdrop-live`).
//
// Marks for tools/browser-smoke/startup.mjs: `performance.mark("ls:backdrop-frame")`
// at the first frame shown, `body.dataset.backdropFrames` counts frames.

import { atmosphereFor } from "../renderer/atmosphere.ts";
import { hashSeed } from "../renderer/rockTextures.ts";
import { mulberry32 } from "../renderer/spriteAtlas.ts";
import type { CameraView, FrameStats, Renderer } from "../renderer/renderer.ts";
import { fovYFor } from "../game/camera.ts";
import { ChunkLoader, type ChunkBudget } from "../game/chunks.ts";
import { LightFx, sceneLights } from "../game/lightFx.ts";
import { buildScene } from "../game/scene.ts";
import { Vfx } from "../game/vfx.ts";
import { parsePose, parseVisuals, type VisualDef } from "../game/views.ts";
import * as q from "../math/quat.ts";
import { CONTROLS_LEN, type GameHandle, type WasmApi } from "../wasm/api.ts";
import { CinematicCamera, DEFAULT_CINEMATIC, Squadron } from "./cinematic.ts";
import { DEFAULT_PATH, FlightPath, wasmProbe } from "./flightPath.ts";

/** Worlds the start screen flies (seed, Enclosure, Density): caverns opening to the sky. */
export const BACKDROP_WORLDS: ReadonlyArray<{ seed: number; enclosure: number; density: number }> = [
  // Curated in the visual gallery (`?gallery=start-flythrough&world=<seed>_<enclosure>_<density>`):
  // each has its planet in view along the flight.
  { seed: 6, enclosure: 35, density: 65 },
  { seed: 10, enclosure: 36, density: 60 },
  { seed: 4, enclosure: 42, density: 55 },
  { seed: 16, enclosure: 40, density: 65 },
  { seed: 2, enclosure: 38, density: 60 },
  { seed: 13, enclosure: 42, density: 60 },
];

/** Chunk meshing per frame while the backdrop loads and flies: well inside a frame, no long tasks. */
export const BACKDROP_LOAD_BUDGET: ChunkBudget = { millis: 10, maxChunks: 12 };
export const BACKDROP_FLY_BUDGET: ChunkBudget = { millis: 4, maxChunks: 6 };
/** Meshing per frame while a quietly loaded level (nothing drawn yet, the workshop preview) waits for its chunks. */
export const BACKDROP_QUIET_LOAD_BUDGET: ChunkBudget = { millis: 20, maxChunks: 32 };
/** The camera starts once at most this many chunks around it are missing, or after HOLD_MAX_SECS. */
const READY_PENDING = 0;
const HOLD_MAX_SECS = 6;
/** Chunk generation per frame (ms; at least one chunk) while a lazily built level is generated. */
const GENERATE_BUDGET_MS = 8;
/** Exposure lift of the backdrop over the game's (the UI shading darkens it). */
export const BACKDROP_EXPOSURE = 1.18;
/** Horizontal field of view of the cinematic camera (degrees at 16:9). */
export const CINEMATIC_FOV_DEG = 84;

/** Camera framing for a panel docked left: look this far left of the flight on wide screens (rad). */
export function frameYawFor(aspect: number): number {
  return aspect >= 1.3 ? 0.2 : 0;
}

export interface BackdropOptions {
  /** Share code to fly (default: one of BACKDROP_WORLDS). */
  code?: string;
  /** Picks the world and seeds the dogfight (default: random). */
  seed?: number;
  /** Fixed step (s) instead of the wall clock (the visual gallery). */
  fixedStep?: number;
  /**
   * Draw nothing until the scene is ready, so the canvas keeps showing the
   * previous world's last frame while this one is built (the workshop preview).
   */
  quietLoad?: boolean;
  /** Chunk generation per frame while the level is built (ms; default GENERATE_BUDGET_MS). */
  generateBudgetMs?: number;
}

export class Backdrop {
  readonly shareCode: string;
  readonly game: GameHandle;
  private readonly renderer: Renderer;
  private readonly chunks: ChunkLoader;
  private readonly camera: CinematicCamera | null;
  private readonly squadron: Squadron | null;
  private readonly vfx: Vfx | null;
  private readonly lightFx = new LightFx();
  private readonly visuals: VisualDef[];
  private readonly fixedStep: number | null;
  private view: CameraView;
  private raf = 0;
  private last = 0;
  private clock = 0;
  private holding = 0;
  private flying = false;
  private stopped = false;
  /** Exposure before the backdrop's lift (restored by `stop`). */
  private readonly exposure: number;
  /** Chunks still to generate (lazily built levels). */
  private generateLeft = Infinity;
  /** Most chunks seen left to generate (the progress denominator). */
  private generateTotal = 0;
  private readonly quietLoad: boolean;
  private readonly generateBudgetMs: number;
  /** Frames rendered. */
  frames = 0;
  /** Holds the current frame (camera, action and shader time stand still; the gallery). */
  hold = false;
  /** Draw statistics of the last frame. */
  lastStats: FrameStats = { chunksTotal: 0, chunksDrawn: 0, triangles: 0, instances: 0 };
  /** Called once the scene is ready to be shown (chunks around the camera are in). */
  onReady: (() => void) | null = null;

  /** Builds the level (synchronous worldgen in ls-wasm) and prepares the renderer. */
  constructor(renderer: Renderer, api: WasmApi, options: BackdropOptions = {}) {
    const seed = options.seed ?? Math.floor(Math.random() * 2 ** 31);
    const rng = mulberry32(seed);
    const world = BACKDROP_WORLDS[seed % BACKDROP_WORLDS.length]!;
    this.shareCode = options.code ?? api.randomShareCode(world.seed, 0, world.enclosure, world.density);
    this.renderer = renderer;
    this.fixedStep = options.fixedStep ?? null;
    this.quietLoad = options.quietLoad ?? false;
    this.generateBudgetMs = options.generateBudgetMs ?? GENERATE_BUDGET_MS;
    // Lazily generated (no long task); older cores generate the whole level here.
    this.game = api.Game.practicePreview?.(this.shareCode) ?? api.Game.practice(this.shareCode, 0);
    this.game.frame(0, new Float32Array(CONTROLS_LEN));
    const spawn = parsePose(this.game.localPose());
    const start = spawn?.position ?? { x: 0, y: 0, z: 0 };
    const heading = spawn ? q.forward(spawn.orientation) : { x: 0, y: 0, z: -1 };
    this.visuals = parseVisuals(this.game.visuals(), api.visualShapeNames());
    const probe = wasmProbe(this.game);
    const atmosphere = atmosphereFor(this.shareCode, this.game.bounds());
    if (probe) {
      // Lean toward the planet (else the sun): the sky's best side faces the camera more often.
      const toward = atmosphere.planet?.direction ?? atmosphere.sunDirection;
      const path = new FlightPath(probe, start, heading, rng, { ...DEFAULT_PATH, toward, towardWeight: 0.45 });
      // Frame the flight right of centre on landscape screens (the start panel docks left).
      this.camera = new CinematicCamera(path, { ...DEFAULT_CINEMATIC, frameYaw: frameYawFor(renderer.aspect) });
      this.squadron = new Squadron(path, probe, this.visuals, rng);
    } else {
      this.camera = null;
      this.squadron = null;
    }
    renderer.setAtmosphere(atmosphere);
    // A touch brighter than in play: the backdrop is seen through the UI's shading.
    this.exposure = renderer.settings.post.exposure;
    renderer.settings.post = { ...renderer.settings.post, exposure: this.exposure * BACKDROP_EXPOSURE };
    renderer.settings.terrainSeed = hashSeed(this.shareCode);
    renderer.settings.skyHaze = renderer.atmosphere ? 1 : 0;
    renderer.hullFrame.state = null;
    const particles = renderer.particles;
    particles?.clear();
    this.vfx = particles ? new Vfx(particles, rng) : null;
    if (this.vfx) {
      this.vfx.lightSink = this.lightFx;
      this.lightFx.watchProjectiles = false;
    }
    this.lightFx.random = rng;
    this.chunks = new ChunkLoader((key, mesh) => renderer.setChunk(key, mesh));
    // The path is planned in the first step (its probes are a task of their own).
    const pose = { position: start, orientation: spawn?.orientation ?? q.identity() };
    this.view = { ...pose, fovY: fovYFor(CINEMATIC_FOV_DEG, renderer.aspect), near: 0.05, far: 600 };
  }

  /** Share of the level generated and the chunks around the camera meshed (0..1; 1 once flying). */
  get progress(): number {
    if (this.flying) return 1;
    if (!Number.isFinite(this.generateLeft) || this.generateTotal === 0) return 0;
    // Generation is most of the wait; meshing the chunks around the camera the rest.
    return 0.9 * (1 - this.generateLeft / this.generateTotal);
  }

  /** True once the camera flies (the scene is worth showing). */
  get ready(): boolean {
    return this.flying;
  }

  /** Current camera pose. */
  get cameraView(): CameraView {
    return this.view;
  }

  /** Runs the backdrop on animation frames until `stop()`. */
  start(): void {
    if (this.raf || this.stopped) return;
    this.last = performance.now();
    const tick = (now: number): void => {
      if (this.stopped || this.renderer.isLost) return;
      const dt = Math.min(0.1, Math.max(0, (now - this.last) / 1000));
      this.last = now;
      this.step(dt);
      this.raf = requestAnimationFrame(tick);
    };
    this.raf = requestAnimationFrame(tick);
  }

  /** One frame: stream chunks, advance the flight and the action, render. */
  step(wallDt: number): void {
    if (this.stopped) return;
    const dt = this.hold ? 0 : (this.fixedStep ?? wallDt);
    const renderer = this.renderer;
    // A lazily built level is generated one chunk at a time (each a few tens of ms) before any meshing,
    // so no frame blocks the page for long; then the chunks around the camera are meshed.
    let generating = false;
    if (this.generateLeft > 0 && this.game.generateNearest) {
      const t0 = performance.now();
      const at = this.view.position;
      do {
        this.generateLeft = this.game.generateNearest(at.x, at.y, at.z);
        this.generateTotal = Math.max(this.generateTotal, this.generateLeft + 1);
      } while (this.generateLeft > 0 && performance.now() - t0 < this.generateBudgetMs);
      generating = this.generateLeft > 0;
    }
    const stats = generating
      ? { meshed: 0, removed: 0, pending: 1, shown: 0 }
      : this.chunks.update(
          this.game,
          this.view.position,
          this.flying ? BACKDROP_FLY_BUDGET : this.quietLoad ? BACKDROP_QUIET_LOAD_BUDGET : BACKDROP_LOAD_BUDGET,
        );
    if (!this.flying && !generating) {
      this.holding += dt;
      if (stats.pending <= READY_PENDING || this.holding >= HOLD_MAX_SECS) {
        this.flying = true;
        this.onReady?.();
      }
    }
    if (this.quietLoad && !this.flying) return;
    const flyDt = this.flying ? dt : 0;
    this.clock += flyDt;
    const time = this.clock;
    const pose = this.camera?.update(flyDt) ?? this.view;
    this.view = { ...pose, fovY: fovYFor(CINEMATIC_FOV_DEG, renderer.aspect), near: 0.05, far: 600 };
    const action = this.squadron && this.flying ? this.squadron.update(time, flyDt, this.camera?.s ?? 0, pose.position) : { ships: [], shots: [], cues: [] };
    const teamOf = (id: number): number | undefined => action.ships.find((s) => s.id === id)?.team;
    renderer.setScene(buildScene(action.ships, action.shots, [], this.visuals, teamOf, time, { vfx: this.vfx !== null }));
    if (this.flying) {
      this.vfx?.update({
        time,
        dt: flyDt,
        camera: pose.position,
        cameraOrientation: pose.orientation,
        player: -1,
        pose: null,
        ships: action.ships,
        shots: action.shots,
        entities: [],
        visuals: this.visuals,
        cues: action.cues,
        beams: [],
        flashes: [],
      });
    }
    const lights = sceneLights({ ships: action.ships, shots: action.shots, visuals: this.visuals, time });
    renderer.setLights(this.lightFx.frame(time, lights));
    if (this.fixedStep !== null) renderer.timeOverride = time;
    this.lastStats = renderer.render(this.view);
    this.frames++;
    if (this.flying && this.frames > 0) {
      if (performance.getEntriesByName("ls:backdrop-frame").length === 0) performance.mark("ls:backdrop-frame");
      document.body.dataset["backdropFrames"] = String(this.frames);
    }
  }

  /** Stops the flight and hands the renderer back empty (chunks, instances, particles, lights, sky). */
  stop(): void {
    if (this.stopped) return;
    this.stopped = true;
    if (this.raf) cancelAnimationFrame(this.raf);
    this.raf = 0;
    delete document.body.dataset["backdropFrames"];
    const r = this.renderer;
    if (!r.isLost) {
      this.chunks.clear();
      r.clearInstances();
      r.particles?.clear();
      r.setLights([]);
      r.setAtmosphere(null);
      r.settings.post = { ...r.settings.post, exposure: this.exposure };
      r.timeOverride = null;
    }
    this.vfx?.reset();
    this.game.free();
  }
}
