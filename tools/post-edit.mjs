#!/usr/bin/env node
// Claude Code PostToolUse hook (.claude/settings.json, docs/adr/0001): runs the
// per-file checks on the file an agent just edited. The lead session runs in the
// OneDrive master and edits files in the work folders, so the checkout is found
// from the edited file (nearest folder above it with .git and the runner), never
// from $CLAUDE_PROJECT_DIR, and that checkout's own runner runs. Files inside
// OneDrive are skipped. On findings: at most 20 lines on stderr, exit code 2
// (Claude Code hands them to the agent).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { isInOneDrive } from "./lib/work-folder.mjs";

const FEEDBACK_EXIT_CODE = 2;
const RUNNER = path.join("tools", "lib", "run-checks.mjs");

/** The edited file named by the hook input, if any. */
export function editedFile(stdin) {
  let input;
  try {
    input = JSON.parse(stdin);
  } catch {
    return undefined;
  }
  const tool = input?.tool_input;
  for (const key of ["file_path", "notebook_path"]) {
    const v = tool?.[key];
    if (typeof v === "string" && v !== "") return v;
  }
  return undefined;
}

/** The colent checkout containing `file` (nearest folder with .git and the runner). */
export function checkoutOf(file) {
  let dir = path.dirname(path.resolve(file));
  for (;;) {
    if (fs.existsSync(path.join(dir, ".git")) && fs.existsSync(path.join(dir, RUNNER))) return dir;
    const parent = path.dirname(dir);
    if (parent === dir) return undefined;
    dir = parent;
  }
}

async function main() {
  const named = editedFile(fs.readFileSync(0, "utf8"));
  if (named === undefined) return;
  const file = path.resolve(named);
  if (!fs.existsSync(file) || isInOneDrive(file)) return;
  const root = checkoutOf(file);
  if (root === undefined) return;
  const rel = path.relative(root, file).split(path.sep).join("/");
  const runner = await import(pathToFileURL(path.join(root, RUNNER)).href);
  const results = await runner.runChecks(root, [rel]);
  if (results.every((r) => r.code === 0)) return;
  console.error(runner.shortReport(results));
  process.exitCode = FEEDBACK_EXIT_CODE;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
