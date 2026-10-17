#!/usr/bin/env node
// Refuses private information of the people working on this repo in tracked
// files (docs/adr/0002-public-repository.md):
//   - built in: the git user email, the OS user name as a path segment, absolute
//     user-home paths (use %USERPROFILE%, ~ or os.homedir() instead), links into the
//     Claude web app, session trailers and session-id front matter lines;
//   - the private list (kept outside the repo, never committed): $LS_PRIVATE_LIST or
//     ~/.config/loose-screws/private-list.json. Names, name parts, emails, account
//     names and project names match as whole words, case-insensitively; `caseSensitive`
//     entries as whole words with their case; the GitHub `handle` only in its GitHub
//     forms (the noreply address and github.com/<handle> URLs).
// Findings name the kind and show the value masked, never the value itself. Run by
// the per-file checks (pre-commit, Claude post-edit) on changed files and on every
// tracked file by pre-push and tools/ci.*; tools/check-commit.mjs reuses the list
// matcher for commit messages and identities. Tests: tools/test/check-private.test.mjs.
//   node tools/check-private.mjs        scan all tracked files, exit 1 on a finding

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** Files the check reads (text; binaries and generated build output are skipped). */
export const SCANNED =
  /\.(rs|toml|ts|mjs|js|json|md|ron|txt|yml|yaml|html|css|ps1|sh|cmd|wgsl)$|(^|\/)(\.gitignore|\.gitattributes|CLAUDE\.md)$|(^|\/)\.githooks\//;

/** Placeholder user names allowed in example paths (tests, docs). */
const PLACEHOLDER_USERS = new Set(["dev", "x", "user", "username", "you", "name", "public", "default"]);

/** Where the private list lives when $LS_PRIVATE_LIST is not set (relative to the home folder). */
export const DEFAULT_LIST_PATH = [".config", "loose-screws", "private-list.json"];

/** The private-list fields matched as whole words, case-insensitively, and how a finding names them. */
export const LIST_KINDS = {
  names: "a name from the private list",
  nameParts: "a name part from the private list",
  emails: "an email address from the private list",
  accountNames: "an account name from the private list",
  projects: "a project name from the private list",
  caseSensitive: "a case-sensitive word from the private list",
};

/** How a finding names the handle outside its GitHub forms. */
export const HANDLE_KIND = "the GitHub handle outside its GitHub forms";

