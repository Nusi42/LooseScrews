// Builds the renderer's instances from ships, projectiles and entities.
//
// Data-driven: projectiles and entities carry a visual id; the match's
// visual table (ls-wasm `Game.visuals()`, derived from data/weapons,
// data/gadgets and built-in entity visuals) gives each id a shape name, a
// colour and a scale. SHAPE_STYLES maps shape names to a mesh kind, a size,
// glow, trail and animation, so a new weapon or entity only needs a shape
// and a colour in its data file, no client code.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { MeshKind } from "../renderer/primitives.ts";
import { emptyScene, type Instance, type Rgb, type SceneInstances } from "../renderer/renderer.ts";
import {
  PROJ_ARMED,
  PROJ_STUCK,
  TINKER_RADIUS,
  type EntityState,
  type ProjectileState,
  type ShipState,
  type VisualDef,
} from "./views.ts";

/** Hull colour per 0-based team (a darker shade of the paint colour). */
export const TEAM_HULL: readonly Rgb[] = [
  [0.95, 0.5, 0.2],
  [0.25, 0.6, 0.95],
];
const NEUTRAL_HULL: Rgb = [0.8, 0.8, 0.8];

export function teamHull(team: number): Rgb {
  return TEAM_HULL[team] ?? NEUTRAL_HULL;
}

/** How one visual shape is drawn. */
export interface ShapeStyle {
  mesh: MeshKind;
  /** Size (instance scale) at visual scale 1; meshes are about 1 u across. */
  size: number;
  /** 0 = lit, 1 = fully glowing. */
  emissive: number;
  /** Trail length in seconds of travel (0 = none); drawn translucent. */
  trail?: number;
  /** Translucent glow shell around the object, as a size factor (0 = none). */
  glow?: number;
  /** Draw in the translucent pass with this opacity (volumes, ghosts). */
  opacity?: number;
  /** Spin (rad/s) about the vertical axis (pickups) or the flight axis. */
  spin?: number;
  /** Vertical bob amplitude (u) for pickups. */
  bob?: number;
  /** Tint with the owner's team colour instead of the visual colour (paint). */
  teamTint?: boolean;
  /** Tint with the entity's team hull colour (decoys look like ships). */
  teamHull?: boolean;
  /** Scale with the entity's replicated radius (debris, dust). */
  radiusScaled?: boolean;
}

/** Styles by shape name (ls-sim `visual::Shape`, lowercase). */
export const SHAPE_STYLES: Readonly<Record<string, ShapeStyle>> = {
  // Projectiles.
  bolt: { mesh: "bolt", size: 1.0, emissive: 1, trail: 0.05 },
  pellet: { mesh: "orb", size: 0.22, emissive: 0.6, trail: 0.03 },
  spike: { mesh: "bolt", size: 1.3, emissive: 0.8, trail: 0.1 },
  rocket: { mesh: "rocket", size: 0.9, emissive: 0.25, trail: 0.15 },
  missile: { mesh: "rocket", size: 0.7, emissive: 0.35, trail: 0.12 },
  mine: { mesh: "mine", size: 0.8, emissive: 0.25, spin: 1.5 },
  orb: { mesh: "orb", size: 0.45, emissive: 1, trail: 0.05, glow: 2 },
  blob: { mesh: "orb", size: 0.6, emissive: 0.7, trail: 0.04 },
  arc: { mesh: "bolt", size: 0.9, emissive: 1, trail: 0.04, glow: 1.6 },
  grenade: { mesh: "orb", size: 0.4, emissive: 0.15, trail: 0.03 },
  harpoon: { mesh: "harpoon", size: 1.2, emissive: 0.3, trail: 0.03 },
  paint: { mesh: "orb", size: 0.45, emissive: 0.8, trail: 0.03, teamTint: true },
  charge: { mesh: "orb", size: 0.6, emissive: 0.9, glow: 2.2, spin: 3 },
  // Entities.
  debris: { mesh: "debris", size: 1.4, emissive: 0, radiusScaled: true },
  scrap: { mesh: "scrap", size: 1.0, emissive: 0.05, spin: 0.2 },
  wreck: { mesh: "wreck", size: 1.2, emissive: 0 },
  crate: { mesh: "crate", size: 0.9, emissive: 0.35, spin: 1.2, bob: 0.15 },
  upgrade: { mesh: "upgrade", size: 0.9, emissive: 0.6, spin: 2, bob: 0.2, glow: 1.6 },
  gadget: { mesh: "gadget", size: 0.9, emissive: 0.5, spin: 1.5, bob: 0.15 },
  decoy: { mesh: "ship", size: 1, emissive: 0.1, teamHull: true },
  ghost: { mesh: "ship", size: 1, emissive: 0.7, opacity: 0.35 },
  dust: { mesh: "sphere", size: 6, emissive: 0.15, opacity: 0.4, radiusScaled: true },
  duck: { mesh: "duck", size: 2.5, emissive: 0.05, bob: 0.1 },
};

