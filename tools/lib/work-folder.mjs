// Master repository and work folders (docs/adr/0001-master-repo-and-work-folders.md).
// The checkout in OneDrive is the master: it holds `main`, receives pushes and
// fast-forward merges, and nothing is built, tested, run, installed or committed
// in it. All work happens in clones outside OneDrive, `<base>\loosescrews.win<N>` on
// branch `WIN<N>` (win1 = the lead). Every dev and CI entry point calls
// `assertWorkFolder`; in the master Cargo refuses through a rustc wrapper
// (`node tools/work-folders.mjs guard-master`). Tests: tools/test/work-folder.test.mjs.

import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** Parent folder of the work folders. */
export const WORK_BASE = path.join(os.homedir(), "<work root>");
/** The master repository (OneDrive). */
export const MASTER = path.join(os.homedir(), "master");

/** Branch of work folder `n`. */
export function branchOf(n) {
  return `WIN${n}`;
}

/** Folder name of work folder `n`. */
export function folderOf(n) {
  return `colent.win${n}`;
}

/** Parses a work-folder number (1-99); throws on anything else. */
export function parseFolderNumber(text) {
  if (text === undefined || !/^\d{1,2}$/.test(text) || Number(text) < 1) {
    throw new Error(`a work folder number is 1-99, not "${text}"`);
  }
  return Number(text);
}

/** Whether `dir` lies in a OneDrive folder (a path segment starting with "OneDrive"). */
export function isInOneDrive(dir) {
  return path
    .resolve(dir)
    .split(/[\\/]/)
    .some((segment) => /^onedrive\b/i.test(segment));
}

/** The refusal message for `what` started in `root`, or undefined in a work folder. */
export function masterRefusal(root, what) {
  if (!isInOneDrive(root)) return undefined;
  return (
    `${what}: refused in ${root}.\n` +
    "This checkout lies in OneDrive: it is the colent master repository, which only receives\n" +
    "pushes and fast-forward merges. Nothing is built, tested, run, installed or committed here.\n" +
    `Work in a clone outside OneDrive: ${WORK_BASE}\\${folderOf(1)} (branch ${branchOf(1)}, the lead)\n` +
    `or ${folderOf(2)}, ${folderOf(3)}, ... (create one with \`node tools/work-folders.mjs create <n>\`\n` +
    'from a work folder; docs/dev-guide.md, "Work folders").'
  );
}

/** Root of the checkout this module belongs to. */
export const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

/** Exits with the refusal when the checkout at `root` is the master. */
export function assertWorkFolder(what, root = REPO_ROOT) {
  const refusal = masterRefusal(root, what);
  if (refusal === undefined) return;
  console.error(refusal);
  process.exit(1);
}

/** The refusing rustc wrapper named by the master's `.cargo/config.local.toml`. */
export const MASTER_RUSTC_WRAPPER = [
  "@echo off",
  "echo error: this checkout is the colent master repository (OneDrive): nothing is built here. 1>&2",
  `echo Work in a clone outside OneDrive, e.g. ${WORK_BASE}\\${folderOf(1)} (docs/dev-guide.md, Work folders). 1>&2`,
  "exit /b 1",
  "",
].join("\r\n");

/** The master's `.cargo/config.local.toml` (gitignored, included by `.cargo/config.toml`). */
export function masterCargoConfig(wrapper) {
  return [
    "# Local only (gitignored), written by `node tools/work-folders.mjs guard-master`.",
    "# This checkout is the master repository (OneDrive): every rustc call is refused.",
    "[build]",
    `rustc-wrapper = ${JSON.stringify(wrapper.replaceAll("\\", "/"))}`,
    "",
  ].join("\n");
}

/** A work folder's gitignored `.claude/settings.local.json`: memory lives in the checkout. */
export function claudeLocalSettings(folder) {
  const memory = path.join(folder, ".claude", "memory").replaceAll("\\", "/");
  return `${JSON.stringify({ autoMemoryDirectory: memory }, null, 2)}\n`;
}