/** Built-in findings that need no private list: [pattern, reason]. */
export const BUILT_IN = [
  [/https?:\/\/(?:[\w-]+\.)*claude\.ai\b|\bclaude\.ai\//i, "a claude.ai link"],
  [/\bClaude-Session\s*:/i, "a Claude-Session line (a session trailer)"],
  [/\boriginSessionId\s*:/i, "an originSessionId line (a session id in front matter)"],
];

function escape(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** `value` masked: first and last character, `*` between. */
export function mask(value) {
  const chars = [...value];
  if (chars.length <= 2) return "*".repeat(chars.length || 1);
  return `${chars[0]}${"*".repeat(chars.length - 2)}${chars[chars.length - 1]}`;
}

/** The private list, or null when the file is missing. Throws when it exists but is not valid JSON. */
export function loadPrivateList(env = process.env, home = os.homedir()) {
  const file = env.LS_PRIVATE_LIST || path.join(home, ...DEFAULT_LIST_PATH);
  let text;
  try {
    text = fs.readFileSync(file, "utf8");
  } catch {
    return null;
  }
  try {
    return JSON.parse(text);
  } catch (e) {
    throw new Error(`the private list ($LS_PRIVATE_LIST or ~/${DEFAULT_LIST_PATH.join("/")}) is not valid JSON: ${e.message}`);
  }
}

const WORD = "[\\p{L}\\p{N}_]";
const wholeWord = (value, flags) => new RegExp(`(?<!${WORD})${escape(value)}(?!${WORD})`, `g${flags}u`);
const compiled = new WeakMap();

/** The list's matchers: [{ re, kind }] plus the handle's matcher and its allowed GitHub forms. */
function compile(list) {
  let c = compiled.get(list);
  if (c) return c;
  const entries = [];
  for (const [field, kind] of Object.entries(LIST_KINDS)) {
    const values = Array.isArray(list[field]) ? list[field] : [];
    for (const v of values) {
      if (typeof v === "string" && v.trim() !== "") entries.push({ re: wholeWord(v.trim(), field === "caseSensitive" ? "" : "i"), kind });
    }
  }
  const handle = typeof list.handle === "string" && list.handle.trim() !== "" ? list.handle.trim() : undefined;
  const h = handle && escape(handle);
  c = {
    entries,
    handle: handle && wholeWord(handle, "i"),
    handleForms: h && new RegExp(`\\d+\\+${h}@users\\.noreply\\.github\\.com|github\\.com/${h}(?![\\w-])`, "giu"),
  };
  compiled.set(list, c);
  return c;
}

/**
 * Private-list matches in `text`: [{ kind, masked }]. `handle: false` skips the
 * handle check (commit identities: the public identity may be named after it).
 */
export function findListValues(text, list, { handle = true } = {}) {
  if (!list) return [];
  const c = compile(list);
  const out = [];
  for (const { re, kind } of c.entries) {
    for (const m of text.matchAll(re)) out.push({ kind, masked: mask(m[0]) });
  }
  if (handle && c.handle) {
    const blanked = text.replace(c.handleForms, (m) => " ".repeat(m.length));
    for (const m of blanked.matchAll(c.handle)) out.push({ kind: HANDLE_KIND, masked: mask(m[0]) });
  }
  return out;
}

/** The private values of the person running the check (empty strings / null when unknown). */
export function privateValues(root = process.cwd(), env = process.env) {
  const email = spawnSync("git", ["config", "user.email"], { cwd: root, encoding: "utf8" }).stdout?.trim() ?? "";
  let user = "";
  try {
    user = os.userInfo().username ?? "";
  } catch {
    user = path.basename(os.homedir());
  }
  return { email, user, list: loadPrivateList(env) };
}

/** Findings ("file:line: reason") for one file's content. */
export function findPrivate(rel, content, { email = "", user = "", list = null } = {}) {
  const out = [];
  const home = /\b[A-Za-z]:(?:\\\\|\\|\/)+Users(?:\\\\|\\|\/)+([^\\/\s"'`<>%$]+)|(?:^|[\s"'`(=])\/(?:c\/Users|home|Users)\/([^\/\s"'`<>%$]+)/gi;
  const userSeg = user.length >= 3 ? new RegExp(`(?:\\\\|/)${escape(user)}(?:\\\\|/|\\b)`, "i") : null;
  // The public commit address is meant to be published.
  const publicEmail = (list?.publicIdentity?.email ?? "").toLowerCase();
  const gitEmail = email && email.toLowerCase() !== publicEmail ? email.toLowerCase() : "";
  const lines = content.split("\n");
  lines.forEach((line, i) => {
    const at = `${rel}:${i + 1}`;
    if (gitEmail && line.toLowerCase().includes(gitEmail)) out.push(`${at}: contains the git user email`);
    for (const m of line.matchAll(home)) {
      const name = (m[1] ?? m[2] ?? "").toLowerCase();
      if (!PLACEHOLDER_USERS.has(name)) out.push(`${at}: absolute user-home path (use %USERPROFILE%, ~ or os.homedir())`);
    }
    if (userSeg && userSeg.test(line) && !out.some((f) => f.startsWith(`${at}:`))) out.push(`${at}: contains the OS user name as a path segment`);
    for (const [re, reason] of BUILT_IN) if (re.test(line)) out.push(`${at}: contains ${reason}`);
    for (const { kind, masked } of findListValues(line, list)) out.push(`${at}: contains ${kind} (${masked})`);
  });
  return out;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const files = spawnSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8" }).stdout.split("\0").filter(Boolean);
  const values = privateValues(root);
  const findings = [];
  for (const rel of files) {
    if (!SCANNED.test(rel)) continue;
    const abs = path.join(root, rel);
    if (!fs.existsSync(abs)) continue;
    findings.push(...findPrivate(rel, fs.readFileSync(abs, "utf8"), values));
  }
  if (findings.length) {
    console.error(`check-private: ${findings.length} finding(s)\n${findings.slice(0, 40).join("\n")}`);
    process.exit(1);
  }
  const listed = values.list ? "built-in checks and the private list" : "built-in checks (no private list)";
  console.log(`check-private: ${files.length} tracked files, no private information (${listed})`);
}
