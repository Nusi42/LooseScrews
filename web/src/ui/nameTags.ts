// Player names over ships: a pool of absolutely positioned labels, placed
// each frame by projecting a point above every living ship with the
// renderer's view-projection matrix. The model is a pure function so Node
// tests can check it.

import * as mat4 from "../math/mat4.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { PlayerState, ShipState } from "../game/views.ts";
import { sideCss } from "../game/sides.ts";

/** Labels fade out towards this distance (u) and are hidden beyond it. */
export const NAME_TAG_RANGE = 90;
/** Enemies further than this are not labelled (teammates always are, in range). */
export const ENEMY_TAG_RANGE = 45;
/** Monochrome levels (M4 B5, milestone-4-plan.md §3.4): grey enemies are labelled only this close, and only in sight. */
export const GREY_TAG_RANGE = 20;
/** Pigmented ships are shown to everyone to this distance, through walls. */
export const PIGMENT_TAG_RANGE = 160;

/**
 * Tag rules of a monochrome level: pigmented ships are marked for everyone
 * through walls; grey (unpigmented) enemies get a tag only within
 * GREY_TAG_RANGE with a clear line of sight (no through-wall indicator);
 * teammates keep their tags.
 */
export interface PigmentTagRules {
  pigmented: ReadonlySet<number>;
  /** True when nothing blocks the view from the camera to the point (ls-wasm raycast). */
  inSight: (p: Vec3) => boolean;
}

export interface ScreenPoint {
  /** CSS pixels from the left / top of the canvas. */
  x: number;
  y: number;
  /** Clip-space w (distance along the view axis). */
  depth: number;
}

/** Projects a world point to canvas pixels; null behind the camera or off screen. */
export function projectToScreen(viewProj: mat4.Mat4, p: Vec3, width: number, height: number): ScreenPoint | null {
  const [x, y, , w] = mat4.transformPoint(viewProj, p);
  if (w <= 1e-3) return null;
  const nx = x / w, ny = y / w;
  if (nx < -1.1 || nx > 1.1 || ny < -1.1 || ny > 1.1) return null;
  return { x: (nx * 0.5 + 0.5) * width, y: (1 - (ny * 0.5 + 0.5)) * height, depth: w };
}

export interface NameTag {
  id: number;
  name: string;
  team: number;
  x: number;
  y: number;
  /** 0..1 */
  opacity: number;
  teammate: boolean;
  /** Monochrome levels: the ship is pigmented (a coloured marker for everyone) or grey. */
  pigment?: boolean;
  grey?: boolean;
}

/**
 * Labels for the visible ships: living, in front of the camera, in range
 * (teammates to NAME_TAG_RANGE, enemies to ENEMY_TAG_RANGE), nearest first.
 */
export function nameTagModel(
  ships: readonly ShipState[],
  players: readonly PlayerState[],
  camera: Vec3,
  viewProj: mat4.Mat4,
  width: number,
  height: number,
  ownTeam: number,
  pigment: PigmentTagRules | null = null,
): NameTag[] {
  const out: Array<NameTag & { dist: number }> = [];
  for (const s of ships) {
    if (!s.alive) continue;
    const dist = Math.hypot(s.position.x - camera.x, s.position.y - camera.y, s.position.z - camera.z);
    const teammate = s.team === ownTeam;
    const pigmented = pigment?.pigmented.has(s.id) ?? false;
    let range = teammate ? NAME_TAG_RANGE : ENEMY_TAG_RANGE;
    if (pigment && pigmented) range = PIGMENT_TAG_RANGE;
    else if (pigment && !teammate) range = GREY_TAG_RANGE;
    if (dist > range) continue;
    if (pigment && !pigmented && !teammate && !pigment.inSight(s.position)) continue;
    const above = { x: s.position.x, y: s.position.y + s.radius * 1.8 + 0.3, z: s.position.z };
    const p = projectToScreen(viewProj, above, width, height);
    if (!p) continue;
    const player = players.find((pl) => pl.id === s.id);
    out.push({
      id: s.id,
      name: player?.name ?? `Player ${s.id + 1}`,
      team: s.team,
      x: p.x,
      y: p.y,
      opacity: Math.max(0.25, Math.min(1, 1.4 - dist / range)),
      teammate,
      ...(pigment ? { pigment: pigmented, grey: !pigmented } : {}),
      dist,
    });
  }
  out.sort((a, b) => a.dist - b.dist);
  return out.map(({ dist: _dist, ...tag }) => tag);
}

/** The DOM layer: reuses one element per label slot. */
export class NameTags {
  readonly root: HTMLElement;
  private readonly pool: HTMLElement[] = [];

  constructor(parent: HTMLElement) {
    this.root = document.createElement("div");
    this.root.className = "name-tags";
    parent.appendChild(this.root);
  }

  /** Places the labels; `teams` picks the side colours (0 = free-for-all). */
  update(tags: readonly NameTag[], teams = 2): void {
    while (this.pool.length < tags.length) {
      const el = document.createElement("div");
      el.className = "name-tag";
      this.root.appendChild(el);
      this.pool.push(el);
    }
    this.pool.forEach((el, i) => {
      const t = tags[i];
      if (!t) {
        el.style.display = "none";
        return;
      }
      el.style.display = "";
      if (el.textContent !== t.name) el.textContent = t.name;
      el.className = `name-tag team-${t.team}${t.teammate ? " mate" : ""}${t.pigment ? " pigment" : ""}${t.grey && !t.teammate ? " grey" : ""}`;
      el.style.setProperty("--side", sideCss(t.team, teams));
      el.style.transform = `translate(${t.x.toFixed(1)}px, ${t.y.toFixed(1)}px) translate(-50%, -100%)`;
      el.style.opacity = t.opacity.toFixed(2);
    });
  }

  dispose(): void {
    this.root.remove();
  }
}
