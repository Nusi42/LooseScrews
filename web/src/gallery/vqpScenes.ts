// Gallery content of the VQ polish package (VQP): ships charging the
// Fusion Charger (charge-up effect, game/chargeFx.ts) and cloaked ships
// (refraction, renderer/cloak.ts). Positions are relative to the camera, so
// gallery.ts `sceneContent` (meshes, lights) and vfxScenes.ts (particle
// history) agree.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { CHARGE_FALLBACK_COLOR, eyeHeight, glowAt, OWN_CHARGE_SIZE, type ChargeGlow } from "../game/chargeFx.ts";
import type { ShipState } from "../game/views.ts";

interface Cam {
  position: Vec3;
  orientation: Quat;
}

/** Ships and charges of the `charge` content at time `t` (scene time `T`). */
export interface ChargeMoment {
  ships: ShipState[];
  charges: ChargeGlow[];
}

/** Seconds to a full Fusion Charger charge (data/weapons/fusion_charger.ron). */
const FULL_SECS = 2;

function frameOf(cam: Cam): { at: (fw: number, rt: number, upw: number) => Vec3; f: Vec3; r: Vec3; u: Vec3 } {
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const at = (fw: number, rt: number, upw: number): Vec3 => v3.add(cam.position, v3.add(v3.scale(f, fw), v3.add(v3.scale(r, rt), v3.scale(u, upw))));
  return { at, f, r, u };
}

/**
 * Three ships charging toward the camera: a Tinker at full charge (since
 * T - 2.4), a Hauler at 45 % and a Skitter overcharging. With `own` (the
 * cockpit scene) the own Tinker charges at 75 % and only the first ship flies.
 */
export function chargeAt(cam: Cam, t: number, T: number, own: number | null = null): ChargeMoment {
  const { at, r, u } = frameOf(cam);
  const facing = (pos: Vec3, side: number): Quat => q.lookRotation(v3.normalize(v3.add(v3.normalize(v3.sub(cam.position, pos)), v3.scale(r, side))), u);
  const specs: Array<{ id: number; type: number; radius: number; pos: Vec3; side: number; start: number; over: boolean }> = [
    { id: 21, type: 1, radius: 0.5, pos: at(5.2, 1.0, -0.15), side: -0.35, start: T - 2.4, over: false },
    { id: 22, type: 2, radius: 0.65, pos: at(9.0, -2.3, 0.55), side: 0.55, start: T - 0.9, over: false },
    { id: 23, type: 0, radius: 0.4, pos: at(12.5, 2.8, 1.3), side: -0.6, start: T - 2.9, over: true },
  ];
  const ships: ShipState[] = [];
  const charges: ChargeGlow[] = [];
  for (const s of own === null ? specs : specs.slice(0, 1)) {
    const orientation = facing(s.pos, s.side);
    ships.push({ id: s.id, team: 1, alive: true, boosting: false, position: s.pos, orientation, shipType: s.type, radius: s.radius, gadgetActive: false });
    if (t >= s.start) {
      const fraction = (t - s.start) / FULL_SECS;
      charges.push(glowAt(s.id, s.pos, orientation, s.radius, { x: 0, y: 0, z: 0 }, fraction, s.over && fraction >= 1, CHARGE_FALLBACK_COLOR, 1, false));
    }
  }
  if (own !== null) {
    const start = T - 0.75 * FULL_SECS;
    if (t >= start) {
      const below = v3.addScaled(cam.position, u, -eyeHeight(1, own));
      charges.push(glowAt(0, below, cam.orientation, own, { x: 0, y: 0, z: 0 }, (t - start) / FULL_SECS, false, CHARGE_FALLBACK_COLOR, OWN_CHARGE_SIZE, true));
    }
  }
  return { ships, charges };
}

/** Ids of the cloaked ships of the `cloak` content. */
export const CLOAKED = new Set([31, 32]);

/**
 * Cloaked ships against the rock: an enemy Tinker crossing close by, a
 * teammate Hauler further back (both cloaked), and an uncloaked Skitter for
 * comparison.
 */
export function cloakAt(cam: Cam): ShipState[] {
  const { at, f, r, u } = frameOf(cam);
  const across = (side: number): Quat => q.lookRotation(v3.normalize(v3.add(v3.scale(r, side), v3.scale(f, -0.45))), u);
  return [
    { id: 31, team: 1, alive: true, boosting: false, position: at(2.4, 0.45, -0.15), orientation: across(1), shipType: 1, radius: 0.5, gadgetActive: true },
    { id: 32, team: 0, alive: true, boosting: false, position: at(4.6, -1.35, 0.25), orientation: across(-1), shipType: 2, radius: 0.65, gadgetActive: true },
    { id: 33, team: 1, alive: true, boosting: false, position: at(7, 1.9, 0.6), orientation: across(-0.6), shipType: 0, radius: 0.4, gadgetActive: false },
  ];
}
