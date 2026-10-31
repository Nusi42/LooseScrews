#!/usr/bin/env node
// Claude Code PostToolUse hook (.claude/settings.json, docs/adr/0001): runs the per-file
// checks on the file an agent just edited. Claude Code passes the tool call as JSON on stdin
// (`tool_input.file_path`, or `notebook_path`). On findings the short report goes to stderr
// with exit code 2, which Claude Code hands to the agent.
//
// Claude Code starts this file from the session's folder ($CLAUDE_PROJECT_DIR), often the
// master, whose copy lags behind until `main` is fast-forwarded. So it stays a small, stable
// loader: it finds the checkout from the edited file (the nearest folder above it with .git
// and tools/lib/after-edit.mjs) and hands the file to that checkout's own after-edit module,
// which does the work (clean a memory note, run the checks, skip a master checkout).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

/** Exit code that makes Claude Code show stderr to the agent. */
const FEEDBACK_EXIT_CODE = 2;

/** The module that handles an edited file, relative to a checkout's root. */
export const AFTER_EDIT = path.join("tools", "lib", "after-edit.mjs");

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

/** The checkout containing `file` (nearest folder with .git and AFTER_EDIT), or undefined. */
export function checkoutOf(file) {
  let dir = path.dirname(path.resolve(file));
  for (;;) {
    if (fs.existsSync(path.join(dir, ".git")) && fs.existsSync(path.join(dir, AFTER_EDIT))) return dir;
    const parent = path.dirname(dir);
    if (parent === dir) return undefined;
    dir = parent;
  }
}

async function main() {
  const named = editedFile(fs.readFileSync(0, "utf8"));
  if (named === undefined) return;
  const file = path.resolve(named);
  if (!fs.existsSync(file)) return;
  const root = checkoutOf(file);
  if (root === undefined) return;
  const rel = path.relative(root, file).split(path.sep).join("/");
  const handler = await import(pathToFileURL(path.join(root, AFTER_EDIT)).href);
  const report = await handler.afterEdit(root, file, rel);
  if (report === undefined) return;
  console.error(report);
  process.exitCode = FEEDBACK_EXIT_CODE;
}

// Only as a script (the tests import editedFile and checkoutOf); a broken hook must not block
// edits, since exit code 2 is feedback to the agent.
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
