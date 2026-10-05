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
import { shipMeshKind, wreckMeshKind, type MeshKind } from "../renderer/models/index.ts";
import { emptyScene, type Instance, type Rgb, type SceneInstances } from "../renderer/renderer.ts";
import { PAINT_COLORS, type Beam, type Flash } from "./mechanics.ts";
import { hidesMesh } from "./vfx.ts";
import { sideColor, TEAM_PALETTE } from "./sides.ts";
import {
  PROJ_ARMED,
  PROJ_STUCK,
  projectilePaintColor,
  TINKER_RADIUS,
  type EntityState,
  type ProjectileState,
  type ShipState,
  type VisualDef,
} from "./views.ts";

/** Hull colour per 0-based team (a darker shade of the paint colour); see game/sides.ts. */
export const TEAM_HULL: readonly Rgb[] = TEAM_PALETTE;

/**
 * Hull colour of a side: the team palette in team modes, a distinct colour
 * per ship in free-for-all (`teams` 0, side ids 0-253).
 */
export function teamHull(team: number, teams = 2): Rgb {
  return sideColor(team, teams);
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
  // "ship" and "wreck" resolve to the owner's / dead ship's hull (shipMeshKind, wreckMeshKind).
  decoy: { mesh: "ship", size: 1, emissive: 0, teamHull: true },
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

/** Opacity of a cloaked ship's shimmer (teammates, and enemies close enough to be sent). */
export const CLOAK_OPACITY = 0.22;
/** Shield bubble size relative to the ship's scale. */
export const SHIELD_SCALE = 1.7;

/** Shimmer opacity of a cloaked ship at `time` (s): a faint flickering outline. */
export function cloakShimmer(time: number, id: number): number {
  return CLOAK_OPACITY * (0.7 + 0.3 * Math.sin(time * 9 + phase(id)));
}

/**
 * Ships (living only), scaled by their type's hitbox. An active gadget
 * shows by its kind (`gadgetOf(id)`: "shield", "cloak", "decoy" or ""): a
 * shield bubble, a cloak shimmer (the ship drawn translucent and
 * flickering; the server only sends cloaked enemies that are close or just
 * fired), or a faint shell for unknown gadgets.
 */
export function addShips(
  scene: SceneInstances,
  ships: readonly ShipState[],
  gadgetOf: (id: number) => string = () => "",
  time = 0,
  teams = 2,
  vfx = false,
  refract = false,
  style?: ShipStyleFn,
): void {
  for (const s of ships) {
    if (!s.alive) continue;
    const scale = (s.radius || TINKER_RADIUS) / TINKER_RADIUS;
    const styled = style?.(s);
    const color = styled?.color ?? teamHull(s.team, teams);
    const gadget = s.gadgetActive ? gadgetOf(s.id) : "";
    const mesh = shipMeshKind(s.shipType);
    // The hull's livery takes the team colour; engines glow brighter while boosting.
    const hull: Instance = {
      position: s.position,
      rotation: s.orientation,
      scale,
      color,
      emissive: 0,
      glow: s.boosting ? 1 : 0,
      blink: phase(s.id),
    };
    if (styled?.keepColor) hull.keepColor = styled.keepColor;
    if (gadget === "cloak") {
      // A lighter shade of the team hull colour, so a cloaked teammate and a
      // cloaked enemy still read apart.
      const shimmer: Rgb = [0.5 + color[0] * 0.5, 0.5 + color[1] * 0.5, 0.5 + color[2] * 0.5];
      if (refract) {
        // VQP: drawn as a lens through the finished scene (renderer/cloak.ts).
        ((scene.refract ??= {})[mesh] ??= []).push({ ...hull, color: shimmer, emissive: 0, opacity: 1 });
      } else {
        push(scene, "translucent", mesh, { ...hull, color: shimmer, emissive: 0.9, opacity: cloakShimmer(time, s.id) });
      }
      continue;
    }
    push(scene, "opaque", mesh, hull);
    if (gadget === "shield") {
      push(scene, "translucent", "sphere", {
        position: s.position,
        rotation: q.fromAxisAngle(UP, time * 0.7),
        scale: scale * SHIELD_SCALE,
        color: [0.55, 0.85, 1],
        emissive: vfx ? 0.45 : 1,
        // With particles the hits ripple across it (game/vfx.ts), so the bubble itself stays faint.
        opacity: (vfx ? 0.1 : 0.22) + (vfx ? 0.03 : 0.06) * Math.sin(time * 5 + phase(s.id)),
      });
    } else if (s.gadgetActive) {
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

/** Leech Arc beams (stretched glowing bolts) and flashes / shockwave shells. */
export function addEffects(scene: SceneInstances, beams: readonly Beam[], flashes: readonly Flash[], time = 0): void {
  for (const b of beams) {
    const d = { x: b.to.x - b.from.x, y: b.to.y - b.from.y, z: b.to.z - b.from.z };
    const len = Math.hypot(d.x, d.y, d.z);
    if (len < 1e-3) continue;
    // A slightly wobbling arc: two thin strands.
    for (let k = 0; k < 2; k++) {
      const width = 0.12 + 0.05 * Math.sin(time * 40 + k * 2);
      const wobble = 0.15 * Math.sin(time * 55 + k * 3.1);
      push(scene, "translucent", "bolt", {
        position: { x: (b.from.x + b.to.x) / 2, y: (b.from.y + b.to.y) / 2 + wobble, z: (b.from.z + b.to.z) / 2 },
        rotation: alongVelocity(d),
        scale: width,
        stretch: len / width,
        color: b.color,
        emissive: 1,
        opacity: 0.8 * Math.max(0.2, b.strength),
      });
    }
    push(scene, "translucent", "sphere", {
      position: b.to,
      rotation: q.identity(),
      scale: 0.6,
      color: b.color,
      emissive: 1,
      opacity: 0.5 * b.strength,
    });
  }
  for (const f of flashes) {
    if (f.strength <= 0 || f.radius <= 0) continue;
    push(scene, "translucent", "sphere", {
      position: f.position,
      rotation: q.identity(),
      scale: f.radius * 2,
      color: f.color,
      emissive: 1,
      opacity: (f.ring ? 0.25 : 0.6) * f.strength,
    });
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
  teams = 2,
  vfx = false,
): void {
  for (const p of shots) {
    const visual = visuals[p.visual];
    // With particles (game/vfx.ts) energy shots are glowing cores with ribbon
    // trails instead of meshes, and nothing gets a mesh trail or glow shell.
    const meshless = vfx && hidesMesh(visual);
    const style = styleFor(visual);
    let color = visual?.color ?? WHITE;
    if (style.teamTint) {
      // Paint blobs show their effect colour (red, blue, yellow); without
      // one, the shooter's team colour.
      const paint = PAINT_COLORS[projectilePaintColor(p.state)];
      const team = teamOf(p.owner);
      if (projectilePaintColor(p.state) !== 0 && paint) color = paint;
      else if (team !== undefined) color = teamHull(team, teams);
    }
    const scale = style.size * (visual?.scale ?? 1);
    let rotation = alongVelocity(p.velocity);
    if (style.spin) rotation = q.mul(rotation, q.fromAxisAngle({ x: 0, y: 0, z: 1 }, time * style.spin));
    let emissive = style.emissive;
    if (p.state & PROJ_ARMED) emissive = Math.sin(time * 10) > 0 ? 1 : 0.2;
    if (!meshless) push(scene, "opaque", style.mesh, { position: p.position, rotation, scale, color, emissive });
    if (vfx) continue;
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

/** Decoy module state bit (state byte 1) while it rubber-bands back. */
export const DECOY_RETURNING = 0x80;

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
  hulls: EntityHulls = {},
  teams = 2,
  vfx = false,
): void {
  for (const e of entities) {
    if (e.kind === "marker" && !visuals[e.visual]) continue;
    const visual = visuals[e.visual];
    const style = styleFor(visual);
    // Decoys and ghosts show their owner's hull, wrecks the dead ship's.
    let mesh = style.mesh;
    let shipLike = false;
    if (mesh === "ship") {
      mesh = shipMeshKind(hulls.shipTypeOf?.(e.owner) ?? 1);
      shipLike = true;
    } else if (mesh === "wreck") {
      mesh = wreckMeshKind(hulls.wreckTypeOf?.(e) ?? 1);
    }
    let color = visual?.color ?? WHITE;
    if (style.teamHull) color = teamHull(e.team, teams);
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
    // A decoy on its way back (module state 0x80) glows hot.
    const returning = e.kind === "decoy" && (e.state[1] & DECOY_RETURNING) !== 0;
    if (returning) emissive = 0.6 + 0.4 * Math.sin(time * 20);
    const inst: Instance = { position, rotation, scale, color, emissive };
    if (shipLike) inst.blink = phase(e.owner);
    if (style.opacity !== undefined) {
      // Borrowed Time ghosts flicker.
      const opacity = e.kind === "ghost" ? style.opacity * (0.6 + 0.4 * Math.sin(time * 13 + ph)) : style.opacity;
      push(scene, "translucent", mesh, { ...inst, opacity });
    } else {
      push(scene, "opaque", mesh, inst);
    }
    // With particles, glow shells are soft sprites (game/vfx.ts).
    if (returning && !vfx) {
      push(scene, "translucent", "sphere", { position, rotation: q.identity(), scale: scale * 1.8, color, emissive: 1, opacity: 0.3 });
    }
    if (style.glow && !vfx) {
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

/** Which hull ship-shaped entities show (VQ4). */
export interface EntityHulls {
  /** Ship type of a ship id (decoys and ghosts take their owner's hull); default Tinker. */
  shipTypeOf?: ((id: number) => number | undefined) | undefined;
  /** Hull type of a wreck (game/wrecks.ts WreckTypes); default Tinker. */
  wreckTypeOf?: ((e: EntityState) => number) | undefined;
}

/** Per-ship colour override (Last Color Standing's colour meter, game/modeVisuals.ts); undefined = the side colour. */
export type ShipStyleFn = (s: ShipState) => { color?: Rgb; keepColor?: number } | undefined;

/** Optional extras for `buildScene` (M2 wave C mechanics). */
export interface SceneExtras extends EntityHulls {
  /** Gadget kind of a ship id ("shield", "cloak", "decoy", ""). */
  gadgetOf?: (id: number) => string;
  beams?: readonly Beam[];
  flashes?: readonly Flash[];
  /** Projectiles not to draw (e.g. the Eyeball the camera rides). */
  hideShot?: ((p: ProjectileState) => boolean) | undefined;
  /** Teams of the match (0 = free-for-all: every side its own colour); default 2. */
  teams?: number;
  /**
   * Particle effects draw the flashes, beams, trails, glow shells and energy
   * shot cores (game/vfx.ts); the scene then leaves those meshes out. Default
   * false (the pre-VQ5 mesh effects, e.g. with `?vfx=off`).
   */
  vfx?: boolean;
  /**
   * Cloaked ships bend the scene behind them (the renderer's refraction
   * pass, VQP) instead of a flickering translucent hull. Default false.
   */
  refractCloak?: boolean;
  /** Ship colour override (M3 C1: Last Color Standing). */
  shipStyle?: ShipStyleFn | null;
}

/** Builds the whole instance scene for one frame. */
export function buildScene(
  ships: readonly ShipState[],
  shots: readonly ProjectileState[],
  entities: readonly EntityState[],
  visuals: readonly VisualDef[],
  teamOf: (owner: number) => number | undefined,
  time: number,
  extras: SceneExtras = {},
): SceneInstances {
  const scene = emptyScene();
  const teams = extras.teams ?? 2;
  const vfx = extras.vfx ?? false;
  addShips(scene, ships, extras.gadgetOf, time, teams, vfx, extras.refractCloak ?? false, extras.shipStyle ?? undefined);
  const hide = extras.hideShot;
  addProjectiles(scene, hide ? shots.filter((p) => !hide(p)) : shots, visuals, teamOf, time, teams, vfx);
  // Ship types of the ships in view, unless the caller knows every player's.
  const shipTypeOf = extras.shipTypeOf ?? ((id: number) => ships.find((s) => s.id === id)?.shipType);
  addEntities(scene, entities, visuals, time, { shipTypeOf, wreckTypeOf: extras.wreckTypeOf }, teams, vfx);
  if (!vfx) addEffects(scene, extras.beams ?? [], extras.flashes ?? [], time);
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
