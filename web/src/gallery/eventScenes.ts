// Gallery strips of the comedy events (milestone-4-plan.md §5 wave C: "for
// each event, gallery strip `event-<id>` (announce, peak, aftermath)"). Pure
// data (imported by scenes.ts, also from Node); gallery/eventScenesRun.ts
// builds them in the page. Every scene renders without the event visuals
// with `--query eventfx=off` (the before shots).
//
// C1 owns this file; C2 and C3 add their events' strips here (one block per
// event).

import type { GalleryScene, GalleryWorld } from "./scenes.ts";

/** Frames of a strip: the announcement, the peak and the aftermath. */
export type EventFrame = "a" | "b" | "c";

/** What an event scene shows: the event, its frame and the event clock to render at. */
export interface EventSceneSpec {
  /** Event id (`data/events/<id>.ron`). */
  id: string;
  frame: EventFrame;
  /** Seconds since the active phase started (negative: announced); with `over`, the clock at the end. */
  t: number;
  /** Seconds after the event ended (aftermath frames), else absent. */
  over?: number;
}

const CAVES: GalleryWorld = { seed: 7, enclosure: 50, density: 50 };
/** Caverns open to the sky (the sun gets in). */
const SUNLIT: GalleryWorld = { seed: 7, enclosure: 47, density: 60 };
/** Mine shafts: flat ceilings for the hiccups' dust. */
const MINES: GalleryWorld = { seed: 7, enclosure: 100, density: 30 };
/** Open sky with islands, the sun and the planet: the Bored level turn. */
const TURN_WORLD: GalleryWorld = { seed: 11, enclosure: 20, density: 55 };
/** Caverns with seven gravity wells. */
const WELL_CAVES: GalleryWorld = { seed: 7, enclosure: 50, density: 50, gravityChaos: 100 };

const FRAME_NAMES: Readonly<Record<EventFrame, string>> = { a: "announce", b: "peak", c: "aftermath" };

function strip(id: string, name: string, world: GalleryWorld, pose: GalleryScene["pose"], frames: Record<EventFrame, { t: number; over?: number; title: string }>, extra: Partial<GalleryScene> = {}): GalleryScene[] {
  return (["a", "b", "c"] as const).map((f) => {
    const spec: EventSceneSpec = { id, frame: f, t: frames[f].t, ...(frames[f].over !== undefined ? { over: frames[f].over } : {}) };
    return {
      id: `event-${id.replace(/_/g, "-")}-${f}`,
      title: `${name} (${FRAME_NAMES[f]}): ${frames[f].title}`,
      covers: "event visuals (M4 C1)",
      world,
      pose,
      content: "event",
      eventScene: spec,
      time: 6.0,
      ...(f === "a" ? { caption: { pack: "overexcited" as const, trigger: `event.${id}` } } : {}),
      ...extra,
    };
  });
}

export const EVENT_SCENES: readonly GalleryScene[] = [
  ...strip("bored_level", "Bored level", TURN_WORLD, "planet", {
    a: { t: -1.4, title: "the banner, sleepy Z's drifting up, the announcer" },
    b: { t: 2.6, title: "the level halfway over: sky, sun and planet turning, grit sliding, the down arrow" },
    c: { t: 0, over: 1.2, title: "up is sideways now; the arrow fades" },
  }),
  ...strip("level_hiccups", "Level hiccups", MINES, "tunnel", {
    a: { t: -0.6, title: "the banner and the tell's first little hic" },
    b: { t: 3.22, title: "a hiccup: HIC!, the camera dips, dust and pebbles shake off the ceiling, bubbles rise" },
    c: { t: 4.4, title: "the dust settling after the hic" },
  }),
  ...strip("gravity_coffee_break", "Gravity coffee break", WELL_CAVES, "well", {
    a: { t: -2.0, title: "a coffee mug drops onto the well's core stone" },
    b: { t: 6.5, title: "the well on its break: dimmed, steaming mug, lounge music" },
    c: { t: 0, over: 0.55, title: "break over: the mug lifts off, the well comes back" },
  }),
  ...strip("wrong_genre", "Wrong genre", CAVES, "tunnel", {
    a: { t: -0.35, title: "the picture glitches into pixels" },
    b: { t: 3.0, title: "a side-scroller: side view, the rock cut away, 16 colours, everyone in one plane" },
    c: { t: 0, over: 0.9, title: "back in the cockpit, 1-UP" },
  }),
  ...strip("big_head_day", "Big head day", SUNLIT, "shade", {
    a: { t: -0.9, title: "balloon heads blown up in squeaky puffs" },
    b: { t: 4.2, title: "balloon heads at the 3x hit size, googly eyes, a hit wobbles one (honk)" },
    c: { t: 0, over: 0.12, title: "they all pop: rubber shreds and confetti" },
  }),
  ...strip("rubber_chickens", "Rubber chickens", SUNLIT, "shade", {
    a: { t: -1.2, title: "the banner (a squeak)" },
    b: { t: 4.0, title: "a volley of rubber chickens and a giant egg" },
    c: { t: 6.0, title: "feathers where they hit" },
  }),
  ...strip("moving_day", "Moving day", SUNLIT, "shade", {
    a: { t: -0.95, title: "pickups packed into boxes, the flaps shut, taped" },
    b: { t: 0.5, title: "the boxes hop to their new places" },
    c: { t: 0, over: 0.22, title: "landed: the flaps pop open" },
  }),
  ...strip("everything_is_cake", "Everything is cake", SUNLIT, "shade", {
    a: { t: -0.7, title: "sprinkles fountain where the cake will start" },
    b: { t: 1.15, title: "the cake wave running over the rock" },
    c: { t: 9, title: "a cake cavern: sponge, cream, jam, frosting, sprinkles, crumbs" },
  }),
];
