// The visual gallery's fixed scenes and the GPU frame budget (docs/visual-quality-plan.md
// §3, VQ8). Pure data and maths: imported by the in-page gallery
// (gallery.ts) and by tools/browser-smoke/gallery.mjs (Node strips the types).

import type { SceneDecor } from "./decor.ts";
import type { ModeSceneKind } from "./modeScenes.ts";

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
 * island 40-150 u away, `border` a close-up of a rock material border
 * (decor.ts pickBorder; falls back to `closeup`); `sun` and `planet` look
 * from the spawn point toward the world's sun or planet
 * (renderer/atmosphere.ts, VQ7), the target a little off the screen centre;
 * `shade` stands in rock's shadow and looks at rock the sun reaches through
 * an opening (shadePose.ts, VQ9; falls back to `tunnel`).
 */
export type PoseKind = "closeup" | "tunnel" | "vista" | "border" | "sun" | "planet" | "shade";

/**
 * What is placed in front of the camera besides terrain. `fit` carves a
 * fixed test shape (VQ1): a rock ball ahead with a dead-end tunnel of
 * FIT_TUNNEL_RADIUS bored along the view. `charge` (VQP) shows ships
 * charging the Fusion Charger (with `ship`: the own charge at the nose),
 * `cloak` cloaked ships bending the scene (vqpScenes.ts).
 */
export type SceneContent = "none" | "lineup" | "fight" | "lights" | "explosion" | "stress" | "fit" | "charge" | "cloak" | "mode";

/** Radius of the `fit` scene's tunnel (u): 1.2 u across. */
export const FIT_TUNNEL_RADIUS = 0.6;

export interface GalleryScene {
  id: string;
  title: string;
  /** Which review finding (visual-quality-plan.md "Acceptance (whole track)") the shot answers. */
  covers: string;
  world: GalleryWorld;
  pose: PoseKind;
  content: SceneContent;
  /** Paint or surface states put on the rock the camera looks at (decor.ts); default none. */
  decor?: SceneDecor;
  /** Shader and effect time (s) the frame is rendered at. */
  time: number;
  /** "lights" content: number of glowing shots (each a dynamic light). */
  lights?: number;
  /**
   * Own ship type (ls-sim ShipType index) whose cockpit frame and fit ring
   * show (VQ1); absent = no cockpit, the plain camera.
   */
  ship?: number;
  /** Pitch (degrees, up positive) added to the picked camera pose; default 0. */
  pitch?: number;
  /** Open levels: the camera moves up to this distance (u) below the hidden ceiling of the room box. */
  belowCeiling?: number;
  /** "mode" content: which mode's visuals (M3 C1, modeScenes.ts). */
  mode?: ModeSceneKind;
}

