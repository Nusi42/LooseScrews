// Customer survey (M4 C2; sim: crates/ls-sim/src/events/customer_survey.rs):
// a teal call-centre drone with a headset holds a cream five-star panel
// ("How was your match today?") right in front of the leader's view. The
// stars twinkle in a wave, inviting a shot; the star the destroying shot
// goes through is the leader's rating: the panel tumbles away with that
// many stars lit, a fanfare (or a sad trombone) plays, and the rating goes
// into the results (MatchResults awards). Unanswered, it floats off.
//
// Entity (`ReplicatedKind::Drone`, role 2): position = the panel's centre,
// facing the leader (forward = toward the leader), radius = half the panel
// width, state `[role, 0, phase, hp percent]`. Outcome cue: the rating 1-5
// (source = the shooter, target = the leader) or 0 (no answer).

import * as q from "../../math/quat.ts";
import * as v3 from "../../math/vec3.ts";
import type { Vec3 } from "../../math/vec3.ts";
import type { Rgb } from "../../renderer/renderer.ts";
import { SPRITE } from "../../renderer/spriteAtlas.ts";
import { lin } from "../vfxLibrary.ts";
import type { EntityState } from "../views.ts";
import { attach, burst, damageSmoke, integrity, NONE, put, rng, ROLE, rotorSpin, type DrawCtx, type Pose, type VisitorInput, type VisitorOut } from "./visitorKit.ts";

export const SURVEY_EVENT = "customer_survey";

export const PANEL: Rgb = [0.86, 0.83, 0.76];
export const SURVEY_TEAL: Rgb = [0.12, 0.62, 0.64];
export const STAR_GOLD: Rgb = [1.0, 0.66, 0.06];
export const STAR_DIM: Rgb = [0.55, 0.53, 0.5];

/** Seconds a rated panel tumbles away before it is gone. */
export const TUMBLE_SECS = 1.8;

/** Panel-space x of star `k` (1-5): star 1 on the leader's left, which is the panel's +X (it faces the leader). */
export function starX(k: number): number {
  return 1 - (k - 1) * 0.5;
}

/** "★★★☆☆" for a rating. */
export function starsText(rating: number): string {
  const n = Math.max(0, Math.min(5, Math.round(rating)));
  return "★".repeat(n) + "☆".repeat(5 - n);
}

/** Brightness (emissive 0..1) of star `k` while the survey waits for an answer: a wave across the row. */
export function starWave(k: number, time: number): number {
  const t = (time * 0.9) % 1.6;
  const at = (k - 1) * 0.22;
  const d = Math.abs(t - at);
  return 0.35 + 0.65 * Math.max(0, 1 - d / 0.25);
}

/** The sound of a rating. */
export function ratingSound(rating: number): string {
  return rating <= 2 ? "visitor.ratingBad" : rating === 3 ? "visitor.ratingMeh" : "visitor.ratingGood";
}

interface Tumble {
  pose: Pose;
  vel: Vec3;
  spin: Vec3;
  rating: number;
  born: number;
}

/** The survey's looks: the drone and its panel, the rated panels tumbling away. */
export class SurveyFx {
  private last = new Map<number, Pose>();
  private seen = new Set<number>();
  private tumbles: Tumble[] = [];

  reset(): void {
    this.last.clear();
    this.seen.clear();
    this.tumbles = [];
  }

  /** The own pilot is being surveyed (the HUD prompt). */
  ownSurveyed(i: VisitorInput): boolean {
    return i.mood.eventId === SURVEY_EVENT && i.mood.phase === "active" && i.mood.target === i.own && i.entities.some((e) => e.kind === "drone" && e.state[0] === ROLE.survey);
  }

  update(i: VisitorInput, ctx: DrawCtx, out: VisitorOut): void {
    for (const o of i.outcomes) {
      if (o.event !== SURVEY_EVENT) continue;
      const leader = o.target !== NONE ? i.name(o.target) : "The leader";
      if (o.byte >= 1 && o.byte <= 5) {
        // The last drawn panel (the drone is gone already) tumbles away with the rating lit.
        const from = [...this.last.values()].pop() ?? { position: o.position, rotation: q.identity(), scale: 1 };
        this.tumbles.push({
          pose: from,
          vel: v3.add(v3.scale(ctx.up, 2.5), v3.scale(q.forward(from.rotation), -1.5)),
          spin: { x: 2.2, y: 0.6, z: 1.4 },
          rating: o.byte,
          born: i.time,
        });
        rated(ctx, from.position, o.byte, Math.floor(i.time * 389));
        out.sounds.push({ id: ratingSound(o.byte), position: null });
        const by = o.source !== NONE ? (o.source === i.own ? "You" : i.name(o.source)) : "Someone";
        out.toasts.push({
          event: SURVEY_EVENT,
          text: o.target === i.own ? `Your match was rated by ${by === "You" ? "yourself" : by}:` : `${by} rated ${leader}'s match:`,
          own: o.target === i.own || o.source === i.own,
          stars: o.byte,
        });
      } else if (o.byte === 0) {
        out.toasts.push({ event: SURVEY_EVENT, text: o.target === i.own ? "No answer? Your feedback matters to us." : `Nobody answered ${leader}'s survey.`, own: o.target === i.own });
      }
    }
    const live = new Set<number>();
    for (const d of i.entities) {
      if (d.kind !== "drone" || d.state[0] !== ROLE.survey) continue;
      live.add(d.id);
      if (!this.seen.has(d.id)) {
        this.seen.add(d.id);
        out.sounds.push({ id: "visitor.survey", position: d.position });
        // The surveyed pilot gets the HUD prompt instead.
        if (i.mood.target !== i.own) out.toasts.push({ event: SURVEY_EVENT, text: `${i.name(i.mood.target)} is being surveyed. Shoot a star to rate them!`, own: false });
      }
      this.draw(i, ctx, d);
    }
    for (const id of [...this.last.keys()]) if (!live.has(id)) this.last.delete(id);
    this.drawTumbles(i, ctx);
  }

