// Charge-up effect (VQP): tracking charges and what one frame draws.

import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import type { ParticleSpec } from "../renderer/particles.ts";
import { SPRITE } from "../renderer/spriteAtlas.ts";
import { CHARGE_FALLBACK_COLOR, chargeLight, chargeNow, ChargeTracker, eyeHeight, glowAt, MUZZLE_AHEAD, orbRadius, OWN_CHARGE_SIZE, type ChargeGlow } from "./chargeFx.ts";
import { CUE, type ChargeMeter, type Cue } from "./mechanics.ts";
import type { ProjectileState, ShipState, VisualDef } from "./views.ts";
import type { VfxSink } from "./vfxLibrary.ts";

class Recorder implements VfxSink {
  now = 0;
  density = 1;
  spawned: ParticleSpec[] = [];
  immediates: ParticleSpec[] = [];
  spawn(p: ParticleSpec): void {
    this.spawned.push(p);
  }
  immediate(p: ParticleSpec): void {
    this.immediates.push(p);
  }
}

const ZERO = { x: 0, y: 0, z: 0 };
const ship = (id: number, alive = true): ShipState => ({ id, team: 1, alive, boosting: false, position: { x: 0, y: 0, z: -10 }, orientation: q.identity(), shipType: 1, radius: 0.5, gadgetActive: false });
const shot = (owner: number, kind: number): ProjectileState => ({ kind, owner, position: ZERO, velocity: ZERO, visual: 0, state: 0 });
const cue = (kind: number, source: number, target: number, value: number): Cue => ({ kind, source, target, position: ZERO, value, tick: 0 });
const PURPLE: VisualDef = { shape: "orb", color: [0.8, 0.35, 1], scale: 1.6 };

function input(over: Partial<Parameters<ChargeTracker["glows"]>[0]> = {}): Parameters<ChargeTracker["glows"]>[0] {
  return { now: 0, player: 0, pose: null, ownRadius: 0.5, ownShipType: 1, own: null, ships: [ship(4)], shots: [], visualOf: () => PURPLE, ...over };
}

test("a remote charge grows from its cue and ends when the shot appears", () => {
  const t = new ChargeTracker();
  t.apply([cue(CUE.chargeStart, 4, 9, 120)], 1, 0, []);
  const half = t.glows(input({ now: 2 }));
  assert.equal(half.length, 1);
  assert.ok(Math.abs((half[0]?.fraction ?? 0) - 0.5) < 1e-9);
  assert.deepEqual(half[0]?.color, PURPLE.color);
  // At the muzzle: ahead of the ship along its aim.
  assert.ok(Math.abs((half[0]?.pos.z ?? 0) - (-10 - 0.5 - MUZZLE_AHEAD)) < 1e-9);
  assert.equal(t.glows(input({ now: 3.5 }))[0]?.fraction, 1);
  // Overcharge cue.
  t.apply([cue(CUE.overcharge, 4, 255, 0)], 3.6, 0, []);
  assert.equal(t.glows(input({ now: 3.7 }))[0]?.overcharge, true);
  // Released: a new projectile of the weapon from that ship.
  assert.equal(t.glows(input({ now: 3.8, shots: [shot(4, 9)] })).length, 0);
  assert.equal(t.active, 0);
});

test("remote charges end with the ship, after a timeout, and ignore the own cues", () => {
  const t = new ChargeTracker();
  t.apply([cue(CUE.chargeStart, 4, 9, 120), cue(CUE.chargeStart, 0, 9, 120)], 0, 0, []);
  assert.equal(t.active, 1, "the own charge comes from Game.charge, not cues");
  assert.equal(t.glows(input({ now: 0.5, ships: [ship(4, false)] })).length, 0);
  t.apply([cue(CUE.chargeStart, 4, 9, 120)], 0, 0, []);
  assert.equal(t.glows(input({ now: 7 })).length, 0, "timed out");
  // An earlier orb still in flight does not end a new charge.
  t.apply([cue(CUE.chargeStart, 4, 9, 120)], 0, 0, [shot(4, 9)]);
  assert.equal(t.glows(input({ now: 0.5, shots: [shot(4, 9)] })).length, 1);
});

test("the own charge sits at the nose below the eye and is smaller", () => {
  const t = new ChargeTracker();
  const own: ChargeMeter = { weapon: 9, fraction: 0.75, heldSecs: 1.5, fullSecs: 2, ready: true, autoRelease: false, overcharge: false };
  const pose = { position: ZERO, orientation: q.identity(), velocity: ZERO, alive: true };
  const g = t.glows(input({ pose, own, ships: [] }));
  assert.equal(g.length, 1);
  assert.equal(g[0]?.own, true);
  assert.equal(g[0]?.size, OWN_CHARGE_SIZE);
  assert.ok((g[0]?.pos.y ?? 0) < -0.1, "below the crosshair");
  assert.ok(Math.abs((g[0]?.pos.y ?? 0) + eyeHeight(1, 0.5)) < 1e-9);
  assert.equal(t.glows(input({ pose: { ...pose, alive: false }, own, ships: [] })).length, 0);
});

test("one frame draws core, orb, halo and ring, sparks stream in, full and overcharge add more", () => {
  const g: ChargeGlow = glowAt(4, ZERO, q.identity(), 0.5, ZERO, 0.4, false, CHARGE_FALLBACK_COLOR, 1, false);
  const sink = new Recorder();
  let carry = 0;
  for (let i = 0; i < 30; i++) carry = chargeNow(sink, () => 0.5, g, i / 60, 1 / 60, carry);
  assert.equal(sink.immediates.length, 30 * 4, "halo, orb, core, ring per frame");
  assert.ok(sink.immediates.some((p) => p.sprite === SPRITE.ring && p.orient === "plane"));
  // Inflow: (25 + 110 k) per second at k = 0.4, all aimed at the core.
  assert.ok(sink.spawned.length >= 30 && sink.spawned.length <= 40, `${sink.spawned.length}`);
  for (const p of sink.spawned) {
    const end = { x: p.pos.x + (p.vel?.x ?? 0) * p.life, y: p.pos.y + (p.vel?.y ?? 0) * p.life, z: p.pos.z + (p.vel?.z ?? 0) * p.life };
    assert.ok(Math.hypot(end.x - g.pos.x, end.y - g.pos.y, end.z - g.pos.z) < 1e-6, "arrives at the core");
  }
  const full = new Recorder();
  chargeNow(full, () => 0.5, { ...g, fraction: 1 }, 0, 1 / 60);
  assert.ok(full.immediates.some((p) => p.sprite === SPRITE.flare), "star flare at full charge");
  const over = new Recorder();
  chargeNow(over, () => 0.5, { ...g, fraction: 1, overcharge: true }, 0, 0.1);
  assert.ok(over.spawned.some((p) => (p.drag ?? 0) > 0), "sparks spit out when overcharged");
  assert.ok(orbRadius(1) > orbRadius(0.2));
});

test("the charge light grows with the charge", () => {
  const g = glowAt(4, ZERO, q.identity(), 0.5, ZERO, 0.1, false, CHARGE_FALLBACK_COLOR, 1, false);
  const a = chargeLight(g);
  const b = chargeLight({ ...g, fraction: 1 });
  assert.ok(b.intensity > a.intensity && (b.range ?? 0) > (a.range ?? 0));
  assert.deepEqual(a.position, g.pos);
});
