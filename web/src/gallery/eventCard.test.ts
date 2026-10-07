import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { emptyMood } from "../game/modeState.ts";
import { STRIP_ACTIVE_SECS, stripBanner } from "./eventCard.ts";

const ron = (id: string): string => readFileSync(new URL(`../../../data/events/${id}.ron`, import.meta.url), "utf8");

test("strip events' active seconds match their data files", () => {
  for (const [id, secs] of Object.entries(STRIP_ACTIVE_SECS)) {
    const m = /active_secs:\s*([0-9.]+)/.exec(ron(id));
    assert.ok(m, id);
    assert.equal(Number(m[1]), secs, id);
  }
});

test("strip banner: card while announced, pill while running, none without an event", () => {
  const mood = (phase: string, secsLeft: number, eventId = "cat_paw") => ({ ...emptyMood(), eventId, phase, phaseEndTick: 600 + Math.round(secsLeft * 60) });
  assert.equal(stripBanner(emptyMood(), 600, ""), null);
  const a = stripBanner(mood("announce", 1.2), 600, "Gasket");
  assert.deepEqual(a, { id: "cat_paw", stage: "announce", secsLeft: 1.2, total: 3, target: "Gasket" });
  const b = stripBanner(mood("active", 9, "tax_drone"), 600, "");
  assert.equal(b?.stage, "active");
  assert.equal(b?.secsLeft, 9);
  assert.equal(b?.total, 25);
  const c = stripBanner(mood("ending", 1, "tax_drone"), 600, "");
  assert.equal(c?.stage, "active");
  assert.equal(c?.secsLeft, 0);
});
