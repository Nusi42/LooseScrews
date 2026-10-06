// The Architect's gallery scenes (M4 C4) as pure data: imported by
// scenes.ts (also from Node, tools/browser-smoke/gallery.mjs);
// gallery/architectScenes.ts builds them in the page. Three-frame strips
// (`-a` the tell, `-b` the peak, `-c` the aftermath) for a rock stroke, the
// Palette ink blackout and the Absurdity+ event as a runner sees them.

import type { GalleryScene, GalleryWorld } from "./scenes.ts";

/** What an Architect scene shows. */
export type ArchitectSceneKind =
  | "architect-free"
  | "architect-overview"
  | "architect-overview-sky"
  | "architect-radial"
  | "architect-panel"
  | "architect-rock-a"
  | "architect-rock-b"
  | "architect-rock-c"
  | "architect-ink-a"
  | "architect-ink-b"
  | "architect-ink-c"
  | "architect-event-a"
  | "architect-event-b"
  | "architect-event-c"
  | "architect-exit"
  | "architect-round-over";

const CAVES: GalleryWorld = { seed: 7, enclosure: 50, density: 50 };
/** Caverns with sky openings: the map view's cut shows sunlit floors. */
const OPEN_CAVES: GalleryWorld = { seed: 7, enclosure: 40, density: 55 };

const scene = (id: ArchitectSceneKind, title: string, world: GalleryWorld, extra: Partial<GalleryScene> = {}): GalleryScene => ({
  id,
  title,
  covers: "The Architect (M4 C4)",
  world,
  pose: "tunnel",
  content: "architect",
  architect: id,
  time: 4.0,
  ...extra,
});

export const ARCHITECT_SCENES: readonly GalleryScene[] = [
  scene("architect-free", "Architect, free flight: an Enclosure+ preview on the rock, two runners marked through it, the stroke dock and budget", CAVES),
  scene("architect-overview", "Architect, map view: the level cut open above the floor, runners' pillars, the exit gate, a Gravity+ telegraph", CAVES),
  scene("architect-overview-sky", "Architect, map view over an open level (islands): strokes and runners from above", OPEN_CAVES),
  scene("architect-radial", "Architect: the stroke wheel (hold R) with Gravity+ picked", CAVES),
  scene("architect-panel", "Architect with the pointer free: the world's slider panel picks strokes, cards clickable", CAVES),
  scene("architect-rock-a", "Runner: an Enclosure+ stroke telegraphed ahead (the tell: ring filling, alert)", CAVES, { ship: 1 }),
  scene("architect-rock-b", "Runner: the stroke acts (the peak: the flash and rock growing in a burst of dust)", CAVES, { ship: 1 }),
  scene("architect-rock-c", "Runner: the aftermath (new rock in the tunnel, dust settling)", CAVES, { ship: 1 }),
  scene("architect-ink-a", "Runner: a Palette stroke telegraphed (the tell)", CAVES, { ship: 1 }),
  scene("architect-ink-b", "Runner: the ink blackout at its peak (squid ink fills the sphere)", CAVES, { ship: 1 }),
  scene("architect-ink-c", "Runner: the ink thinning as the blackout ends (aftermath)", CAVES, { ship: 1 }),
  scene("architect-event-a", "Runner: an Absurdity+ stroke telegraphed (the tell)", CAVES, { ship: 1 }),
  scene("architect-event-b", "Runner: Absurdity+ pops (confetti, the event starts)", CAVES, { ship: 1 }),
  scene("architect-event-c", "Runner: the confetti settling (aftermath)", CAVES, { ship: 1 }),
  scene("architect-exit", "Runner: the exit gate ahead and the round banner", CAVES, { ship: 1 }),
  scene("architect-round-over", "Runner through the exit: the round goes to the runners", CAVES),
];
