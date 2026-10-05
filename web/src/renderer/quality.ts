// Render quality presets (docs/visual-quality-plan.md §3): one object that
// every visual package adds its switches to. The renderer starts at "high"
// and, unless `?quality=` forces a level, picks once after the first
// AUTO_SECONDS of GPU-timed frames: the median GPU frame time against the
// "high" budget (8 ms) and the "medium" budget (14 ms). Without
// `timestamp-query` it stays where it started (frame intervals would also
// count CPU work such as the initial terrain load).

import type { ParticleQuality } from "./particles.ts";
import type { VolumetricQuality } from "./volumetric.ts";

export type QualityLevel = "low" | "medium" | "high";

export interface RenderQuality {
  level: QualityLevel;
  /** MSAA sample count of the scene target (WebGPU supports 1 and 4). */
  msaa: 1 | 4;
  /** Bloom mip levels (0 = no bloom). */
  bloomMips: number;
  /** Terrain detail (VQ3b): macro, grain and vein samples on top of the rock layer (9 vs 3 texture samples). */
  terrainDetail: boolean;
  /** Dynamic point lights per frame (VQ6, renderer/lights.ts; at most MAX_LIGHTS = 128). */
  maxLights: number;
  /** Particle ring size, spawn density and dust motes (VQ5, particles.ts). */
  particles: ParticleQuality;
  /** Screen-space light shafts from the sun in open levels (VQ7). */
  shafts: boolean;
  /** Volumetric sun shafts through the shadow map (VQ9, volumetric.ts): march resolution divisor (0 = off) and steps. */
  volumetric: VolumetricQuality;
}

/**
 * "medium" keeps MSAA 4x: WebGPU has no 2x (the plan's medium MSAA 2x), so it
 * saves on bloom and lights (and, from later packages, particles and shafts).
 * "low" renders without MSAA and bloom but still tonemaps (the HDR target
 * is core WebGPU, so every adapter runs the same shaders).
 */
export const QUALITY_PRESETS: Readonly<Record<QualityLevel, RenderQuality>> = {
  high: { level: "high", msaa: 4, bloomMips: 6, terrainDetail: true, maxLights: 128, particles: { capacity: 32768, density: 1, motes: 1500, halfResPx: 32 }, shafts: true, volumetric: { divisor: 2, steps: 24 } },
  medium: { level: "medium", msaa: 4, bloomMips: 4, terrainDetail: true, maxLights: 32, particles: { capacity: 16384, density: 0.6, motes: 800, halfResPx: 20 }, shafts: false, volumetric: { divisor: 3, steps: 16 } },
  low: { level: "low", msaa: 1, bloomMips: 0, terrainDetail: false, maxLights: 16, particles: { capacity: 8192, density: 0.35, motes: 0, halfResPx: 12 }, shafts: false, volumetric: { divisor: 0, steps: 0 } },
};

/** GPU frame budgets (ms) of "high" and "medium" (plan §3). */
export const HIGH_BUDGET_MS = 8;
export const MEDIUM_BUDGET_MS = 14;
/** Seconds of GPU-timed frames before the automatic pick. */
export const AUTO_SECONDS = 5;

/** `?quality=low|medium|high`, or null when absent or unknown. */
export function qualityFromSearch(search: string): QualityLevel | null {
  const q = new URLSearchParams(search).get("quality");
  return q === "low" || q === "medium" || q === "high" ? q : null;
}

/** The level whose GPU budget a frame time (ms, measured at "high") fits. */
export function levelForGpuTime(ms: number): QualityLevel {
  if (ms <= HIGH_BUDGET_MS) return "high";
  if (ms <= MEDIUM_BUDGET_MS) return "medium";
  return "low";
}

function median(xs: readonly number[]): number {
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? (s[m] ?? 0) : ((s[m - 1] ?? 0) + (s[m] ?? 0)) / 2;
}

/** One-shot automatic quality pick from GPU frame times. */
export class QualityAuto {
  private readonly samples: number[] = [];
  private start: number | null = null;
  private done = false;

  /** `forced`: a `?quality=` level; the pick is then skipped. */
  constructor(forced: QualityLevel | null = null) {
    this.done = forced !== null;
  }

  get decided(): boolean {
    return this.done;
  }

  /**
   * Feeds the latest GPU frame time (ms; null when unknown) at `now` (ms).
   * Returns the picked level once, after AUTO_SECONDS of samples; null
   * otherwise. Never picks a level above `current` (only steps down).
   */
  sample(gpuMs: number | null, now: number, current: QualityLevel): QualityLevel | null {
    if (this.done || gpuMs === null) return null;
    this.start ??= now;
    this.samples.push(gpuMs);
    if (now - this.start < AUTO_SECONDS * 1000 || this.samples.length < 30) return null;
    this.done = true;
    const picked = levelForGpuTime(median(this.samples));
    const order: QualityLevel[] = ["low", "medium", "high"];
    return order.indexOf(picked) < order.indexOf(current) ? picked : current;
  }
}
