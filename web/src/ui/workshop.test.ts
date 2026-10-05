import assert from "node:assert/strict";
import { test } from "node:test";
import type { Catalog } from "../rules/types.ts";
import { CATALOG } from "../rules/catalog.ts";
import { decodeRules } from "../rules/code.ts";
import { parseJoin } from "./party.ts";
import {
  ARCHETYPES,
  archetypeId,
  archetypeOf,
  beltUpRules,
  cleanSeed,
  DEFAULT_SLIDERS,
  defaultDraft,
  diceSeed,
  draftKind,
  hostPlan,
  readDraft,
  sliderArray,
  sliderMeaning,
  SLIDERS,
  slidersFrom,
  verticalityBox,
  wantsWorkshop,
  wasmCodec,
  wellCount,
  writeDraft,
  type CodecApi,
  type WorldDraft,
} from "./workshop.ts";

test("the slider table is in share-code order with the generator's defaults", () => {
  assert.deepEqual(
    SLIDERS.map((s) => s.key),
    ["enclosure", "density", "verticality", "gravityChaos", "palette", "absurdity", "instability"],
  );
  // ls-worldgen `Sliders::default()`.
  assert.deepEqual(sliderArray(DEFAULT_SLIDERS), [50, 40, 50, 0, 0, 20, 30]);
  assert.deepEqual(sliderArray(slidersFrom([1, 2, 3, 4, 5, 6, 7])), [1, 2, 3, 4, 5, 6, 7]);
  // Out of range and missing values: clamped, defaults.
  assert.deepEqual(sliderArray(slidersFrom([-5, 250, 49.6])), [0, 100, 50, 0, 0, 20, 30]);
});

test("every slider value has a one-line meaning on both archetypes", () => {
  for (const a of ARCHETYPES) {
    for (const s of SLIDERS) {
      const seen = new Set<string>();
      for (let v = 0; v <= 100; v++) {
        const m = sliderMeaning(s.key, v, a);
        assert.ok(m.length > 0 && m.length <= 40, `${a} ${s.key} ${v}: "${m}"`);
        seen.add(m);
      }
      // Every slider says something different somewhere along its range.
      assert.ok(seen.size >= 2, `${a} ${s.key} never changes its meaning`);
    }
  }
  assert.equal(sliderMeaning("enclosure", 0), "Open space");
  assert.equal(sliderMeaning("enclosure", 100), "Mine shafts");
  assert.equal(sliderMeaning("gravityChaos", 0), "No gravity wells");
  assert.equal(sliderMeaning("gravityChaos", 100), "7 gravity wells, fierce pull");
  assert.equal(sliderMeaning("absurdity", 0), "No comedy events");
  assert.equal(sliderMeaning("instability", 0), "Solid rock, no cave-ins");
  assert.equal(sliderMeaning("palette", 85), "Black-and-white ink, pigment");
  // Belt meanings (§3.8): asteroid sizes, counts per sector, thickness, shepherd moons.
  assert.equal(sliderMeaning("enclosure", 0, "belt"), "Asteroids 1.5-3 u");
  assert.equal(sliderMeaning("enclosure", 100, "belt"), "Asteroids 4-10 u, the biggest hollow");
  assert.equal(sliderMeaning("density", 0, "belt"), "15 asteroids, 3 boulders a sector");
  assert.equal(sliderMeaning("density", 100, "belt"), "60 asteroids, 10 boulders a sector");
  assert.equal(sliderMeaning("verticality", 0, "belt"), "Belt 16 u thick");
  assert.equal(sliderMeaning("verticality", 50, "belt"), "Belt 26 u thick");
  assert.equal(sliderMeaning("verticality", 100, "belt"), "Belt 36 u thick");
  assert.equal(sliderMeaning("gravityChaos", 19, "belt"), "No shepherd moons");
  assert.match(sliderMeaning("gravityChaos", 20, "belt"), /^8 shepherd moons/);
  // Palette, Absurdity and Instability mean the same everywhere.
  for (const k of ["palette", "absurdity", "instability"] as const) {
    for (const v of [0, 30, 60, 90]) assert.equal(sliderMeaning(k, v, "belt"), sliderMeaning(k, v, "classic"));
  }
});

