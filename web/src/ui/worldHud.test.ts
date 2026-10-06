import assert from "node:assert/strict";
import { test } from "node:test";

import * as mat4 from "../math/mat4.ts";
import * as q from "../math/quat.ts";
import { GREY_TAG_RANGE, nameTagModel, PIGMENT_TAG_RANGE } from "./nameTags.ts";
import { moodFace, pullArc, pullView, PULL_RING } from "./worldHud.ts";

test("the pull indicator: an arc and chevron toward a sideways pull, a dot or broken ring along the view", () => {
  assert.equal(pullView(null).visible, false);
  const right = pullView({ strength: 0.5, ofThrust: 0.2, screen: { x: 1, y: 0 }, ahead: true });
  assert.ok(right.visible && right.arc && right.chevron);
  assert.equal(right.label, "PULL 20%");
  // The arc is centred on the right of the ring.
  assert.ok(pullArc(0, 0.2).startsWith(`M ${(100 + Math.cos(-0.1) * PULL_RING).toFixed(1)}`));
  const strong = pullView({ strength: 1, ofThrust: 0.4, screen: { x: 0, y: 1 }, ahead: true });
  assert.ok(strong.width > right.width && strong.opacity >= right.opacity);
  const along = pullView({ strength: 0.3, ofThrust: 0.12, screen: null, ahead: false });
  assert.ok(along.visible && along.arc === null && !along.ahead);
  for (const m of ["calm", "bored", "grumpy", "giddy"]) assert.ok(moodFace(m).includes("mood-head"), m);
  assert.notEqual(moodFace("grumpy"), moodFace("giddy"));
});

test("monochrome name tags: pigmented ships through walls far away, grey enemies only close and in sight", () => {
  const vp = mat4.multiply(mat4.perspective(1.2, 16 / 9, 0.05, 600), mat4.viewFromPose({ x: 0, y: 0, z: 0 }, q.identity()));
  const f = q.forward(q.identity());
  const ship = (id: number, team: number, d: number) => ({ id, team, alive: true, boosting: false, position: { x: f.x * d, y: f.y * d, z: f.z * d }, orientation: q.identity(), shipType: 1, radius: 0.5, gadgetActive: false });
  const players = [1, 2, 3, 4].map((id) => ({ id, team: 0, shipType: 1, gadget: 255, name: `P${id}` }));
  const ships = [ship(1, 1, PIGMENT_TAG_RANGE - 10), ship(2, 1, GREY_TAG_RANGE + 5), ship(3, 1, GREY_TAG_RANGE - 5), ship(4, 0, 60)];
  const blocked = new Set<number>();
  const rules = { pigmented: new Set([1]), inSight: (p: { x: number; y: number; z: number }) => !blocked.has(Math.round(Math.hypot(p.x, p.y, p.z))) };
  const tags = nameTagModel(ships, players, { x: 0, y: 0, z: 0 }, vp, 1920, 1080, 0, rules);
  assert.deepEqual(tags.map((t) => t.id).sort(), [1, 3, 4]);
  assert.equal(tags.find((t) => t.id === 1)?.pigment, true);
  assert.equal(tags.find((t) => t.id === 3)?.grey, true);
  blocked.add(GREY_TAG_RANGE - 5);
  assert.deepEqual(nameTagModel(ships, players, { x: 0, y: 0, z: 0 }, vp, 1920, 1080, 0, rules).map((t) => t.id).sort(), [1, 4], "no tag through walls for grey enemies");
  // Without the rules (colour levels) the old ranges hold.
  assert.deepEqual(nameTagModel(ships, players, { x: 0, y: 0, z: 0 }, vp, 1920, 1080, 0).map((t) => t.id).sort(), [2, 3, 4]);
});
