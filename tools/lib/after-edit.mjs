// What the Claude Code post-edit hook does with an edited file (docs/adr/0001), in the
// checkout that holds the file. tools/post-edit.mjs runs from the session's folder
// ($CLAUDE_PROJECT_DIR, often the master, whose code lags behind until `main` is
// fast-forwarded), so it only finds the checkout and loads this module from it: the
// checkout's own, current code decides. Tests: tools/test/hooks.test.mjs.
//
// Claude Code writes bookkeeping keys into the front matter of memory notes
// (`.claude/memory/*.md`), at the top level or indented under `metadata`: originSessionId
// (a session id, which must never be committed; the private-information check refuses it),
// modified (a time stamp) and node_type. They are removed before the checks run.
import fs from "node:fs";

import { runChecks, shortReport } from "./run-checks.mjs";
import { isMaster } from "./work-folder.mjs";

/** Whether repo-relative `rel` (`/` separators) is a memory note. */
export function isMemoryNote(rel) {
  return /^\.claude\/memory\/.*\.md$/.test(rel);
}

/** Front-matter keys Claude Code writes into memory notes; none of them belongs in git. */
export const HARNESS_KEYS = ["originSessionId", "modified", "node_type"];

/** `content` without the HARNESS_KEYS lines (any indentation) in its front matter; the body stays. */
export function withoutHarnessKeys(content) {
  const m = /^(---\r?\n)([\s\S]*?)(\r?\n---(?:\r?\n|$))/.exec(content);
  if (!m) return content;
  const harness = new RegExp(`^[ \\t]*(?:${HARNESS_KEYS.join("|")})[ \\t]*:`);
  const kept = m[2].split(/\r?\n/).filter((l) => !harness.test(l));
  const front = kept.join(m[1].endsWith("\r\n") ? "\r\n" : "\n");
  return `${m[1]}${front}${m[3]}${content.slice(m[0].length)}`;
}

/** Removes the HARNESS_KEYS from the note at `file` (`rel`: its repo-relative path); whether it changed. */
export function cleanNote(file, rel) {
  if (!isMemoryNote(rel)) return false;
  const content = fs.readFileSync(file, "utf8");
  const cleaned = withoutHarnessKeys(content);
  if (cleaned === content) return false;
  fs.writeFileSync(file, cleaned);
  return true;
}

/**
 * Handles one edited file: cleans a memory note, then runs the per-file checks. Files of a
 * master checkout are left alone (nothing runs there). Resolves to the short report of the
 * failed checks, or undefined.
 */
export async function afterEdit(root, file, rel) {
  if (isMaster(root)) return undefined;
  cleanNote(file, rel);
  const results = await runChecks(root, [rel]);
  return results.every((r) => r.code === 0) ? undefined : shortReport(results);
}