test("Verticality picks the five level boxes of about the same chunk count", () => {
  const boxes = [0, 19, 20, 39, 40, 59, 60, 79, 80, 100].map(verticalityBox);
  assert.deepEqual(boxes[0], [10, 2, 10]);
  assert.deepEqual(boxes[1], [10, 2, 10]);
  assert.deepEqual(boxes[2], [9, 3, 9]);
  assert.deepEqual(boxes[4], [8, 4, 8]);
  assert.deepEqual(boxes[6], [7, 5, 7]);
  assert.deepEqual(boxes[9], [6, 7, 6]);
  for (const [x, y, z] of boxes) assert.ok(Math.abs(x * y * z - 256) <= 60, `${x}x${y}x${z}`);
  assert.equal(sliderMeaning("verticality", 50), "Balanced, 256 × 128 × 256 u");
  assert.equal(sliderMeaning("verticality", 100), "Towering, 192 × 224 × 192 u");
});

test("gravity wells follow §3.3: none at 0, then 1 + slider / 15, at most 7", () => {
  assert.equal(wellCount(0), 0);
  assert.equal(wellCount(1), 1);
  assert.equal(wellCount(14), 1);
  assert.equal(wellCount(15), 2);
  assert.equal(wellCount(89), 6);
  assert.equal(wellCount(90), 7);
  assert.equal(wellCount(100), 7);
});

test("seeds are whole u64 numbers; the dice rolls short ones", () => {
  assert.equal(cleanSeed(" 42 "), "42");
  assert.equal(cleanSeed("007"), "7");
  assert.equal(cleanSeed("1 000 000"), "1000000");
  assert.equal(cleanSeed("18446744073709551615"), "18446744073709551615");
  assert.equal(cleanSeed("18446744073709551616"), null);
  for (const bad of ["", "-1", "1.5", "seed", "0x10", "123456789012345678901"]) assert.equal(cleanSeed(bad), null, bad);
  assert.equal(diceSeed(() => 0), "1");
  assert.equal(diceSeed(() => 0.999_999_9), "999999");
  let r = 0.1;
  const rng = (): number => (r = (r * 9301 + 49297) % 233280 / 233280);
  for (let i = 0; i < 100; i++) assert.notEqual(cleanSeed(diceSeed(rng)), null);
  const d = defaultDraft(() => 0.5);
  assert.equal(d.archetype, "classic");
  assert.deepEqual(sliderArray(d.sliders), [38, 60, 50, 0, 0, 20, 30]);
  assert.equal(sliderMeaning("enclosure", d.sliders.enclosure), "Caverns open to the sky");
});

test("?workshop opens the workshop, ?workshop=<code> on a world", () => {
  assert.equal(wantsWorkshop({ search: "" }), false);
  assert.equal(wantsWorkshop({ search: "?backdrop=off" }), false);
  assert.equal(wantsWorkshop({ search: "?workshop" }), true);
  assert.equal(wantsWorkshop({ search: "?workshop=" }), true);
  assert.equal(wantsWorkshop({ search: "?transport=ws&workshop=LS0C-0000" }), "LS0C-0000");
});

test("archetype ids are append only (ls-worldgen Archetype)", () => {
  assert.equal(archetypeId("classic"), 0);
  assert.equal(archetypeId("belt"), 1);
  assert.equal(archetypeOf(0), "classic");
  assert.equal(archetypeOf(1), "belt");
  assert.equal(archetypeOf(7), "classic");
  assert.equal(archetypeOf(undefined), "classic");
  assert.equal(draftKind({ archetype: "belt", sliders: DEFAULT_SLIDERS }), "Asteroid belt");
  assert.equal(draftKind({ archetype: "classic", sliders: DEFAULT_SLIDERS }), "Caverns");
});

test("drafts survive storage; damaged entries are dropped", () => {
  const d: WorldDraft = { seed: "123", sliders: slidersFrom([1, 2, 3, 4, 5, 6, 7]), archetype: "belt" };
  assert.deepEqual(readDraft(writeDraft(d)), d);
  for (const bad of [null, "", "{", "null", "[]", '{"seed":"x","sliders":[1,2,3,4,5,6,7]}', '{"seed":"1","sliders":[1,2]}', '{"seed":"1","sliders":[1,2,3,4,5,6,"7"]}']) {
    assert.equal(readDraft(bad), null, String(bad));
  }
  // An unknown archetype falls back to classic.
  assert.equal(readDraft('{"seed":"1","sliders":[1,2,3,4,5,6,7],"archetype":"donut"}')?.archetype, "classic");
});

