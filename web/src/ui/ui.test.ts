import assert from "node:assert/strict";
import { test } from "node:test";
import { formatTimer, meterPercent } from "./hud.ts";
import { checkShareCode, normalizeShareCode, SHARE_CODE_PRINTED_LEN } from "./shareCode.ts";

// Printed by ls-worldgen (crates/ls-worldgen/tests/fixtures/golden_hashes.txt).
const REAL_CODE = "LS04-0000-0000-000A-K4M6-8001-8F1P";

test("share codes are normalised (case, spaces, dashes, Crockford look-alikes)", () => {
  assert.equal(normalizeShareCode("  ls-abcd efgh-OIL "), "LSABCDEFGH011");
  assert.deepEqual(checkShareCode(REAL_CODE), { ok: true, code: REAL_CODE });
  // Undashed, lowercase, spaced and with look-alikes: same code back in printed form.
  assert.deepEqual(checkShareCode(" ls04 oooo 0000-OOoa k4m6 8OO1 8f1p "), { ok: true, code: REAL_CODE });
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
