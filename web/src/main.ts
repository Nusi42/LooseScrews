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
// M3 (modes and lobby): the mode HUD (ui/modeHud.ts) shows the scoreline,
// timer, stage, lives, colour meter and objective markers of any mode; the
// lobby card (ui/lobby.ts) shows during warm-up and countdown (Enter ready,
// T team, O spectate); the results screen (ui/results.ts) after each match;
// eliminated players and spectators fly the spectator camera
// (game/spectator.ts). A match transition (a new Welcome on the open
// connection) swaps the level, rules and visuals in place without a reload
// (`FrameResult.transition`). `#ui[data-stage]` and `#ui[data-matches]`
// expose the lobby stage and the match count to the browser smoke test.
// An older server that closes the connection after a match is still
// rejoined with the same settings (game/rejoin.ts).
// VQ1 (game/scaleCues.ts): the horizontal FOV setting with a boost kick, the
// cockpit hull frame (renderer/hullFrame.ts) and the fit ring around the
// crosshair (ui/fitRing.ts, probes in ls-wasm).
// First impression (attract/): a live flythrough behind the start screen and
// the hulls turning in the ship picker (StartIntro, which also creates the
// renderer the game then uses), and in the lobby a cinematic camera through
// the next level while the mouse is free (LobbyCinematic).
// M4 C1 (game/events/): the comedy events' banner, announcer lines and
// visuals (camera overrides, props, the pixel look, the cake wave).

