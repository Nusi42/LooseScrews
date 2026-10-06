// Visual gallery pages of the first-impression pass (FI): the start screen,
// the lobby card and the in-game ship bay over the live backdrop (and, M4
// A8, the world workshop over the preview of its world), rendered
// like the other gallery scenes (gallery/gallery.ts hands scenes with a
// `page` here). The backdrop steps at a fixed 60 Hz from a fixed seed up to
// the scene's time, then holds that frame (camera, ships, shots and shader
// time frozen), so screenshots repeat. The hull views (shipBay.ts) hold
// their turntable time too; their GPU time is reported as the pass "bay".

import type { GalleryScene } from "../gallery/scenes.ts";
import type { GalleryStatus } from "../gallery/gallery.ts";
import { parseShipChoices } from "../game/mechanics.ts";
import type { LobbyState, MatchResults, ResultRow } from "../game/modeState.ts";
import { Renderer } from "../renderer/renderer.ts";
import type { GpuTimings } from "../renderer/gpuTimer.ts";
import { LobbyOverlay, lobbyModel } from "../ui/lobby.ts";
import { ResultsScreen, resultsModel } from "../ui/results.ts";
import { sampleArchitectResults } from "../gallery/architectScenes.ts";
import { ShipPanel, shipPanelModel } from "../ui/shipSelect.ts";
import { setStartWorld, showStartScreen } from "../ui/startScreen.ts";
import { draftKind, showWorkshop, slidersFrom, wasmCodec } from "../ui/workshop.ts";
import { loadWasm } from "../wasm/load.ts";
import { Backdrop } from "./backdrop.ts";
import { PanelBay, ShipBayView } from "./shipBay.ts";
import { describeWorld } from "./worlds.ts";

const nextFrame = (): Promise<void> => new Promise((r) => requestAnimationFrame(() => r()));
/** Frames rendered at the held pose before the page reports ready. */
const WARMUP_FRAMES = 20;
/** Turntable time of the hull views in the shots (s). */
const BAY_TIME = 2.2;

/** A lobby a few seconds into the warm-up: Team Brawl, six pilots, some ready. */
export function sampleLobby(code: string): { lobby: LobbyState; names: Map<number, string> } {
  const names = new Map<number, string>([
    [0, "Scrapling"],
    [1, "Rivet"],
    [2, "Torque"],
    [3, "Sprocket"],
    [4, "Gasket"],
    [5, "Ratchet"],
  ]);
  const lobby: LobbyState = {
    phase: "lobby",
    phaseTicksLeft: 60 * 42,
    roomCode: "",
    nextMode: 0,
    nextModeId: "team_brawl",
    nextRuleset: "Team Brawl",
    nextShareCode: code,
    minPlayers: 2,
    maxPlayers: 8,
    teams: 2,
    players: [
      { id: 0, team: -1, ready: false, spectator: false },
      { id: 1, team: 0, ready: true, spectator: false },
      { id: 2, team: 1, ready: true, spectator: false },
      { id: 3, team: 1, ready: false, spectator: false },
      { id: 4, team: 0, ready: true, spectator: false },
      { id: 5, team: -1, ready: false, spectator: true },
    ],
  };
  return { lobby, names };
}

/**
 * A finished match for the "results" pages (M4 B3): free-for-all Hot
 * Reactor or Team Brawl with six pilots; with `awards`, Hat rain's Hat Baron.
 */
export function sampleResults(awards: boolean, teams: boolean): MatchResults {
  const names = ["Scrapling", "Rivet", "Torque", "Sprocket", "Gasket", "Ratchet"];
  const scores = teams ? [9, 7, 6, 4, 3, 1] : [5, 4, 3, 2, 1, 0];
  const rows: ResultRow[] = names.map((name, id) => ({
    id,
    side: teams ? id % 2 : id,
    kills: [9, 7, 6, 4, 3, 1][id]!,
    deaths: [2, 3, 5, 4, 6, 7][id]!,
    score: scores[id]!,
    name,
  }));
  return {
    tick: 18_000,
    kind: teams ? 1 : 4,
    mode: teams ? "team_brawl" : "hot_reactor",
    teams: teams ? 2 : 0,
    winner: teams ? 0 : 0,
    teamScores: teams ? [21, 12] : [],
    rows,
    awards: awards ? [{ event: "hat_rain", player: teams ? 0 : 3, value: 7 }] : [],
  };
}

/** Adds the hull view's passes to the scene's timings as one "bay" pass. */
function withBay(gpu: GpuTimings | null, bay: GpuTimings | null): GpuTimings | null {
  if (!gpu || !bay) return gpu;
  return { ...gpu, passes: [...gpu.passes, ["bay", bay.total]], total: gpu.total + bay.total, worst: gpu.worst + bay.worst };
}

