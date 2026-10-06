// Gallery strips of the visitor events (M4 C2; docs/milestone-4-plan.md §5
// wave C: per event three frames, the tell, the peak and the aftermath):
// `event-<id>-a|b|c` for the Tax drone, Pizza delivery, Customer survey,
// Mimic pickups and Gossip wall. Pure data (imported by scenes.ts, also from
// Node); gallery/visitorRun.ts builds them in the page from synthetic sim
// states, through the same code that draws them in play
// (game/events/visitors.ts). `--query visitors=off` renders the before
// shots (the props as plain shapes).

import type { GalleryScene, GalleryWorld } from "./scenes.ts";

export const VISITOR_EVENTS = ["tax_drone", "pizza_delivery", "customer_survey", "mimic_pickups", "gossip_wall"] as const;
export type VisitorEventId = (typeof VISITOR_EVENTS)[number];
export type StripFrame = "a" | "b" | "c";

/** What a visitor scene shows: the event and the frame of its strip. */
export interface VisitorSceneKind {
  event: VisitorEventId;
  frame: StripFrame;
}

/** Worlds of the strips: caverns open to the sky (sunlit rock) and darker caves (the sirens and the face's light show). */
const WORLDS: Readonly<Record<VisitorEventId, GalleryWorld>> = {
  tax_drone: { seed: 3, enclosure: 50, density: 50 },
  pizza_delivery: { seed: 7, enclosure: 47, density: 60 },
  customer_survey: { seed: 12, enclosure: 47, density: 60 },
  mimic_pickups: { seed: 5, enclosure: 47, density: 60 },
  gossip_wall: { seed: 7, enclosure: 50, density: 50 },
};

const TITLES: Readonly<Record<VisitorEventId, Readonly<Record<StripFrame, string>>>> = {
  tax_drone: {
    a: "Tax drone, the tell: waiting at its entry, siren flashing, the announcer names the leader",
    b: "Tax drone, the peak: pressed against the leader, energy motes sucked into the printer, the receipt grows",
    c: "Tax drone, the aftermath: shot down in a burst of shredded receipts, a scrap pile left behind",
  },
  pizza_delivery: {
    a: "Pizza delivery, the tell: the drone waiting with a steaming box, the announcer names the hungry pilot",
    b: "Pizza delivery, the peak: hovering by the recipient, the pizza assembling slice by slice under its ship",
    c: "Pizza delivery, the aftermath: delivered in a burst of cheese and green repair sparkles",
  },
  customer_survey: {
    a: "Customer survey, the tell: the drone pops up in front of the leader with its five-star panel",
    b: "Customer survey, the peak: from the leader's cockpit, the panel blocking a quarter of the view",
    c: "Customer survey, the aftermath: rated two stars, the panel tumbles away, sad trombone",
  },
  mimic_pickups: {
    a: "Mimic pickups, the tell: the picked crates tremble and twitch",
    b: "Mimic pickups, the peak: mimics hop at a pilot, lids open on teeth, one biting",
    c: "Mimic pickups, the aftermath: a mimic shot to splinters, its loot real again",
  },
  gossip_wall: {
    a: "Gossip wall, the tell: a stone face pushes out of the rock, dust crumbling",
    b: "Gossip wall, the peak: the face gossips, every pilot marked through the rock",
    c: "Gossip wall, the aftermath: the face winks and sinks back into the rock",
  },
};

/** The announcer line shown over the tell (ui/announcer.ts, M4 A6). */
const CAPTIONS: Readonly<Record<VisitorEventId, NonNullable<GalleryScene["caption"]>>> = {
  tax_drone: { pack: "passiveAggressive", trigger: "event.tax_drone" },
  pizza_delivery: { pack: "overexcited", trigger: "event.pizza_delivery" },
  customer_survey: { pack: "passiveAggressive", trigger: "event.customer_survey" },
  mimic_pickups: { pack: "overexcited", trigger: "event.mimic_pickups" },
  gossip_wall: { pack: "tired", trigger: "event.gossip_wall" },
};

function scene(event: VisitorEventId, frame: StripFrame): GalleryScene {
  const s: GalleryScene = {
    id: `event-${event}-${frame}`,
    title: TITLES[event][frame],
    covers: "event visuals (M4 C2)",
    world: WORLDS[event],
    pose: "tunnel",
    content: "visitor",
    visitor: { event, frame },
    time: 6.0,
  };
  if (frame === "a") s.caption = CAPTIONS[event];
  // The survey's peak is the leader's own view: the cockpit frame shows.
  if (event === "customer_survey" && frame === "b") s.ship = 1;
  return s;
}

export const VISITOR_SCENES: readonly GalleryScene[] = VISITOR_EVENTS.flatMap((e) => (["a", "b", "c"] as const).map((f) => scene(e, f)));
