import assert from "node:assert/strict";
import { test } from "node:test";
import * as q from "../math/quat.ts";
import { packHeader, packVolumes, type ModeVolume } from "./modeFx.ts";
import { packInstances } from "./renderer.ts";
import {
  MODE_FX_HEADER_FLOATS,
  MODE_FX_KIND,
  MODE_FX_MAX_VOLUMES,
  MODE_FX_UNIFORM_FLOATS,
  MODE_FX_VOLUME_FLOATS,
  modeFxWgsl,
} from "./shaders/modeFx.ts";
import { INSTANCED_WGSL, TRANSLUCENT_WGSL } from "./shaders/objects.ts";
import { COMPOSITE_WGSL, KEEP_ALPHA } from "./shaders/post.ts";

test("mode fx uniform layout matches the WGSL structs", () => {
  const wgsl = modeFxWgsl(false);
  const fields = (name: string): string[] => {
    const m = new RegExp(`struct ${name} \\{([^}]*)\\}`).exec(wgsl);
    assert.ok(m, name);
    return (m[1] ?? "").split("\n").map((l) => l.replace(/\/\/.*$/, "").trim()).filter((l) => l.includes(":"));
  };
  const volume = fields("Volume");
  assert.ok(volume.every((l) => l.endsWith("vec4f,")));
  assert.equal(volume.length * 4, MODE_FX_VOLUME_FLOATS);
  const header = fields("ModeFx").filter((l) => l.endsWith("vec4f,"));
  assert.equal(header.length * 4, MODE_FX_HEADER_FLOATS);
  assert.match(wgsl, new RegExp(`vols: array<Volume, ${MODE_FX_MAX_VOLUMES}>`));
  assert.equal(MODE_FX_UNIFORM_FLOATS, MODE_FX_HEADER_FLOATS + MODE_FX_MAX_VOLUMES * MODE_FX_VOLUME_FLOATS);
  assert.match(modeFxWgsl(true), /texture_depth_multisampled_2d/);
  assert.match(wgsl, /texture_depth_2d/);
});

test("volumes pack kind, centre, radius, linear colour and parameters", () => {
  const out = new Float32Array(MODE_FX_UNIFORM_FLOATS);
  const vols: ModeVolume[] = [
    { kind: "wall", center: { x: 1, y: 2, z: 3 }, radius: 40, color: [1, 1, 1], intensity: 2, warn: 1, grind: 0 },
    { kind: "zone", center: { x: 0, y: 0, z: 0 }, radius: 15, color: [1, 0, 0], intensity: 1, film: 0.1, grid: 0.2, contour: 1.5, ring: 0.25, pulse: 2 },
    { kind: "beam", center: { x: 5, y: 0, z: 5 }, width: 0.4, height: 60, color: [0, 1, 0], intensity: 1 },
    { kind: "halo", center: { x: 0, y: 1, z: 0 }, radius: 1.2, color: [0, 0, 1], intensity: 1 },
  ];
  assert.equal(packVolumes(vols, out), 4);
  const at = (i: number, k: number): number => out[MODE_FX_HEADER_FLOATS + i * MODE_FX_VOLUME_FLOATS + k]!;
  assert.deepEqual([at(0, 0), at(0, 1), at(0, 2), at(0, 3)], [1, 2, 3, 40]);
  assert.equal(at(0, 8), MODE_FX_KIND.wall);
  assert.equal(at(0, 9), 1, "warn");
  assert.ok(Math.abs(at(0, 11) - 0.9) < 1e-6, "default band");
  assert.equal(at(1, 8), MODE_FX_KIND.zone);
  assert.ok(Math.abs(at(1, 13) - 0.25) < 1e-6, "ring fill");
  assert.equal(at(1, 14), 2, "pulse");
  assert.equal(at(2, 8), MODE_FX_KIND.beam);
  assert.equal(at(2, 9), 60, "height");
  assert.equal(at(3, 8), MODE_FX_KIND.halo);
  // A zone without a ring marks it with a negative fill.
  packVolumes([{ kind: "zone", center: { x: 0, y: 0, z: 0 }, radius: 15, color: [1, 0, 0], intensity: 1, film: 0.1, grid: 0.2, contour: 1.5 }], out);
  assert.equal(at(0, 13), -1);
  // Display colours are linearised (0.5 display ~ 0.214 linear).
  packVolumes([{ ...vols[3]!, color: [0.5, 0.5, 0.5] } as ModeVolume], out);
  assert.ok(Math.abs(at(0, 4) - 0.214) < 0.002);
  // At most MODE_FX_MAX_VOLUMES.
  assert.equal(packVolumes(Array.from({ length: 20 }, () => vols[3]!), out), MODE_FX_MAX_VOLUMES);
});

test("header carries the camera basis scaled by the half-FOV tangents", () => {
  const out = new Float32Array(MODE_FX_UNIFORM_FLOATS);
  packHeader({ position: { x: 1, y: 2, z: 3 }, orientation: q.identity(), fovY: Math.PI / 2, near: 0.05, far: 600 }, 2, 12, 0.01, 3, { width: 800, height: 400 }, 1, out);
  assert.deepEqual([...out.subarray(0, 4)], [1, 2, 3, 12]);
  // fov 90 deg: tan = 1 vertically, 2 horizontally at aspect 2.
  assert.ok(Math.abs(Math.hypot(out[4]!, out[5]!, out[6]!) - 2) < 1e-6);
  assert.ok(Math.abs(Math.hypot(out[8]!, out[9]!, out[10]!) - 1) < 1e-6);
  assert.ok(Math.abs(out[7]! - 0.05) < 1e-6 && out[11] === 600);
  assert.ok(Math.abs(out[15]! - 0.01) < 1e-6, "fog density");
  assert.deepEqual([...out.subarray(16, 20)], [3, 800, 400, 1]);
});

test("Last Color Standing keep mask: instances, object shader and composite agree", () => {
  // keepColor shares the opacity slot: 1 + keep for opaque instances.
  const base = { position: { x: 0, y: 0, z: 0 }, rotation: q.identity(), scale: 1, color: [1, 0, 0] as const, emissive: 0 };
  assert.equal(packInstances([{ ...base }])[13], 1);
  assert.equal(packInstances([{ ...base, keepColor: 1 }])[13], 2);
  assert.equal(packInstances([{ ...base, keepColor: 0.5 }])[13], 1.5);
  assert.equal(packInstances([{ ...base, opacity: 0.25 }])[13], 0.25, "translucent opacity unchanged");
  assert.ok(INSTANCED_WGSL.includes(`* ${KEEP_ALPHA}`), "opaque objects write the keep mask into alpha");
  assert.ok(!TRANSLUCENT_WGSL.includes(`* ${KEEP_ALPHA}`), "translucent objects do not");
  assert.ok(COMPOSITE_WGSL.includes(`/ ${KEEP_ALPHA}`), "the composite reads it back");
  // 64 keep levels fit in rgba16float's mantissa just above 1.
  assert.ok(KEEP_ALPHA <= 1 / 16 && KEEP_ALPHA >= 1 / 64);
});
