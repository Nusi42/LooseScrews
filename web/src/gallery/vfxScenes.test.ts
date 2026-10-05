import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import type { VfxSink } from "../game/vfxLibrary.ts";
import type { VisualDef } from "../game/views.ts";
import { ParticleRing, type ParticleSpec } from "../renderer/particles.ts";
import { QUALITY_PRESETS } from "../renderer/quality.ts";
import { LINEUP_RADII } from "./gallery.ts";
import { sceneById } from "./scenes.ts";
import { momentAt, simulateVfx } from "./vfxScenes.ts";

const cam = { position: { x: 0, y: 0, z: 0 }, orientation: q.identity() };
const visuals: VisualDef[] = [
  { shape: "bolt", color: [0.47, 1, 0.55], scale: 0.8 },
  { shape: "rocket", color: [1, 0.55, 0.23], scale: 1 },
];
const impact = { x: 4, y: -0.5, z: -16 };

/** The renderer's ring at "high" as a sink (what the gallery page feeds). */
function ringSink(): VfxSink & { ring: ParticleRing } {
  const ring = new ParticleRing(QUALITY_PRESETS.high.particles.capacity);
  return {
    ring,
    density: 1,
    get now() {
      return ring.now;
    },
    set now(t: number) {
      ring.now = t;
    },
    spawn: (p: ParticleSpec) => ring.spawn(p),
    immediate: (p: ParticleSpec) => ring.immediate(p),
    beginFrame: () => {
      ring.immCount = 0;
    },
  };
}

test("the fight's history: bolts fired, the own rocket launched and exploded, a shield hit", () => {
  const scene = sceneById("fight")!;
  const sink = ringSink();
  const vfx = simulateVfx(sink, scene, cam, visuals, impact, LINEUP_RADII, (id) => (id === 2 ? "shield" : ""));
  assert.ok(vfx);
  assert.ok(vfx.counts.muzzles >= 3, `muzzles ${vfx.counts.muzzles}`);
  assert.equal(vfx.counts.explosions, 1);
  assert.equal(vfx.counts.ripples, 1);
  assert.equal(sink.now, scene.time, "the particle clock ends at the scene's time");
  assert.ok(sink.ring.liveCount() > 100);
  // The shots at the scene's time are the ones gallery.ts draws as meshes.
  const m = momentAt(scene, cam, visuals, impact, LINEUP_RADII, scene.time)!;
  assert.equal(m.shots.filter((s) => s.visual === 0).length, 2);
  assert.equal(m.shots.filter((s) => s.visual === 1).length, 1);
});

test("the explosion scene: one ship death, the other ship smokes; scenes without content simulate nothing", () => {
  const sink = ringSink();
  const vfx = simulateVfx(sink, sceneById("explosion")!, cam, visuals, impact, LINEUP_RADII);
  assert.equal(vfx?.counts.deaths, 1);
  assert.equal(simulateVfx(ringSink(), sceneById("cave-tunnel")!, cam, visuals, impact, LINEUP_RADII), null);
});

test("the stress scene keeps about 20 k particles alive at 'high' (the plan's particle budget load)", () => {
  const sink = ringSink();
  simulateVfx(sink, sceneById("particles-stress")!, cam, visuals, impact, LINEUP_RADII);
  const live = sink.ring.liveCount();
  assert.ok(live >= 18_000 && live <= 32_768, `${live} live`);
  // Long-lived smoke is not evicted by the flood of trail sections.
  assert.ok(sink.ring.long.used > 1000);
});
