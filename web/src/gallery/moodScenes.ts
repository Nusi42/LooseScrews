// Gallery scenes of M4 B5's world visuals (docs/milestone-4-plan.md §5 wave
// B, "B5"): gravity wells close up, with debris and lensing; a cave-in's
// warning and its rubble; pigment pickups and pigmented against grey ships
// in a Palette 90 ink level; the four moods; and three frames of the Bored
// level turning the sky. Pure data (imported by scenes.ts, also from Node);
// gallery/worldScenes.ts builds them in the page. Every scene renders
// without the B5 visuals with `--query worldfx=off` (the before shots).

import type { GalleryScene, GalleryWorld } from "./scenes.ts";

/** What a world scene shows (gallery/worldScenes.ts). */
export type WorldSceneKind =
  | "wells-close"
  | "wells-debris"
  | "well-lensing"
  | "cave-in-warning"
  | "rubble"
  | "pigment-pickup"
  | "pigmented-vs-grey"
  | "mood-calm"
  | "mood-bored"
  | "mood-grumpy"
  | "mood-giddy"
  | "bored-turn-a"
  | "bored-turn-b"
  | "bored-turn-c";

const WELL_CAVES: GalleryWorld = { seed: 7, enclosure: 50, density: 50, gravityChaos: 100 };
const WELL_SKY: GalleryWorld = { seed: 7, enclosure: 10, density: 60, gravityChaos: 100 };
/** Mine shafts: flat ceilings, where a cave-in's cracks and rubble read best. */
const MINES: GalleryWorld = { seed: 7, enclosure: 100, density: 30 };
const INK: GalleryWorld = { seed: 7, enclosure: 50, density: 50, palette: 90 };
/** Caverns open to the sky (the sun gets in; the moods show on sunlit rock and in the motes). */
const SUNLIT: GalleryWorld = { seed: 7, enclosure: 47, density: 60 };
/** Open sky with islands, the sun and the planet: the Bored level turn. */
const TURN_WORLD: GalleryWorld = { seed: 11, enclosure: 20, density: 55 };

/** Seconds the Bored level turn takes (data/events/bored_level.ron: 5 s). */
export const BORED_TURN_SECS = 5;

/** Progress (0..1) of the three Bored level frames. */
export const BORED_TURN_FRAMES: Readonly<Record<string, number>> = { "bored-turn-a": 0.08, "bored-turn-b": 0.5, "bored-turn-c": 1 };

const scene = (id: WorldSceneKind, title: string, world: GalleryWorld, pose: GalleryScene["pose"], extra: Partial<GalleryScene> = {}): GalleryScene => ({
  id,
  title,
  covers: "world visuals (M4 B5)",
  world,
  pose,
  content: "world",
  worldFx: id,
  time: 4.0,
  ...extra,
});

export const MOOD_SCENES: readonly GalleryScene[] = [
  scene("wells-close", "Gravity well up close from the cockpit: core stone with violet veins, accretion swirl, lensed hollow, pull indicator", WELL_CAVES, "well", { ship: 1 }),
  scene("wells-debris", "Gravity well in open sky with debris caught in its swirl", WELL_SKY, "well"),
  scene("well-lensing", "Gravity well bending the stars and islands behind it (lensing, photon ring)", WELL_SKY, "well"),
  scene("cave-in-warning", "Cave-in warning: cracks glowing in the ceiling, dust and pebbles trickling (1 s into the 1.5 s warning)", MINES, "tunnel"),
  scene("rubble", "Cave-in aftermath: rubble down from the ceiling, dust settling, a pale dust coat", MINES, "tunnel"),
  scene("pigment-pickup", "Palette 90: a pigment pickup (three primaries) and a splash in a side's colour, in colour through the ink", INK, "tunnel"),
  scene("pigmented-vs-grey", "Palette 90: two pigmented ships in colour (dripping paint) against two grey ones; name tags", INK, "tunnel"),
  scene("mood-calm", "Mood: calm (cool clear grade, pale blue motes, HUD badge)", SUNLIT, "shade"),
  scene("mood-bored", "Mood: bored (washed-out flat grade, dim grey motes)", SUNLIT, "shade"),
  scene("mood-grumpy", "Mood: grumpy (dark warm hard grade, ember motes)", SUNLIT, "shade"),
  scene("mood-giddy", "Mood: giddy (warm saturated grade, rainbow motes)", SUNLIT, "shade"),
  scene("bored-turn-a", "Bored level, the turn starting: sky, sun and shadows as they were", TURN_WORLD, "planet"),
  scene("bored-turn-b", "Bored level halfway: the sky, sun and planet have turned 45 degrees", TURN_WORLD, "planet"),
  scene("bored-turn-c", "Bored level done: up is sideways, the sun lights the islands from a new side", TURN_WORLD, "planet"),
];
