// Loose Screws browser client entry point.
//
// Flow: load ls-wasm -> start screen (online or practice) -> WebGPU renderer ->
// connect (WebSocket, Hello/Welcome) or build a local practice world -> frame loop.
// Each frame: feed received packets to ls-wasm, step with the sampled controls
// (prediction online, local simulation in practice), send inputs, stream and
// remesh terrain chunks within a time budget, draw ships, projectiles and
// entities from the data-driven visual table (game/scene.ts), place name
// tags, update the HUD (with the weapon panel) and the overlays (Tab
// scoreboard, F3 debug overlay, end-of-match screen).
// Mechanics UI (M2 wave C, game/mechanics.ts): game cues become effects
// (Flash Pod whiteout, screen shake by Tectonic zone, banners, Leech Arc
// beams, flashes), the camera rides a guided Eyeball, the HUD shows the
// charge meter, gadget cooldown and paint colour (V), and the ship bay (B,
// or by itself while destroyed) picks the ship type for the next respawn.
// While the tab is hidden (no animation frames) a timer keeps the online
// session alive with idle input, so the server neither drops the player
// after its idle timeout nor keeps repeating held buttons.
// When the server restarts a finished match it closes the connection; the
// client then reconnects and rejoins with the same settings (game/rejoin.ts).

import { InputController } from "./input/input.ts";
import { GpuUnavailableError } from "./renderer/gpu.ts";
import { DEFAULT_SETTINGS, Renderer, type FrameStats } from "./renderer/renderer.ts";
import { boundsCenter, ShipCamera } from "./game/camera.ts";
import { ChunkLoader, LOADING_BUDGET, PLAY_BUDGET, type ChunkStats } from "./game/chunks.ts";
import { DEFAULT_CONTROL_SETTINGS } from "./game/controls.ts";
import { rejoin, RejoinCancelled, type PlayerPrefs } from "./game/rejoin.ts";
import { GameSession, type FrameResult } from "./game/session.ts";
import { buildScene, dustDensityAt, dustFog } from "./game/scene.ts";
import { connectTransport, transportPreference } from "./net/connect.ts";
import { serverUrl } from "./net/transport.ts";
import {
  EffectsState,
  eyeballPose,
  gadgetKey,
  guidedProjectile,
  shakeOffset,
  shipForKey,
  shipPositionLookup,
  type EffectsContext,
} from "./game/mechanics.ts";
import { DebugOverlay, formatDebugLines, FpsMeter } from "./ui/debugOverlay.ts";
import { EffectsOverlay } from "./ui/effects.ts";
import { ShipPanel, shipPanelModel } from "./ui/shipSelect.ts";
import * as q from "./math/quat.ts";
import { showErrorScreen } from "./ui/errorScreen.ts";
import { Hud } from "./ui/hud.ts";
import { NameTags, nameTagModel } from "./ui/nameTags.ts";
import { EndScreen, endScreenModel, Scoreboard, scoreboardModel } from "./ui/scoreboard.ts";
import { showLoading, showStartScreen, type StartOptions } from "./ui/startScreen.ts";
import { loadWasm } from "./wasm/load.ts";
import { addRuleEditorLink, showRuleEditor, wantsRuleEditor } from "./rules/editor.ts";
import { showShowcase, wantsShowcase } from "./renderer/models/showcase.ts";
import { WreckTypes } from "./game/wrecks.ts";
import { parseInvite } from "./rules/model.ts";
import type { WasmApi } from "./wasm/api.ts";

/** Largest frame time passed to the simulation; longer stalls (tab switch) are clamped. */
const MAX_FRAME_DT = 0.1;
/** Background pump interval while the tab is hidden (browsers throttle it to ~1 Hz). */
const KEEP_ALIVE_MS = 250;
/** Overlay text refresh interval (s). */
const OVERLAY_INTERVAL = 0.25;

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

/** How a game ended. */
type GameEnd = { kind: "restart" } | { kind: "disconnected"; detail: string } | { kind: "stopped" };

