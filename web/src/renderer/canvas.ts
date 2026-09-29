// Keeps a canvas's backing store matched to its on-screen size in device pixels.

export class CanvasSizer {
  private pending: { width: number; height: number } | null = null;
  private readonly observer: ResizeObserver;
  private readonly canvas: HTMLCanvasElement;
  private readonly maxDimension: number;

  constructor(canvas: HTMLCanvasElement, maxDimension: number) {
    this.canvas = canvas;
    this.maxDimension = maxDimension;
    this.observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        // Prefer exact device-pixel sizes (avoids blurry rounding on fractional DPR).
        const dp = entry.devicePixelContentBoxSize?.[0];
        if (dp) {
          this.pending = { width: dp.inlineSize, height: dp.blockSize };
        } else {
          const box = entry.contentBoxSize[0];
          const dpr = window.devicePixelRatio || 1;
          const w = box ? box.inlineSize : canvas.clientWidth;
          const h = box ? box.blockSize : canvas.clientHeight;
          this.pending = { width: Math.round(w * dpr), height: Math.round(h * dpr) };
        }
      }
    });
    try {
      this.observer.observe(canvas, { box: "device-pixel-content-box" });
    } catch {
      this.observer.observe(canvas);
    }
    const dpr = window.devicePixelRatio || 1;
    this.pending = { width: Math.round(canvas.clientWidth * dpr), height: Math.round(canvas.clientHeight * dpr) };
  }

  /** Applies a pending resize; returns true when the backing size changed. Call once per frame. */
  update(): boolean {
    if (!this.pending) return false;
    const width = clamp(this.pending.width, 1, this.maxDimension);
    const height = clamp(this.pending.height, 1, this.maxDimension);
    this.pending = null;
    if (this.canvas.width === width && this.canvas.height === height) return false;
    this.canvas.width = width;
    this.canvas.height = height;
    return true;
  }

  get aspect(): number {
    return this.canvas.width / Math.max(1, this.canvas.height);
  }

  dispose(): void {
    this.observer.disconnect();
  }
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, Math.floor(v)));
}
