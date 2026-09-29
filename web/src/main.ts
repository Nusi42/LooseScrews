// Loose Screws browser client entry point.
//
// Flow: load ls-wasm -> start screen (online or practice) -> WebGPU renderer ->
// connect (WebSocket, Hello/Welcome) or build a local practice world -> frame loop.
// Each frame: feed received packets to ls-wasm, step with the sampled controls
// (prediction online, local simulation in practice), send inputs, stream and
// remesh terrain chunks within a time budget, draw, update the HUD.
// While the tab is hidden (no animation frames) a timer keeps the online
// session alive with idle input, so the server neither drops the player
// after its idle timeout nor keeps repeating held buttons.

import { InputController } from "./input/input.ts";
import { GpuUnavailableError } from "./renderer/gpu.ts";
import { Renderer, type FrameStats } from "./renderer/renderer.ts";
import { boundsCenter, ShipCamera } from "./game/camera.ts";
import { ChunkLoader, LOADING_BUDGET, PLAY_BUDGET, type ChunkStats } from "./game/chunks.ts";
import { GameSession, type FrameResult } from "./game/session.ts";
import { projectileInstances, shipInstances } from "./game/views.ts";
import { serverUrl } from "./net/transport.ts";
import { showErrorScreen } from "./ui/errorScreen.ts";
import { Hud } from "./ui/hud.ts";
import { showLoading, showStartScreen, type StartOptions } from "./ui/startScreen.ts";
import { loadWasm } from "./wasm/load.ts";
import type { WasmApi } from "./wasm/api.ts";

/** Largest frame time passed to the simulation; longer stalls (tab switch) are clamped. */
const MAX_FRAME_DT = 0.1;
/** Background pump interval while the tab is hidden (browsers throttle it to ~1 Hz). */
const KEEP_ALIVE_MS = 250;

function mustGet<T extends HTMLElement>(id: string, ctor: new () => T): T {
  const el = document.getElementById(id);
  if (!(el instanceof ctor)) throw new Error(`missing #${id}`);
  return el;
}

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** Lets the browser paint (e.g. a loading card) before a long synchronous step. */
function nextPaint(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => setTimeout(resolve, 0)));
}

async function start(): Promise<void> {
  const canvas = mustGet("game", HTMLCanvasElement);
  const ui = mustGet("ui", HTMLDivElement);

  let api: WasmApi;
  try {
    api = await loadWasm();
  } catch (err) {
    showErrorScreen(ui, "Could not load the game core", message(err), "other");
    return;
  }

  const options = await showStartScreen(ui, (code) => {
    const info = api.checkShareCode(code);
    try {
      return info.ok ? { ok: true, code: info.code } : { ok: false, error: info.error };
    } finally {
      info.free();
    }
  });

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
  // The running session, so a lost device can end it (closing the socket
  // instead of leaving an idle ship in the match until the server times out).
  let active: GameSession | null = null;
  renderer.lost
    .then((reason) => {
      active?.dispose();
      showErrorScreen(ui, "GPU device lost", reason, "lost");
    })
    .catch(() => {});

  const loading = showLoading(ui, options.mode === "online" ? "Connecting…" : "Generating world…");
  let session: GameSession;
  try {
    await nextPaint();
    session =
      options.mode === "online"
        ? await GameSession.connect(api, serverUrl(location), options.name, options.horizonAssist)
        : GameSession.practice(api, options.shareCode, options.horizonAssist);
  } catch (err) {
    loading.remove();
    const title = options.mode === "online" ? "Could not join the server" : "Could not build the practice world";
    showErrorScreen(ui, title, message(err), options.mode === "online" ? "network" : "other");
    return;
  }
  loading.remove();
  if (renderer.isLost) {
    session.dispose();
    return;
  }
  active = session;
  runGame(canvas, ui, renderer, session, options);
}

