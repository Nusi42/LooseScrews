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

export async function initGpu(canvas: HTMLCanvasElement): Promise<GpuContext> {
  if (!("gpu" in navigator) || !navigator.gpu) {
    throw new GpuUnavailableError("no-webgpu", "This browser does not support WebGPU.");
  }
  const adapter = await navigator.gpu.requestAdapter({ powerPreference: "high-performance" });
  if (!adapter) {
    throw new GpuUnavailableError("no-adapter", "WebGPU is available but no suitable GPU adapter was found.");
  }
  let device: GPUDevice;
  try {
    device = await adapter.requestDevice();
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