  private draw(i: VisitorInput, ctx: DrawCtx, d: EntityState): void {
    // The panel sits exactly where the sim puts it (shots land on it); the drone above bobs.
    const panel: Pose = { position: d.position, rotation: d.rotation, scale: 1 };
    this.last.set(d.id, panel);
    drawPanel(ctx, panel, (k) => starWave(k, i.time + d.id * 0.1));
    const bob = Math.sin(i.time * 2.4 + d.id) * 0.04;
    const body = attach(panel, { x: 0, y: 0.95 + bob, z: 0.04 });
    put(ctx, "survey_drone", body, SURVEY_TEAL);
    put(ctx, "visitor_rotor", attach(body, { x: -0.52, y: 0.15, z: 0.05 }, rotorSpin(i.time, 0), 0.32), [0.1, 0.1, 0.11]);
    put(ctx, "visitor_rotor", attach(body, { x: 0.52, y: 0.15, z: 0.05 }, rotorSpin(i.time, 1), 0.32), [0.1, 0.1, 0.11]);
    damageSmoke(ctx.sink, body.position, integrity(d), i.dt, rng(Math.floor(i.time * 60) + d.id));
    ctx.lights.push({ position: attach(panel, { x: 0, y: 0, z: -1.2 }).position, color: [1, 0.9, 0.7], intensity: 1.6, range: 5 });
  }

  private drawTumbles(i: VisitorInput, ctx: DrawCtx): void {
    this.tumbles = this.tumbles.filter((t) => i.time - t.born < TUMBLE_SECS);
    for (const t of this.tumbles) {
      const age = Math.max(0, i.time - t.born);
      const pos = v3.add(v3.add(t.pose.position, v3.scale(t.vel, age)), v3.scale(ctx.up, -4.5 * age * age));
      const spin = q.mul(q.fromAxisAngle({ x: 1, y: 0, z: 0 }, t.spin.x * age), q.fromAxisAngle({ x: 0, y: 0, z: 1 }, t.spin.z * age));
      const shrink = age > TUMBLE_SECS - 0.4 ? Math.max(0.01, (TUMBLE_SECS - age) / 0.4) : 1;
      const pose: Pose = { position: pos, rotation: q.normalize(q.mul(t.pose.rotation, spin)), scale: shrink };
      drawPanel(ctx, pose, (k) => (k <= t.rating ? 1 : 0), t.rating);
    }
  }
}

/** The panel with its five stars; `glow(k)` lights star k (1-5); a rated panel greys out the stars above the rating. */
function drawPanel(ctx: DrawCtx, panel: Pose, glow: (k: number) => number, rating = 0): void {
  put(ctx, "survey_panel", panel, PANEL);
  for (let k = 1; k <= 5; k++) {
    const lit = glow(k);
    const dim = rating > 0 && k > rating;
    put(ctx, "survey_star", attach(panel, { x: starX(k), y: -0.07, z: -0.04 }, q.identity(), 0.95 + 0.25 * lit * (rating > 0 ? 1 : 0.3)), dim ? STAR_DIM : STAR_GOLD, { emissive: dim ? 0 : 0.12 + 0.5 * lit });
  }
}

/** Stars (and confetti for a good rating, grey dust for a bad one) bursting where the panel was. */
function rated(ctx: DrawCtx, at: Vec3, rating: number, seed: number): void {
  const r = rng(seed);
  burst(ctx.sink, 6 * rating, (_, rnd) => ({
    pos: at,
    vel: { x: (rnd() - 0.5) * 6, y: rnd() * 5, z: (rnd() - 0.5) * 6 },
    life: 0.9 + rnd() * 0.5,
    sprite: SPRITE.flare,
    color: lin(STAR_GOLD, 7),
    size: 0.3,
    size1: 0.05,
    drag: 1.5,
    spin: (rnd() - 0.5) * 6,
  }), r);
  if (rating >= 4) {
    const colours: Rgb[] = [[1, 0.3, 0.4], [0.3, 0.8, 1], [1, 0.85, 0.2], [0.5, 1, 0.4], [0.85, 0.4, 1]];
    burst(ctx.sink, 60, (k, rnd) => ({
      pos: at,
      vel: { x: (rnd() - 0.5) * 10, y: rnd() * 8, z: (rnd() - 0.5) * 10 },
      life: 1.6 + rnd(),
      sprite: SPRITE.shard,
      color: colours[k % colours.length] ?? STAR_GOLD,
      alpha: 1,
      alpha1: 0.6,
      size: 0.09,
      drag: 2.4,
      blend: "alpha",
      spin: (rnd() - 0.5) * 16,
    }), r);
  } else if (rating <= 2) {
    burst(ctx.sink, 8, (_, rnd) => ({
      pos: v3.add(at, { x: (rnd() - 0.5) * 1.2, y: 0.3, z: (rnd() - 0.5) * 1.2 }),
      vel: { x: 0, y: 0.3, z: 0 },
      life: 1.8,
      sprite: SPRITE.smoke3,
      color: [0.35, 0.36, 0.4],
      alpha: 0.55,
      alpha1: 0,
      size: 0.5,
      size1: 1.6,
      blend: "alpha",
    }), r);
  }
}
