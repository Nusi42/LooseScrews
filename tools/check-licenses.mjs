#!/usr/bin/env node
// Dependency license gate for the npm lockfiles (the Rust side is deny.toml, run by
// `cargo deny check licenses`). The project is AGPL-3.0-or-later and may also be
// offered under other terms by its maintainer, so no dependency may be under GPL,
// LGPL or AGPL only: a license expression passes when one of its options avoids
// them ("(MIT OR GPL-3.0)" passes, "GPL-3.0" and "MIT AND LGPL-2.1" fail). A package
// without a license (or "UNLICENSED", "SEE LICENSE IN ...") fails unless REVIEWED
// lists it with a reason. docs/dev-guide.md, "Dependency licenses".
//   node tools/check-licenses.mjs [package-lock.json...]   (default: LOCKFILES)

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { REPO_ROOT } from "./lib/work-folder.mjs";

/** The lockfiles the check reads (repository-relative). */
export const LOCKFILES = ["web/package-lock.json", "tools/browser-smoke/package-lock.json", "tools/readme-media/package-lock.json"];

/** Packages without a usable license field, reviewed by hand: package name -> reason. */
export const REVIEWED = {};

/** GPL, LGPL and AGPL in every version and suffix (GPL-2.0+, LGPL-2.1-or-later, AGPL-3.0-only, ...). */
export const COPYLEFT = /^(A|L)?GPL(-|$|\+|v)/i;

/** Tokens of an SPDX expression: "(", ")", "AND", "OR", "WITH" and license ids. */
function tokenize(expr) {
  return expr.replace(/[()]/g, " $& ").trim().split(/\s+/).filter(Boolean);
}

/**
 * Whether an SPDX expression can be satisfied without GPL, LGPL or AGPL.
 * OR needs one acceptable option, AND needs every part; "X WITH exception" counts as X.
 * Throws on a malformed expression.
 */
export function expressionOk(expr) {
  const tokens = tokenize(expr);
  let at = 0;
  const peek = () => tokens[at]?.toUpperCase();
  function primary() {
    const t = tokens[at++];
    if (t === undefined) throw new Error(`unexpected end of "${expr}"`);
    if (t === "(") {
      const v = orExpr();
      if (tokens[at++] !== ")") throw new Error(`missing ")" in "${expr}"`);
      return v;
    }
    if (t === ")" || ["AND", "OR", "WITH"].includes(t.toUpperCase())) throw new Error(`unexpected "${t}" in "${expr}"`);
    if (peek() === "WITH") at += 2;
    return !COPYLEFT.test(t);
  }
  function andExpr() {
    let v = primary();
    while (peek() === "AND") {
      at++;
      v = primary() && v;
    }
    return v;
  }
  function orExpr() {
    let v = andExpr();
    while (peek() === "OR") {
      at++;
      v = andExpr() || v;
    }
    return v;
  }
  const v = orExpr();
  if (at !== tokens.length) throw new Error(`unexpected "${tokens[at]}" in "${expr}"`);
  return v;
}

/** A lockfile entry's license as one expression, or null when there is none to judge. */
export function licenseExpression(entry) {
  const l = entry.license ?? entry.licenses;
  const one = (x) => (typeof x === "string" ? x : typeof x?.type === "string" ? x.type : null);
  if (Array.isArray(l)) {
    const parts = l.map(one);
    return parts.length && parts.every(Boolean) ? parts.map((p) => `(${p})`).join(" OR ") : null;
  }
  const s = one(l)?.trim();
  if (!s || /^UNLICENSED$/i.test(s) || /^SEE LICEN[CS]E IN /i.test(s)) return null;
  return s;
}

/** Package name of a lockfile path ("node_modules/a/node_modules/@s/b" -> "@s/b"). */
export function packageName(key, entry) {
  return entry.name ?? key.slice(key.lastIndexOf("node_modules/") + "node_modules/".length);
}

/** Problems of one parsed lockfile ("<lockfile>: <name>@<version>: <problem>"); the root entry is skipped. */
export function lockfileProblems(lockfile, lock, reviewed = REVIEWED) {
  const problems = [];
  for (const [key, entry] of Object.entries(lock.packages ?? {})) {
    if (key === "" || entry.link) continue;
    const name = packageName(key, entry);
    const at = `${lockfile}: ${name}@${entry.version ?? "?"}`;
    const expr = licenseExpression(entry);
    if (expr === null) {
      if (!reviewed[name]) problems.push(`${at}: no license (review it and list it in REVIEWED with a reason)`);
      continue;
    }
    try {
      if (!expressionOk(expr)) problems.push(`${at}: "${expr}" has no option outside GPL, LGPL and AGPL`);
    } catch (e) {
      problems.push(`${at}: ${e.message}`);
    }
  }
  return problems;
}

function main() {
  const args = process.argv.slice(2);
  const files = args.length ? args.map((f) => path.relative(REPO_ROOT, path.resolve(f)).split(path.sep).join("/")) : LOCKFILES;
  const problems = files.flatMap((f) => lockfileProblems(f, JSON.parse(fs.readFileSync(path.join(REPO_ROOT, f), "utf8"))));
  if (problems.length === 0) return;
  console.error("dependency licenses: no dependency may be under GPL, LGPL or AGPL only (docs/dev-guide.md, \"Dependency licenses\"):");
  console.error(problems.join("\n"));
  process.exit(1);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
