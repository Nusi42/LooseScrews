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