async function start(): Promise<void> {
  const canvas = mustGet("game", HTMLCanvasElement);
  const ui = mustGet("ui", HTMLDivElement);

  // `?editor=rules`: the rule set editor (no game core or GPU needed).
  if (wantsRuleEditor(location)) {
    showRuleEditor(ui, { baseUrl: location.href, ...parseInvite(location.href) });
    return;
  }
  // `?view=ships`: the VQ4 model showcase (ship bay, wrecks, props, in flight).
  if (wantsShowcase(location)) {
    await showShowcase(canvas, ui, loadWasm);
    return;
  }

  let api: WasmApi;
  try {
    api = await loadWasm();
  } catch (err) {
    showErrorScreen(ui, "Could not load the game core", message(err), "other");
    return;
  }

  const editorLink = addRuleEditorLink(ui, location.href);
  const options = await showStartScreen(ui, (code) => {
    const info = api.checkShareCode(code);
    try {
      return info.ok ? { ok: true, code: info.code } : { ok: false, error: info.error };
    } finally {
      info.free();
    }
  });
  editorLink.remove();

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

  const prefs: PlayerPrefs = {
    name: options.name,
    horizonAssist: options.horizonAssist,
    debugOverlay: false,
    controls: { ...DEFAULT_CONTROL_SETTINGS },
    shipType: options.shipType,
  };
  const url = serverUrl(location);
  // WebTransport first where available, WebSocket fallback; `?transport=ws|wt` forces one.
  const open = (u: string) => connectTransport(u, { prefer: transportPreference(location) });
  const connect = (): Promise<GameSession> =>
    GameSession.connect(api, url, prefs.name, prefs.horizonAssist, open, prefs.controls, prefs.shipType);

  const loading = showLoading(ui, options.mode === "online" ? "Connecting…" : "Generating world…");
  let session: GameSession;
  try {
    await nextPaint();
    session =
      options.mode === "online"
        ? await connect()
        : GameSession.practice(api, options.shareCode, prefs.horizonAssist, prefs.shipType);
  } catch (err) {
    loading.remove();
    const title = options.mode === "online" ? "Could not join the server" : "Could not build the practice world";
    showErrorScreen(ui, title, message(err), options.mode === "online" ? "network" : "other");
    return;
  }
  loading.remove();
  const weaponNames = api.weaponNames();

  for (;;) {
    if (renderer.isLost) {
      session.dispose();
      return;
    }
    active = session;
    const end = await runGame(canvas, ui, renderer, session, options, prefs, weaponNames);
    active = null;
    if (end.kind === "stopped") return;
    if (end.kind === "disconnected") {
      showErrorScreen(ui, "Disconnected", end.detail, "network");
      return;
    }
    // Match restart: reconnect with the same settings.
    const card = showLoading(ui, "Match over. Joining the next match…");
    try {
      session = await rejoin(connect, {
        cancelled: () => renderer.isLost,
        onAttempt: (attempt, lastError) => {
          if (attempt > 0) card.set(`Joining the next match… (attempt ${attempt + 1}: ${message(lastError)})`);
        },
      });
    } catch (err) {
      card.remove();
      if (err instanceof RejoinCancelled) return;
      showErrorScreen(ui, "Could not rejoin", message(err), "match-over");
      return;
    }
    card.remove();
  }
}

