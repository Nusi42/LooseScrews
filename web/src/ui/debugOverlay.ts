// Debug overlay (toggle with F3): frame rate, ping, transport, prediction
// corrections, predicted shots and terrain chunk streaming.

import type { DebugStats } from "../game/stats.ts";

/** Frames per second over a sliding window of frame timestamps. */
export class FpsMeter {
  private readonly windowMs: number;
  private readonly times: number[] = [];

  constructor(windowMs = 1000) {
    this.windowMs = windowMs;
  }

  /** Records a frame at `now` (ms) and returns the current rate. */
  tick(now: number): number {
    this.times.push(now);
    while (this.times.length > 2 && (this.times[0] ?? now) < now - this.windowMs) this.times.shift();
    return this.fps;
  }

  get fps(): number {
    const n = this.times.length;
    if (n < 2) return 0;
    const span = (this.times[n - 1] ?? 0) - (this.times[0] ?? 0);
    return span > 0 ? ((n - 1) * 1000) / span : 0;
  }

  /** Longest frame in the window (ms). */
  get worstFrameMs(): number {
    let worst = 0;
    for (let i = 1; i < this.times.length; i++) worst = Math.max(worst, (this.times[i] ?? 0) - (this.times[i - 1] ?? 0));
    return worst;
  }
}

export interface DebugInfo {
  fps: number;
  worstFrameMs: number;
  mode: "online" | "practice";
  /** Active transport ("WebSocket", "WebTransport", …); null offline. */
  transport: string | null;
  /** Packets skipped under send backpressure, if the transport counts them. */
  droppedSends: number | null;
  stats: DebugStats;
  chunks: { meshedTotal: number; pending: number; shown: number; drawn: number; triangles: number };
  shareCode: string;
  speed: number;
  gpuError: string | null;
}

/** The overlay's text lines. */
export function formatDebugLines(d: DebugInfo): string[] {
  const s = d.stats;
  const lines = [`${d.fps.toFixed(0)} fps · worst frame ${d.worstFrameMs.toFixed(1)} ms`];
  if (d.mode === "online") {
    const ping = s.pingMs === null ? "—" : `${Math.round(s.pingMs)} ms`;
    const dropped = d.droppedSends ? ` · ${d.droppedSends} sends skipped` : "";
    lines.push(`ping ${ping} · ${d.transport ?? "?"}${s.synced ? "" : " · syncing"}${dropped}`);
    lines.push(
      `prediction: ${s.corrected} corrections · ${s.snapped} snaps · ${s.accepted} ok` +
        (s.corrected + s.snapped > 0 ? ` · last error ${s.lastError.toFixed(2)} u` : ""),
    );
    lines.push(
      `shots: ${s.shotsPredicted} predicted · ${s.shotsConfirmed} confirmed · ${s.shotsDropped} dropped` +
        (s.shotsUnpredicted > 0 ? ` · ${s.shotsUnpredicted} unpredicted` : ""),
    );
  } else {
    lines.push("practice (offline)");
  }
  const c = d.chunks;
  lines.push(
    `chunks: ${c.meshedTotal} meshed · ${c.pending} pending · ${c.drawn}/${c.shown} drawn · ${Math.round(c.triangles / 1000)}k tris`,
  );
  lines.push(`${d.speed.toFixed(1)} u/s · ${d.shareCode}`);
  if (d.gpuError) lines.push(`GPU error: ${d.gpuError}`);
  return lines;
}

export class DebugOverlay {
  readonly root: HTMLElement;
  private shown = false;
  private last = "";

  constructor(parent: HTMLElement) {
    this.root = document.createElement("pre");
    this.root.className = "debug-overlay hidden";
    parent.appendChild(this.root);
  }

  get visible(): boolean {
    return this.shown;
  }

  toggle(): void {
    this.shown = !this.shown;
    this.root.classList.toggle("hidden", !this.shown);
  }

  update(lines: readonly string[]): void {
    const text = lines.join("\n");
    if (text === this.last) return;
    this.last = text;
    this.root.textContent = text;
  }

  dispose(): void {
    this.root.remove();
  }
}
