// Which hull a wreck shows. The sim's wreck entity carries only its scrap
// (`[scrap, 0, 0, 0]`, no ship type), so the client remembers where ships
// of each type died and gives a wreck that appears there soon after the
// dead ship's type (models/wrecks.ts breaks that hull into its pieces).
// Wrecks seen without a matching death (joined late, out of view) get a
// type from their entity id, so they still look like one of the hulls.
// Purely cosmetic; never fed back into the sim.

import type { Vec3 } from "../math/vec3.ts";
import type { EntityState, ShipState } from "./views.ts";

/** A death and a new wreck match within this distance (u)... */
export const WRECK_MATCH_DISTANCE = 3;
/** ...and this time (s). */
export const WRECK_MATCH_SECONDS = 3;
/** Ship types (Skitter, Tinker, Hauler, Tug). */
const SHIP_TYPES = 4;

interface Death {
  position: Vec3;
  shipType: number;
  time: number;
}

/** A ship as the tracker needs it (remote ships, or the own ship from its pose). */
export type TrackedShip = Pick<ShipState, "id" | "alive" | "position" | "shipType">;

export class WreckTypes {
  private readonly alive = new Map<number, boolean>();
  private deaths: Death[] = [];
  private readonly known = new Map<number, number>();

  /** Feeds one frame: notes deaths, assigns new wrecks, forgets removed ones. */
  observe(ships: readonly TrackedShip[], entities: readonly EntityState[], time: number): void {
    for (const s of ships) {
      const was = this.alive.get(s.id);
      if (was === true && !s.alive) this.deaths.push({ position: s.position, shipType: s.shipType, time });
      this.alive.set(s.id, s.alive);
    }
    this.deaths = this.deaths.filter((d) => time - d.time <= WRECK_MATCH_SECONDS);
    const present = new Set<number>();
    for (const e of entities) {
      if (e.kind !== "wreck") continue;
      present.add(e.id);
      if (this.known.has(e.id)) continue;
      let best = -1;
      let bestDist = WRECK_MATCH_DISTANCE;
      this.deaths.forEach((d, i) => {
        const dist = Math.hypot(d.position.x - e.position.x, d.position.y - e.position.y, d.position.z - e.position.z);
        if (dist <= bestDist) {
          bestDist = dist;
          best = i;
        }
      });
      if (best >= 0) {
        this.known.set(e.id, this.deaths[best]!.shipType);
        this.deaths.splice(best, 1);
      } else {
        this.known.set(e.id, fallbackType(e.id));
      }
    }
    for (const id of [...this.known.keys()]) if (!present.has(id)) this.known.delete(id);
  }

  /** Hull type of a wreck entity. */
  typeOf(e: Pick<EntityState, "id">): number {
    return this.known.get(e.id) ?? fallbackType(e.id);
  }

  /** Forgets everything (new match). */
  clear(): void {
    this.alive.clear();
    this.deaths = [];
    this.known.clear();
  }
}

/** Stable hull type for a wreck whose death was not seen. */
export function fallbackType(id: number): number {
  return ((id * 2654435761) >>> 0) % SHIP_TYPES;
}
