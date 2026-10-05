// Fit ring (visual-quality-plan.md VQ1): a thin ring around the crosshair
// that shows the own hull's cross-section where it would first touch rock
// flying straight ahead. ls-wasm sweeps the ship's sphere along the view
// (`Game.sweepSphere`, measured like the simulation's collision) and casts
// one ray (`Game.raycast`, what the crosshair points at). The ring is the
// ship's radius projected at the depth of that contact, so it is the size
// of the ship at the opening: a ring bigger than the hole means you won't
// fit. It turns amber when the hull would touch something before the point
// you aim at (a gap too narrow, or a wall you would scrape), and fades in
// only within FIT_RING_RANGE of the contact.

import type { Vec3 } from "../math/vec3.ts";

/** The ring shows only for contacts closer than this (u)... */
export const FIT_RING_RANGE = 12;
/** ...and is fully visible from this distance in (u). */
export const FIT_RING_FULL = 8;
/** The aim point lies this much (u) beyond the contact: the hull is blocked before it. */
export const FIT_BLOCKED_MARGIN = 0.6;

/** `Game.sweepSphere` result: travel, contact point and normal. */
export interface SweepContact {
  travel: number;
  point: Vec3;
  normal: Vec3;
}

/** Parses the flat `[travel, px, py, pz, nx, ny, nz]` array (empty = clear). */
export function parseSweep(a: ArrayLike<number> | undefined | null): SweepContact | null {
  if (!a || a.length < 7) return null;
  const n = (i: number): number => a[i] ?? 0;
  return { travel: n(0), point: { x: n(1), y: n(2), z: n(3) }, normal: { x: n(4), y: n(5), z: n(6) } };
}

export interface FitRingInput {
  /** Sweep contact, or null when the path is clear within range. */
  contact: SweepContact | null;
  /** Distance to the rock under the crosshair, or null when it lies beyond the contact (the ray stops there). */
  aim: number | null;
  /** Eye position and unit view direction. */
  eye: Vec3;
  forward: Vec3;
  /** Own hull radius (u). */
  radius: number;
  /** Vertical field of view (rad) and viewport height (CSS px). */
  fovY: number;
  viewportHeight: number;
}

export type FitState = "surface" | "blocked";

export interface FitRingModel {
  visible: boolean;
  /** Ring radius in CSS px. */
  radiusPx: number;
  opacity: number;
  state: FitState;
  /** Depth of the contact along the view (u). */
  depth: number;
}

export const HIDDEN_RING: FitRingModel = { visible: false, radiusPx: 0, opacity: 0, state: "surface", depth: 0 };

const smoothstep = (e0: number, e1: number, x: number): number => {
  const t = Math.min(1, Math.max(0, (x - e0) / (e1 - e0)));
  return t * t * (3 - 2 * t);
};

export function fitRingModel(i: FitRingInput): FitRingModel {
  const c = i.contact;
  if (!c || !(i.radius > 0) || !(i.viewportHeight > 0)) return HIDDEN_RING;
  const f = i.forward;
  const depth = (c.point.x - i.eye.x) * f.x + (c.point.y - i.eye.y) * f.y + (c.point.z - i.eye.z) * f.z;
  if (!(depth > 0) || depth > FIT_RING_RANGE) return HIDDEN_RING;
  // A disc of the hull's radius facing the camera at the contact's depth.
  const focal = i.viewportHeight / 2 / Math.tan(i.fovY / 2);
  const radiusPx = (focal * i.radius) / Math.max(depth, 0.05);
  // Fade in with distance; fade out again once the ring outgrows the screen
  // (pressed against a wall it says nothing new).
  const h = i.viewportHeight;
  const opacity = smoothstep(FIT_RING_RANGE, FIT_RING_FULL, depth) * (1 - smoothstep(0.3 * h, 0.55 * h, radiusPx));
  if (opacity <= 0.01) return HIDDEN_RING;
  const blocked = i.aim === null || i.aim > depth + FIT_BLOCKED_MARGIN;
  return { visible: true, radiusPx, opacity, state: blocked ? "blocked" : "surface", depth };
}

const SVG_NS = "http://www.w3.org/2000/svg";

/**
 * The ring in the HUD: a full-screen SVG with a thin circle and four
 * corner brackets, centred on the crosshair. Only attributes change per
 * frame (no layout).
 */
export class FitRing {
  readonly root: SVGSVGElement;
  private readonly group: SVGGElement;
  private readonly circle: SVGCircleElement;
  private readonly ticks: SVGPathElement;
  private last = "";

  constructor(parent: HTMLElement) {
    this.root = document.createElementNS(SVG_NS, "svg");
    this.root.setAttribute("class", "fit-ring");
    this.root.setAttribute("aria-hidden", "true");
    this.group = document.createElementNS(SVG_NS, "g");
    this.circle = document.createElementNS(SVG_NS, "circle");
    this.circle.setAttribute("class", "fit-ring-circle");
    this.ticks = document.createElementNS(SVG_NS, "path");
    this.ticks.setAttribute("class", "fit-ring-ticks");
    this.group.append(this.circle, this.ticks);
    this.root.appendChild(this.group);
    parent.appendChild(this.root);
    this.update(HIDDEN_RING);
  }

  update(m: FitRingModel): void {
    const r = Math.round(m.radiusPx * 2) / 2;
    const key = m.visible ? `${r}|${m.opacity.toFixed(2)}|${m.state}` : "hidden";
    if (key === this.last) return;
    this.last = key;
    if (!m.visible) {
      this.group.setAttribute("opacity", "0");
      return;
    }
    this.group.setAttribute("opacity", m.opacity.toFixed(2));
    this.group.setAttribute("class", `fit-ring-${m.state}`);
    this.circle.setAttribute("r", String(r));
    this.ticks.setAttribute("d", bracketPath(r));
  }

  dispose(): void {
    this.root.remove();
  }
}

/**
 * Four short brackets on the diagonals just outside the circle (relative to
 * the centre, which the SVG's transform puts on the crosshair).
 */
export function bracketPath(r: number): string {
  const out = r + 3;
  const len = Math.min(10, Math.max(4, r * 0.25));
  const parts: string[] = [];
  for (const [sx, sy] of [[1, 1], [-1, 1], [-1, -1], [1, -1]] as const) {
    const a = Math.PI / 4;
    const x = sx * Math.cos(a) * out;
    const y = sy * Math.sin(a) * out;
    // An L-shaped corner mark pointing at the ring.
    parts.push(`M${(x + sx * len).toFixed(1)} ${y.toFixed(1)}H${x.toFixed(1)}V${(y + sy * len).toFixed(1)}`);
  }
  return parts.join("");
}