function runGame(
  canvas: HTMLCanvasElement,
  ui: HTMLElement,
  renderer: Renderer,
  session: GameSession,
  options: StartOptions,
  prefs: PlayerPrefs,
  weaponNames: readonly string[],
): Promise<GameEnd> {
  return new Promise((resolve) => {
    const game = session.game;
    const camera = new ShipCamera(boundsCenter(game.bounds()));
    const chunks = new ChunkLoader((key, mesh) => renderer.setChunk(key, mesh));

    const hud = new Hud(ui);
    hud.show(true);
    const scoreboard = new Scoreboard(ui);
    const endScreen = new EndScreen(ui);
    const overlay = new DebugOverlay(ui);
    if (prefs.debugOverlay) overlay.toggle();
    const nameTags = new NameTags(ui);
    const fpsMeter = new FpsMeter();
    const effectsOverlay = new EffectsOverlay(ui);
    const shipPanel = new ShipPanel(ui);
    const effects = new EffectsState();
    const wrecks = new WreckTypes();
    let shipBayOpen = false;
    let shipError = "";
    let dead = false;
    const pickShip = (shipType: number): void => {
      const err = session.selectShip(shipType);
      shipError = err ?? "";
      if (!err) {
        prefs.shipType = shipType;
        shipBayOpen = false;
      }
    };
    shipPanel.onPick = pickShip;
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
    let frameStats: FrameStats = { chunksTotal: 0, chunksDrawn: 0, triangles: 0, instances: 0 };
    let chunkStats: ChunkStats = { meshed: 0, removed: 0, pending: 1, shown: 0 };
    let initialLoad = true;
    let overlayTimer = 0;
    let stopped = false;

    /** Tears the game down (keeping the renderer) and resolves `end`. */
    const finish = (end: GameEnd): void => {
      if (stopped) return;
      stopped = true;
      clearInterval(keepAlive);
      document.removeEventListener("visibilitychange", onVisibility);
      prefs.horizonAssist = session.controls.horizonAssist;
      prefs.controls = { ...session.controls.settings };
      prefs.debugOverlay = overlay.visible;
      input.dispose();
      session.dispose();
      if (!renderer.isLost) {
        // The next match builds its own meshes and instances.
        chunks.clear();
        renderer.clearInstances();
        renderer.settings.fogDensity = DEFAULT_SETTINGS.fogDensity;
        renderer.settings.fogColor = DEFAULT_SETTINGS.fogColor;
      }
      hud.root.remove();
      effectsOverlay.dispose();
      shipPanel.dispose();
      nameTags.dispose();
      scoreboard.dispose();
      endScreen.dispose();
      overlay.dispose();
      resolve(end);
    };
    const end = (r: FrameResult): void =>
      finish(r.matchRestart ? { kind: "restart" } : { kind: "disconnected", detail: r.disconnected ?? "" });

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
        finish({ kind: "stopped" });
        return;
      }
      const dt = Math.min(MAX_FRAME_DT, Math.max(0, (now - last) / 1000));
      last = now;
      fpsMeter.tick(now);

      const state = input.sample();
      if (state.toggleDebug) {
        overlay.toggle();
        overlayTimer = 0;
      }
      if (state.toggleShipSelect) {
        shipBayOpen = !shipBayOpen;
        shipError = "";
      }
      // While the ship bay shows, 1-4 pick a ship type instead of a weapon.
      const bay = shipBayOpen || dead;
      if (bay && state.weaponKey >= 1 && state.weaponKey <= 4) {
        const choice = shipForKey(session.shipChoices(), state.weaponKey);
        if (choice) pickShip(choice.shipType);
        else shipError = "The rule set does not allow that ship";
        state.weaponKey = 0;
      }
      const r = session.frame(dt, state);
      if (r.disconnected) {
        end(r);
        return;
      }
      dead = r.pose !== null && !r.pose.alive;

      const time = now / 1000;
      const shipPos = shipPositionLookup(game.player, r.pose, r.ships);
      const fxCtx: EffectsContext = {
        player: game.player,
        camera: camera.current.position,
        shipPos,
        weaponsOfflineSecs: r.weaponsOffline,
      };
      effects.apply(r.cues, time, fxCtx);
      const fx = effects.frame(time, fxCtx);
      // Eyeball: while the own missile is guided, the camera rides it.
      const guided = guidedProjectile(r.projectiles, game.player);
      const shipUp = r.pose ? q.up(r.pose.orientation) : q.up(camera.current.orientation);
      const ride = guided ? eyeballPose(guided, shipUp) : null;
      const view = camera.update(r.pose, ride, shakeOffset(fx.shake, time));
      chunkStats = chunks.update(game, view.position, initialLoad ? LOADING_BUDGET : PLAY_BUDGET);
      if (initialLoad && chunkStats.pending === 0) initialLoad = false;

      const players = r.players;
      // Wrecks show the hull of the ship that died there (VQ4).
      const own = players.find((p) => p.id === game.player);
      const tracked = r.pose && own ? [...r.ships, { id: game.player, alive: r.pose.alive, position: r.pose.position, shipType: own.shipType }] : r.ships;
      wrecks.observe(tracked, r.entities, time);
      renderer.setScene(
        buildScene(r.ships, r.projectiles, r.entities, session.visuals, session.teamOf(r.ships), time, {
          gadgetOf: (id) => {
            for (const p of players) if (p.id === id) return gadgetKey(session.gadgetIds, p.gadget);
            return "";
          },
          beams: fx.beams,
          flashes: fx.flashes,
          // The guided Eyeball is the camera; hide only that one missile.
          hideShot: guided ? (p) => p === guided : undefined,
          shipTypeOf: (id) => players.find((p) => p.id === id)?.shipType,
          wreckTypeOf: (e) => wrecks.typeOf(e),
        }),
      );
      // Inside a dust cloud the fog closes in (visibility is cut).
      const fog = dustFog(DEFAULT_SETTINGS, dustDensityAt(view.position, r.entities, session.visuals));
      renderer.settings.fogDensity = fog.fogDensity;
      renderer.settings.fogColor = fog.fogColor;
      frameStats = renderer.render(view);
      nameTags.update(
        nameTagModel(
          r.ships,
          r.players,
          view.position,
          renderer.viewProjection(view),
          canvas.clientWidth,
          canvas.clientHeight,
          game.team,
        ),
      );

      // End-of-match screen replaces the Tab scoreboard while the result shows.
      const info = session.matchInfo();
      const ownTeam = game.team;
      const wantBoard = info.over || state.scoreboard;
      if (wantBoard) {
        const board = scoreboardModel(session.scoreboard(), info, ownTeam, session.isOnline);
        if (info.over) endScreen.update(endScreenModel(info, ownTeam, session.isOnline), board);
        else scoreboard.update(board);
      }
      endScreen.show(info.over);
      scoreboard.show(!info.over && state.scoreboard);

      hud.updateFeed(r.feed, now);
      const status = initialLoad
        ? `Loading terrain (${chunkStats.pending} chunks left)`
        : info.over
          ? ""
          : (r.hud.status ?? "");
      hud.update({
        ...r.hud,
        status,
        scavenging: state.scavenge,
        debug: "",
        gadget: r.gadget,
        charge: r.charge ? { name: weaponNames[r.charge.weapon] ?? "Charge", meter: r.charge } : null,
        paintColor: r.paintColor,
        weaponsOffline: Math.max(r.weaponsOffline, fx.weaponsOffline),
      });
      effectsOverlay.update({
        whiteout: fx.whiteout,
        banners: fx.banners,
        eyeball: guided !== null,
        cloaked: r.gadget.active && r.gadget.key === "cloak",
      });
      shipPanel.update(shipPanelModel(bay || shipBayOpen || dead ? session.shipChoices() : [], shipBayOpen, dead, shipError));

      overlayTimer -= dt;
      if (overlay.visible && overlayTimer <= 0) {
        overlayTimer = OVERLAY_INTERVAL;
        const speed = r.pose ? Math.hypot(r.pose.velocity.x, r.pose.velocity.y, r.pose.velocity.z) : 0;
        overlay.update(
          formatDebugLines({
            fps: fpsMeter.fps,
            worstFrameMs: fpsMeter.worstFrameMs,
            mode,
            transport: session.transportName,
            droppedSends: session.droppedSends,
            stats: session.debugStats(),
            chunks: {
              meshedTotal: chunks.meshedTotal,
              pending: chunkStats.pending,
              shown: frameStats.chunksTotal,
              drawn: frameStats.chunksDrawn,
              triangles: frameStats.triangles,
            },
            shareCode: codeLabel,
            speed,
            gpuError: renderer.lastError ?? null,
          }),
        );
      }
      requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  });
}

start().catch((err: unknown) => {
  const ui = document.getElementById("ui") ?? document.body;
  showErrorScreen(ui, "Failed to start", message(err), "other");
});
