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

import type { Renderer } from "../renderer/renderer.ts";
import { Renderer as RendererClass } from "../renderer/renderer.ts";
import type { WasmApi } from "../wasm/api.ts";
import { setStartWorld, START_SHIPS, type StartScreenHooks } from "../ui/startScreen.ts";
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

export class StartIntro {
  private readonly ui: HTMLElement;
  private readonly created: Promise<Renderer>;
  private backdrop: Backdrop | null = null;
  private stopped = false;

  /** Starts creating the renderer after the next paint; the backdrop follows when `backdrop` is true. */
  constructor(canvas: HTMLCanvasElement, ui: HTMLElement, api: WasmApi, backdrop = true) {
    this.ui = ui;
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
          const t0 = performance.now();
          const b = new Backdrop(renderer, api);
          performance.measure("ls:backdrop-world", { start: t0, end: performance.now() });
          this.backdrop = b;
          b.onReady = () => {
            if (this.stopped) return;
            document.body.classList.add("backdrop-live");
            setStartWorld(ui, describeWorld(b.shareCode));
          };
          b.start();
        })
        .catch((err: unknown) => console.warn("start backdrop:", err));
    }
  }

  /** The renderer for the game (rejects like `Renderer.create` without WebGPU), its meshes all built. */
  renderer(): Promise<Renderer> {
    return this.created.then((r) => {
      r.prepareMeshes();
      return r;
    });
  }

  /** Hooks for `showStartScreen`: the hull view behind the ship picker. */
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
    };
  }

  /** Stops the backdrop (the renderer stays, emptied, for the game). */
  stop(): void {
    if (this.stopped) return;
    this.stopped = true;
    this.backdrop?.stop();
    this.backdrop = null;
    document.body.classList.remove("backdrop-live");
    setStartWorld(this.ui, "");
  }
}
