// The event card / pill in the spectacle and visitor gallery strips (M4 D5).
// Those strips run their events' own visuals without C1's hub, which draws the
// card in play for all 18 events; this puts the same card (ui/eventBanner.ts)
// over the frame from the strip's scripted mood, as the hub's `bannerModel`
// would: the card with its countdown while the event is announced, the pill
// with the ring of the time left while it runs, nothing once it is over.

import { ANNOUNCE_SECS, TICK_HZ } from "../game/events/fx.ts";
import type { MoodState } from "../game/modeState.ts";
import { EventHud, type EventBannerModel } from "../ui/eventBanner.ts";

/** `active_secs` of the strip events (data/events/<id>.ron; eventCard.test.ts checks them). */
export const STRIP_ACTIVE_SECS: Readonly<Record<string, number>> = {
  tax_drone: 25,
  pizza_delivery: 30,
  customer_survey: 12,
  mimic_pickups: 20,
  gossip_wall: 15,
  duck_parade: 45,
  hat_rain: 25,
  cat_paw: 3,
  disco_inferno: 20,
  tired_announcer: 30,
};

/** The banner the hub would show for a scripted mood at `nowTick` (null without a running event). */
export function stripBanner(mood: MoodState, nowTick: number, target: string): EventBannerModel | null {
  if (!mood.eventId || !mood.phase) return null;
  const left = Math.max(0, (mood.phaseEndTick - nowTick) / TICK_HZ);
  if (mood.phase === "announce") return { id: mood.eventId, stage: "announce", secsLeft: Math.min(left, ANNOUNCE_SECS), total: ANNOUNCE_SECS, target };
  const total = Math.max(left, STRIP_ACTIVE_SECS[mood.eventId] ?? left, 0.1);
  return { id: mood.eventId, stage: "active", secsLeft: mood.phase === "active" ? left : 0, total, target };
}

/** Puts the event card or pill of `mood` over the strip frame. */
export function showStripCard(ui: HTMLElement, mood: MoodState, nowTick: number, target: string): EventHud | null {
  const banner = stripBanner(mood, nowTick, target);
  if (!banner) return null;
  const hud = new EventHud(ui);
  hud.update({ banner, hints: { down: null, retro: null, zzz: 0, pops: [] }, sideView: false });
  return hud;
}