/** A fake ls-wasm codec: codes are `LS:<seed>:<sliders>:<archetype>`. */
function fakeApi(archetypes: string[]): CodecApi {
  return {
    levelArchetypes: () => archetypes,
    makeShareCode: (seed, sliders, archetype) => {
      if (archetype >= archetypes.length) throw new Error("unknown archetype");
      return `LS:${seed}:${[...sliders].join(",")}:${archetype}`;
    },
    checkShareCode: (text) => {
      const m = /^LS:(\d+):([\d,]+):(\d)$/.exec(text);
      return m
        ? { ok: true, code: text, error: "", seed: m[1]!, sliders: m[2]!.split(",").map(Number), archetype: Number(m[3]), free: () => {} }
        : { ok: false, code: "", error: "Not a share code", free: () => {} };
    },
  };
}

test("the codec round-trips drafts through ls-wasm and refuses archetypes the build lacks", () => {
  const codec = wasmCodec(fakeApi(["classic", "belt"]));
  assert.deepEqual(codec.archetypes, ["classic", "belt"]);
  const d: WorldDraft = { seed: "99", sliders: slidersFrom([10, 20, 30, 40, 50, 60, 70]), archetype: "belt" };
  const made = codec.encode(d);
  assert.ok(made.ok);
  const back = codec.decode(made.code);
  assert.ok(back.ok);
  assert.deepEqual(back.draft, d);
  assert.deepEqual(codec.decode("nope"), { ok: false, error: "Not a share code" });

  const old = wasmCodec(fakeApi(["classic"]));
  assert.deepEqual(old.archetypes, ["classic"]);
  const refused = old.encode(d);
  assert.equal(refused.ok, false);
  if (!refused.ok) assert.match(refused.error, /Asteroid belt/);
  // A core without the workshop exports: a clear error, no exception.
  const none = wasmCodec({ checkShareCode: fakeApi([]).checkShareCode });
  assert.equal(none.encode({ ...d, archetype: "classic" }).ok, false);
});

test("Host it: an invite link with code=, and Belt Up proposed on belt worlds", () => {
  const code = "LS0C-0000-0000-000A-K4M6-8001-8F1Y";
  const page = "http://127.0.0.1:7777/?transport=ws&workshop";
  const classic = hostPlan(page, code, "classic");
  const join = parseJoin(classic.link);
  assert.equal(join.world, code.replace(/-/g, ""));
  assert.equal(join.rules, null);
  assert.equal(new URL(classic.link).searchParams.get("transport"), "ws");
  assert.equal(new URL(classic.link).searchParams.has("workshop"), false);
  assert.equal(classic.command, `ls-server --code ${code}`);
  assert.equal(new URL(classic.editor).searchParams.get("editor"), "rules");
  assert.equal(new URL(classic.editor).searchParams.get("world"), code.replace(/-/g, ""));

  // A catalog with Belt Up (M4 B6 adds it to the generated one).
  const withBelt: Catalog = {
    ...CATALOG,
    modes: [...CATALOG.modes, { ...CATALOG.modes[0]!, kind: "BeltUp" as never, name: "Belt Up" }],
    defaults: { ...CATALOG.defaults, BeltUp: CATALOG.defaults.PaintTheVoid } as Catalog["defaults"],
  };
  const rules = beltUpRules(withBelt);
  assert.ok(rules?.startsWith("LR"));
  const belt = hostPlan(page, code, "belt", true, withBelt);
  assert.equal(belt.rules, rules);
  assert.equal(parseJoin(belt.link).rules, rules);
  assert.equal(belt.command, `ls-server --code ${code} --ruleset ${rules}`);
  assert.match(belt.note, /Belt Up/);
  // Declined: the server's rules.
  assert.equal(hostPlan(page, code, "belt", false, withBelt).rules, null);
  // Classic worlds never propose Belt Up.
  assert.equal(hostPlan(page, code, "classic", true, withBelt).rules, null);
  // Without Belt Up in the build: the server's rules, and the note says so.
  const plain: Catalog = { ...CATALOG, modes: CATALOG.modes.filter((m) => (m.kind as string) !== "BeltUp") };
  const noBelt = hostPlan(page, code, "belt", true, plain);
  assert.equal(noBelt.rules, null);
  assert.match(noBelt.note, /no Belt Up/);
});

test("the Belt Up proposal is the build's Belt Up defaults when the catalog has them", () => {
  const rules = beltUpRules(CATALOG);
  if (rules === null) return; // before M4 B6: no Belt Up mode yet
  const decoded = decodeRules(rules, CATALOG);
  assert.equal(decoded.mode.kind as string, "BeltUp");
});
