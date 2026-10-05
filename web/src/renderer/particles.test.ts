import assert from "node:assert/strict";
import { test } from "node:test";
import { IMMEDIATE_MAX, LONG_LIFE, LONG_SHARE, packFlags, packParticle, ParticleRing, type ParticleSpec } from "./particles.ts";
import { QUALITY_PRESETS } from "./quality.ts";
import { ORIENT, PARTICLE_FLOATS, PARTICLE_UNIFORM_FLOATS, particlesWgsl, PASS_KIND, PFLAG, PARTICLE_COMPOSITE_WGSL } from "./shaders/particles.ts";
import { bakeSprites, mipCount, mulberry32, SPRITE, SPRITE_LAYERS, spriteMips } from "./spriteAtlas.ts";

const spark = (over: Partial<ParticleSpec> = {}): ParticleSpec => ({
  pos: { x: 1, y: 2, z: 3 },
  vel: { x: 4, y: 5, z: 6 },
  life: 0.3,
  sprite: SPRITE.spark,
  color: [7, 3, 1],
  size: 0.1,
  ...over,
});

test("a particle packs into six vec4s in the shader's order", () => {
  const out = new Float32Array(PARTICLE_FLOATS);
  packParticle(out, 0, spark({ color1: [0.5, 0.25, 0], alpha: 0.75, alpha1: 0.25, size1: 0.4, drag: 2, stretch: 0.03, spin: 1.5, rot: 0.5, fadeIn: 0.25, t0: 9 }), 4);
  assert.deepEqual([...out.subarray(0, 4)], [1, 2, 3, 9], "position, t0");
  assert.deepEqual([...out.subarray(4, 8)].map((x) => Math.round(x * 100) / 100), [4, 5, 6, 0.3], "velocity, life");
  assert.deepEqual([...out.subarray(8, 12)], [7, 3, 1, 0.75]);
  assert.deepEqual([...out.subarray(12, 16)], [0.5, 0.25, 0, 0.25]);
  assert.deepEqual([...out.subarray(16, 19)].map((x) => Math.round(x * 100) / 100), [0.1, 0.4, 2]);
  assert.deepEqual([...out.subarray(20, 24)].map((x) => Math.round(x * 100) / 100), [0.03, 1.5, 0.5, 0.25]);
  // Defaults: spawn time = now, end colour = start colour, opacity 1 -> 0, end size = start size.
  packParticle(out, 0, spark(), 4);
  assert.equal(out[3], 4);
  assert.deepEqual([...out.subarray(12, 16)], [7, 3, 1, 0]);
  assert.equal(out[11], 1);
  assert.equal(out[17], out[16]);
});

test("flags are an exact integer: sprite, alpha blend, orientation, flame heat", () => {
  const f = packFlags({ sprite: SPRITE.smoke2, blend: "alpha", orient: "plane", hot: true });
  assert.ok(Number.isInteger(f) && f < 2 ** 24, "stored as an integer-valued float (no denormal bit-cast)");
  assert.equal(f & PFLAG.spriteMask, SPRITE.smoke2);
  assert.ok(f & PFLAG.alpha);
  assert.equal((f >> PFLAG.orientShift) & 3, ORIENT.plane);
  assert.ok(f & PFLAG.hot);
  assert.equal(packFlags({ sprite: SPRITE.glow }), 0, "additive camera-facing glow");
  assert.equal(Math.fround(f), f);
});

test("short- and long-lived particles fill separate rings, oldest first", () => {
  const ring = new ParticleRing(100);
  const long = Math.round(100 * LONG_SHARE);
  assert.equal(ring.long.capacity, long);
  assert.equal(ring.short.capacity, 100 - long);
  ring.now = 10;
  ring.spawn(spark({ life: LONG_LIFE + 1 }));
  for (let i = 0; i < 3; i++) ring.spawn(spark());
  assert.equal(ring.short.used, 3);
  assert.equal(ring.long.used, 1);
  assert.equal(ring.drawCount, 4);
  // Draw instances map to the short ring, then the long ring, then the immediates.
  assert.equal(ring.slotOf(0), 0);
  assert.equal(ring.slotOf(2), 2);
  assert.equal(ring.slotOf(3), ring.long.start);
  assert.equal(ring.slotOf(4), ring.capacity);
  assert.deepEqual(ring.takeDirty(), [[0, 3], [ring.long.start, 1]]);
  assert.deepEqual(ring.takeDirty(), [], "taken once");
  // A flood of sparks wraps the short ring but never evicts the smoke.
  for (let i = 0; i < 500; i++) ring.spawn(spark());
  assert.equal(ring.short.used, ring.short.capacity);
  assert.equal(ring.long.used, 1);
  assert.equal(ring.data[ring.long.start * PARTICLE_FLOATS + 7], Math.fround(LONG_LIFE + 1), "the long-lived particle is still there");
  const dirty = ring.takeDirty();
  assert.ok(dirty.every(([s, n]) => s >= 0 && n > 0 && s + n <= ring.short.capacity), JSON.stringify(dirty));
  assert.equal(dirty.reduce((a, [, n]) => a + n, 0), ring.short.capacity, "the whole short ring is re-uploaded once");
});

