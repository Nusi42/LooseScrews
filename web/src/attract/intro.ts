// Start-screen glue (first-impression pass, FI): creates the game's renderer
// while the start screen is already interactive, runs the live backdrop
// (backdrop.ts) behind it and the hull view (shipBay.ts) behind the ship
// picker, and hands the renderer to the game when the player presses Play.
//
// Order, so the start screen is interactive at once: the screen shows
// (main.ts), then after its first paint the renderer is created (adapter,
// device, pipelines), then on the next idle moment the backdrop level is
// generated and its chunks stream in under a small per-frame budget. Without
// WebGPU nothing shows behind the screen; `renderer()` then rejects with the
// same error the game reports. `?backdrop=off` skips the flythrough.
//
// World workshop (M4 A8): `preview(code)` swaps the flythrough for the world
// of `code`. The new world is built while the canvas keeps the old one's last
// frame (blurred by `body.preview-busy`), then fades in sharp; the workshop
// gets the build progress. `?workshop` (or `?workshop=<code>`) opens the
// workshop at once.

import type { Renderer } from "../renderer/renderer.ts";
import { Renderer as RendererClass } from "../renderer/renderer.ts";
import type { WasmApi } from "../wasm/api.ts";
import { setStartWorld, START_SHIPS, type StartScreenHooks } from "../ui/startScreen.ts";
import { draftKind, showWorkshop, wantsWorkshop, wasmCodec, type WorldCodec } from "../ui/workshop.ts";
import { Backdrop } from "./backdrop.ts";
import { ShipBayView, type BaySlot } from "./shipBay.ts";
import { describeWorld } from "./worlds.ts";

/** Resolves after the browser painted the current DOM. */
export function afterPaint(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => setTimeout(resolve, 0)));
}

/** Resolves when the main thread is idle (or after `timeout` ms). */
export function whenIdle(timeout = 300): Promise<void> {
  return new Promise((resolve) => {
    const ric = (globalThis as { requestIdleCallback?: (cb: () => void, o: { timeout: number }) => number }).requestIdleCallback;
    if (ric) ric(() => resolve(), { timeout });
    else setTimeout(resolve, 30);
  });
}

export function wantsBackdrop(loc: Pick<Location, "search">): boolean {
  return new URLSearchParams(loc.search).get("backdrop") !== "off";
}

/** Preview state for the workshop: the code being shown, build progress (0..1), ready. */
export interface PreviewState {
  code: string;
  progress: number;
  ready: boolean;
}

/** Progress report interval while a preview world is built (ms). */
const PREVIEW_POLL_MS = 120;
/**
 * Chunk generation per frame while a workshop preview is built (ms): nothing
 * is drawn meanwhile, so most of the frame goes to the new world and the
 * sliders still answer at about 40 Hz.
 */
const PREVIEW_GENERATE_MS = 20;

export class StartIntro {
  private readonly ui: HTMLElement;
  private readonly api: WasmApi;
  private readonly created: Promise<Renderer>;
  private readonly backdropOn: boolean;
  private backdrop: Backdrop | null = null;
  private stopped = false;
  /** The backdrop has begun (later code changes swap it). */
  private begun = false;
  /** The world the workshop wants shown (null: one of BACKDROP_WORLDS). */
  private previewCode: string | null = null;
  private poll: ReturnType<typeof setInterval> | null = null;
  private codecCache: WorldCodec | null = null;
  /** Receives the preview's progress while the workshop is open. */
  private previewListener: ((s: PreviewState) => void) | null = null;

  /** Starts creating the renderer after the next paint; the backdrop follows when `backdrop` is true. */
  constructor(canvas: HTMLCanvasElement, ui: HTMLElement, api: WasmApi, backdrop = true) {
    this.ui = ui;
    this.api = api;
    this.backdropOn = backdrop;
    this.created = afterPaint().then(async () => {
      const t0 = performance.now();
      const r = await RendererClass.create(canvas);
      performance.measure("ls:renderer-create", { start: t0, end: performance.now() });
      return r;
    });
    // Errors surface when the game asks for the renderer (`renderer()`).
    this.created.catch(() => {});
    if (backdrop) {
      this.created
        .then(async (renderer) => {
          await whenIdle();
          if (this.stopped || renderer.isLost) return;
          this.begun = true;
          const t0 = performance.now();
          this.show(renderer, this.previewCode, false);
          performance.measure("ls:backdrop-world", { start: t0, end: performance.now() });
        })
        .catch((err: unknown) => console.warn("start backdrop:", err));
    }
  }

  private get codec(): WorldCodec {
    this.codecCache ??= wasmCodec(this.api);
    return this.codecCache;
  }

