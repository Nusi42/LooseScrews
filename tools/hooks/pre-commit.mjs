#!/usr/bin/env node
// Git pre-commit (.githooks/pre-commit): the per-file checks on the staged files
// (working-tree copies; stage the whole file you fixed). Refused in the master.
import { spawnSync } from "node:child_process";

import { runChecks, shortReport } from "../lib/run-checks.mjs";
import { REPO_ROOT, assertWorkFolder } from "../lib/work-folder.mjs";

assertWorkFolder("git commit");
const staged = spawnSync("git", ["diff", "--cached", "--name-only", "--diff-filter=ACMR", "-z"], { cwd: REPO_ROOT, encoding: "utf8" })
  .stdout.split("\0").filter(Boolean);
const results = await runChecks(REPO_ROOT, staged);
if (results.some((r) => r.code !== 0)) {
  console.error(`pre-commit: checks failed\n${shortReport(results)}\n(fix: cargo fmt --all; see docs/dev-guide.md "Work folders")`);
  process.exit(1);
}
