// Ship bay hull view (first-impression pass, FI): the four VQ4 hulls turning
// on their own canvas behind the ship cards (start screen picker and the
// in-game ship bay), lit like a studio shot: a key light from above and in
// front, a warm rim from behind, a cool fill from below, the engines and nav
// lights glowing (bloom). No particles: exhaust plumes would trail across the
// neighbouring cards at this scale.
//
// A second Renderer on the game renderer's GPU device draws it (no second
// device, shaders shared by the browser's cache). Each hull sits behind its
// card: `setSlots` names the card elements, and every frame the hulls are
// placed where the cards' boxes project (bayLayout), sized to fit them.
// Draws only while it is shown; `dispose` releases everything but the device.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import { ENGINE_LIGHT } from "../game/lightFx.ts";
import { buildScene } from "../game/scene.ts";
import type { ShipState } from "../game/views.ts";
import type { PointLight } from "../renderer/lights.ts";
import { Renderer, type CameraView, type Rgb } from "../renderer/renderer.ts";
import type { QualityLevel } from "../renderer/quality.ts";
import type { CardBackdrop, CardSlot } from "../ui/shipSelect.ts";

/** Camera of the bay: at the origin looking down -Z. */
export const BAY_FOV_Y = (34 * Math.PI) / 180;
/** Depth of the hulls in front of the camera (u). */
export const BAY_DEPTH = 7;

export interface BaySlot {
  shipType: number;
  /** The card (or a box inside it) the hull turns behind. */
  el: HTMLElement;
  selected: boolean;
  allowed: boolean;
  /** Team livery (0 orange, 1 blue). */
  team: number;
}

/** Where a hull goes: centre and display radius in world units. */
export interface BayPlacement {
  position: Vec3;
  radius: number;
}

/**
 * Places a hull behind a box of `w` x `h` px whose centre is (`cx`, `cy`) px
 * on a canvas of `cw` x `ch` px: the point at BAY_DEPTH that projects onto
 * the centre, with a radius that fills `fill` of the box's smaller half-extent.
 */
export function bayLayout(cx: number, cy: number, w: number, h: number, cw: number, ch: number, fill = 0.82): BayPlacement {
  const tanY = Math.tan(BAY_FOV_Y / 2);
  const aspect = cw / Math.max(1, ch);
  const ndcX = (cx / Math.max(1, cw)) * 2 - 1;
  const ndcY = 1 - (cy / Math.max(1, ch)) * 2;
  const unitsPerPx = (2 * BAY_DEPTH * tanY) / Math.max(1, ch);
  // Hulls are long: the length (about 2 radii) spans the width, the height is a fraction.
  const radius = Math.min((w / 2) * fill, (h / 2) * fill * 1.6) * unitsPerPx;
  return { position: { x: ndcX * BAY_DEPTH * tanY * aspect, y: ndcY * BAY_DEPTH * tanY, z: -BAY_DEPTH }, radius };
}

/** Turntable orientation: yaw about the vertical, tilted toward the camera, a gentle roll. */
export function turntable(time: number, index: number, selected: boolean): Quat {
  const yaw = time * (selected ? 0.75 : 0.45) + index * 1.9 + 0.6;
  const tilt = q.fromAxisAngle({ x: 1, y: 0, z: 0 }, 0.42);
  const roll = q.fromAxisAngle({ x: 0, y: 0, z: 1 }, 0.08 * Math.sin(time * 0.9 + index));
  return q.mul(tilt, q.mul(roll, q.fromAxisAngle({ x: 0, y: 1, z: 0 }, yaw)));
}

/** Studio lights around a hull at `p` of radius `r`: rim (warm, behind), fill (cool, below left), top kicker. */
export function studioLights(p: Vec3, r: number, selected: boolean): PointLight[] {
  const k = selected ? 1.3 : 1;
  const warm: Rgb = [1, 0.66, 0.38];
  const cool: Rgb = [0.4, 0.62, 1];
  // Intensity is radiance 1 u away; the lights sit about 2-3 radii out, so scale with r².
  return [
    { position: { x: p.x + r * 1.8, y: p.y + r * 1.2, z: p.z - r * 2.4 }, color: warm, intensity: 1.6 * k * r * r, range: r * 8 },
    { position: { x: p.x - r * 2.4, y: p.y - r * 1.4, z: p.z + r * 0.6 }, color: cool, intensity: 1.1 * k * r * r, range: r * 8 },
  ];
}

/** A soft engine light behind a turning hull (the game's engine light, scaled to the display size). */
export function engineGlow(p: Vec3, orientation: Quat, r: number, boosting: boolean): PointLight {
  const e = boosting ? ENGINE_LIGHT.boost : ENGINE_LIGHT.idle;
  const back = q.forward(orientation);
  return {
    position: { x: p.x - back.x * r * 1.5, y: p.y - back.y * r * 1.5, z: p.z - back.z * r * 1.5 },
    color: e.color,
    intensity: (boosting ? 0.5 : 0.3) * r * r,
    range: r * 4,
  };
}

export class ShipBayView {
  readonly canvas: HTMLCanvasElement;
  private readonly renderer: Renderer;
  private slots: BaySlot[] = [];
  private raf = 0;
  private start = performance.now();
  private disposed = false;
  /** Fixed shader and turntable time (s) for screenshots; null = the wall clock. */
  timeOverride: number | null = null;

