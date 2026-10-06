// The comedy event props of M4 C1 (eventProps.ts) and the toy palette.

import { test } from "node:test";
import assert from "node:assert/strict";
import { validateMesh } from "../mesh.ts";
import { signedVolume } from "./kit.ts";
import { balloonFaceModel, balloonModel, boxFlapModel, cardboardBoxModel, coffeeMugModel, giantEggModel, pupilModel, rubberChickenModel, tapeStripModel } from "./eventProps.ts";
import { MESH_BUILDERS } from "./index.ts";
import { TOY_MATERIALS, TOY_PART_BASE, TOY_WGSL, toyMaterialOf, toyPart } from "./toyPalette.ts";
import { MODEL_SHADING_WGSL } from "./shading.ts";

test("toy parts: one palette entry per part, every one in the shader", () => {
  assert.equal(toyPart("instance"), TOY_PART_BASE);
  assert.equal(toyMaterialOf(toyPart("cardboard"))?.name, "cardboard");
  assert.equal(toyMaterialOf(3), null);
  assert.ok(TOY_PART_BASE + TOY_MATERIALS.length <= 254, "below PART_PLAIN");
  for (let i = 0; i < TOY_MATERIALS.length; i++) assert.ok(TOY_WGSL.includes(`case ${i}u`), `entry ${i}`);
  assert.ok(MODEL_SHADING_WGSL.includes("shade_toy("));
  assert.throws(() => toyPart("nope"));
});

test("event props: valid, wound outward, in toy materials, the size the visuals assume", () => {
  const props = [
    ["chicken", rubberChickenModel(), ["rubberYellow", "rubberRed", "rubberOrange", "glossBlack"], 0.5, 0.8],
    ["giant_egg", giantEggModel(), ["egg", "speckle"], 0.5, 0.65],
    ["moving_box", cardboardBoxModel(), ["cardboard", "cardboardPrint", "stampRed", "tape"], 0.8, 0.9],
    ["box_flap", boxFlapModel(), ["cardboard"], 0.6, 0.75],
    ["box_tape", tapeStripModel(), ["tape"], 0.9, 1.01],
    ["balloon", balloonModel(), ["instance"], 1.1, 1.2],
    ["balloon_face", balloonFaceModel(), ["glossWhite", "glossBlack", "pink"], 1, 1.2],
    ["pupil", pupilModel(), ["glossBlack"], 0.08, 0.1],
    ["coffee_mug", coffeeMugModel(), ["glossWhite", "mugBlue", "coffee", "rubberRed"], 0.6, 0.75],
  ] as const;
  for (const [name, model, materials, rMin, rMax] of props) {
    const mesh = model.build();
    validateMesh(mesh);
    assert.ok(signedVolume(model.tris) > 0, `${name} wound outward`);
    for (const m of materials) assert.ok(model.tris.some((t) => t.part === toyPart(m)), `${name} has ${m}`);
    const r = model.maxRadius();
    assert.ok(r >= rMin && r <= rMax, `${name} radius ${r}`);
    assert.ok(mesh.indices.length / 3 < 6000, `${name} triangles`);
    assert.ok(name in MESH_BUILDERS, `${name} is a mesh kind`);
  }
});