test("a wrapping ring uploads two ranges, live counts follow the clock, clear forgets everything", () => {
  const ring = new ParticleRing(10);
  const short = ring.short.capacity;
  ring.now = 0;
  for (let i = 0; i < short - 2; i++) ring.spawn(spark());
  ring.takeDirty();
  for (let i = 0; i < 4; i++) ring.spawn(spark());
  assert.deepEqual(ring.takeDirty(), [[short - 2, 2], [0, 2]]);
  assert.equal(ring.liveCount(0.1), short);
  assert.equal(ring.liveCount(0.5), 0, "all burnt out after their 0.3 s");
  ring.spawn(spark({ t0: 1 }));
  assert.equal(ring.liveCount(0.5), 0, "not born yet");
  ring.immediate(spark());
  assert.equal(ring.immCount, 1);
  assert.equal(ring.liveCount(0), short, "the immediate counts, the overwritten slot does not");
  ring.clear();
  assert.equal(ring.drawCount, 0);
  assert.equal(ring.liveCount(0.1), 0);
  assert.ok(ring.data.every((x) => x === 0));
});

test("immediates beyond IMMEDIATE_MAX are dropped, not written over the ring", () => {
  const ring = new ParticleRing(8);
  for (let i = 0; i < IMMEDIATE_MAX + 5; i++) ring.immediate(spark());
  assert.equal(ring.immCount, IMMEDIATE_MAX);
  assert.equal(ring.immDropped, 5);
  assert.equal(ring.data.length, (8 + IMMEDIATE_MAX) * PARTICLE_FLOATS);
});

test("the sprite atlas: soft round shapes that vanish at the quad edge, full mip chains", () => {
  const size = 32;
  const layers = bakeSprites(size);
  assert.equal(layers.length, SPRITE_LAYERS);
  for (const [i, img] of layers.entries()) {
    assert.ok(img, `layer ${i}`);
    assert.equal(img.r.length, size * size);
    // Corners are empty (no square edges), every value in 0..1.
    for (const k of [0, size - 1, size * (size - 1), size * size - 1]) assert.equal(img.r[k], 0, `layer ${i} corner`);
    assert.ok(img.r.every((x) => x >= 0 && x <= 1) && img.g.every((x) => x >= 0 && x <= 1));
  }
  const centre = (size / 2) * size + size / 2;
  const glow = layers[SPRITE.glow]!;
  assert.ok((glow.r[centre] ?? 0) > 0.8, "bright glow centre");
  const ring = layers[SPRITE.ring]!;
  assert.ok((ring.r[centre] ?? 1) < 0.2, "a ring is empty in the middle");
  const smoke = layers[SPRITE.smoke0]!;
  assert.notDeepEqual(smoke.r, layers[SPRITE.smoke1]!.r, "smoke variants differ");
  assert.deepEqual(bakeSprites(size)[SPRITE.smoke0]!.r, smoke.r, "the bake is deterministic");
  const mips = spriteMips(glow);
  assert.equal(mips.length, mipCount(size));
  assert.deepEqual(mips.map((m) => m.length), [32, 16, 8, 4, 2, 1].map((s) => s * s * 2));
  const rnd = mulberry32(1);
  const xs = Array.from({ length: 1000 }, rnd);
  assert.ok(xs.every((x) => x >= 0 && x < 1));
  assert.ok(Math.abs(xs.reduce((a, b) => a + b, 0) / xs.length - 0.5) < 0.05);
});

test("particle WGSL: entry points for particles, motes and the composite, both depth bindings", () => {
  const ms = particlesWgsl(true);
  const single = particlesWgsl(false);
  for (const src of [ms, single]) {
    assert.match(src, /fn vs_main\(/);
    assert.match(src, /fn vs_mote\(/);
    assert.match(src, /fn fs_main\(/);
    // Flags are read as an integer value, never bit-cast.
    assert.match(src, /u32\(p\.e\.w \+ 0\.5\)/);
    assert.doesNotMatch(src, /bitcast/);
  }
  assert.match(ms, /texture_depth_multisampled_2d/);
  assert.match(single, /var scene_depth: texture_depth_2d;/);
  assert.match(PARTICLE_COMPOSITE_WGSL, /fn fs_main/);
  assert.equal(PARTICLE_UNIFORM_FLOATS % 4, 0);
  assert.match(ms, new RegExp(`pu\\.pass_info\\.x == ${PASS_KIND.big}\\.0`));
});

test("quality presets size the particle system: high 32 k, medium 16 k, low 8 k without motes", () => {
  assert.equal(QUALITY_PRESETS.high.particles.capacity, 32768);
  assert.equal(QUALITY_PRESETS.high.particles.density, 1);
  assert.equal(QUALITY_PRESETS.high.particles.motes, 1500);
  assert.ok(QUALITY_PRESETS.medium.particles.capacity < QUALITY_PRESETS.high.particles.capacity);
  assert.ok(QUALITY_PRESETS.medium.particles.density < 1);
  assert.equal(QUALITY_PRESETS.low.particles.motes, 0);
  for (const q of Object.values(QUALITY_PRESETS)) assert.ok(q.particles.halfResPx > 0, `${q.level} draws big sprites at half resolution`);
});