const CAVES: GalleryWorld = { seed: 7, enclosure: 50, density: 50 };
/** Caverns just open enough to show the sky (Enclosure 47): the sun gets in through openings (VQ9). */
const SUNLIT_CAVES: GalleryWorld = { seed: 7, enclosure: 47, density: 60 };

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
    id: "open-sun",
    title: "Open sky toward the sun (light shafts)",
    covers: "backdrop, sky, light shafts",
    world: { seed: 7, enclosure: 20, density: 70 },
    pose: "sun",
    content: "none",
    time: 1.0,
  },
  {
    id: "open-planet",
    title: "Open space with the planet",
    covers: "backdrop, sky",
    world: { seed: 11, enclosure: 0, density: 40 },
    pose: "planet",
    content: "none",
    time: 1.0,
  },
  {
    id: "cave-opening",
    title: "Caverns opening to the sky",
    covers: "backdrop, fog, light shafts",
    world: { seed: 7, enclosure: 36, density: 50 },
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
  { id: "material-border", title: "Rock material border", covers: "textures, polygon switches", world: CAVES, pose: "border", content: "none", time: 1.0 },
  {
    id: "paint-edge",
    title: "Paint edges (plain, stripes, dots)",
    covers: "polygon switches",
    world: CAVES,
    pose: "closeup",
    content: "none",
    decor: "paint",
    time: 1.0,
  },
  {
    id: "surface-states",
    title: "Hot, charged, glowing and scorched rock",
    covers: "polygon switches, effects",
    world: CAVES,
    pose: "closeup",
    content: "none",
    decor: "states",
    time: 1.0,
  },
  { id: "ship-lineup", title: "The four hulls", covers: "ships", world: CAVES, pose: "tunnel", content: "lineup", time: 1.0 },
  { id: "fight", title: "Fight frame with an explosion", covers: "effects", world: CAVES, pose: "tunnel", content: "fight", time: 2.15 },
  {
    id: "light-volley",
    title: "Dynamic lights: a volley down a mine shaft",
    covers: "effects, lighting",
    world: { seed: 7, enclosure: 100, density: 30 },
    pose: "tunnel",
    content: "lights",
    time: 1.0,
    lights: 12,
  },
  {
    id: "light-storm",
    title: "64 dynamic lights (VQ6 budget)",
    covers: "effects, lighting",
    world: CAVES,
    pose: "tunnel",
    content: "lights",
    time: 1.0,
    lights: 61,
  },
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
  {
    id: "fit-tunnel",
    title: "Tinker before a 1.2 u tunnel (fits)",
    covers: "fit",
    world: CAVES,
    pose: "tunnel",
    content: "fit",
    time: 1.0,
    ship: 1,
  },
  {
    id: "fit-tunnel-hauler",
    title: "Hauler before a 1.2 u tunnel (does not fit)",
    covers: "fit",
    world: CAVES,
    pose: "tunnel",
    content: "fit",
    time: 1.0,
    ship: 2,
  },
  { id: "cockpit-skitter", title: "Skitter cockpit", covers: "fit, ships", world: CAVES, pose: "tunnel", content: "none", time: 1.0, ship: 0 },
  {
    id: "cave-sunbeam",
    title: "Sun through cave openings: shadowed rock, sunlit patches (VQ9)",
    covers: "lighting, sun occlusion",
    world: SUNLIT_CAVES,
    pose: "shade",
    content: "none",
    time: 1.0,
  },
  {
    id: "sun-shafts",
    title: "Volumetric sun shaft through a rock arch into a cavern (VQ9)",
    covers: "lighting, sun occlusion, atmosphere",
    world: { seed: 11, enclosure: 45, density: 60 },
    pose: "shade",
    content: "none",
    time: 1.0,
  },
  {
    id: "dark-cave-fight",
    title: "Fight in the cave's shade: shots, the explosion and engines light the rock (VQ9)",
    covers: "lighting, sun occlusion, effects",
    world: SUNLIT_CAVES,
    pose: "shade",
    content: "fight",
    time: 2.15,
  },
  { id: "cockpit-tug", title: "Tug cockpit", covers: "fit, ships", world: CAVES, pose: "tunnel", content: "none", time: 1.0, ship: 3 },
  { id: "cockpit-hauler", title: "Hauler cockpit", covers: "fit, ships", world: CAVES, pose: "tunnel", content: "none", time: 1.0, ship: 2 },
  { id: "cockpit-sky", title: "Tinker cockpit under the open sky", covers: "fit, ships, backdrop", world: { seed: 7, enclosure: 20, density: 70 }, pose: "vista", content: "none", time: 1.0, ship: 1 },
  {
    id: "open-pillars",
    title: "Open sky: pillars fading out before the hidden ceiling",
    covers: "backdrop",
    world: { seed: 7, enclosure: 20, density: 70 },
    pose: "vista",
    content: "none",
    time: 1.0,
    pitch: 14,
    belowCeiling: 16,
  },
  { id: "charge-up", title: "Fusion Charger charge-up: full, 45 %, overcharge", covers: "effects", world: CAVES, pose: "tunnel", content: "charge", time: 3.0 },
  { id: "charge-cockpit", title: "Own Fusion Charger charging (75 %) from the cockpit", covers: "effects", world: CAVES, pose: "tunnel", content: "charge", time: 3.0, ship: 1 },
  { id: "cloak", title: "Cloaked ships bending the rock behind them", covers: "effects, ships", world: CAVES, pose: "tunnel", content: "cloak", time: 1.0 },
  // M3 C1 mode visuals.
  { id: "squeeze-warn", title: "The Squeeze: walls glowing before a close, next zone", covers: "modes, effects", world: { seed: 7, enclosure: 20, density: 70 }, pose: "vista", content: "mode", mode: "squeeze-warn", time: 4.0 },
  { id: "squeeze-close", title: "The Squeeze: walls grinding inward, a ship scraping", covers: "modes, effects", world: { seed: 7, enclosure: 20, density: 70 }, pose: "vista", content: "mode", mode: "squeeze-close", time: 4.0 },
  { id: "heist-carrier", title: "Heist: core carrier and extraction zones", covers: "modes, effects", world: { seed: 7, enclosure: 100, density: 30 }, pose: "tunnel", content: "mode", mode: "heist-carrier", time: 4.0 },
  { id: "heist-dig", title: "Heist: the embedded core marked on the rock", covers: "modes, effects", world: CAVES, pose: "closeup", content: "mode", mode: "heist-dig", time: 4.0 },
  { id: "reactor", title: "Hot Reactor: holder, fuse radius and ring", covers: "modes, effects", world: CAVES, pose: "vista", content: "mode", mode: "reactor", time: 4.0 },
  { id: "last-color", title: "Last Color Standing: monochrome world, colour meters on hulls", covers: "modes, ships", world: CAVES, pose: "tunnel", content: "mode", mode: "last-color", time: 4.0 },
  { id: "teams4", title: "Four team colours: liveries and paint", covers: "modes, ships, paint", world: CAVES, pose: "closeup", content: "mode", mode: "teams4", decor: "teams4", time: 4.0 },
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
  // VQ9: shadow map updates (only on frames with a terrain change; a crater
  // redraws two chunks' rectangles, a full redraw happens once at load) and
  // the volumetric shafts (half resolution, 32 steps; skipped in sealed levels).
  shadow: 0.3,
  volumetric: 0.6,
  // M3 C1 mode effects (modeFx.ts): zones, beams and halos cost little; a Squeeze wall filling the screen is the worst case.
  mode: 1.0,
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
