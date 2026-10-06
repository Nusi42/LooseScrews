// Belt Up gallery scenes (M4 C7, docs/milestone-4-plan.md §5 wave C "C7"):
// the start lights at a slot, racing inside the belt, skimming its surface,
// the open line in the radiation, the dive to the exit, the exit portal and
// sign, the whole ring from above, and the belt in a Palette 90 ink level.
// Pure data (imported by scenes.ts, also from Node); gallery/raceSceneRun.ts
// builds them in the page. `?racefx=off` draws the level without the race
// visuals (the before shots).

import type { GalleryScene, GalleryWorld } from "./scenes.ts";

/** What a race scene shows. */
export type RaceSceneKind = "start" | "inside" | "skim" | "open" | "dive" | "exit" | "results" | "overview" | "ink";

/** The belt every race scene flies (the A1 belt scenes' world). */
export const RACE_WORLD: GalleryWorld = { seed: 21, enclosure: 50, density: 60, verticality: 50, gravityChaos: 40, archetype: 1 };

const scene = (kind: RaceSceneKind, title: string, extra: Partial<GalleryScene> = {}): GalleryScene => ({
  id: `belt-${kind}`,
  title,
  covers: "Belt Up (M4 C7)",
  world: RACE_WORLD,
  pose: "belt",
  content: "race",
  race: kind,
  time: 6.0,
  ...extra,
});

export const RACE_SCENES: readonly GalleryScene[] = [
  scene("start", "Belt Up start: three of five red lights lit over the slot, the gate ahead, the HUD's lights", { ship: 1 }),
  scene("inside", "Racing inside the belt: dust glowing in the sun, the next gate bright, rivals ahead, a boulder", { ship: 1 }),
  scene("skim", "Skimming the belt's surface: the aurora overhead, a rival sizzling in the radiation, the dose meter", { ship: 1 }),
  scene("open", "The open line: aurora curtains all around, the hull sizzling, static and fringe on the screen edges", { ship: 1 }),
  scene("dive", "The dive: out of the belt's inner edge toward the EXIT sign and its portal", { ship: 1 }),
  scene("exit", "The exit: the swirling portal under the giant EXIT sign, a rival diving in"),
  scene("results", "Heat results over the belt: places, times, points gained and the totals"),
  scene("overview", "The whole ring from above: the dust band, the gates (passed ones in your colour), the aurora, the EXIT"),
  scene("ink", "Belt Up at Palette 90: the belt in ink, the gates and the exit", { world: { ...RACE_WORLD, palette: 90 }, ship: 1 }),
];

