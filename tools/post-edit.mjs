#!/usr/bin/env node
// Claude Code PostToolUse hook (.claude/settings.json, docs/adr/0001): runs the
// per-file checks on the file an agent just edited. A session may run in one
// checkout and edit files in another, so the checkout is found from the edited
// file (nearest folder above it with .git and the runner), never from
// $CLAUDE_PROJECT_DIR, and that checkout's own runner runs. Files of a checkout
// whose role is master are skipped. Claude Code writes bookkeeping lines into the
// front matter of memory notes (`.claude/memory/*.md`): `originSessionId` (a session
// id, which must never be committed), `modified` (a timestamp) and `node_type`; they
// are stripped before the checks. On findings: at most 20 lines on stderr, exit
// code 2 (Claude Code hands them to the agent).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { isMaster } from "./lib/work-folder.mjs";

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

/** The checkout containing `file` (nearest folder with .git and the runner). */
export function checkoutOf(file) {
  let dir = path.dirname(path.resolve(file));
  for (;;) {
    if (fs.existsSync(path.join(dir, ".git")) && fs.existsSync(path.join(dir, RUNNER))) return dir;
    const parent = path.dirname(dir);
    if (parent === dir) return undefined;
    dir = parent;
  }
}

/** Whether repo-relative `rel` is a memory note (`.claude/memory/**.md`). */
export function isMemoryNote(rel) {
  return /^\.claude\/memory\/.*\.md$/.test(rel);
}

/** Front-matter keys Claude Code writes into memory notes; none of them belongs in git. */
export const HARNESS_KEYS = ["originSessionId", "modified", "node_type"];

/** `content` without the HARNESS_KEYS lines in its front matter (unchanged when there are none). */
export function stripSessionId(content) {
  const m = /^(---\r?\n)([\s\S]*?)(\r?\n---(?:\r?\n|$))/.exec(content);
  if (!m) return content;
  const harness = new RegExp(`^\\s*(?:${HARNESS_KEYS.join("|")})\\s*:`);
  const kept = m[2].split(/\r?\n/).filter((l) => !harness.test(l));
  const front = kept.join(m[1].endsWith("\r\n") ? "\r\n" : "\n");
  return `${m[1]}${front}${m[3]}${content.slice(m[0].length)}`;
}

async function main() {
  const named = editedFile(fs.readFileSync(0, "utf8"));
  if (named === undefined) return;
  const file = path.resolve(named);
  if (!fs.existsSync(file)) return;
  const root = checkoutOf(file);
  if (root === undefined || isMaster(root)) return;
  const rel = path.relative(root, file).split(path.sep).join("/");
  if (isMemoryNote(rel)) {
    const content = fs.readFileSync(file, "utf8");
    const stripped = stripSessionId(content);
    if (stripped !== content) fs.writeFileSync(file, stripped);
  }
  const runner = await import(pathToFileURL(path.join(root, RUNNER)).href);
  const results = await runner.runChecks(root, [rel]);
  if (results.every((r) => r.code === 0)) return;
  console.error(runner.shortReport(results));
  process.exitCode = FEEDBACK_EXIT_CODE;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