function runGame(canvas: HTMLCanvasElement, ui: HTMLElement, renderer: Renderer, session: GameSession, options: StartOptions): void {
  const game = session.game;
  const camera = new ShipCamera(boundsCenter(game.bounds()));
  const chunks = new ChunkLoader((key, mesh) => renderer.setChunk(key, mesh));

  const hud = new Hud(ui);
  hud.show(true);
  const input = new InputController(canvas);
  input.onLockChange = (locked) => hud.showPrompt(!locked);
  hud.showPrompt(true);
  input.requestLock(); // may succeed while the Play click still counts as a user gesture

  const mode = session.isOnline ? "online" : "practice";
  const codeLabel = game.shareCode;
  if (session.isOnline && options.shareCode && options.shareCode !== codeLabel) {
    hud.updateFeed([`The server plays ${codeLabel}; your code works in practice`], performance.now());
  }

  let last = performance.now();
  let fps = 60;
  let frameStats: FrameStats = { chunksTotal: 0, chunksDrawn: 0, triangles: 0, instances: 0 };
  let chunkStats: ChunkStats = { meshed: 0, removed: 0, pending: 1, shown: 0 };
  let initialLoad = true;
  let debugTimer = 0;
  let stopped = false;

  const end = (r: FrameResult): void => {
    if (stopped) return;
    stopped = true;
    clearInterval(keepAlive);
    document.removeEventListener("visibilitychange", onVisibility);
    input.dispose();
    session.dispose();
    if (r.matchRestart) showErrorScreen(ui, "Match over", r.disconnected ?? "", "match-over");
    else showErrorScreen(ui, "Disconnected", r.disconnected ?? "", "network");
  };

  // Hidden tab: no animation frames. Drain packets and send idle input (with
  // the periodic ping) so the connection stays alive and held buttons are
  // released on the server.
  const pump = (): void => {
    if (stopped || !document.hidden) return;
    const now = performance.now();
    const dt = Math.min(MAX_FRAME_DT, Math.max(0, (now - last) / 1000));
    last = now;
    const r = session.idleFrame(dt);
    if (r) end(r);
  };
  const keepAlive = setInterval(pump, KEEP_ALIVE_MS);
  const onVisibility = (): void => {
    if (!document.hidden) return;
    // Release everything right away, not only on the next timer tick.
    const r = session.idleFrame(0);
    if (r) end(r);
  };
  document.addEventListener("visibilitychange", onVisibility);

  const frame = (now: number): void => {
    if (stopped) return;
    // A lost device cannot render; the `lost` handler disposes the session
    // and shows the error screen.
    if (renderer.isLost || session.isDisposed) {
      stopped = true;
      clearInterval(keepAlive);
      document.removeEventListener("visibilitychange", onVisibility);
      input.dispose();
      return;
    }
    const dt = Math.min(MAX_FRAME_DT, Math.max(0, (now - last) / 1000));
    last = now;
    if (dt > 0) fps += (1 / dt - fps) * 0.05;

    const state = input.sample();
    const r = session.frame(dt, state);
    if (r.disconnected) {
      end(r);
      return;
    }

    const view = camera.update(r.pose);
    chunkStats = chunks.update(game, view.position, initialLoad ? LOADING_BUDGET : PLAY_BUDGET);
    if (initialLoad && chunkStats.pending === 0) initialLoad = false;

    renderer.setInstances("ships", shipInstances(r.ships));
    renderer.setInstances("projectiles", projectileInstances(r.projectiles, session.weaponIds, session.teamOf(r.ships)));
    frameStats = renderer.render(view);

    hud.updateFeed(r.feed, now);
    const status = initialLoad ? `Loading terrain (${chunkStats.pending} chunks left)` : (r.hud.status ?? "");
    const values = { ...r.hud, status, scavenging: state.scavenge };
    debugTimer -= dt;
    if (debugTimer <= 0) {
      debugTimer = 0.25;
      const speed = r.pose ? Math.hypot(r.pose.velocity.x, r.pose.velocity.y, r.pose.velocity.z) : 0;
      hud.update({
        ...values,
        debug:
          `${fps.toFixed(0)} fps · chunks ${frameStats.chunksDrawn}/${frameStats.chunksTotal} ` +
          `(+${chunkStats.pending}) · ${Math.round(frameStats.triangles / 1000)}k tris · ` +
          `${speed.toFixed(1)} u/s · ${mode} · ${codeLabel}` +
          (renderer.lastError ? ` · GPU error: ${renderer.lastError}` : ""),
      });
    } else {
      hud.update(values);
    }
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

start().catch((err: unknown) => {
  const ui = document.getElementById("ui") ?? document.body;
  showErrorScreen(ui, "Failed to start", message(err), "other");
});
