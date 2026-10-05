// Per-pass GPU timing with the optional WebGPU `timestamp-query` feature.
//
// Every timed render pass gets a pair of timestamps (`timestampWrites`, begin
// and end). At the end of the frame the query set is resolved and copied into
// a mappable readback buffer from a small pool; when the map resolves (a frame
// or two later), the durations are summed per pass label (several passes may
// share one label, e.g. the bloom chain all counts as "post") and fed into a
// sliding window. Without the feature `GpuTimer.create` returns null and the
// renderer runs untimed.
//
// Chrome quantises timestamps to 100 µs unless it runs with
// `--enable-webgpu-developer-features`; the window average smooths that out
// in normal browsers, and the visual gallery (tools/browser-smoke/gallery.mjs)
// launches the browser with the flag.

/** Pass labels the renderer uses (others are allowed; the overlay lists them in this order first). */
export const PASS_ORDER: readonly string[] = ["shadow", "scene", "terrain", "objects", "particles", "lights", "volumetric", "sky", "mode", "refract", "post"];

/** Maximum timed passes per frame (two queries each). */
export const MAX_TIMED_PASSES = 32;

/** Frames averaged by the window. */
export const TIMING_WINDOW = 60;

/** GPU time per pass label (ms, window average) and the frame total (sum of the passes). */
export interface GpuTimings {
  /** Pass label -> average milliseconds per frame, in PASS_ORDER (then first-seen) order. */
  passes: Array<[string, number]>;
  /** Average sum of all passes per frame (ms). */
  total: number;
  /** Worst frame total in the window (ms). */
  worst: number;
  /** Frames in the window. */
  frames: number;
}

/**
 * Milliseconds per label from resolved timestamps (ns, begin/end pairs in
 * `labels` order). Pairs whose end precedes the begin (a reset or an
 * unordered implementation) count as 0.
 */
export function durationsFromTicks(ticks: ArrayLike<bigint>, labels: readonly string[]): Map<string, number> {
  const out = new Map<string, number>();
  labels.forEach((label, i) => {
    const begin = ticks[2 * i];
    const end = ticks[2 * i + 1];
    let ms = 0;
    if (begin !== undefined && end !== undefined && end > begin) ms = Number(end - begin) / 1e6;
    out.set(label, (out.get(label) ?? 0) + ms);
  });
  return out;
}

/** Sliding window of per-frame pass durations. */
export class TimingWindow {
  private readonly size: number;
  private readonly frames: Array<Map<string, number>> = [];

  constructor(size = TIMING_WINDOW) {
    this.size = size;
  }

  push(frame: Map<string, number>): void {
    this.frames.push(frame);
    while (this.frames.length > this.size) this.frames.shift();
  }

  clear(): void {
    this.frames.length = 0;
  }

  get length(): number {
    return this.frames.length;
  }

  /** Averages over the window; null while it is empty. */
  summary(): GpuTimings | null {
    const n = this.frames.length;
    if (n === 0) return null;
    const sums = new Map<string, number>();
    let total = 0;
    let worst = 0;
    for (const f of this.frames) {
      let frameTotal = 0;
      for (const [label, ms] of f) {
        sums.set(label, (sums.get(label) ?? 0) + ms);
        frameTotal += ms;
      }
      total += frameTotal;
      worst = Math.max(worst, frameTotal);
    }
    const labels = [...sums.keys()].sort((a, b) => rank(a) - rank(b));
    return { passes: labels.map((l) => [l, (sums.get(l) ?? 0) / n]), total: total / n, worst, frames: n };
  }
}

function rank(label: string): number {
  const i = PASS_ORDER.indexOf(label);
  return i < 0 ? PASS_ORDER.length : i;
}

interface Readback {
  buffer: GPUBuffer;
  labels: string[];
  busy: boolean;
}

/** Timestamp queries for the render passes of one frame at a time. */
export class GpuTimer {
  readonly window = new TimingWindow();
  private readonly device: GPUDevice;
  private readonly querySet: GPUQuerySet;
  private readonly resolveBuffer: GPUBuffer;
  private readonly pool: Readback[] = [];
  private labels: string[] = [];
  private pending: Readback | null = null;
  private destroyed = false;

  /** A timer when the device has `timestamp-query`, else null. */
  static create(device: GPUDevice): GpuTimer | null {
    if (!device.features.has("timestamp-query")) return null;
    try {
      return new GpuTimer(device);
    } catch {
      return null;
    }
  }

  private constructor(device: GPUDevice) {
    this.device = device;
    this.querySet = device.createQuerySet({ label: "pass timestamps", type: "timestamp", count: 2 * MAX_TIMED_PASSES });
    this.resolveBuffer = device.createBuffer({
      label: "timestamp resolve",
      size: 2 * MAX_TIMED_PASSES * 8,
      usage: GPUBufferUsage.QUERY_RESOLVE | GPUBufferUsage.COPY_SRC,
    });
  }

  /** Starts a frame (forgets the previous frame's labels). */
  beginFrame(): void {
    this.labels = [];
  }

  /** `timestampWrites` for the next render pass, counted under `label`; undefined when the frame is full. */
  pass(label: string): GPURenderPassTimestampWrites | undefined {
    const i = this.labels.length;
    if (i >= MAX_TIMED_PASSES) return undefined;
    this.labels.push(label);
    return { querySet: this.querySet, beginningOfPassWriteIndex: 2 * i, endOfPassWriteIndex: 2 * i + 1 };
  }

  /** Resolves this frame's queries into a free readback buffer (skips the frame when all are in flight). */
  endFrame(encoder: GPUCommandEncoder): void {
    const n = this.labels.length;
    this.pending = null;
    if (n === 0) return;
    let rb = this.pool.find((r) => !r.busy);
    if (!rb) {
      if (this.pool.length >= 4) return;
      rb = {
        buffer: this.device.createBuffer({
          label: "timestamp readback",
          size: 2 * MAX_TIMED_PASSES * 8,
          usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST,
        }),
        labels: [],
        busy: false,
      };
      this.pool.push(rb);
    }
    encoder.resolveQuerySet(this.querySet, 0, 2 * n, this.resolveBuffer, 0);
    encoder.copyBufferToBuffer(this.resolveBuffer, 0, rb.buffer, 0, 2 * n * 8);
    rb.busy = true;
    rb.labels = this.labels;
    this.pending = rb;
  }

  /** Call after `queue.submit`: maps the frame's readback and feeds the window when it resolves. */
  afterSubmit(): void {
    const rb = this.pending;
    this.pending = null;
    if (!rb) return;
    rb.buffer
      .mapAsync(GPUMapMode.READ)
      .then(() => {
        if (this.destroyed) return;
        const ticks = new BigUint64Array(rb.buffer.getMappedRange(0, rb.labels.length * 16).slice(0));
        rb.buffer.unmap();
        this.window.push(durationsFromTicks(ticks, rb.labels));
        rb.busy = false;
      })
      .catch(() => {
        rb.busy = false;
      });
  }

  timings(): GpuTimings | null {
    return this.window.summary();
  }

  destroy(): void {
    this.destroyed = true;
    this.querySet.destroy();
    this.resolveBuffer.destroy();
    for (const r of this.pool) r.buffer.destroy();
  }
}
