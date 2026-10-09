#!/usr/bin/env node
// Refuses private information of the people working on this repo in tracked
// files: the git user email, the OS user name as a path segment, and absolute
// user-home paths (use %USERPROFILE%, ~ or os.homedir() instead). Run by the
// per-file checks (pre-commit, Claude post-edit) on changed files and on every
// tracked file by pre-push and tools/ci.*. Tests: tools/test/check-private.test.mjs.
//   node tools/check-private.mjs        scan all tracked files, exit 1 on a finding

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** Files the check reads (text; binaries and generated build output are skipped). */
export const SCANNED = /\.(rs|toml|ts|mjs|js|json|md|ron|txt|yml|yaml|html|css|ps1|sh|cmd|wgsl)$|(^|\/)(\.gitignore|\.gitattributes|CLAUDE\.md)$/;

/** Placeholder user names allowed in example paths (tests, docs). */
const PLACEHOLDER_USERS = new Set(["dev", "x", "user", "username", "you", "name", "public", "default"]);

function escape(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** The private values of the person running the check (empty strings when unknown). */
export function privateValues(root = process.cwd()) {
  const email = spawnSync("git", ["config", "user.email"], { cwd: root, encoding: "utf8" }).stdout?.trim() ?? "";
  let user = "";
  try {
    user = os.userInfo().username ?? "";
  } catch {
    user = path.basename(os.homedir());
  }
  return { email, user };
}

/** Findings ("file:line: reason") for one file's content. */
export function findPrivate(rel, content, { email = "", user = "" } = {}) {
  const out = [];
  const home = /\b[A-Za-z]:(?:\\\\|\\|\/)+Users(?:\\\\|\\|\/)+([^\\/\s"'`<>%$]+)|(?:^|[\s"'`(=])\/(?:c\/Users|home|Users)\/([^\/\s"'`<>%$]+)/gi;
  const userSeg = user.length >= 3 ? new RegExp(`(?:\\\\|/)${escape(user)}(?:\\\\|/|\\b)`, "i") : null;
  const lines = content.split("\n");
  lines.forEach((line, i) => {
    const at = `${rel}:${i + 1}`;
    if (email && line.toLowerCase().includes(email.toLowerCase())) out.push(`${at}: contains the git user email`);
    for (const m of line.matchAll(home)) {
      const name = (m[1] ?? m[2] ?? "").toLowerCase();
      if (!PLACEHOLDER_USERS.has(name)) out.push(`${at}: absolute user-home path (use %USERPROFILE%, ~ or os.homedir())`);
    }
    if (userSeg && userSeg.test(line) && !out.some((f) => f.startsWith(`${at}:`))) out.push(`${at}: contains the OS user name as a path segment`);
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
  console.log(`check-private: ${files.length} tracked files, no private information`);
}
