// Tests of tools/hooks/pre-push.mjs: step logs (the output of every step is kept).
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { STEPS, logName, runStep } from "../hooks/pre-push.mjs";

test("every step has a distinct log name", () => {
  const names = STEPS.map(([name], i) => logName(i, name));
  assert.equal(new Set(names).size, STEPS.length);
  assert.equal(logName(5, "cargo test --workspace"), "06-cargo-test-workspace.log");
});

test("runStep keeps the output and the exit code of a failing step", async () => {
  const dir = mkdtempSync(join(tmpdir(), "pre-push-"));
  const log = join(dir, "step.log");
  const code = await runStep("node", ["-e", "\"console.log('out-line'); console.error('err-line'); process.exit(3)\""], dir, log);
  assert.equal(code, 3);
  const text = readFileSync(log, "utf8");
  assert.match(text, /out-line/);
  assert.match(text, /err-line/);
  assert.match(text, /\[exit code 3,/);
});