  private constructor(canvas: HTMLCanvasElement, renderer: Renderer) {
    this.canvas = canvas;
    this.renderer = renderer;
    const s = renderer.settings;
    s.fogDensity = 0;
    s.fogColor = [0.025, 0.03, 0.045];
    s.sunDirection = { x: -0.45, y: 0.75, z: 0.55 };
    s.headlight = 0.35;
    s.skyHaze = 0;
    s.post = { ...s.post, vignette: 0.15, exposure: 1.45, bloomStrength: 0.9 };
  }

  /**
   * A bay view on a new canvas inside `host` (absolutely positioned, filling
   * it, under the host's other children), drawn with `device`.
   */
  static async create(host: HTMLElement, device: GPUDevice, quality: QualityLevel = "medium"): Promise<ShipBayView> {
    const canvas = document.createElement("canvas");
    canvas.className = "bay-canvas";
    host.prepend(canvas);
    try {
      const renderer = await Renderer.create(canvas, { device, quality, particles: false });
      return new ShipBayView(canvas, renderer);
    } catch (err) {
      canvas.remove();
      throw err;
    }
  }

  setSlots(slots: readonly BaySlot[]): void {
    this.slots = [...slots];
  }

  /** Draws every animation frame while shown. */
  show(visible: boolean): void {
    if (this.disposed) return;
    if (visible && !this.raf) {
      const tick = (now: number): void => {
        if (this.disposed || this.renderer.isLost) return;
        this.frame(now);
        this.raf = requestAnimationFrame(tick);
      };
      this.raf = requestAnimationFrame(tick);
    } else if (!visible && this.raf) {
      cancelAnimationFrame(this.raf);
      this.raf = 0;
    }
  }

  /** Renders one frame at `now` (ms). */
  frame(now: number): void {
    if (this.disposed) return;
    const time = this.timeOverride ?? (now - this.start) / 1000;
    const box = this.canvas.getBoundingClientRect();
    if (box.width < 2 || box.height < 2) return;
    const ships: ShipState[] = [];
    const lights: PointLight[] = [];
    this.slots.forEach((slot, i) => {
      const r = slot.el.getBoundingClientRect();
      if (r.width < 2 || r.height < 2) return;
      const place = bayLayout(r.left + r.width / 2 - box.left, r.top + r.height / 2 - box.top, r.width, r.height, box.width, box.height);
      const radius = place.radius * (slot.selected ? 1 : 0.94);
      const orientation = turntable(time, i, slot.selected);
      ships.push({
        id: 900 + i,
        team: slot.team,
        alive: true,
        boosting: slot.selected,
        position: place.position,
        orientation,
        shipType: slot.shipType,
        radius,
        gadgetActive: false,
      });
      lights.push(...studioLights(place.position, radius, slot.selected), engineGlow(place.position, orientation, radius, slot.selected));
    });
    const renderer = this.renderer;
    renderer.setScene(buildScene(ships, [], [], [], () => undefined, time));
    renderer.setLights(lights);
    if (this.timeOverride !== null) renderer.timeOverride = this.timeOverride;
    const view: CameraView = { position: { x: 0, y: 0, z: 0 }, orientation: q.identity(), fovY: BAY_FOV_Y, near: 0.1, far: 60 };
    renderer.render(view);
  }

  /** GPU time of the bay view (gallery). */
  gpuTimings(): ReturnType<Renderer["gpuTimings"]> {
    return this.renderer.gpuTimings();
  }

  dispose(): void {
    if (this.disposed) return;
    this.show(false);
    this.disposed = true;
    this.renderer.destroy();
    this.canvas.remove();
  }
}

/**
 * The in-game ship bay's hull view (ui/shipSelect.ts `CardBackdrop`): a
 * ShipBayView filling the panel, created on the first show, drawing only
 * while the panel shows. Hulls wear the own team's livery.
 */
export class PanelBay implements CardBackdrop {
  private readonly host: HTMLElement;
  private readonly device: GPUDevice;
  private readonly quality: QualityLevel;
  private view: ShipBayView | null = null;
  private creating = false;
  private disposed = false;
  private cards: readonly CardSlot[] | null = null;
  team: number;

  constructor(host: HTMLElement, device: GPUDevice, team = 0, quality: QualityLevel = "medium") {
    this.host = host;
    this.device = device;
    this.team = team;
    this.quality = quality;
  }

  /** The hull view once created (gallery). */
  get hullView(): ShipBayView | null {
    return this.view;
  }

  update(cards: readonly CardSlot[] | null): void {
    this.cards = cards;
    if (this.disposed) return;
    if (cards && !this.view && !this.creating) {
      this.creating = true;
      ShipBayView.create(this.host, this.device, this.quality)
        .then((v) => {
          this.creating = false;
          if (this.disposed) {
            v.dispose();
            return;
          }
          this.view = v;
          this.host.classList.add("has-bay");
          requestAnimationFrame(() => v.canvas.classList.add("live"));
          this.apply();
        })
        .catch((err: unknown) => {
          console.warn("ship bay view:", err);
        });
    }
    this.apply();
  }

  private apply(): void {
    const v = this.view;
    if (!v) return;
    const cards = this.cards;
    if (!cards) {
      v.show(false);
      return;
    }
    v.setSlots(cards.map((c) => ({ shipType: c.shipType, el: c.el, selected: c.selected, allowed: c.allowed, team: this.team })));
    v.show(true);
  }

  dispose(): void {
    this.disposed = true;
    this.view?.dispose();
    this.view = null;
  }
}
