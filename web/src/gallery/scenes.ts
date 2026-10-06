// The visual gallery's fixed scenes and the GPU frame budget (docs/visual-quality-plan.md
// §3, VQ8). Pure data and maths: imported by the in-page gallery
// (gallery.ts) and by tools/browser-smoke/gallery.mjs (Node strips the types).

import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { SceneDecor } from "./decor.ts";
import { LEVEL_SCENES } from "./levelScenes.ts";
import { MOOD_SCENES, type WorldSceneKind } from "./moodScenes.ts";
import { EVENT_SCENES, type EventSceneSpec } from "./eventScenes.ts";
import type { ModeSceneKind } from "./modeScenes.ts";
import { VISITOR_SCENES, type VisitorSceneKind } from "./visitorScenes.ts";
import { SPECTACLE_SCENES, type SpectacleSceneKind } from "./spectacleScenes.ts";
import { ARCHITECT_SCENES, type ArchitectSceneKind } from "./architectSceneList.ts";

/**
 * World of a scene: the share code is `randomShareCode(seed, 0, enclosure,
 * density)`, or with any of the optional sliders or an archetype
 * `worldShareCode(seed, 0, sliders, archetype)` (the same seed; the other
 * sliders at their defaults).
 */
export interface GalleryWorld {
  seed: number;
  enclosure: number;
  density: number;
  verticality?: number;
  gravityChaos?: number;
  palette?: number;
  /** Level archetype (ls-math `Archetype` index): 0 classic (default), 1 asteroid belt. */
  archetype?: number;
}

/** The seven sliders of a scene world in share-code order (ls-worldgen defaults for the rest). */
export function worldSliders(w: GalleryWorld): number[] {
  return [w.enclosure, w.density, w.verticality ?? 50, w.gravityChaos ?? 0, w.palette ?? 0, 20, 30];
}