/** Fallback for shapes this client does not know yet. */
export const DEFAULT_STYLE: ShapeStyle = { mesh: "bolt", size: 1, emissive: 1 };

export function styleFor(visual: VisualDef | undefined): ShapeStyle {
  return (visual && SHAPE_STYLES[visual.shape]) || DEFAULT_STYLE;
}

const WHITE: Rgb = [1, 1, 1];
const UP: Vec3 = { x: 0, y: 1, z: 0 };

/** Orientation looking along `v` (identity for a zero vector). */
export function alongVelocity(v: Vec3): Quat {
  const speed = Math.hypot(v.x, v.y, v.z);
  if (speed < 1e-6) return q.identity();
  // lookRotation needs an up hint that is not parallel to the direction.
  const hint = Math.abs(v.y) > 0.99 * speed ? { x: 1, y: 0, z: 0 } : UP;
  return q.lookRotation(v, hint);
}

/** Deterministic per-id phase in 0..2π so pickups do not bob in lockstep. */
export function phase(id: number): number {
  const x = Math.sin(id * 12.9898) * 43758.5453;
  return (x - Math.floor(x)) * Math.PI * 2;
}

function push(scene: SceneInstances, pass: "opaque" | "translucent", mesh: MeshKind, inst: Instance): void {
  (scene[pass][mesh] ??= []).push(inst);
}

/** Ships (living only), scaled by their type's hitbox; an active gadget adds a shimmer shell. */
export function addShips(scene: SceneInstances, ships: readonly ShipState[]): void {
  for (const s of ships) {
    if (!s.alive) continue;
    const scale = (s.radius || TINKER_RADIUS) / TINKER_RADIUS;
    const color = teamHull(s.team);
    push(scene, "opaque", "ship", {
      position: s.position,
      rotation: s.orientation,
      scale,
      color,
      emissive: s.boosting ? 0.35 : 0.1,
    });
    if (s.gadgetActive) {
      push(scene, "translucent", "sphere", {
        position: s.position,
        rotation: q.identity(),
        scale: scale * 1.6,
        color,
        emissive: 0.8,
        opacity: 0.18,
      });
    }
  }
}

/**
 * Projectiles by visual id, oriented along their velocity. `teamOf` gives a
 * shooter's team (paint takes its colour); `time` (s) animates spins and
 * blinking armed mines.
 */
export function addProjectiles(
  scene: SceneInstances,
  shots: readonly ProjectileState[],
  visuals: readonly VisualDef[],
  teamOf: (owner: number) => number | undefined,
  time: number,
): void {
  for (const p of shots) {
    const visual = visuals[p.visual];
    const style = styleFor(visual);
    let color = visual?.color ?? WHITE;
    if (style.teamTint) {
      const team = teamOf(p.owner);
      if (team !== undefined) color = teamHull(team);
    }
    const scale = style.size * (visual?.scale ?? 1);
    let rotation = alongVelocity(p.velocity);
    if (style.spin) rotation = q.mul(rotation, q.fromAxisAngle({ x: 0, y: 0, z: 1 }, time * style.spin));
    let emissive = style.emissive;
    if (p.state & PROJ_ARMED) emissive = Math.sin(time * 10) > 0 ? 1 : 0.2;
    push(scene, "opaque", style.mesh, { position: p.position, rotation, scale, color, emissive });
    if (style.glow) {
      push(scene, "translucent", "sphere", {
        position: p.position,
        rotation: q.identity(),
        scale: scale * style.glow,
        color,
        emissive: 1,
        opacity: 0.3,
      });
    }
    const speed = Math.hypot(p.velocity.x, p.velocity.y, p.velocity.z);
    if (style.trail && speed > 0 && !(p.state & PROJ_STUCK)) {
      const length = speed * style.trail;
      const width = Math.max(0.15, scale * 0.5);
      // The bolt mesh is 1 u long at scale 1: stretch it to `length`, centred behind the head.
      const back = length / 2 + scale * 0.3;
      push(scene, "translucent", "bolt", {
        position: {
          x: p.position.x - (p.velocity.x / speed) * back,
          y: p.position.y - (p.velocity.y / speed) * back,
          z: p.position.z - (p.velocity.z / speed) * back,
        },
        rotation: alongVelocity(p.velocity),
        scale: width,
        stretch: length / width,
        color,
        emissive: 1,
        opacity: 0.35,
      });
    }
  }
}

