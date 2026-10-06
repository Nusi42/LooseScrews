// Gallery strips of M4 C3's spectacle events (milestone-4-plan.md §5 wave
// C: "for each event, gallery strip `event-<id>` (announce, peak,
// aftermath)"): Duck parade, Hat rain, Cat paw, Disco inferno and the Tired
// announcer, three frames each, plus a sheet of the eight hats. Pure data
// (imported by scenes.ts, also from Node); gallery/spectacleRun.ts builds
// them in the page with synthetic props driven through the real event
// visuals (game/events/*.ts). `--query worldfx=off` renders the same frames
// without them (the before shots: the props as the scene builder drew them).

import type { GalleryScene, GalleryWorld } from "./scenes.ts";

/** What a spectacle scene shows (gallery/spectacleRun.ts). */
export type SpectacleSceneKind =
  | "duck_parade-a"
  | "duck_parade-b"
  | "duck_parade-c"
  | "hat_rain-a"
  | "hat_rain-b"
  | "hat_rain-c"
  | "cat_paw-a"
  | "cat_paw-b"
  | "cat_paw-c"
  | "disco_inferno-a"
  | "disco_inferno-b"
  | "disco_inferno-c"
  | "tired_announcer-a"
  | "tired_announcer-b"
  | "tired_announcer-c"
  | "hat-sheet";

const CAVES: GalleryWorld = { seed: 7, enclosure: 50, density: 50 };
/** Wide caverns open to the sky (the start screen's backdrop world): room for a parade of giant ducks. */
const WIDE: GalleryWorld = { seed: 6, enclosure: 35, density: 65 };
/** Caverns open to the sky: hats rain in daylight. */
const SUNLIT: GalleryWorld = { seed: 7, enclosure: 47, density: 60 };

const scene = (
  kind: SpectacleSceneKind,
  title: string,
  world: GalleryWorld,
  pose: GalleryScene["pose"],
  extra: Partial<GalleryScene> = {},
): GalleryScene => ({
  id: kind === "hat-sheet" ? "event-hat-sheet" : `event-${kind}`,
  title,
  covers: "comedy events (M4 C3)",
  world,
  pose,
  content: "event",
  spectacle: kind,
  time: 6.0,
  ...extra,
});

export const SPECTACLE_SCENES: readonly GalleryScene[] = [
  scene("duck_parade-a", "Duck parade, announce: the convoy of giant rubber ducks waddles into the cavern", WIDE, "vista", { caption: { pack: "overexcited", trigger: "event.duck_parade" } }),
  scene("duck_parade-b", "Duck parade, peak: the lead duck (top hat) shoves a ship aside, the next one squeezes through a gap, soap bubbles", WIDE, "vista"),
  scene("duck_parade-c", "Duck parade, aftermath: the cool duck (sunglasses) waddles off, bubbles drifting", WIDE, "vista"),
  scene("hat_rain-a", "Hat rain, announce: the first hats flutter down from above", SUNLIT, "shade", { caption: { pack: "passiveAggressive", trigger: "event.hat_rain" } }),
  scene("hat_rain-b", "Hat rain, peak: hats raining and resting on rock, stacks swaying on the hulls, three hats give a pilot away", SUNLIT, "shade", { ship: 1 }),
  scene("hat_rain-c", "Hat rain, aftermath: the Hat Baron's tower of hats; a wreck's hats scattered for the taking", SUNLIT, "shade"),
  scene("cat_paw-a", "Cat paw, announce: the paw's shadow darkens on the boundary wall, claws glowing through the rock", CAVES, "tunnel", { caption: { pack: "tired", trigger: "event.cat_paw" } }),
  scene("cat_paw-b", "Cat paw, peak: the paw rakes along the wall, claws out, rock spraying, motion blur", CAVES, "tunnel"),
  scene("cat_paw-c", "Cat paw, aftermath: three fresh claw grooves in the wall, the paw withdrawing, dust settling", CAVES, "tunnel"),
  scene("disco_inferno-a", "Disco inferno, announce: the lights dim, the mirror ball comes down", CAVES, "vista", { caption: { pack: "overexcited", trigger: "event.disco_inferno" } }),
  scene("disco_inferno-b", "Disco inferno, peak: lights at 8 %, eight coloured spots sweeping, reflections wheeling, light trails behind every ship", CAVES, "vista"),
  scene("disco_inferno-c", "Disco inferno, aftermath: the lights come back up, the last light trails fading", CAVES, "vista"),
  scene("tired_announcer-a", "Tired announcer, announce: the announcer yawns, kill-cams for everyone", WIDE, "vista", { caption: { pack: "tired", trigger: "event.tired_announcer" } }),
  scene("tired_announcer-b", "Tired announcer, peak: a kill-cam replays the killer's view in the corner, narrated deadpan", WIDE, "vista", { caption: { pack: "tired", trigger: "killcam" } }),
  scene("tired_announcer-c", "Tired announcer, aftermath: the kill-cam's last second, the blast and the K.O. stamp", WIDE, "vista", { caption: { pack: "passiveAggressive", trigger: "killcam" } }),
  scene("hat-sheet", "Hat rain: the eight hats (top hat, cowboy, party cone, chef, crown, propeller beanie, fez, viking)", SUNLIT, "shade"),
];