export async function runPageScene(canvas: HTMLCanvasElement, ui: HTMLElement, scene: GalleryScene, status: GalleryStatus): Promise<void> {
  const api = await loadWasm();
  // `&world=<seed>_<enclosure>_<density>` and `&t=<seconds>` try other worlds and moments (curating BACKDROP_WORLDS).
  const params = new URLSearchParams(location.search);
  const override = /^(\d+)_(\d+)_(\d+)$/.exec(params.get("world") ?? "");
  const w = override ? { seed: Number(override[1]), enclosure: Number(override[2]), density: Number(override[3]) } : scene.world;
  const flyTime = Number(params.get("t") ?? "") || scene.time;
  let caption: string | null = null;
  const ws = scene.workshop;
  const codec = wasmCodec(api);
  if (scene.page === "workshop" && ws) {
    // `&seed=<u64>` tries other seeds (curating the workshop shots).
    const seed = /^\d{1,20}$/.test(params.get("seed") ?? "") ? (params.get("seed") as string) : ws.seed;
    const draft = { seed, sliders: slidersFrom(ws.sliders), archetype: ws.archetype };
    const made = codec.encode(draft);
    if (!made.ok) throw new Error(`workshop world: ${made.error}`);
    status.shareCode = made.code;
    caption = `${draftKind(draft)} · ${made.code}`;
  } else {
    status.shareCode = api.randomShareCode(w.seed, 0, w.enclosure, w.density);
  }
  const renderer = await Renderer.create(canvas);
  renderer.detailedTiming = true;
  status.canTime = renderer.canTime;
  const backdrop = new Backdrop(renderer, api, { code: status.shareCode, seed: w.seed, fixedStep: 1 / 60 });
  const bays: Array<{ gpuTimings(): GpuTimings | null }> = [];

  if (scene.page === "start") {
    let view: ShipBayView | null = null;
    void showStartScreen(ui, undefined, {
      // The "Make a world" entry shows (the workshop has its own pages).
      workshop: () => new Promise(() => {}),
      ships: (host, cards) => {
        const slots = (sel: number) =>
          cards.map((c, i) => ({ shipType: i, el: c.querySelector<HTMLElement>(".ship-pick-model") ?? c, selected: i === sel, allowed: true, team: i % 2 }));
        let selected = 1;
        void ShipBayView.create(host, renderer.device, "medium").then((v) => {
          view = v;
          v.timeOverride = BAY_TIME;
          host.classList.add("has-bay");
          v.canvas.classList.add("live");
          v.setSlots(slots(selected));
          v.show(true);
          bays.push(v);
        });
        return {
          select: (i) => {
            selected = i;
            view?.setSlots(slots(i));
          },
          dispose: () => view?.dispose(),
        };
      },
    });
  } else if (scene.page === "workshop") {
    const root = document.createElement("div");
    root.className = "screen start-screen shown in-workshop";
    root.innerHTML = `<div class="start-shade" aria-hidden="true"></div><footer class="start-footer"><span class="start-world"></span></footer>`;
    ui.appendChild(root);
    const handle = showWorkshop(root, { codec, code: status.shareCode, remember: false, pageHref: "https://play.example/" });
    handle.setPreview({ code: status.shareCode, progress: 1, ready: true });
    if (ws?.host) handle.showHost();
    if (handle.draft.archetype !== ws?.archetype) throw new Error("workshop scene: the archetype did not round-trip through the share code");
  } else if (scene.page === "lobby") {
    const lobbyCard = new LobbyOverlay(ui, { ready: () => {}, team: () => {}, spectate: () => {} });
    const { lobby, names } = sampleLobby(status.shareCode);
    lobbyCard.update(lobbyModel(lobby, 0, names, 42));
    lobbyCard.show(true);
  } else if (scene.page === "results") {
    const screen = new ResultsScreen(ui);
    const teams = scene.results?.teams === true;
    const arch = scene.results?.architect === true ? sampleArchitectResults() : null;
    const results = arch?.results ?? sampleResults(scene.results?.awards === true, teams);
    screen.update(
      resultsModel({
        results,
        architect: arch?.architect ?? null,
        own: 0,
        ownSide: 0,
        online: true,
        modeFlags: 0,
        scoreTotal: 0,
        lobby: null,
        secsLeft: 12,
      }),
    );
    screen.show(true);
  } else if (scene.page === "bay") {
    const panel = new ShipPanel(ui);
    const bay = new PanelBay(panel.root, renderer.device, 0, "medium");
    panel.backdrop = bay;
    const choices = parseShipChoices(backdrop.game.shipChoices?.());
    panel.update(shipPanelModel(choices, true, false));
    // The view is created on the first update; hold its turntable once it exists.
    for (let i = 0; i < 300 && !bay.hullView; i++) await nextFrame();
    const view = bay.hullView;
    if (view) {
      view.timeOverride = BAY_TIME;
      bays.push(view);
    }
  }

  // Fly to the scene's time at 60 Hz (rendering each step), then hold.
  for (let i = 0; i < 1200 && !backdrop.ready; i++) {
    backdrop.step(1 / 60);
    await nextFrame();
  }
  if (!backdrop.ready) throw new Error("backdrop terrain did not finish loading");
  document.body.classList.add("backdrop-live");
  setStartWorld(ui, caption ?? describeWorld(status.shareCode));
  const steps = Math.round(flyTime * 60);
  for (let i = 0; i < steps; i++) {
    backdrop.step(1 / 60);
    if (i % 4 === 0) await nextFrame();
  }
  backdrop.hold = true;
  status.camera = backdrop.cameraView;

  const frame = (): void => {
    backdrop.step(0);
    status.frames++;
    const stats = backdrop.lastStats;
    status.chunksDrawn = stats.chunksDrawn;
    status.triangles = stats.triangles;
    status.instances = stats.instances;
    let gpu = renderer.gpuTimings();
    for (const b of bays) gpu = withBay(gpu, b.gpuTimings());
    status.gpu = gpu;
    status.lights = renderer.lightStats;
    status.gpuError = renderer.lastError;
    status.particles = renderer.particles?.liveCount() ?? null;
    if (status.frames === WARMUP_FRAMES) status.phase = "ready";
    if (!renderer.isLost) requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}