  /** Caption under the flythrough: the world kind and its code. */
  private caption(code: string): string {
    const d = this.codec.decode(code);
    return d.ok ? `${draftKind(d.draft)} · ${d.code}` : describeWorld(code);
  }

  /**
   * Flies the world of `code` (null: a curated one). `quiet`: keep the last
   * frame of the current world on the canvas until the new one is ready.
   */
  private show(renderer: Renderer, code: string | null, quiet: boolean): void {
    this.backdrop?.stop();
    this.backdrop = null;
    if (this.poll) clearInterval(this.poll);
    this.poll = null;
    const body = document.body;
    if (quiet) body.classList.add("preview-busy");
    let b: Backdrop;
    try {
      b = new Backdrop(renderer, this.api, { ...(code ? { code } : {}), quietLoad: quiet, ...(quiet ? { generateBudgetMs: PREVIEW_GENERATE_MS } : {}) });
    } catch (err) {
      body.classList.remove("preview-busy");
      console.warn("start backdrop:", err);
      return;
    }
    this.backdrop = b;
    const report = (): void => this.previewListener?.({ code: b.shareCode, progress: b.progress, ready: b.ready });
    report();
    this.poll = setInterval(() => {
      if (this.backdrop !== b || b.ready) {
        if (this.poll) clearInterval(this.poll);
        this.poll = null;
        return;
      }
      report();
    }, PREVIEW_POLL_MS);
    b.onReady = () => {
      if (this.stopped || this.backdrop !== b) return;
      body.classList.add("backdrop-live");
      body.classList.remove("preview-busy");
      body.dataset["previewCode"] = b.shareCode;
      setStartWorld(this.ui, this.caption(b.shareCode));
      report();
    };
    b.start();
  }

  /**
   * Shows the world of `code` behind the start screen (the workshop preview).
   * Before the backdrop has begun the code is remembered and flown first.
   */
  preview(code: string): void {
    if (this.stopped || !this.backdropOn) return;
    this.previewCode = code;
    if (!this.begun) return;
    this.created
      .then((renderer) => {
        if (this.stopped || renderer.isLost || this.previewCode !== code) return;
        this.show(renderer, code, true);
      })
      .catch(() => {});
  }

  /** The renderer for the game (rejects like `Renderer.create` without WebGPU), its meshes all built. */
  renderer(): Promise<Renderer> {
    return this.created.then((r) => {
      r.prepareMeshes();
      return r;
    });
  }

  /** Hooks for `showStartScreen`: the hull view behind the ship picker, the world workshop. */
  get hooks(): StartScreenHooks {
    return {
      ships: (host, cards) => {
        let view: ShipBayView | null = null;
        let selected = 1;
        let disposed = false;
        const slots = (): BaySlot[] =>
          cards.map((card, i) => ({
            shipType: i,
            el: card.querySelector<HTMLElement>(".ship-pick-model") ?? card,
            selected: i === selected,
            allowed: true,
            team: i % 2,
          }));
        this.created
          .then(async (renderer) => {
            if (disposed) return;
            const v = await ShipBayView.create(host, renderer.device, renderer.quality.level === "low" ? "low" : "medium");
            if (disposed) {
              v.dispose();
              return;
            }
            view = v;
            host.classList.add("has-bay");
            v.setSlots(slots());
            v.show(true);
            requestAnimationFrame(() => v.canvas.classList.add("live"));
          })
          .catch((err: unknown) => console.warn("ship bay view:", err));
        return {
          select: (shipType) => {
            selected = Math.max(0, Math.min(START_SHIPS.length - 1, shipType));
            view?.setSlots(slots());
          },
          dispose: () => {
            disposed = true;
            view?.dispose();
            view = null;
          },
        };
      },
      workshop: (host, code) => {
        const handle = showWorkshop(host, { codec: this.codec, preview: (c) => this.preview(c), code });
        this.previewListener = (s) => handle.setPreview(s);
        if (!this.backdropOn) handle.setPreview({ code: "", progress: 0, ready: true });
        else if (this.backdrop) {
          const b = this.backdrop;
          handle.setPreview({ code: b.shareCode, progress: b.progress, ready: b.ready });
        }
        return handle.done.then((exit) => {
          this.previewListener = null;
          return exit;
        });
      },
      openWorkshop: typeof location === "undefined" ? false : wantsWorkshop(location),
    };
  }

  /** Stops the backdrop (the renderer stays, emptied, for the game). */
  stop(): void {
    if (this.stopped) return;
    this.stopped = true;
    if (this.poll) clearInterval(this.poll);
    this.poll = null;
    this.previewListener = null;
    this.backdrop?.stop();
    this.backdrop = null;
    document.body.classList.remove("backdrop-live", "preview-busy");
    delete document.body.dataset["previewCode"];
    setStartWorld(this.ui, "");
  }
}
