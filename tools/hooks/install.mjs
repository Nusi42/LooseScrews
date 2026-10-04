#!/usr/bin/env node
// Installs the Git hooks (core.hooksPath = .githooks). Run by `npm ci` / `npm
// install` in web/ (its `prepare`) and by `node tools/work-folders.mjs create`.
// Refused in the master (nothing is installed there).
import { spawnSync } from "node:child_process";

import { REPO_ROOT, assertWorkFolder } from "../lib/work-folder.mjs";

assertWorkFolder("npm install / hook setup");
const r = spawnSync("git", ["config", "core.hooksPath", ".githooks"], { cwd: REPO_ROOT, stdio: "inherit" });
if (r.status !== 0) process.exit(r.status ?? 1);
console.log("git hooks: core.hooksPath = .githooks (pre-commit: per-file checks, pre-push: fast CI subset)");
