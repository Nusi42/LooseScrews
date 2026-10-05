import assert from "node:assert/strict";
import { test } from "node:test";
import { FEED_LINES, FEED_MS, formatTimer, meterPercent, pushFeed } from "./hud.ts";
import { checkShareCode, normalizeShareCode, SHARE_CODE_PRINTED_LEN } from "./shareCode.ts";
import { cleanName, DEFAULT_NAME, readShareCode, readStored } from "./startScreen.ts";

// Printed by ls-worldgen (crates/ls-worldgen/tests/fixtures/golden_hashes.txt).
const REAL_CODE = "LS0G-0000-0000-0001-AC58-T2XM-57ND";

test("share codes are normalised (case, spaces, dashes, Crockford look-alikes)", () => {
  assert.equal(normalizeShareCode("  ls-abcd efgh-OIL "), "LSABCDEFGH011");
  assert.deepEqual(checkShareCode(REAL_CODE), { ok: true, code: REAL_CODE });
  // Undashed, lowercase, spaced and with look-alikes: same code back in printed form.
  assert.deepEqual(checkShareCode(" ls0g oooo 0000-OOOl ac58 t2xm 57nd "), { ok: true, code: REAL_CODE });
  assert.deepEqual(checkShareCode(REAL_CODE.replace(/-/g, "")), { ok: true, code: REAL_CODE });
  assert.equal(REAL_CODE.length, SHARE_CODE_PRINTED_LEN);
});

test("malformed share codes are rejected with a message", () => {
  const body = REAL_CODE.slice(2).replace(/-/g, "");
  const bad = [
    "",
    "XX-1234",
    "LS",
    "LS-",
    "LS-0ABC",
    "LS" + body.slice(1), // 25 symbols
    "LS" + body + "0", // 27 symbols
    "LS" + body.slice(1) + "U", // U is not Crockford
  ];
  for (const code of bad) {
    const r = checkShareCode(code);
    assert.equal(r.ok, false, code);
    if (!r.ok) assert.ok(r.error.length > 0);
  }
});

test("timer and meter formatting", () => {
  assert.equal(formatTimer(300), "5:00");
  assert.equal(formatTimer(61.2), "1:02");
  assert.equal(formatTimer(-3), "0:00");
  assert.equal(meterPercent(50, 100), 50);
  assert.equal(meterPercent(150, 100), 100);
  assert.equal(meterPercent(5, 0), 0);
});

test("kill feed keeps the newest lines and expires old ones", () => {
  let feed = pushFeed([], ["a", "b"], 0);
  assert.deepEqual(feed.map((e) => e.text), ["a", "b"]);
  feed = pushFeed(feed, ["c", "d", "e", "f"], 100);
  assert.equal(feed.length, FEED_LINES);
  assert.equal(feed[0]!.text, "b");
  feed = pushFeed(feed, [], FEED_MS + 50);
  assert.deepEqual(feed.map((e) => e.text), ["c", "d", "e", "f"], "lines from t=0 expired");
});

test("start screen: empty code means server/random world; validator runs after the format check", () => {
  assert.deepEqual(readShareCode("   "), { ok: true, code: null });
  assert.deepEqual(readShareCode(REAL_CODE.toLowerCase()), { ok: true, code: REAL_CODE });
  let calls = 0;
  const validate = (code: string) => {
    calls++;
    return code === REAL_CODE ? { ok: true as const, code } : { ok: false as const, error: "checksum" };
  };
  assert.equal(readShareCode("XX", validate).ok, false);
  assert.equal(calls, 0, "format errors never reach the validator");
  assert.deepEqual(readShareCode(REAL_CODE, validate), { ok: true, code: REAL_CODE });
  const typo = REAL_CODE.replace("AC58", "AC59");
  assert.deepEqual(readShareCode(typo, validate), { ok: false, error: "checksum" });
});

test("pilot names are trimmed to the server's limits", () => {
  assert.equal(cleanName("  Ace  "), "Ace");
  assert.equal(cleanName(""), DEFAULT_NAME);
  assert.equal(cleanName("a\u0007b"), "ab");
  const long = cleanName("é".repeat(40));
  assert.ok(new TextEncoder().encode(long).length <= 32);
});

test("damaged stored start preferences fall back to the defaults", () => {
  // `null` used to throw on `stored.shipType` and stop the page at "Failed to start".
  for (const raw of [null, "", "null", "42", '"x"', "[1]", "{", "true"]) assert.deepEqual(readStored(raw), {}, String(raw));
  assert.deepEqual(readStored('{"name":7,"horizonAssist":"no","shipType":"2","shareCode":null}'), {});
  assert.deepEqual(readStored('{"name":"Ace","horizonAssist":false,"shipType":2,"shareCode":"LS1"}'), {
    name: "Ace",
    horizonAssist: false,
    shipType: 2,
    shareCode: "LS1",
  });
});
