// WebGPU adapter/device/context setup with explicit, user-presentable errors.

export type GpuFailureReason = "no-webgpu" | "no-adapter" | "no-device" | "no-context";

/** Thrown when WebGPU cannot be used; `reason` selects the help text on the error screen. */
export class GpuUnavailableError extends Error {
  override name = "GpuUnavailableError";
  readonly reason: GpuFailureReason;

  constructor(reason: GpuFailureReason, message: string) {
    super(message);
    this.reason = reason;
  }
}

export interface GpuContext {
  device: GPUDevice;
  context: GPUCanvasContext;
  format: GPUTextureFormat;
}

/**
 * Optional device features the renderer uses when the adapter has them:
 * `timestamp-query` for the per-pass GPU timings (gpuTimer.ts).
 */
export const OPTIONAL_FEATURES: readonly GPUFeatureName[] = ["timestamp-query"];

/**
 * Adapter power preference: `?gpu=low-power` picks the integrated GPU on
 * machines with two (budget checks for the "medium" preset), anything else
 * the fast one.
 */
export function powerPreferenceFrom(search: string): GPUPowerPreference {
  return new URLSearchParams(search).get("gpu") === "low-power" ? "low-power" : "high-performance";
}

/**
 * A canvas context on an existing device (a second canvas drawn by the same
 * GPU device, e.g. the ship bay's hull view next to the game's canvas).
 */
export function contextOn(canvas: HTMLCanvasElement, device: GPUDevice): GpuContext {
  const context = canvas.getContext("webgpu");
  if (!context) throw new GpuUnavailableError("no-context", "Could not create a WebGPU canvas context.");
  const format = navigator.gpu.getPreferredCanvasFormat();
  context.configure({ device, format, alphaMode: "opaque" });
  return { device, context, format };
}

export async function initGpu(canvas: HTMLCanvasElement): Promise<GpuContext> {
  if (!("gpu" in navigator) || !navigator.gpu) {
    throw new GpuUnavailableError("no-webgpu", "This browser does not support WebGPU.");
  }
  const search = typeof location === "undefined" ? "" : location.search;
  const adapter = await navigator.gpu.requestAdapter({ powerPreference: powerPreferenceFrom(search) });
  if (!adapter) {
    throw new GpuUnavailableError("no-adapter", "WebGPU is available but no suitable GPU adapter was found.");
  }
  let device: GPUDevice;
  try {
    const requiredFeatures = OPTIONAL_FEATURES.filter((f) => adapter.features.has(f));
    device = await adapter.requestDevice({ requiredFeatures });
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    throw new GpuUnavailableError("no-device", `Could not create a GPU device: ${detail}`);
  }
  const context = canvas.getContext("webgpu");
  if (!context) {
    device.destroy();
    throw new GpuUnavailableError("no-context", "Could not create a WebGPU canvas context.");
  }
  const format = navigator.gpu.getPreferredCanvasFormat();
  context.configure({ device, format, alphaMode: "opaque" });
  return { device, context, format };
}

/** What `watchDevice` reports about a device. */
export interface DeviceWatch {
  /** True once the device is lost. */
  lost: boolean;
  /** Stops listening for uncaptured errors (the device may outlive the watcher). */
  stop(): void;
}

/**
 * Listens for a device's uncaptured errors (`onError` gets the message) and
 * its loss. A renderer on another renderer's device (the FI ship bay) calls
 * `stop` when it is destroyed: the shared device would otherwise keep the
 * listener, and through it the destroyed renderer, for the page's lifetime
 * (and log each error once per renderer ever created on it). The loss flag
 * lives in the returned object, not in the caller, for the same reason.
 */
export function watchDevice(device: Pick<GPUDevice, "addEventListener" | "lost">, onError: (message: string) => void): DeviceWatch {
  const abort = new AbortController();
  const watch: DeviceWatch = { lost: false, stop: () => abort.abort() };
  device.addEventListener(
    "uncapturederror",
    (event) => onError((event as GPUUncapturedErrorEvent).error.message),
    { signal: abort.signal },
  );
  device.lost
    .then(() => {
      watch.lost = true;
    })
    .catch(() => {});
  return watch;
}
