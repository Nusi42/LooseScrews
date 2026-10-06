// Names, accents and icons of the 18 comedy events for the event banner
// (ui/eventBanner.ts; M4 C1). Names match `data/events/<id>.ron`. Icons are
// inline SVG markup in a 48 x 48 view box, drawn in `currentColor` (the
// accent) with white details; events without their own icon yet use the
// generic star. C2 and C3 replace the icons of their events here.

export interface EventInfo {
  name: string;
  /** Kicker line over the name. */
  kicker: string;
  /** Accent (CSS colour). */
  accent: string;
  /** SVG markup inside `<svg viewBox="0 0 48 48">`. */
  icon: string;
}

const STAR = `<path d="M24 5l5.6 12.1 13.2 1.5-9.8 9 2.7 13.1L24 34.1l-11.7 6.6 2.7-13.1-9.8-9 13.2-1.5z" fill="currentColor"/><circle cx="24" cy="24" r="4" fill="#fff"/>`;

const ICONS: Record<string, string> = {
  bored_level: `<rect x="9" y="9" width="30" height="30" rx="6" fill="none" stroke="currentColor" stroke-width="3.5" transform="rotate(-20 24 24)"/><path d="M17 26c2 2 4 2 6 0M25 26c2 2 4 2 6 0" stroke="#fff" stroke-width="2.6" fill="none" stroke-linecap="round"/><path d="M33 8h6l-6 7h6M39 2h4l-4 4h4" stroke="#fff" stroke-width="2" fill="none" stroke-linejoin="round"/>`,
  level_hiccups: `<circle cx="21" cy="27" r="13" fill="currentColor"/><circle cx="16" cy="25" r="2" fill="#fff"/><circle cx="26" cy="25" r="2" fill="#fff"/><ellipse cx="21" cy="33" rx="3" ry="2.4" fill="#fff"/><circle cx="37" cy="13" r="4.5" fill="none" stroke="#fff" stroke-width="2.4"/><circle cx="43" cy="5" r="2.6" fill="none" stroke="#fff" stroke-width="2"/>`,
  gravity_coffee_break: `<path d="M10 18h22v14a8 8 0 0 1-8 8h-6a8 8 0 0 1-8-8z" fill="currentColor"/><path d="M32 22h3a5 5 0 0 1 0 10h-3" fill="none" stroke="currentColor" stroke-width="3.5"/><path d="M15 13c-2-3 2-4 0-8M21 13c-2-3 2-4 0-8M27 13c-2-3 2-4 0-8" stroke="#fff" stroke-width="2.4" fill="none" stroke-linecap="round"/>`,
  wrong_genre: `<rect x="5" y="12" width="38" height="24" rx="6" fill="currentColor"/><path d="M13 19v10M8 24h10" stroke="#fff" stroke-width="3.5" stroke-linecap="square"/><rect x="29" y="18" width="5" height="5" fill="#fff"/><rect x="35" y="25" width="5" height="5" fill="#fff"/>`,
  big_head_day: `<circle cx="24" cy="19" r="15" fill="currentColor"/><circle cx="18" cy="16" r="4.2" fill="#fff"/><circle cx="30" cy="16" r="4.2" fill="#fff"/><circle cx="19" cy="17" r="2" fill="#111"/><circle cx="29" cy="15" r="2" fill="#111"/><path d="M16 24q8 7 16 0" stroke="#111" stroke-width="2.6" fill="none" stroke-linecap="round"/><path d="M24 34l-2 3h4zM24 37c0 4-3 5-2 9" stroke="currentColor" stroke-width="2" fill="none"/>`,
  rubber_chickens: `<path d="M8 30c0-8 7-13 15-12l4-7c2-3 6-3 7 0l1 4 6 2-6 3c-1 5-3 9-8 12-6 4-19 5-19-2z" fill="currentColor"/><path d="M41 15l5 1-5 2z" fill="#ff8c1a"/><path d="M30 9c1-3 4-4 5-1 2-2 4 0 3 2" fill="#ff3b2f"/><circle cx="32" cy="13" r="1.6" fill="#111"/><path d="M18 36l-2 8M24 36l1 8" stroke="#ff8c1a" stroke-width="2.4" stroke-linecap="round"/>`,
  moving_day: `<path d="M8 18l16-8 16 8v18l-16 8-16-8z" fill="currentColor"/><path d="M8 18l16 8 16-8M24 26v18" stroke="#000" stroke-opacity="0.35" stroke-width="2" fill="none"/><path d="M16 14l16 8" stroke="#fff" stroke-width="4"/><path d="M30 33l3-5 3 5h-2v4h-2v-4z" fill="#fff"/>`,
  everything_is_cake: `<path d="M8 26h32v14H8z" fill="currentColor"/><path d="M8 26c3 5 5 5 8 0 3 5 5 5 8 0 3 5 5 5 8 0 3 5 5 5 8 0" fill="#fff"/><path d="M8 34h32" stroke="#fff" stroke-opacity="0.6" stroke-width="2"/><rect x="22" y="12" width="4" height="12" rx="1" fill="#fff"/><path d="M24 4c3 3 3 6 0 7-3-1-3-4 0-7z" fill="#ffd34d"/>`,
};