/** Size of a scrap pile or wreck by its scrap (state byte 0, whole units). */
export function scrapScale(amount: number): number {
  return Math.min(2.5, 0.6 + 0.12 * Math.sqrt(Math.max(0, amount)));
}

/**
 * Entities by visual id: pickups bob and spin, scrap piles grow with their
 * scrap, debris and dust use their radius when known, ghosts and dust are
 * translucent, decoys look like their team's ships.
 */
export function addEntities(
  scene: SceneInstances,
  entities: readonly EntityState[],
  visuals: readonly VisualDef[],
  time: number,
): void {
  for (const e of entities) {
    if (e.kind === "marker" && !visuals[e.visual]) continue;
    const visual = visuals[e.visual];
    const style = styleFor(visual);
    let color = visual?.color ?? WHITE;
    if (style.teamHull) color = teamHull(e.team);
    let scale = style.size * (visual?.scale ?? 1);
    if (style.radiusScaled && e.radius > 0) scale = e.radius * 2 * (visual?.scale ?? 1);
    if (e.kind === "scrapPile" || e.kind === "wreck") scale *= scrapScale(e.state[0]);
    let position = e.position;
    let rotation = e.rotation;
    const ph = phase(e.id);
    if (style.bob) position = { ...position, y: position.y + Math.sin(time * 2 + ph) * style.bob };
    if (style.spin) rotation = q.mul(q.fromAxisAngle(UP, time * style.spin + ph), rotation);
    if (e.kind === "debris" && e.state[1] === 0) {
      // Moving debris tumbles.
      const axis = { x: Math.sin(ph), y: Math.cos(ph), z: 0.5 };
      const len = Math.hypot(axis.x, axis.y, axis.z);
      rotation = q.mul(rotation, q.fromAxisAngle({ x: axis.x / len, y: axis.y / len, z: axis.z / len }, time * 0.8));
    }
    let emissive = style.emissive;
    if (e.kind.endsWith("Pickup")) emissive = Math.min(1, emissive + 0.15 * Math.sin(time * 4 + ph));
    const inst: Instance = { position, rotation, scale, color, emissive };
    if (style.opacity !== undefined) {
      push(scene, "translucent", style.mesh, { ...inst, opacity: style.opacity });
    } else {
      push(scene, "opaque", style.mesh, inst);
    }
    if (style.glow) {
      push(scene, "translucent", "sphere", {
        position,
        rotation: q.identity(),
        scale: scale * style.glow,
        color,
        emissive: 1,
        opacity: 0.2,
      });
    }
  }
}

/** Builds the whole instance scene for one frame. */
export function buildScene(
  ships: readonly ShipState[],
  shots: readonly ProjectileState[],
  entities: readonly EntityState[],
  visuals: readonly VisualDef[],
  teamOf: (owner: number) => number | undefined,
  time: number,
): SceneInstances {
  const scene = emptyScene();
  addShips(scene, ships);
  addProjectiles(scene, shots, visuals, teamOf, time);
  addEntities(scene, entities, visuals, time);
  return scene;
}

/**
 * How deep the camera is inside dust clouds: 0 outside, up to 1 at a
 * cloud's centre. Main thickens the fog with it (visibility is cut).
 */
export function dustDensityAt(cam: Vec3, entities: readonly EntityState[], visuals: readonly VisualDef[]): number {
  let best = 0;
  for (const e of entities) {
    if (e.kind !== "dustCloud") continue;
    const style = styleFor(visuals[e.visual]);
    const radius = e.radius > 0 ? e.radius : style.size / 2;
    const d = Math.hypot(cam.x - e.position.x, cam.y - e.position.y, cam.z - e.position.z);
    if (d < radius) best = Math.max(best, 1 - d / radius);
  }
  return Math.min(1, best);
}

/** Fog density (exp2 per u) at the centre of a dust cloud. */
export const DUST_FOG_DENSITY = 0.22;
/** Fog colour inside dust. */
export const DUST_FOG_COLOR: Rgb = [0.34, 0.31, 0.26];

/** Fog settings with the camera `dust` deep inside a dust cloud (0 = the base fog). */
export function dustFog(base: { fogDensity: number; fogColor: Rgb }, dust: number): { fogDensity: number; fogColor: Rgb } {
  const d = Math.max(0, Math.min(1, dust));
  if (d === 0) return { fogDensity: base.fogDensity, fogColor: base.fogColor };
  const mix = (a: number, b: number): number => a + (b - a) * d;
  return {
    fogDensity: mix(base.fogDensity, DUST_FOG_DENSITY),
    fogColor: [mix(base.fogColor[0], DUST_FOG_COLOR[0]), mix(base.fogColor[1], DUST_FOG_COLOR[1]), mix(base.fogColor[2], DUST_FOG_COLOR[2])],
  };
}
