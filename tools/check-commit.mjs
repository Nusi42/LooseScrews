#!/usr/bin/env node
// Commit messages and commit identities of the public repository
// (docs/adr/0002-public-repository.md). Used by the commit-msg hook (the message
// being written, the author and committer ident) and by pre-push (every pushed
// commit). Tests: tools/test/check-commit.test.mjs.
//   - Messages follow Conventional Commits: `<type>(<scope>): <subject>`, first line
//     at most 72 characters, no trailing period, the subject starts lowercase. Merge
//     commits say `Merge <what it brought>`; git's `Revert "..."` and `fixup!` /
//     `squash!` subjects are accepted. Trailers: the Claude co-author line and
//     `Signed-off-by: <name> <email>` lines (`git commit -s`); their names and emails
//     are checked against the private list like the rest of the message.
//   - Messages hold no internal logistics or process jargon, no URL except the public
//     repository's, no session references, and nothing from the private list
//     (tools/check-private.mjs, whose matcher this reuses).
//   - Identities: with a private list that names a `publicIdentity`, author and
//     committer must be exactly that; nothing may match the private list.
//
//   node tools/check-commit.mjs [--merge] <message file>   exit 1 on a problem
//   node tools/check-commit.mjs --ident "<Name <email> <time> <tz>>"

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { BUILT_IN, findListValues, loadPrivateList } from "./check-private.mjs";

/** Conventional Commit types. */
export const TYPES = ["feat", "fix", "perf", "refactor", "test", "docs", "build", "ci", "chore", "style", "revert"];

/** Maximum length of the first line. */
export const MAX_SUBJECT = 72;

/** The one URL a message may contain (and anything below it). */
export const PUBLIC_REPO = "github.com/Nusi42/LooseScrews";

/** How to fix a refused message (printed by the hooks). */
export const MESSAGE_HINT =
  'Commit messages: Conventional Commits, e.g. "fix(net): drop stale snapshots" (docs/adr/0002-public-repository.md).\n' +
  'Merges need an explicit message: git merge -m "Merge <what it brought>" <branch>.';

/** How to fix a refused identity. */
export const IDENTITY_HINT =
  "Commit identity: git config user.name / user.email (the public identity of the private list), " +
  'see docs/dev-guide.md, "Commit messages and identity".';

const SUBJECT = new RegExp(`^(${TYPES.join("|")})(\\([a-z0-9-]+\\))?!?: (.*)$`);
const TRAILER = /^([A-Za-z][A-Za-z0-9]*(?:-[A-Za-z0-9]+)*):\s/;
const CO_AUTHOR = /^Claude [A-Za-z0-9][A-Za-z0-9 .-]* <noreply@anthropic\.com>$/;
const SIGN_OFF = /^\S.* <[^<>\s@]+@[^<>\s]+>$/;

/**
 * Terms no message may contain: [pattern, label]. Whole words, case-insensitive;
 * code-like patterns (milestone codes, acronyms) are case-sensitive so that Rust
 * types (`f32`, `i64`) and plain words ("dry run") pass.
 */
export const FORBIDDEN = [
  [/\bwin(?:\d+|n)\b/gi, "logistics"],
  [/\bwork[- ]folders?\b/gi, "logistics"],
  [/\bmasters?\b/gi, "logistics"],
  [/\blead\b/gi, "logistics"],
  [/\bfast[- ]forward(?:s|ed|ing)?\b/gi, "logistics"],
  [/\borigin\/[\w./-]*/gi, "logistics"],
  [/\bWIP\b/gi, "logistics"],
  [/\bwork in progress\b/gi, "logistics"],
  [/\bwaves?\b/gi, "process jargon"],
  [/\bwork[- ]packages?\b/gi, "process jargon"],
  [/\bWP\d+\b/gi, "process jargon"],
  [/\bcontracts?\b/gi, "process jargon"],
  [/(?<!\b(?:weapon|gadget) )\bslots?\b/gi, "process jargon"],
  [/\bM\d+\b/g, "a milestone code"],
  [/\b[A-I]\d{1,2}\b/g, "a package code"],
  [/\bCQ-[\w-]*/gi, "a package code"],
  [/\b(?:SSOT|ODR|DRY)\b/g, "process jargon"],
  [/(?<!\b(?:projectile|key|file|window|entity) )\bowners?\b(?! trust\b)/gi, "process jargon"],
  [/\bsessions?\b/gi, "process jargon"],
  [/\bClaude-Session\b/gi, "a session reference"],
  [/\bnext-steps\b/gi, "process jargon"],
  [/\borchestrators?\b/gi, "process jargon"],
  [/\bcolent\b/gi, "the old project name"],
  [/\bone ?drive\b/gi, "a local path"],
  [/\bself ?made\b/gi, "a local path"],
];

