import assert from "node:assert/strict";
import { test } from "node:test";
import { bracketPath, FIT_RING_FULL, FIT_RING_RANGE, fitRingModel, HIDDEN_RING, parseSweep, type FitRingInput } from "./fitRing.ts";
import { DEFAULT_VIEW, loadViewSettings, readViewSettings, saveViewSettings, VIEW_STORAGE_KEY, type KeyValueStore } from "./viewSettings.ts";

const FOV_Y = 1.2;
const H = 1080;
const FOCAL = H / 2 / Math.tan(FOV_Y / 2);

/** A contact straight ahead at `depth`, `lateral` off the axis. */
function input(depth: number, over: Partial<FitRingInput> = {}, lateral = 0): FitRingInput {
  return {
    contact: { travel: Math.max(0, depth - 0.5), point: { x: lateral, y: 0, z: -depth }, normal: { x: 0, y: 0, z: 1 } },
    aim: depth,
    eye: { x: 0, y: 0, z: 0 },
    forward: { x: 0, y: 0, z: -1 },
    radius: 0.5,
    fovY: FOV_Y,
    viewportHeight: H,
    ...over,
  };
}

test("sweep results parse; empty means a clear path", () => {
  assert.equal(parseSweep(new Float32Array(0)), null);
  assert.equal(parseSweep(undefined), null);
  const c = parseSweep([2, 1, 2, 3, 0, 0, 1]);
  assert.deepEqual(c, { travel: 2, point: { x: 1, y: 2, z: 3 }, normal: { x: 0, y: 0, z: 1 } });
});

test("the ring is the hull's radius projected at the contact's depth", () => {
  const m = fitRingModel(input(4));
  assert.ok(m.visible);
  assert.ok(Math.abs(m.radiusPx - (FOCAL * 0.5) / 4) < 1e-9);
  assert.equal(m.depth, 4);
  // A Hauler's ring is 1.3x the Tinker's at the same place.
  const big = fitRingModel(input(4, { radius: 0.65 }));
  assert.ok(Math.abs(big.radiusPx / m.radiusPx - 1.3) < 1e-9);
  // Only the depth along the view counts, not the contact's offset.
  assert.ok(Math.abs(fitRingModel(input(4, {}, 0.6)).radiusPx - m.radiusPx) < 1e-9);
});

test("the ring fades in within 12 u and is hidden beyond", () => {
  assert.deepEqual(fitRingModel(input(FIT_RING_RANGE + 0.5)), HIDDEN_RING);
  const far = fitRingModel(input(FIT_RING_RANGE - 1));
  const near = fitRingModel(input(FIT_RING_FULL - 1));
  assert.ok(far.visible && far.opacity > 0 && far.opacity < 1, `${far.opacity}`);
  assert.equal(near.opacity, 1);
  assert.deepEqual(fitRingModel({ ...input(4), contact: null }), HIDDEN_RING);
  // Pressed against the rock the ring would outgrow the screen: it fades away.
  assert.deepEqual(fitRingModel(input(0.6)), HIDDEN_RING);
});

test("blocked: the hull touches before the aim point (a gap too narrow)", () => {
  // Aiming through a hole: the ray goes on, the hull stops at the rim.
  assert.equal(fitRingModel(input(3, { aim: 9 })).state, "blocked");
  assert.equal(fitRingModel(input(3, { aim: null })).state, "blocked");
  // Aiming at a wall: the contact is the aimed rock.
  assert.equal(fitRingModel(input(3, { aim: 3.4 })).state, "surface");
});

test("bad input hides the ring", () => {
  assert.deepEqual(fitRingModel(input(4, { radius: 0 })), HIDDEN_RING);
  assert.deepEqual(fitRingModel(input(4, { viewportHeight: 0 })), HIDDEN_RING);
  // A contact behind the eye.
  assert.deepEqual(fitRingModel(input(-2)), HIDDEN_RING);
});

test("four corner brackets sit outside the ring", () => {
  const d = bracketPath(100);
  assert.equal(d.match(/M/g)?.length, 4);
  for (const n of d.match(/-?\d+(\.\d+)?/g) ?? []) assert.ok(Math.abs(Number(n)) >= 70 && Math.abs(Number(n)) <= 90, n);
});

// ----- view settings ------------------------------------------------------------------------

function memoryStore(init: Record<string, string> = {}): KeyValueStore & { data: Record<string, string> } {
  const data = { ...init };
  return {
    data,
    getItem: (k) => data[k] ?? null,
    setItem: (k, v) => {
      data[k] = v;
    },
  };
}

test("view settings default, clamp and survive broken storage", () => {
  assert.deepEqual(loadViewSettings(memoryStore()), DEFAULT_VIEW);
  assert.deepEqual(loadViewSettings(null), DEFAULT_VIEW);
  assert.deepEqual(loadViewSettings(memoryStore({ [VIEW_STORAGE_KEY]: "{not json" })), DEFAULT_VIEW);
  assert.deepEqual(readViewSettings({ fov: 200, hullFrame: "yes", fitRing: false }), { fov: 120, hullFrame: true, fitRing: false });
  const throwing: KeyValueStore = {
    getItem: () => {
      throw new Error("denied");
    },
    setItem: () => {
      throw new Error("denied");
    },
  };
  assert.deepEqual(loadViewSettings(throwing), DEFAULT_VIEW);
  assert.doesNotThrow(() => saveViewSettings({ fov: 90, hullFrame: false, fitRing: true }, throwing));
});

test("view settings round-trip through storage", () => {
  const store = memoryStore();
  saveViewSettings({ fov: 87, hullFrame: false, fitRing: false }, store);
  assert.deepEqual(JSON.parse(store.data[VIEW_STORAGE_KEY] ?? ""), { fov: 87, hullFrame: false, fitRing: false });
  assert.deepEqual(loadViewSettings(store), { fov: 87, hullFrame: false, fitRing: false });
});

test("view settings chosen without localStorage still reach the game on this page", () => {
  // Node has no localStorage, like a browser with site data blocked: the
  // start screen saves, the game's ScaleCues loads with the default store.
  assert.equal(typeof (globalThis as { localStorage?: unknown }).localStorage, "undefined");
  const chosen = { fov: DEFAULT_VIEW.fov + 7, hullFrame: !DEFAULT_VIEW.hullFrame, fitRing: !DEFAULT_VIEW.fitRing };
  saveViewSettings(chosen);
  assert.deepEqual(loadViewSettings(), readViewSettings(chosen));
});
