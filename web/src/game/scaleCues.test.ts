import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import { FIT_BLOCKED_MARGIN, HIDDEN_RING } from "../ui/fitRing.ts";
import { DEFAULT_VIEW } from "../ui/viewSettings.ts";
import { fovYFor } from "./camera.ts";
import { PROBE_EVERY_FRAMES, probeFitRing, ScaleCues, SWEEP_RANGE, type ProbeGame, type ScaleCueFrame } from "./scaleCues.ts";

const frame = (over: Partial<ScaleCueFrame> = {}): ScaleCueFrame => ({
  dt: 1 / 60,
  aspect: 16 / 9,
  viewportHeight: 1080,
  flying: true,
  boosting: false,
  ...over,
});

const ORIGIN = { x: 0, y: 0, z: 0 };

/** A wall 5 u ahead (-Z) that the fake probes report. */
function fakeGame(calls: string[] = []): ProbeGame {
  return {
    shipRadius: 0.65,
    sweepSphere: (ox, oy, oz, _dx, _dy, dz, r, max) => {
      calls.push(`sweep ${dz} ${r} ${max}`);
      return new Float32Array([5 - r, ox, oy, oz - 5, 0, 0, 1]);
    },
    raycast: (_ox, _oy, _oz, _dx, _dy, dz, max) => {
      calls.push(`ray ${dz} ${max}`);
      return 5;
    },
  };
}

test("the fit ring probes along the view with the own hull radius, every other frame", () => {
  const calls: string[] = [];
  const cues = new ScaleCues({ ...DEFAULT_VIEW });
  const m = cues.fitRing(frame(), fakeGame(calls), ORIGIN, q.identity(), 1.2);
  assert.ok(m.visible);
  assert.equal(m.state, "surface");
  assert.ok(Math.abs(m.depth - 5) < 1e-6);
  // The ray stops just past the blocked margin behind the contact.
  assert.deepEqual(calls, [`sweep -1 0.65 ${SWEEP_RANGE}`, `ray -1 ${5 + FIT_BLOCKED_MARGIN + 0.05}`]);
  // The next frame reuses the result; the one after probes again.
  assert.equal(cues.fitRing(frame(), fakeGame(calls), ORIGIN, q.identity(), 1.2), m);
  assert.equal(calls.length, 2);
  cues.fitRing(frame(), fakeGame(calls), ORIGIN, q.identity(), 1.2);
  assert.equal(calls.length, 2 + 2 * (PROBE_EVERY_FRAMES - 1));
});

test("no ring when switched off, not flying, or without the probes", () => {
  const off = new ScaleCues({ ...DEFAULT_VIEW, fitRing: false });
  assert.deepEqual(off.fitRing(frame(), fakeGame(), ORIGIN, q.identity(), 1.2), HIDDEN_RING);
  const on = new ScaleCues({ ...DEFAULT_VIEW });
  assert.deepEqual(on.fitRing(frame({ flying: false }), fakeGame(), ORIGIN, q.identity(), 1.2), HIDDEN_RING);
  assert.deepEqual(probeFitRing({}, ORIGIN, q.identity(), 0.5, 1.2, 1080), HIDDEN_RING);
  // A clear path: no contact.
  const clear: ProbeGame = { ...fakeGame(), sweepSphere: () => new Float32Array(0) };
  assert.deepEqual(probeFitRing(clear, ORIGIN, q.identity(), 0.5, 1.2, 1080), HIDDEN_RING);
});

test("the hull frame follows the setting and the own ship", () => {
  const on = new ScaleCues({ ...DEFAULT_VIEW });
  assert.deepEqual(on.hullFrame(frame({ boosting: true }), 3, 0.55, [1, 0, 0]), { shipType: 3, radius: 0.55, color: [1, 0, 0], glow: 1 });
  assert.equal(on.hullFrame(frame({ flying: false }), 3, 0.55, [1, 0, 0]), null);
  assert.equal(new ScaleCues({ ...DEFAULT_VIEW, hullFrame: false }).hullFrame(frame(), 1, 0.5, [1, 1, 1]), null);
});

test("the FOV follows the setting and kicks only while flying with boost", () => {
  const cues = new ScaleCues({ ...DEFAULT_VIEW, fov: 90 });
  assert.equal(cues.fovY(frame()), fovYFor(90, 16 / 9));
  // Boost without flying (spectating) does not kick.
  assert.equal(cues.fovY(frame({ boosting: true, flying: false, dt: 1 })), fovYFor(90, 16 / 9));
  const kicked = cues.fovY(frame({ boosting: true, dt: 1 }));
  assert.ok(Math.abs(kicked - fovYFor(90, 16 / 9, 4)) < 1e-12);
  cues.reset();
  assert.equal(cues.fovY(frame()), fovYFor(90, 16 / 9));
});