import { InputController } from "./input/input.ts";
import { GpuUnavailableError } from "./renderer/gpu.ts";
import { atmosphereFor } from "./renderer/atmosphere.ts";
import { DEFAULT_SETTINGS, Renderer, type FrameStats } from "./renderer/renderer.ts";
import { hashSeed } from "./renderer/rockTextures.ts";
import { boundsCenter, ShipCamera } from "./game/camera.ts";
import { ChunkLoader, LOADING_BUDGET, PLAY_BUDGET, type ChunkStats } from "./game/chunks.ts";
import { DEFAULT_CONTROL_SETTINGS } from "./game/controls.ts";
import { rejoin, RejoinCancelled, type PlayerPrefs } from "./game/rejoin.ts";
import { GameSession, type FrameResult } from "./game/session.ts";
import { buildScene, dustDensityAt, dustFog, teamHull } from "./game/scene.ts";
import { ScaleCues, type ScaleCueFrame } from "./game/scaleCues.ts";
import { TINKER_RADIUS } from "./game/views.ts";
import { connectTransport, transportPreference } from "./net/connect.ts";
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
import { Scoreboard, scoreboardModel } from "./ui/scoreboard.ts";
import { LobbyOverlay, lobbyModel, nextTeamPick } from "./ui/lobby.ts";
import { ModeHud, modeHudModel, objectiveMarkers } from "./ui/modeHud.ts";
import { practiceResults, ResultsScreen, resultsModel } from "./ui/results.ts";
import { LobbyClock, matchFlow, type FlowView } from "./game/matchFlow.ts";
import { SpectatorCamera } from "./game/spectator.ts";
import { emptyInput } from "./input/input.ts";
import { showLoading, showStartScreen, type StartOptions } from "./ui/startScreen.ts";
import { sessionView, startSocial } from "./ui/party.ts";
import { loadWasm } from "./wasm/load.ts";
import { StartIntro, wantsBackdrop } from "./attract/intro.ts";
import { LobbyCinematic } from "./attract/lobby.ts";
import { PanelBay } from "./attract/shipBay.ts";
import { runGallery, wantsGallery } from "./gallery/gallery.ts";
import { addRuleEditorLink, showRuleEditor, wantsRuleEditor } from "./rules/editor.ts";
import { showShowcase, wantsShowcase } from "./renderer/models/showcase.ts";
import { WreckTypes } from "./game/wrecks.ts";
import { LightFx, sceneLights } from "./game/lightFx.ts";
import { Vfx } from "./game/vfx.ts";
import { chargeLight, ChargeTracker } from "./game/chargeFx.ts";
import { addModeProps, easeMonochrome, isMonochrome, modeLights, ModeVfx, modeVolumes, monochromeGrade, playerStyleFor, shipStyleFor, type ModeSceneInput } from "./game/modeVisuals.ts";
import { NEUTRAL_GRADE } from "./renderer/post.ts";
import { parseInvite } from "./rules/model.ts";
import type { WasmApi } from "./wasm/api.ts";
import { audioEngine, MatchAudio } from "./audio/index.ts";
import { wantsWorldFx, WorldVisuals } from "./game/worldFx.ts";
import { VisitorVisuals, wantsVisitors } from "./game/events/visitors.ts";
import { transformPoint } from "./math/mat4.ts";
import type { PointLight } from "./renderer/lights.ts";
import { showAudioDemo, wantsAudioDemo } from "./audio/demo.ts";
import { emptyOutput, EventVisuals, fxFromMood, parseEventFx, wantsEventFx, wellPoints } from "./game/events/index.ts";
import { EventHud } from "./ui/eventBanner.ts";
import { SpectacleVisuals } from "./game/events/spectacles.ts";
import { ArchitectClient } from "./game/architect/index.ts";
import { RaceClient } from "./game/race/index.ts";
import { applyEventParam } from "./game/eventParam.ts";
import { installGameHooks } from "./game/debugHooks.ts";

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

  // `?gallery=<scene>`: a fixed-pose render for the visual gallery (tools/browser-smoke/gallery.mjs).
  if (wantsGallery(location)) {
    await runGallery(canvas, ui, location);
    return;
  }

  // `?editor=rules`: the rule set editor (no game core or GPU needed).
  if (wantsRuleEditor(location)) {
    showRuleEditor(ui, { baseUrl: location.href, ...parseInvite(location.href) });
    return;
  }
  // `?audio=demo`: the sound check page (every sound, the announcer packs; M4 A6).
  if (wantsAudioDemo(location)) {
    showAudioDemo(ui);
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

  // Invite links, the server list, parties and duels (ui/party.ts).
  const social = await startSocial(ui, api, location);
  const editorLink = addRuleEditorLink(ui, location.href);
  // The renderer is created behind the interactive start screen, which it decorates (FI).
  const intro = new StartIntro(canvas, ui, api, wantsBackdrop(location));
  const options = await showStartScreen(
    ui,
    (code) => {
      const info = api.checkShareCode(code);
      try {
        return info.ok ? { ok: true, code: info.code } : { ok: false, error: info.error };
      } finally {
        info.free();
      }
    },
    intro.hooks,
  );
  editorLink.remove();

  let renderer: Renderer;
  try {
    renderer = await intro.renderer();
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
  const url = social.serverUrl;
  // WebTransport first where available, WebSocket fallback; `?transport=ws|wt` forces one.
  const open = social.opener((u: string) => connectTransport(u, { prefer: transportPreference(location) }));
  const connect = (): Promise<GameSession> =>
    GameSession.connect(social.api, url, prefs.name, prefs.horizonAssist, open, prefs.controls, prefs.shipType);

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
    intro.stop();
    const title = options.mode === "online" ? "Could not join the server" : "Could not build the practice world";
    showErrorScreen(ui, title, message(err), options.mode === "online" ? "network" : "other");
    return;
  }
  loading.remove();
  intro.stop();
  // M4 C6: `?event=<id>` forces comedy events in practice (and `window.__lsEvents` for scripts).
  if (options.mode !== "online") applyEventParam(session.game, location.search, api.eventIds?.() ?? []);
  if (renderer.isLost) {
    // Lost while connecting: the `lost` handler shows the error; leave the server at once.
    session.dispose();
    return;
  }
  if (options.mode === "online") social.attach(prefs.name, () => (active ? sessionView(active.game) : null));
  const weaponNames = api.weaponNames();

  for (;;) {
    if (renderer.isLost) {
      session.dispose();
      return;
    }
    active = session;
    installGameHooks(session);
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
    let camera = new ShipCamera(boundsCenter(game.bounds()));
    // The world's sky, fog and grade (VQ7) before its chunks arrive.
    renderer.setAtmosphere(atmosphereFor(game.shareCode, game.bounds()));
    // FOV setting with the boost kick, fit ring and cockpit frame (VQ1).
    const cues = new ScaleCues();
    const chunks = new ChunkLoader((key, mesh) => renderer.setChunk(key, mesh));

    const hud = new Hud(ui);
    hud.show(true);
    const modeHud = new ModeHud(ui);
    const scoreboard = new Scoreboard(ui);
    const results = new ResultsScreen(ui);
    const lobby = new LobbyOverlay(ui, {
      ready: (ready) => session.setReady(ready),
      team: (team) => session.pickTeam(team),
      spectate: (spectate) => session.setSpectate(spectate),
    });
    const lobbyClock = new LobbyClock();
    let spectator: SpectatorCamera | null = null;
    // Lobby with the mouse free: a cinematic flight through the next level (FI).
    const cinematic = new LobbyCinematic();
    let flow: FlowView = { stage: "match", showLobby: false, showResults: false, spectating: false };
    const overlay = new DebugOverlay(ui);
    if (prefs.debugOverlay) overlay.toggle();
    const nameTags = new NameTags(ui);
    const fpsMeter = new FpsMeter();
    const effectsOverlay = new EffectsOverlay(ui);
    const shipPanel = new ShipPanel(ui);
    let effects = new EffectsState();
    let wrecks = new WreckTypes();
    // Dynamic lights (VQ6): timed pulses (muzzle, impacts, explosions); effect systems emit into it too.
    const lightFx = new LightFx();
    const charging = new ChargeTracker(); // VQP charge-up effects
    // VQ5 particle effects (null with `?vfx=off`: the mesh effects draw instead).
    const particles = renderer.particles;
    const vfx = particles ? new Vfx(particles) : null;
    if (vfx) {
      // The frame's immediate sprites start at the top of `frame` (the event hub and
      // the architect emit theirs before the effect library runs).
      vfx.beginsFrame = false;
      // VQ5 effects emit their own light pulses (muzzle, impacts, explosions, deaths) into VQ6.
      vfx.lightSink = lightFx;
      lightFx.watchProjectiles = false;
    }
    particles?.clear();
    // Mode visuals (M3 C1): particles and events of walls, cores, reactors; the monochrome blend.
    const modeVfx = particles ? new ModeVfx(particles) : null;
    if (modeVfx) modeVfx.lightSink = lightFx;
    // Sound and the announcer (M4 A6; silent until the first user gesture).
    const audio = new MatchAudio(ui, game.shareCode);
    // World visuals (M4 B5): gravity wells, cave-ins, pigment, the level's mood; `?worldfx=off` skips them.
    const world = new WorldVisuals(particles, wantsWorldFx(location), audioEngine());
    // Belt Up (M4 C7): the belt's look on belt levels, the race HUD in Belt Up.
    const race = new RaceClient(ui, particles, audioEngine());
    let worldShake = 0;
    // Comedy event visuals (M4 C1; `?eventfx=off` keeps only the banner and the announcer).
    const eventHud = new EventHud(ui);
    // Visitor events (M4 C2): drones, mimics, the gossip face, their HUD, run by the hub; `?visitors=off` skips them.
    const visitors = new VisitorVisuals(particles, wantsVisitors(location), audioEngine(), ui);
    const events = new EventVisuals({ particles, sound: audioEngine(), hud: eventHud, announcer: audio.announcer, enabled: wantsEventFx(location), visitors });
    let eventShake = 0;
    // M4 C3: Duck parade, Hat rain, Cat paw, Disco inferno, the Tired announcer's kill-cams.
    const spectacles = new SpectacleVisuals(ui, particles, wantsWorldFx(location), audioEngine(), (t, v) => audio.announcer.say(t, v));
    let spectacleShake = 0;
    // The Architect (M4 C4): the architect's god camera and strokes, the exit and stroke looks, its HUD.
    const architect = new ArchitectClient(ui, game, (p) => (session.link?.isOpen ? (session.link.send(p, false), true) : false), particles, audioEngine());
    let mono = 0;
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
    const panelBay = new PanelBay(shipPanel.root, renderer.device, Math.max(0, game.team), renderer.quality.level === "low" ? "low" : "medium");
    shipPanel.backdrop = panelBay;
    const input = new InputController(canvas);
    input.onLockChange = (locked) => hud.showPrompt(!locked);
    hud.showPrompt(true);
    input.requestLock(); // may succeed while the Play click still counts as a user gesture

    const mode = session.isOnline ? "online" : "practice";
    if (session.isOnline && options.shareCode && options.shareCode !== game.shareCode) {
      hud.updateFeed([`The server plays ${game.shareCode}; your code works in practice`], performance.now());
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
        renderer.particles?.clear();
        renderer.setAtmosphere(null);
      }
      hud.root.remove();
      effectsOverlay.dispose();
      shipPanel.dispose();
      nameTags.dispose();
      modeHud.dispose();
      scoreboard.dispose();
      results.dispose();
      lobby.dispose();
      audio.dispose();
      world.dispose();
      events.dispose();
      eventHud.dispose();
      if (!renderer.isLost) events.apply(renderer, emptyOutput());
      spectacles.dispose();
      architect.dispose();
      renderer.cutaway = null;
      race.dispose();
      delete ui.dataset["stage"];
      delete ui.dataset["matches"];
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
      // Spectating: the keys and mouse fly the spectator camera, not the (absent) ship.
      const r = session.frame(dt, flow.spectating ? { ...emptyInput(), pointerLocked: state.pointerLocked } : state);
      if (r.disconnected) {
        end(r);
        return;
      }
      if (r.transition) {
        // The next match on the same connection: new level, rules, team and
        // visuals. Old chunks come back from takeUnloads; reset the rest.
        camera = new ShipCamera(boundsCenter(game.bounds()));
        renderer.setAtmosphere(atmosphereFor(game.shareCode, game.bounds()));
        cues.reset();
        initialLoad = true;
        effects = new EffectsState();
        wrecks = new WreckTypes();
        lightFx.clear();
        vfx?.reset();
        charging.forget();
        modeVfx?.reset();
        world.reset();
        events.reset();
        spectacles.reset();
        architect.reset();
        race.reset();
        audio.newMatch(game.shareCode);
        mono = 0;
        particles?.clear();
        spectator = null;
        cinematic.reset();
        lobbyClock.reset();
        shipBayOpen = false;
        shipError = "";
        // The next match can put the player on another team: the bay's hulls wear its livery.
        panelBay.setTeam(Math.max(0, game.team));
        if (!renderer.isLost) renderer.clearInstances();
      }
      flow = matchFlow({ online: session.isOnline, lobby: r.lobby, mode: r.mode, results: r.results, own: game.player });
      ui.dataset["stage"] = flow.stage;
      ui.dataset["matches"] = String(session.matchCount + 1);
      const names = new Map(r.players.map((p) => [p.id, p.name]));
      const lobbySecs = lobbyClock.secsLeft(r.lobby, now);
      const lobbyNow = flow.showLobby && r.lobby ? lobbyModel(r.lobby, game.player, names, lobbySecs) : null;
      if (r.lobby) {
        const me = r.lobby.players.find((p) => p.id === game.player);
        if (state.toggleReady && flow.showLobby && !me?.spectator) session.setReady(!(me?.ready ?? false));
        if (state.cycleTeam && flow.showLobby && r.lobby.teams > 0) session.pickTeam(nextTeamPick(me?.team ?? -1, r.lobby.teams));
        if (state.toggleSpectate) session.setSpectate(!(me?.spectator ?? false));
      }
      dead = r.pose !== null && !r.pose.alive && !flow.spectating;

      const time = now / 1000;
      if (particles) {
        particles.now = time;
        particles.beginFrame();
      }
      const shipPos = shipPositionLookup(game.player, r.pose, r.ships);
      const fxCtx: EffectsContext = {
        player: game.player,
        camera: camera.current.position,
        shipPos,
        weaponsOfflineSecs: r.weaponsOffline,
      };
      effects.apply(r.cues, time, fxCtx);
      const fx = effects.frame(time, fxCtx, r.localBeam);
      // Eyeball: while the own missile is guided, the camera rides it.
      const guided = guidedProjectile(r.projectiles, game.player);
      const shipUp = r.pose ? q.up(r.pose.orientation) : q.up(camera.current.orientation);
      const ride = guided ? eyeballPose(guided, shipUp) : null;
      const cine = cinematic.update(dt, game, flow.showLobby && !state.pointerLocked && !flow.spectating && !guided, camera.current, renderer.aspect);
      const s0 = session.controls.settings;
      const arch = architect.update({
        dt,
        time,
        nowTick: game.nowTick?.() ?? 0,
        input: state,
        mode: r.mode,
        own: game.player,
        names,
        bounds: game.bounds(),
        ships: r.ships,
        lookScale: s0.lookSensitivity,
        invertY: s0.invertY,
        start: camera.current,
        shareCode: game.shareCode,
        quiet: flow.showResults || flow.showLobby,
      });
      renderer.cutaway = arch.cutaway;
      if (flow.spectating && !arch.amArchitect) {
        spectator ??= new SpectatorCamera(camera.current);
        const s = session.controls.settings;
        spectator.update(dt, state, r.ships, s.lookSensitivity, game.bounds(), s.invertY);
        // Belt Up: a finished racer watches the leader (until they take the camera over).
        const lead = race.followTarget(r.mode, r.ships, spectator.following, state.fireSecondary);
        if (lead !== null) spectator.mode = { kind: "follow", id: lead };
      } else {
        spectator = null;
      }
      const cueFrame: ScaleCueFrame = {
        dt,
        aspect: renderer.aspect,
        viewportHeight: canvas.clientHeight,
        flying: r.pose !== null && r.pose.alive && !flow.spectating && !guided && !flow.showResults && !cine,
        boosting: state.boost && (r.hud.boost ?? 0) > 0,
      };
      camera.params.fovY = cine ? cinematic.fovY(renderer.aspect) : cues.fovY(cueFrame);
      const shake = Math.max(fx.shake, vfx?.shake(time) ?? 0, worldShake, eventShake, spectacleShake);
      const camView = camera.update(r.pose, arch.pose ?? (spectator ? spectator.view : (cine ?? ride)), shakeOffset(shake, time));
      // Wrecks show the hull of the ship that died there (VQ4); the visitors follow the own ship too.
      const own = r.players.find((p) => p.id === game.player);
      const tracked = r.pose && own ? [...r.ships, { id: game.player, alive: r.pose.alive, position: r.pose.position, shipType: own.shipType }] : r.ships;
      const ownAlive = r.pose && r.pose.alive && !flow.spectating ? r.pose : null;
      // Comedy events (M4 C1, C2's visitors through the same hub): what the running event does, its banner and visuals; the side view and the hiccup bump move the camera.
      const evTick = game.nowTick?.() ?? 0;
      const evUp = game.up?.();
      const evTeams = session.teamOf(r.ships);
      const ev = events.update(
        {
          time,
          dt,
          tick: evTick,
          fx: game.eventFx ? parseEventFx(game.eventFx(evTick)) : fxFromMood(r.mode.mood, evTick),
          mood: r.mode.mood,
          camera: camView,
          own: r.pose && r.pose.alive && !flow.spectating && !guided ? { id: game.player, team: game.team, position: r.pose.position, orientation: r.pose.orientation, velocity: r.pose.velocity, shipType: r.players.find((p) => p.id === game.player)?.shipType ?? 1, radius: game.shipRadius ?? TINKER_RADIUS } : null,
          ships: r.ships,
          shots: r.projectiles,
          entities: r.entities,
          visuals: session.visuals,
          cues: r.cues,
          wells: wellPoints(game.wells?.()),
          up: evUp && evUp.length >= 3 ? { x: evUp[0] ?? 0, y: evUp[1] ?? 1, z: evUp[2] ?? 0 } : r.mode.mood.up,
          colorOf: (id) => teamHull(evTeams(id) ?? 0, r.mode.teams),
          raycast: game.raycast ? (o, d, max) => game.raycast?.(o.x, o.y, o.z, d.x, d.y, d.z, max) ?? -1 : undefined,
          quiet: flow.showResults || flow.showLobby,
          screenHeight: canvas.height,
          visitors: { own: game.player, ownPosition: ownAlive ? ownAlive.position : null, ships: tracked, names, pickupPoints: game.pickupPoints?.() },
        },
        (id) => names.get(id) ?? "",
      );
      eventShake = ev.shake;
      const view = ev.view ?? camView;
      chunkStats = chunks.update(game, view.position, initialLoad ? LOADING_BUDGET : PLAY_BUDGET);
      if (initialLoad && chunkStats.pending === 0) initialLoad = false;

      const players = r.players;
      wrecks.observe(tracked, r.entities, time);
      const gadgetOf = (id: number): string => {
        for (const p of players) if (p.id === id) return gadgetKey(session.gadgetIds, p.gadget);
        return "";
      };
      const hideShot = events.hideShot(guided ? (p: (typeof r.projectiles)[number]) => p === guided : undefined, ev);
      const drawnShips = events.ships(r.ships, ev);
      const drawnShots = events.shots(r.projectiles, ev);
      charging.apply(r.cues, time, game.player, r.projectiles);
      const charges = charging.glows({
        now: time,
        player: game.player,
        pose: flow.spectating || guided ? null : r.pose,
        ownRadius: game.shipRadius ?? TINKER_RADIUS,
        ownShipType: r.players.find((p) => p.id === game.player)?.shipType ?? 1,
        own: r.charge,
        ships: r.ships,
        shots: r.projectiles,
        visualOf: (w) => session.visuals[game.projectileVisual?.(w) ?? -1],
      });
      const teamOfShip = session.teamOf(r.ships);
      vfx?.update({
        time,
        dt,
        camera: view.position,
        cameraOrientation: view.orientation,
        player: game.player,
        pose: r.pose,
        ships: drawnShips,
        shots: drawnShots,
        entities: r.entities,
        visuals: session.visuals,
        cues: r.cues,
        beams: fx.beams,
        flashes: fx.flashes,
        gadgetOf,
        hullOf: (t) => session.shipChoices().find((c) => c.shipType === t)?.hull ?? 100,
        hideShot,
        charges,
        teamColorOf: (id) => {
          const team = id === game.player ? game.team : teamOfShip(id);
          return team === undefined ? undefined : teamHull(team, r.mode.teams);
        },
      });
      const modeInput: ModeSceneInput = {
        mode: r.mode,
        nowTick: game.nowTick?.() ?? 0,
        time,
        own: game.player,
        ownPos: r.pose && r.pose.alive && !flow.spectating ? r.pose.position : null,
        ships: r.ships,
        camera: view.position,
        warmup: flow.stage === "warmup" || flow.stage === "countdown",
      };
      modeVfx?.update({ ...modeInput, dt });
      // M4 B5 (the frame's immediate sprites started at the top of the frame).
      const wf = world.update(
        {
          time,
          dt,
          nowTick: game.nowTick?.() ?? 0,
          camera: view.position,
          orientation: view.orientation,
          wells: game.wells?.(),
          up: game.up?.(),
          shipExt: game.shipExt?.(),
          monochrome: game.monochrome ?? false,
          modeStyles: isMonochrome(r.mode),
          cues: r.cues,
          mood: r.mode.mood,
          own: ownAlive ? { id: game.player, team: game.team, position: ownAlive.position, shipType: own?.shipType ?? 1 } : null,
          ownTeam: game.team,
          ships: r.ships,
          entities: r.entities,
          teams: r.mode.teams,
          raycast: game.raycast ? (o, d, max) => game.raycast?.(o.x, o.y, o.z, d.x, d.y, d.z, max) ?? -1 : undefined,
          quiet: flow.showResults || flow.showLobby,
        },
        (trigger) => audio.announcer.say(trigger),
      );
      worldShake = wf.shake;
      const sf = spectacles.update({
        time,
        dt,
        nowTick: game.nowTick?.() ?? 0,
        camera: view.position,
        orientation: view.orientation,
        mood: r.mode.mood,
        cues: r.cues,
        ships: r.ships,
        own: ownAlive ? { id: game.player, team: game.team, position: ownAlive.position, orientation: ownAlive.orientation, shipType: own?.shipType ?? 1, radius: game.shipRadius ?? TINKER_RADIUS } : null,
        entities: r.entities,
        shots: r.projectiles,
        shipExt: game.shipExt?.(),
        colorOf: (id) => teamHull(id === game.player ? game.team : (teamOfShip(id) ?? 0), r.mode.teams),
        nameOf: (id) => names.get(id) ?? "",
        up: wf.up,
        raycast: game.raycast ? (o, d, max) => game.raycast?.(o.x, o.y, o.z, d.x, d.y, d.z, max) ?? -1 : undefined,
        quiet: flow.showResults || flow.showLobby,
      });
      spectacleShake = sf.shake;
      audio.update({ time, dt, player: game.player, online: session.isOnline, stage: flow.stage, camera: view, pose: r.pose, ships: r.ships, projectiles: r.projectiles, entities: r.entities, cues: r.cues, visuals: session.visuals, boosting: cueFrame.boosting, gadgetOf, gadgetIds: session.gadgetIds, names });
      // The cinematic camera sees the own ship too.
      // ...and so does an event's side view (M4 C1).
      const shownShips = (cine || ev.thirdPerson) && r.pose && own ? [...drawnShips, cinematic.ownShip(game.player, game.team, r.pose, own.shipType, game.shipRadius ?? TINKER_RADIUS)] : drawnShips;
      // Event pickups hidden by C1 (Moving day) and C2 (real pickups under their mimics; both in the hub's output).
      const shownEntities = ev.hideEntities.size > 0 ? r.entities.filter((e) => !ev.hideEntities.has(e.id)) : r.entities;
      // Entity kinds whose looks the world, visitor (hub) and spectacle modules draw themselves.
      const skipKinds = new Set([...(world.skipKinds ?? []), ...(events.skipKinds ?? []), ...spectacles.skipKinds]);
      const scene = buildScene(shownShips, drawnShots, shownEntities, session.visuals, session.teamOf(r.ships), time, {
          gadgetOf,
          beams: fx.beams,
          flashes: fx.flashes,
          // The guided Eyeball is the camera; hide only that one missile.
          hideShot,
          shipTypeOf: (id) => players.find((p) => p.id === id)?.shipType,
          wreckTypeOf: (e) => wrecks.typeOf(e),
          teams: r.mode.teams,
          vfx: vfx !== null,
          refractCloak: true,
          shipStyle: shipStyleFor(r.mode, r.mode.teams) ?? wf.shipStyle,
          skipKinds,
        });
      addModeProps(scene, modeInput);
      const worldLights: PointLight[] = [...wf.lights];
      world.addToScene(scene, r.entities, time, r.mode.teams, worldLights);
      events.addToScene(scene, worldLights, ev);
      spectacles.addToScene(scene);
      worldLights.push(...sf.lights);
      renderer.settings.lightLevel = sf.lightLevel;
      race.frame(
        {
          game,
          time,
          dt,
          view,
          mode: r.mode,
          pose: r.pose,
          ships: r.ships,
          names,
          ownColor: playerStyleFor(r.mode, r.mode.teams)?.(game.player, game.team)?.color ?? teamHull(game.team, r.mode.teams),
          hull: r.hud.hull ?? 0,
          hullMax: r.hud.hullMax ?? 100,
          spectating: flow.spectating,
          quiet: flow.showResults || flow.showLobby,
          width: canvas.clientWidth,
          height: canvas.clientHeight,
          say: (t) => audio.announcer.say(t),
        },
        renderer,
        scene,
        worldLights,
      );
      renderer.setScene(scene);
      renderer.setModeVolumes([...arch.volumes, ...modeVolumes(modeInput)]);
      renderer.setWorldVolumes(sf.volumes.length > 0 ? [...sf.volumes, ...wf.volumes] : wf.volumes);
      renderer.setWorldUp(wf.up, boundsCenter(game.bounds()));
      // Without VQ5 effects the light pulses come from watching projectiles and ships.
      if (!vfx) {
        lightFx.observe(r.projectiles, session.visuals, time);
        lightFx.observeShips(r.ships, time);
      }
      const lights = sceneLights({
        ships: r.ships,
        shots: r.projectiles,
        entities: r.entities,
        visuals: session.visuals,
        flashes: fx.flashes,
        beams: fx.beams,
        objectives: r.mode.objectives,
        gadgetOf,
        nowTick: game.nowTick?.() ?? 0,
        time,
      });
      for (const g of charges) lights.push(chargeLight(g));
      modeLights(modeInput, lights);
      lights.push(...worldLights, ...arch.lights);
      renderer.setLights(lightFx.frame(time, lights));
      // Last Color Standing: the world fades to monochrome; ships keep their meter's colour.
      mono = easeMonochrome(mono, isMonochrome(r.mode) && !modeInput.warmup && !flow.showResults ? 1 : 0, dt);
      renderer.settings.post.grade = monochromeGrade(world.grade(renderer.atmosphere?.grade ?? NEUTRAL_GRADE), mono);
      if (particles) {
        particles.moteColor = wf.motes.color;
        particles.moteRainbow = wf.motes.rainbow;
      }
      // Inside a dust cloud the fog closes in (visibility is cut).
      const atmo = renderer.atmosphere ?? DEFAULT_SETTINGS;
      const dust = dustDensityAt(view.position, r.entities, session.visuals);
      const fog = dustFog(atmo, dust);
      renderer.settings.fogDensity = fog.fogDensity * arch.fogScale;
      // Disco inferno dims the headlight with the light (M4 C3); the architect's view has its own (C4).
      renderer.settings.headlight = DEFAULT_SETTINGS.headlight * sf.headlight * arch.headlightScale;
      renderer.settings.fogColor = fog.fogColor;
      renderer.settings.terrainSeed = hashSeed(session.game.shareCode); // per-world rock look (VQ3b)
      // Inside dust the flat dust colour replaces the sky's haze.
      renderer.settings.skyHaze = renderer.atmosphere ? 1 - Math.min(1, dust) : 0;
      // Last Color Standing: the own cockpit frame keeps its meter's colour like the other hulls (D5).
      const ownStyle = playerStyleFor(r.mode, r.mode.teams)?.(game.player, game.team) ?? wf.ownStyle ?? undefined;
      const frameState = cues.hullFrame(cueFrame, own?.shipType ?? 1, game.shipRadius ?? TINKER_RADIUS, ownStyle?.color ?? teamHull(game.team, r.mode.teams));
      renderer.hullFrame.state = ev.thirdPerson ? null : frameState && ownStyle ? { ...frameState, keepColor: ownStyle.keepColor } : frameState;
      events.apply(renderer, ev);
      hud.fitRing.update(cues.fitRing(cueFrame, game, r.pose?.position ?? view.position, view.orientation, view.fovY));
      // Per-pass GPU timing (separate terrain/objects passes) while the overlay shows.
      renderer.detailedTiming = overlay.visible;
      frameStats = renderer.render(view);
      spectacles.renderKillCam(renderer, {
        visuals: session.visuals,
        teamOf: (id) => (id === game.player ? game.team : teamOfShip(id)),
        teams: r.mode.teams,
        create: (c, device) => Renderer.create(c, { device, quality: "low", particles: false, shadows: false }),
      });
      nameTags.update(
        nameTagModel(
          r.ships,
          r.players,
          view.position,
          renderer.viewProjection(view),
          canvas.clientWidth,
          canvas.clientHeight,
          game.team,
          game.monochrome && wf.shipStyle
            ? {
                pigmented: wf.pigmented,
                inSight: (p) => {
                  const d = { x: p.x - view.position.x, y: p.y - view.position.y, z: p.z - view.position.z };
                  const len = Math.hypot(d.x, d.y, d.z);
                  if (len < 1e-3 || !game.raycast) return true;
                  const hit = game.raycast(view.position.x, view.position.y, view.position.z, d.x / len, d.y / len, d.z / len, len);
                  return hit < 0 || hit > len - 1;
                },
              }
            : null,
        ),
        r.mode.teams,
      );
      hud.pull.update(flow.showResults || flow.spectating ? null : wf.pull);
      const vp = renderer.viewProjection(view);
      events.updateHud((p) => {
        const [x, y, , w] = transformPoint(vp, p);
        return { x, y, w };
      }, canvas.clientWidth, canvas.clientHeight, time);
      hud.mood.update(flow.showResults ? null : wf.mood);

      // Mode HUD, objective markers, lobby card, results and the Tab scoreboard.
      const ownSide = game.team;
      const nowTick = game.nowTick?.() ?? 0;
      const hudModel = modeHudModel({
        mode: r.mode,
        stage: flow.stage,
        own: game.player,
        ownSide,
        nowTick,
        timeLeft: r.hud.timeLeft ?? 0,
        names,
        ownPos: r.pose && r.pose.alive ? r.pose.position : null,
        spectating: flow.spectating,
        following: spectator?.following ?? null,
      });
      modeHud.update(hudModel);
      modeHud.updateMarkers(
        flow.showResults || flow.showLobby
          ? []
          : [
              ...objectiveMarkers(
                r.mode,
                game.player,
                names,
                nowTick,
                renderer.viewProjection(view),
                view.position,
                canvas.clientWidth,
                canvas.clientHeight,
                ownSide,
              ),
              ...architect.runnerMarkers(architect.view, game.player, r.ships, names, renderer.viewProjection(view), view.position, canvas.clientWidth, canvas.clientHeight),
            ],
      );
      hud.setInstagib(hudModel.instagib);
      hud.setSpectating(flow.spectating);
      hud.show(!flow.showResults);
      if (lobbyNow) lobby.update(lobbyNow);
      lobby.show(lobbyNow !== null && !flow.showResults);
      if (flow.showResults) {
        const res = r.results ?? practiceResults(r.mode, session.scoreboard(), names);
        results.update(
          resultsModel({
            results: res,
            own: game.player,
            ownSide,
            online: session.isOnline,
            modeFlags: r.mode.flags,
            scoreTotal: r.mode.scoreTotal,
            lobby: r.lobby,
            secsLeft: lobbySecs,
            names,
            architect: { rounds: architect.rounds, mode: r.mode },
          }),
        );
      }
      results.show(flow.showResults);
      const showBoard = !flow.showResults && state.scoreboard;
      if (showBoard) scoreboard.update(scoreboardModel(session.scoreboard(), r.mode, ownSide, session.isOnline));
      scoreboard.show(showBoard);

      hud.updateFeed(r.feed, now);
      const status = initialLoad
        ? `Loading terrain (${chunkStats.pending} chunks left)`
        : flow.showResults || flow.spectating || !r.mode.running
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
      shipPanel.update(
        shipPanelModel(
          !flow.showResults && (bay || shipBayOpen || dead) ? session.shipChoices() : [],
          shipBayOpen && !flow.showResults,
          dead && !flow.showResults,
          shipError,
        ),
      );

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
            shareCode: game.shareCode, // the current match's world (a transition changes it)
            speed,
            gpuError: renderer.lastError ?? null,
            gpu: renderer.gpuTimings(),
            quality: renderer.quality.level,
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
