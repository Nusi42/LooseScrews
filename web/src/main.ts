// Loose Screws browser client entry point.
//
// Flow: start screen -> WebGPU renderer -> world -> frame loop.
// Wave 2d renders a procedural test scene with an offline preview world; in
// wave 3 the marked plug-in points take terrain chunks from ls-wasm and ship
// state from the network (snapshots + prediction) instead.

import { FlightCamera } from "./input/flightCamera.ts";
import { InputController } from "./input/input.ts";
import { GpuUnavailableError } from "./renderer/gpu.ts";
import { Renderer, type FrameStats } from "./renderer/renderer.ts";
import { DemoWorld } from "./game/demo.ts";
import { generateTestScene } from "./scene/testScene.ts";
import { showErrorScreen } from "./ui/errorScreen.ts";
import { Hud } from "./ui/hud.ts";
import { showStartScreen, type StartOptions } from "./ui/startScreen.ts";

/** Largest simulated step per frame; longer stalls (tab switch) are clamped. */
const MAX_FRAME_DT = 0.1;

function mustGet<T extends HTMLElement>(id: string, ctor: new () => T): T {
  const el = document.getElementById(id);
  if (!(el instanceof ctor)) throw new Error(`missing #${id}`);
  return el;
}

async function start(): Promise<void> {
  const canvas = mustGet("game", HTMLCanvasElement);
  const ui = mustGet("ui", HTMLDivElement);

  const options = await showStartScreen(ui);

  let renderer: Renderer;
  try {
    renderer = await Renderer.create(canvas);
  } catch (err) {
    if (err instanceof GpuUnavailableError) {
      showErrorScreen(ui, "WebGPU unavailable", err.message, err.reason);
      return;
    }
    throw err;
  }
  renderer.lost
    .then((reason) => showErrorScreen(ui, "GPU device lost", reason, "lost"))
    .catch(() => {});

  runGame(canvas, ui, renderer, options);
}

function runGame(canvas: HTMLCanvasElement, ui: HTMLElement, renderer: Renderer, options: StartOptions): void {
  // Plug-in point (wave 3): decode `options.shareCode` (or accept the server's)
  // and stream chunk meshes from ls-wasm into renderer.setChunk(key, mesh).
  const scene = generateTestScene();
  for (const [key, mesh] of scene.chunks) renderer.setChunk(key, mesh);

  const camera = new FlightCamera(scene.spawnPosition);
  camera.orientation = scene.spawnOrientation;
  camera.horizonAssist = options.horizonAssist;

  // Plug-in point (wave 3): replace DemoWorld with the networked match state.
  const world = new DemoWorld(scene);

  const hud = new Hud(ui);
  hud.show(true);
  const input = new InputController(canvas);
  input.onLockChange = (locked) => hud.showPrompt(!locked);
  hud.showPrompt(true);
  input.requestLock(); // may succeed while the Play click still counts as a user gesture

  const codeLabel = options.shareCode ?? "server world";
  let last = performance.now();
  let fps = 60;
  let stats: FrameStats = { chunksTotal: 0, chunksDrawn: 0, triangles: 0, instances: 0 };
  let debugTimer = 0;

  const frame = (now: number): void => {
    // A lost device cannot render; the error screen is shown by the `lost` handler.
    if (renderer.isLost) return;
    const dt = Math.min(MAX_FRAME_DT, Math.max(0, (now - last) / 1000));
    last = now;
    if (dt > 0) fps += (1 / dt - fps) * 0.05;

    const state = input.sample();
    const boosting = world.canBoost(state);
    camera.step(state, dt, boosting);
    world.update(dt, state, camera, boosting);

    renderer.setInstances("ships", world.ships());
    renderer.setInstances("projectiles", world.projectileInstances());
    stats = renderer.render(camera.view());

    debugTimer -= dt;
    const hudValues = { ...world.hud(), horizonAssist: camera.horizonAssist, scavenging: state.scavenge };
    if (debugTimer <= 0) {
      debugTimer = 0.25;
      hud.update({
        ...hudValues,
        debug:
          `${fps.toFixed(0)} fps · chunks ${stats.chunksDrawn}/${stats.chunksTotal} · ` +
          `${Math.round(stats.triangles / 1000)}k tris · ${camera.speed.toFixed(1)} u/s · ${codeLabel}` +
          (renderer.lastError ? ` · GPU error: ${renderer.lastError}` : ""),
      });
    } else {
      hud.update(hudValues);
    }
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

start().catch((err: unknown) => {
  const message = err instanceof Error ? err.message : String(err);
  const ui = document.getElementById("ui") ?? document.body;
  showErrorScreen(ui, "Failed to start", message, "other");
});