/** Whether the world needs `worldShareCode` (sliders beyond Enclosure and Density, or a belt). */
export function needsWorldCode(w: GalleryWorld): boolean {
  return w.verticality !== undefined || w.gravityChaos !== undefined || w.palette !== undefined || (w.archetype ?? 0) !== 0;
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
 * an opening (shadePose.ts, VQ9; falls back to `tunnel`); `corner` walks
 * toward a rock edge or corner 4-16 u away (two neighbouring probes whose
 * faces turn by 60° or more, raycast.ts pickCorner) and looks at it from
 * 7 u (falls back to `tunnel`). Level scenes
 * (levelScenes.ts, M4 A1): `well` inside a gravity well's hollow facing its
 * core, `belt` over the asteroid belt's outer edge, `slot` behind a belt
 * start slot looking prograde, `centre` across the level box at mid
 * height, `rise` up the box from near its floor (the first three fall back
 * to `vista` without the level's sites).
 */
export type PoseKind = "closeup" | "tunnel" | "vista" | "border" | "sun" | "planet" | "shade" | "sunwall" | "corner" | "well" | "belt" | "slot" | "centre" | "rise";

/**
 * What is placed in front of the camera besides terrain. `fit` carves a
 * fixed test shape (VQ1): a rock ball ahead with a dead-end tunnel of
 * FIT_TUNNEL_RADIUS bored along the view. `charge` (VQP) shows ships
 * charging the Fusion Charger (with `ship`: the own charge at the nose),
 * `cloak` cloaked ships bending the scene (vqpScenes.ts), `paperweight` a
 * landed Paperweight mid-air hauling debris and a ship in while the next
 * one flies (weaponScenes.ts, M4 B7), `paperweight-wall` the same with the
 * landed stone on the rock ahead.
 */
export type SceneContent = "none" | "lineup" | "fight" | "lights" | "explosion" | "stress" | "fit" | "charge" | "cloak" | "mode" | "shadow" | "paperweight" | "paperweight-wall" | "world" | "event" | "visitor" | "architect";

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
   * A first-impression page over the live backdrop (attract/galleryPages.ts):
   * the start screen, the lobby card or the in-game ship bay; `time` is how
   * long the backdrop flies before the shot. Pose and content do not apply.
   */
  page?: "start" | "lobby" | "bay" | "workshop" | "results";
  /** "workshop" page: the world in the workshop (and behind it). */
  workshop?: WorkshopScene;
  /** "results" page: the sample match's results, with the comedy events' titles (M4 B3: the Hat Baron) or without. */
  results?: { awards: boolean; teams?: boolean; survey?: boolean; architect?: boolean };
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
  /** "architect" content: which Architect view (M4 C4, architectSceneList.ts / architectScenes.ts). */
  architect?: ArchitectSceneKind;
  /** "world" content: which world visual (M4 B5, moodScenes.ts / worldScenes.ts). */
  worldFx?: WorldSceneKind;
  /** "event" content: a comedy event's frame (M4 C1, eventScenes.ts / eventScenesRun.ts). */
  eventScene?: EventSceneSpec;
  /** "visitor" content: which visitor event and frame of its strip (M4 C2, visitorScenes.ts / visitorRun.ts). */
  visitor?: VisitorSceneKind;
  /** "event" content: which spectacle event frame (M4 C3, spectacleScenes.ts / spectacleRun.ts). */
  spectacle?: SpectacleSceneKind;
  /**
   * A fixed camera instead of the picked pose, for scenes whose before and
   * after shots must match while the meshes (and so the picked pose) change
   * (M4 A2 `mine-corner`: picked once with `pose`, then pinned).
   */
  camera?: { position: Vec3; orientation: Quat };
  /** Ship types (lineup) drawn with their colour kept through the monochrome ink (pigmented, M4 A7 keep mask). */
  keepShips?: readonly number[];
  /** An announcer caption over the frame (M4 A6, ui/announcer.ts): voice pack and line trigger. */
  caption?: { pack: "tired" | "overexcited" | "passiveAggressive"; trigger: string };
}

/** A world made in the workshop (ui/workshop.ts) for the "workshop" gallery pages (M4 A8). */
export interface WorkshopScene {
  /** Seed as a decimal u64. */
  seed: string;
  /** The seven sliders in share-code order. */
  sliders: readonly number[];
  archetype: "classic" | "belt";
  /** Show the host box open. */
  host?: boolean;
}

const CAVES: GalleryWorld = { seed: 7, enclosure: 50, density: 50 };
/** Caverns just open enough to show the sky (Enclosure 47): the sun gets in through openings (VQ9). */
const SUNLIT_CAVES: GalleryWorld = { seed: 7, enclosure: 47, density: 60 };
/** The start screen's first backdrop world (attract/backdrop.ts BACKDROP_WORLDS[0]). */
const BACKDROP: GalleryWorld = { seed: 6, enclosure: 35, density: 65 };

/** M4 A7: the Palette steps (§3.3 of milestone-4-plan.md) on caves (with the four hulls), mines and open sky. */
const PALETTE_STEPS = [0, 40, 65, 85, 100] as const;

function paletteScenes(): GalleryScene[] {
  const out: GalleryScene[] = [];
  for (const p of PALETTE_STEPS) {
    out.push({ id: `palette-${p}`, title: `Palette ${p}: caves with the four hulls`, covers: "palette, ink", world: { ...CAVES, palette: p }, pose: "tunnel", content: "lineup", time: 1.0 });
  }
  for (const p of PALETTE_STEPS) {
    out.push({ id: `palette-${p}-mine`, title: `Palette ${p}: mine shaft`, covers: "palette, ink", world: { seed: 7, enclosure: 100, density: 30, palette: p }, pose: "tunnel", content: "none", time: 1.0 });
  }
  for (const p of PALETTE_STEPS) {
    out.push({ id: `palette-${p}-sky`, title: `Palette ${p}: open sky with islands`, covers: "palette, ink", world: { seed: 7, enclosure: 20, density: 70, palette: p }, pose: "vista", content: "none", time: 1.0 });
  }
  out.push({
    id: "palette-100-paint",
    title: "Palette 100: paint keeps its colour in the ink world",
    covers: "palette, ink, paint",
    world: { ...CAVES, palette: 100 },
    pose: "closeup",
    content: "none",
    decor: "paint",
    time: 1.0,
  });
  out.push({
    id: "palette-100-pigment",
    title: "Palette 100: Skitter and Hauler pigmented (colour kept), Tinker and Tug grey",
    covers: "palette, ink, ships",
    world: { ...CAVES, palette: 100 },
    pose: "tunnel",
    content: "lineup",
    time: 1.0,
    keepShips: [0, 2],
  });
  out.push({
    id: "palette-85-fight",
    title: "Palette 85: fight frame with an explosion in the ink world",
    covers: "palette, ink, effects",
    world: { ...CAVES, palette: 85 },
    pose: "tunnel",
    content: "fight",
    time: 2.15,
  });
  return out;
}

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
  {
    id: "mine-corner",
    title: "Mine shaft corners and edges up close (M4 A2: dual-contouring placement)",
    covers: "textures, polygon switches",
    world: { seed: 7, enclosure: 100, density: 30 },
    pose: "corner",
    // Picked with the `corner` pose on the A2 meshes, then pinned so the
    // before shots (older meshes, another pick) show the same rock.
    camera: {
      position: { x: 33.10193870017096, y: 67.85334618982269, z: 201.7128839405682 },
      orientation: { x: 0.010131473546369701, y: 0.9588073227240413, z: 0.034474733313723964, w: -0.2817753784443956 },
    },
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
  // M4 A6: the announcer caption, one voice pack each, over the fight frame from the cockpit.
  { id: "announcer-tired", title: "Announcer: Doug (tired) announces an event", covers: "announcer, M4 A6", world: CAVES, pose: "tunnel", content: "fight", time: 2.15, ship: 1, caption: { pack: "tired", trigger: "event.tax_drone" } },
  { id: "announcer-overexcited", title: "Announcer: Sparky (overexcited) calls first blood", covers: "announcer, M4 A6", world: CAVES, pose: "tunnel", content: "fight", time: 2.15, ship: 1, caption: { pack: "overexcited", trigger: "firstBlood" } },
  { id: "announcer-passive", title: "Announcer: Brenda (passive-aggressive) on a triple kill", covers: "announcer, M4 A6", world: CAVES, pose: "tunnel", content: "fight", time: 2.15, ship: 1, caption: { pack: "passiveAggressive", trigger: "multiKill3" } },
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
  {
    id: "ship-shadow",
    title: "Hulls in the sun cast shadows on the rock and on each other (D5)",
    covers: "ships, lighting",
    world: SUNLIT_CAVES,
    pose: "sunwall",
    content: "shadow",
    time: 1.0,
  },
  { id: "charge-up", title: "Fusion Charger charge-up: full, 45 %, overcharge", covers: "effects", world: CAVES, pose: "tunnel", content: "charge", time: 3.0 },
  { id: "charge-cockpit", title: "Own Fusion Charger charging (75 %) from the cockpit", covers: "effects", world: CAVES, pose: "tunnel", content: "charge", time: 3.0, ship: 1 },
  { id: "cloak", title: "Cloaked ships bending the rock behind them", covers: "effects, ships", world: CAVES, pose: "tunnel", content: "cloak", time: 1.0 },
  // M4 B7: the Paperweight, flying and landed (a gravity well).
  { id: "paperweight", title: "Paperweight: one landed mid-air hauling debris and a Tinker in, the next one tumbling", covers: "weapons, effects", world: { seed: 7, enclosure: 20, density: 70 }, pose: "vista", content: "paperweight", time: 2.0 },
  { id: "paperweight-wall", title: "Paperweight stuck on the rock ahead, a gravity well for 6 s", covers: "weapons, effects", world: CAVES, pose: "closeup", content: "paperweight-wall", time: 2.0 },
  // M3 C1 mode visuals.
  { id: "squeeze-warn", title: "The Squeeze: walls glowing before a close, next zone", covers: "modes, effects", world: { seed: 7, enclosure: 20, density: 70 }, pose: "vista", content: "mode", mode: "squeeze-warn", time: 4.0 },
  { id: "squeeze-close", title: "The Squeeze: walls grinding inward, a ship scraping", covers: "modes, effects", world: { seed: 7, enclosure: 20, density: 70 }, pose: "vista", content: "mode", mode: "squeeze-close", time: 4.0 },
  { id: "heist-carrier", title: "Heist: core carrier and extraction zones", covers: "modes, effects", world: { seed: 7, enclosure: 100, density: 30 }, pose: "tunnel", content: "mode", mode: "heist-carrier", time: 4.0 },
  { id: "heist-dig", title: "Heist: the embedded core marked on the rock", covers: "modes, effects", world: CAVES, pose: "closeup", content: "mode", mode: "heist-dig", time: 4.0 },
  { id: "reactor", title: "Hot Reactor: holder, fuse radius and ring", covers: "modes, effects", world: CAVES, pose: "vista", content: "mode", mode: "reactor", time: 4.0 },
  { id: "last-color", title: "Last Color Standing: monochrome world, colour meters on hulls", covers: "modes, ships", world: CAVES, pose: "tunnel", content: "mode", mode: "last-color", time: 4.0 },
  { id: "last-color-cockpit", title: "Last Color Standing from the cockpit: the own frame keeps its colour (D5)", covers: "modes, ships", world: CAVES, pose: "tunnel", content: "mode", mode: "last-color", time: 4.0, ship: 1 },
  { id: "teams4", title: "Four team colours: liveries and paint", covers: "modes, ships, paint", world: CAVES, pose: "closeup", content: "mode", mode: "teams4", decor: "teams4", time: 4.0 },
  // First-impression pass (FI): the pages over the live flythrough (caverns open to the sky).
  { id: "start-screen", title: "Start screen over the live flythrough", covers: "first impression", world: BACKDROP, pose: "tunnel", content: "none", time: 9.0, page: "start" },
  { id: "start-flythrough", title: "Start flythrough, later in the flight", covers: "first impression", world: BACKDROP, pose: "tunnel", content: "none", time: 24.0, page: "start" },
  { id: "lobby-flythrough", title: "Lobby card over the cinematic flight", covers: "first impression", world: BACKDROP, pose: "tunnel", content: "none", time: 16.0, page: "lobby" },
  { id: "ship-bay", title: "Ship bay: the four hulls turning", covers: "first impression, ships", world: BACKDROP, pose: "tunnel", content: "none", time: 6.0, page: "bay" },
  // M4 A1: level shapes, gravity wells and the asteroid belt (generator version 4).
  ...LEVEL_SCENES,
  ...paletteScenes(),
  // M4 B5: wells, cave-ins, pigment, moods and the Bored level turn.
  ...MOOD_SCENES,
  // M4 C1-C3: the comedy events' strips (announce, peak, aftermath).
  ...EVENT_SCENES,
  // M4 C2: the visitor events, three frames each (tell, peak, aftermath).
  ...VISITOR_SCENES,
  // M4 C3: the spectacle events' strips (announce, peak, aftermath) and the hat sheet.
  ...SPECTACLE_SCENES,
  // M4 C4: The Architect (map view, strokes, exit, results).
  ...ARCHITECT_SCENES,
  // World workshop (M4 A8): the workshop page over the preview of the world it makes.
  { id: "workshop", title: "World workshop over its preview", covers: "world workshop", world: BACKDROP, pose: "tunnel", content: "none", time: 8.0, page: "workshop", workshop: { seed: "6", sliders: [38, 60, 50, 30, 0, 40, 30], archetype: "classic" } },
  { id: "workshop-tall", title: "World workshop: a tall world (Verticality 90)", covers: "world workshop", world: BACKDROP, pose: "tunnel", content: "none", time: 8.0, page: "workshop", workshop: { seed: "5150", sliders: [55, 45, 90, 0, 20, 60, 50], archetype: "classic" } },
  // Results screen (M4 B3): the Hat Baron's badge under the headline, and the same table without titles.
  { id: "results", title: "Results: free-for-all, no event titles", covers: "results, M4 B3", world: BACKDROP, pose: "tunnel", content: "none", time: 12.0, page: "results", results: { awards: false } },
  { id: "results-hat-baron", title: "Results: the Hat Baron (Hat rain)", covers: "results, Hat rain, M4 B3", world: BACKDROP, pose: "tunnel", content: "none", time: 12.0, page: "results", results: { awards: true } },
  { id: "results-survey", title: "Results: the Hat Baron and the leader's Customer rating (two stars)", covers: "results, Customer survey, M4 C2", world: BACKDROP, pose: "tunnel", content: "none", time: 12.0, page: "results", results: { awards: true, survey: true } },
  { id: "results-hat-baron-teams", title: "Results: the Hat Baron in a team match, the own pilot", covers: "results, Hat rain, M4 B3", world: BACKDROP, pose: "tunnel", content: "none", time: 12.0, page: "results", results: { awards: true, teams: true } },
  { id: "results-architect", title: "Results: The Architect, points as architect and as runner, the three rounds", covers: "results, The Architect, M4 C4", world: BACKDROP, pose: "tunnel", content: "none", time: 12.0, page: "results", results: { awards: false, architect: true } },
  { id: "workshop-belt", title: "World workshop: an asteroid belt, hosting as Belt Up", covers: "world workshop, Belt Up", world: BACKDROP, pose: "tunnel", content: "none", time: 8.0, page: "workshop", workshop: { seed: "77", sliders: [50, 60, 50, 30, 0, 30, 20], archetype: "belt", host: true } },
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
  // M4 A7: the Palette slider's ink lines (outlines and hatching; only from Palette 50).
  ink: 0.8,
  // M4 B5: gravity wells (lensing, glow, core stone), cave-in cracks and rubble (wells.ts; only the rectangle they cover).
  wells: 0.4,
  // FI: the hull view behind the ship cards (attract/shipBay.ts; its own small canvas, "medium").
  bay: 0.8,
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