const URL = /\b(?:[a-z][a-z0-9+.-]*:\/\/|www\.)[^\s<>"'`)\]]+|\b(?:[a-z0-9-]+\.)+[a-z]{2,}\/[^\s<>"'`)\]]*/gi;
const ALLOWED_URL = new RegExp(`^(?:https?://)?(?:www\\.)?${PUBLIC_REPO.replace(/[./]/g, "\\$&")}(?:[/#?]\\S*)?\\.?$`, "i");

/** The message as git stores it: comment lines and everything below the scissors line dropped. */
export function cleanMessage(text) {
  const lines = [];
  for (const line of text.replace(/\r\n/g, "\n").split("\n")) {
    if (/^# -+ >8 -+$/.test(line)) break;
    if (line.startsWith("#")) continue;
    lines.push(line.replace(/\s+$/, ""));
  }
  while (lines.length && lines[0] === "") lines.shift();
  while (lines.length && lines[lines.length - 1] === "") lines.pop();
  return lines.join("\n");
}

function checkSubject(subject, merge) {
  const out = [];
  if (merge) {
    if (!/^Merge \S/.test(subject)) out.push('a merge commit\'s subject is "Merge <what it brought>"');
  } else if (/^(?:fixup|squash|amend)! \S/.test(subject)) {
    return checkSubject(subject.replace(/^(?:fixup|squash|amend)! /, ""), merge).filter((p) => !p.startsWith("the first line is"));
  } else if (/^Revert ".+"$/.test(subject)) {
    return out;
  } else {
    const m = SUBJECT.exec(subject);
    if (!m) {
      out.push(`the subject is not "<type>(<scope>): <subject>" (types: ${TYPES.join(", ")}; scope [a-z0-9-]+, optional)`);
    } else if (m[3] === "") {
      out.push("the subject text is empty");
    } else if (/^\p{Lu}/u.test(m[3])) {
      out.push("the subject starts with a lowercase letter");
    }
  }
  if ([...subject].length > MAX_SUBJECT) out.push(`the first line is ${[...subject].length} characters (at most ${MAX_SUBJECT})`);
  if (/\.$/.test(subject)) out.push("the subject ends without a period");
  return out;
}

function checkTrailers(paragraphs) {
  if (paragraphs.length < 2) return [];
  const out = [];
  for (const line of paragraphs[paragraphs.length - 1]) {
    const m = TRAILER.exec(line);
    if (!m) continue;
    const value = line.slice(m[0].length).trim();
    const key = m[1].toLowerCase();
    if (key === "co-authored-by" && CO_AUTHOR.test(value)) continue;
    if (key === "signed-off-by" && SIGN_OFF.test(value)) continue;
    out.push(
      `trailer "${m[1]}:" is not allowed (allowed: "Co-Authored-By: Claude <model> <noreply@anthropic.com>", "Signed-off-by: <name> <email>")`,
    );
  }
  return out;
}

function checkTerms(text, list) {
  const out = [];
  for (const [re, label] of FORBIDDEN) {
    for (const m of text.matchAll(re)) out.push(`"${m[0]}" (${label}) is not allowed in a commit message`);
  }
  for (const m of text.matchAll(URL)) {
    if (!ALLOWED_URL.test(m[0])) out.push(`a URL other than ${PUBLIC_REPO} is not allowed (${m[0].replace(/^(\S{0,24}).*$/, "$1")}...)`);
  }
  for (const [re, reason] of BUILT_IN) if (re.test(text)) out.push(`contains ${reason}`);
  for (const { kind, masked } of findListValues(text, list)) out.push(`contains ${kind} (${masked})`);
  return [...new Set(out)];
}

/** Problems of commit message `text` (empty when it is fine). `merge`: the commit has several parents. */
export function checkMessage(text, { merge = false, list = null } = {}) {
  const msg = cleanMessage(text);
  if (msg === "") return ["the message is empty"];
  const lines = msg.split("\n");
  const out = checkSubject(lines[0], merge);
  if (lines.length > 1 && lines[1] !== "") out.push("the second line is blank (subject, blank line, body)");
  const paragraphs = msg.split(/\n{2,}/).map((p) => p.split("\n"));
  out.push(...checkTrailers(paragraphs), ...checkTerms(msg, list));
  return out;
}

/** Problems of a git ident `Name <email> <time> <tz>` (author or committer). */
export function checkIdentity(ident, list = null) {
  const m = /^(.*?) ?<([^<>]*)>(?: (\d+) ([+-]\d{4}))?$/.exec(ident.trim());
  if (!m) return ["is not a git identity (Name <email> <time> <tz>)"];
  const [, name, email] = m;
  if (email.trim() === "") return ["has an empty email"];
  if (!list) return [];
  const pub = list.publicIdentity;
  if (pub && typeof pub.name === "string" && typeof pub.email === "string") {
    if (name === pub.name && email.toLowerCase() === pub.email.toLowerCase()) return [];
  }
  const out = [];
  for (const [what, value] of [["name", name], ["email", email]]) {
    for (const { kind, masked } of findListValues(value, list, { handle: false })) out.push(`${what} contains ${kind} (${masked})`);
  }
  if (pub) out.push("is not the public identity (publicIdentity of the private list)");
  return out;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const list = loadPrivateList();
  let problems;
  if (args[0] === "--ident" && args[1] !== undefined) {
    problems = checkIdentity(args[1], list).map((p) => `identity ${p}`);
  } else {
    const merge = args.includes("--merge");
    const file = args.find((a) => !a.startsWith("--"));
    if (file === undefined) {
      console.error('usage: node tools/check-commit.mjs [--merge] <message file> | --ident "<ident>"');
      process.exit(2);
    }
    problems = checkMessage(fs.readFileSync(file, "utf8"), { merge, list });
  }
  if (problems.length) {
    console.error(`check-commit:\n${problems.map((p) => `  ${p}`).join("\n")}`);
    process.exit(1);
  }
}
