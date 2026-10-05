// The visual gallery's fixed scenes and the GPU frame budget (docs/visual-quality-plan.md
// §3, VQ8). Pure data and maths: imported by the in-page gallery
// (gallery.ts) and by tools/browser-smoke/gallery.mjs (Node strips the types).

/** World of a scene: the share code is `randomShareCode(seed, 0, enclosure, density)`. */
export interface GalleryWorld {
  seed: number;
  enclosure: number;
  density: number;
}

/**
 * How the camera pose is picked from the spawn point by CPU ray casts
 * against the chunk meshes (raycast.ts): `closeup` a rock wall about 4 u
 * ahead, `tunnel` the longest near-horizontal view, `vista` a far wall or
 * island 40-150 u away.
 */
export type PoseKind = "closeup" | "tunnel" | "vista";

/** What is placed in front of the camera besides terrain. */
export type SceneContent = "none" | "lineup" | "fight" | "explosion" | "stress";

export interface GalleryScene {
  id: string;
  title: string;
  /** Which review finding (visual-quality-plan.md "Acceptance (whole track)") the shot answers. */
  covers: string;
  world: GalleryWorld;
  pose: PoseKind;
  content: SceneContent;
  /** Shader and effect time (s) the frame is rendered at. */
  time: number;
}

const CAVES: GalleryWorld = { seed: 7, enclosure: 50, density: 50 };

export const SCENES: readonly GalleryScene[] = [
  { id: "cave-closeup", title: "Cave wall close-up", covers: "textures, polygon switches", world: CAVES, pose: "closeup", content: "none", time: 1.0 },
  { id: "cave-tunnel", title: "Cave tunnel", covers: "textures, lighting", world: CAVES, pose: "tunnel", content: "none", time: 1.0 },
  {
    id: "open-sky",
    title: "Open sky with islands",
    covers: "backdrop, fog",
    world: { seed: 7, enclosure: 20, density: 70 },
    pose: "vista",
    content: "none",
    time: 1.0,
  },
  {
    id: "mine-shaft",
    title: "Mine shaft",
    covers: "textures, lighting",
    world: { seed: 7, enclosure: 100, density: 30 },
    pose: "tunnel",
    content: "none",
    time: 1.0,
  },
  { id: "ship-lineup", title: "The four hulls", covers: "ships", world: CAVES, pose: "tunnel", content: "lineup", time: 1.0 },
  { id: "fight", title: "Fight frame with an explosion", covers: "effects", world: CAVES, pose: "tunnel", content: "fight", time: 2.15 },
  {
    id: "explosion",
    title: "A ship blowing up, a burning ship, smoke from a rocket hit",
    covers: "effects",
    world: CAVES,
    pose: "tunnel",
    content: "explosion",
    time: 3.0,
  },
  {
    id: "particles-stress",
    title: "Particle budget: about 20 k live particles (an explosion every frame, 120 bolts)",
    covers: "effects (GPU budget)",
    world: CAVES,
    pose: "tunnel",
    content: "stress",
    time: 3.0,
  },
];

export function sceneById(id: string): GalleryScene | undefined {
  return SCENES.find((s) => s.id === id);
}

/**
 * GPU budget per timed pass at 1080p "high" on the reference mid GPU (GTX
 * 1660 / RX 6600 / M1 class), ms. MSAA's 0.8 ms is split between the two
 * scene passes it multiplies (terrain 2.0 + 0.4, objects 0.5 + 0.4).
 */
export const PASS_BUDGETS_MS: Readonly<Record<string, number>> = {
  terrain: 2.4,
  objects: 0.9,
  particles: 1.0,
  lights: 1.0,
  sky: 0.6,
  post: 1.2,
};

/** Whole frame (sum of the passes) at "high" on the reference mid GPU. */
export const FRAME_BUDGET_MS = 8;

/** A pass fails the check when it exceeds its share by more than this factor. */
export const BUDGET_TOLERANCE = 1.25;

export interface BudgetResult {
  ok: boolean;
  /** One line per pass over budget ("terrain 3.20 ms > 3.00 ms (2.40 x 1.25)"). */
  failures: string[];
}

/**
 * Checks per-pass GPU times (ms) against PASS_BUDGETS_MS x BUDGET_TOLERANCE,
 * and the sum against FRAME_BUDGET_MS x BUDGET_TOLERANCE. `scale` stretches
 * every budget for a GPU slower or faster than the reference (1.75 for the
 * integrated "medium" target: 14 ms / 8 ms). Passes without a budget only
 * count toward the frame.
 */
export function checkBudget(passes: ReadonlyArray<readonly [string, number]>, scale = 1): BudgetResult {
  const failures: string[] = [];
  let total = 0;
  for (const [label, ms] of passes) {
    total += ms;
    const share = PASS_BUDGETS_MS[label];
    if (share === undefined) continue;
    const limit = share * scale * BUDGET_TOLERANCE;
    if (ms > limit) failures.push(`${label} ${ms.toFixed(2)} ms > ${limit.toFixed(2)} ms (${share} x ${scale} x ${BUDGET_TOLERANCE})`);
  }
  const frameLimit = FRAME_BUDGET_MS * scale * BUDGET_TOLERANCE;
  if (total > frameLimit) failures.push(`frame ${total.toFixed(2)} ms > ${frameLimit.toFixed(2)} ms`);
  return { ok: failures.length === 0, failures };
}