const INFO: Record<string, Omit<EventInfo, "icon">> = {
  bored_level: { name: "Bored level", kicker: "The level is turning over", accent: "#b9b4a6" },
  tax_drone: { name: "Tax drone", kicker: "Audit incoming", accent: "#7fd6a6" },
  gravity_coffee_break: { name: "Gravity coffee break", kicker: "Wells off for 20 s", accent: "#c98b5a" },
  rubber_chickens: { name: "Rubber chickens", kicker: "Every shot squeaks", accent: "#ffd22e" },
  cat_paw: { name: "Cat paw", kicker: "Something is curious", accent: "#ff9ec7" },
  tired_announcer: { name: "Tired announcer", kicker: "Kill-cams for everyone", accent: "#8fa6c8" },
  wrong_genre: { name: "Wrong genre", kicker: "Press start", accent: "#ff004d" },
  gossip_wall: { name: "Gossip wall", kicker: "Everyone sees everyone", accent: "#c78bff" },
  pizza_delivery: { name: "Pizza delivery", kicker: "Hold still and eat", accent: "#ff8a3d" },
  everything_is_cake: { name: "Everything is cake", kicker: "Craters are bigger. And tasty", accent: "#ff9ec7" },
  duck_parade: { name: "Duck parade", kicker: "Let them through", accent: "#ffd22e" },
  hat_rain: { name: "Hat rain", kicker: "Grab a hat", accent: "#7fc8ff" },
  customer_survey: { name: "Customer survey", kicker: "Please rate your match", accent: "#7fd6ff" },
  disco_inferno: { name: "Disco inferno", kicker: "Lights off", accent: "#d06bff" },
  big_head_day: { name: "Big head day", kicker: "Hit spheres times three", accent: "#ff7ab6" },
  moving_day: { name: "Moving day", kicker: "The pickups are moving", accent: "#d9a066" },
  mimic_pickups: { name: "Mimic pickups", kicker: "Some pickups bite", accent: "#ff6b5a" },
  level_hiccups: { name: "Level hiccups", kicker: "Hic", accent: "#8fd3ff" },
};

/** Banner data of an event id (unknown ids: the id as a name and the star). */
export function eventInfo(id: string): EventInfo {
  const i = INFO[id];
  return { name: i?.name ?? id.replace(/_/g, " "), kicker: i?.kicker ?? "", accent: i?.accent ?? "#ffb347", icon: ICONS[id] ?? STAR };
}

/** Event ids that have their own icon. */
export function hasOwnIcon(id: string): boolean {
  return id in ICONS;
}
