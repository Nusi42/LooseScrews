// Player names over ships: a pool of absolutely positioned labels, placed
// each frame by projecting a point above every living ship with the
// renderer's view-projection matrix. The model is a pure function so Node
// tests can check it.

import * as mat4 from "../math/mat4.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { PlayerState, ShipState } from "../game/views.ts";

/** Labels fade out towards this distance (u) and are hidden beyond it. */
export const NAME_TAG_RANGE = 90;
/** Enemies further than this are not labelled (teammates always are, in range). */
export const ENEMY_TAG_RANGE = 45;

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
): NameTag[] {
  const out: Array<NameTag & { dist: number }> = [];
  for (const s of ships) {
    if (!s.alive) continue;
    const dist = Math.hypot(s.position.x - camera.x, s.position.y - camera.y, s.position.z - camera.z);
    const teammate = s.team === ownTeam;
    if (dist > (teammate ? NAME_TAG_RANGE : ENEMY_TAG_RANGE)) continue;
    const above = { x: s.position.x, y: s.position.y + s.radius * 1.8 + 0.3, z: s.position.z };
    const p = projectToScreen(viewProj, above, width, height);
    if (!p) continue;
    const player = players.find((pl) => pl.id === s.id);
    const range = teammate ? NAME_TAG_RANGE : ENEMY_TAG_RANGE;
    out.push({
      id: s.id,
      name: player?.name ?? `Player ${s.id + 1}`,
      team: s.team,
      x: p.x,
      y: p.y,
      opacity: Math.max(0.25, Math.min(1, 1.4 - dist / range)),
      teammate,
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

  update(tags: readonly NameTag[]): void {
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
      el.className = `name-tag team-${t.team}${t.teammate ? " mate" : ""}`;
      el.style.transform = `translate(${t.x.toFixed(1)}px, ${t.y.toFixed(1)}px) translate(-50%, -100%)`;
      el.style.opacity = t.opacity.toFixed(2);
    });
  }

  dispose(): void {
    this.root.remove();
  }
}
