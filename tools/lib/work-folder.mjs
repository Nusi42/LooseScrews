// Checkout roles and work folders (docs/adr/0001-master-repo-and-work-folders.md,
// docs/adr/0002-public-repository.md). A checkout is the master when its role is
// `master`: the gitignored `.env.local` at its root holds `LS_CHECKOUT_ROLE=master`,
// and the environment variable LS_CHECKOUT_ROLE overrides the file. The master holds
// `main`, receives pushes and fast-forward merges, and nothing is built, tested, run,
// installed or committed in it. Any other checkout (a work folder
// `<work root>/<prefix>.win<N>` on branch `WIN<N>`, or a plain clone without any
// configuration) works normally. Every dev and CI entry point calls
// `assertWorkFolder`; in the master Cargo refuses through a rustc wrapper
// (`node tools/work-folders.mjs guard-master`). Tests: tools/test/work-folder.test.mjs.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** Folder-name prefix of the work folders when the checkout's name does not give one. */
export const DEFAULT_PREFIX = "loosescrews";

/** The environment variable (and `.env.local` key) naming the checkout's role. */
export const ROLE_KEY = "LS_CHECKOUT_ROLE";

/** The role that refuses builds, tests, runs, installs and commits. */
export const MASTER_ROLE = "master";

/** The gitignored per-checkout configuration file (at the checkout root). */
export const ENV_LOCAL = ".env.local";

/** Branch of work folder `n`. */
export function branchOf(n) {
  return `WIN${n}`;
}

const WORK_FOLDER_NAME = /^(.+)\.win(\d{1,2})$/;

/** The work-folder prefix of the checkout at `dir`: its folder name without `.win<N>`, else DEFAULT_PREFIX. */
export function folderPrefix(dir) {
  const m = WORK_FOLDER_NAME.exec(path.basename(path.resolve(dir)));
  return m ? m[1] : DEFAULT_PREFIX;
}

/** The number of the work folder named `name` with `prefix` (`<prefix>.win<N>`), or undefined. */
export function workFolderNumber(name, prefix) {
  const m = WORK_FOLDER_NAME.exec(name);
  return m && m[1] === prefix ? Number(m[2]) : undefined;
}

/** Folder name of work folder `n`. */
export function folderOf(n, prefix = DEFAULT_PREFIX) {
  return `${prefix}.win${n}`;
}

/** Parses a work-folder number (1-99); throws on anything else. */
export function parseFolderNumber(text) {
  if (text === undefined || !/^\d{1,2}$/.test(text) || Number(text) < 1) {
    throw new Error(`a work folder number is 1-99, not "${text}"`);
  }
  return Number(text);
}

/** The value of `key` in `.env`-style `text` (`KEY=value`, optional quotes, `#` comments), or undefined. */
export function envValue(text, key) {
  let value;
  for (const raw of text.split(/\r?\n/)) {
    const m = /^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*?)\s*$/.exec(raw);
    if (!m || m[1] !== key) continue;
    const v = m[2];
    value = /^(["']).*\1$/.test(v) ? v.slice(1, -1) : v.replace(/\s+#.*$/, "");
  }
  return value;
}

/** `text` (a `.env.local`) with `key=value` set: an existing line is replaced, else one is appended. */
export function withEnvValue(text, key, value) {
  const line = `${key}=${value}`;
  const lines = text === "" ? [] : text.replace(/\r?\n$/, "").split(/\r?\n/);
  const at = lines.findIndex((l) => envValue(l, key) !== undefined);
  if (at >= 0) lines[at] = line;
  else lines.push(line);
  return `${lines.join("\n")}\n`;
}

/** The role of the checkout at `root`: $LS_CHECKOUT_ROLE, else its `.env.local`, else "". */
export function checkoutRole(root, env = process.env) {
  const fromEnv = env[ROLE_KEY];
  if (fromEnv !== undefined && fromEnv.trim() !== "") return fromEnv.trim();
  try {
    return (envValue(fs.readFileSync(path.join(root, ENV_LOCAL), "utf8"), ROLE_KEY) ?? "").trim();
  } catch {
    return "";
  }
}

/** Whether the checkout at `root` is the master (role `master`). */
export function isMaster(root, env = process.env) {
  return checkoutRole(root, env) === MASTER_ROLE;
}

/** The refusal message for `what` started in `root`, or undefined outside the master. */
export function masterRefusal(root, what, env = process.env) {
  if (!isMaster(root, env)) return undefined;
  return (
    `${what}: refused.\n` +
    `This checkout is the master (${ROLE_KEY}=${MASTER_ROLE}): it only receives pushes and\n` +
    "fast-forward merges. Nothing is built, tested, run, installed or committed here.\n" +
    `Work in a work folder instead, e.g. <work root>/${folderOf(1)} (branch ${branchOf(1)});\n` +
    "create one with `node tools/work-folders.mjs create <n>` from a work folder\n" +
    '(docs/dev-guide.md, "Work folders").'
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

/** The refusing rustc wrapper named by the master's `.cargo/config.local.toml` (cmd escapes `<>` with `^`). */
export const MASTER_RUSTC_WRAPPER = [
  "@echo off",
  "echo error: this checkout is the master: nothing is built here. 1>&2",
  `echo Work in a work folder, e.g. ^<work root^>/${folderOf(1)} (docs/dev-guide.md, Work folders). 1>&2`,
  "exit /b 1",
  "",
].join("\r\n");

/** The master's `.cargo/config.local.toml` (gitignored, included by `.cargo/config.toml`). */
export function masterCargoConfig(wrapper) {
  return [
    "# Local only (gitignored), written by `node tools/work-folders.mjs guard-master`.",
    "# This checkout is the master: every rustc call is refused.",
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
