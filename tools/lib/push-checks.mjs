// The checks of the commits a push adds and the runner of the plan's steps
// (tools/hooks/pre-push.mjs, tools/check-linear.mjs). What a push adds: the commits on no
// remote-tracking branch and not under the remote's old value (`pushedRange`).
// Tests: tools/test/pre-push.test.mjs, tools/test/hooks.test.mjs.

import { spawn, spawnSync } from "node:child_process";
import { createWriteStream } from "node:fs";

import { checkIdentity, checkMessage } from "../check-commit.mjs";
import { hasCommit, parsePushLines, pushedRange } from "./push-plan.mjs";

function gitOut(cwd, args, input) {
  const r = spawnSync("git", args, { cwd, input, maxBuffer: 1 << 28 });
  if (r.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${r.stderr}`);
  return r.stdout;
}

/** The commits a push of `refs` adds (deletions skipped), newest first per ref. */
export function pushedCommits(cwd, refs) {
  const shas = new Set();
  for (const ref of refs) {
    const range = pushedRange(ref, (sha) => hasCommit(cwd, sha));
    if (range === undefined) continue;
    const out = gitOut(cwd, ["rev-list", ...range]).toString("utf8");
    for (const sha of out.split("\n").filter(Boolean)) shas.add(sha);
  }
  return [...shas];
}

/** Raw commits `shas`: [{ sha, parents, author, committer, message }] (one git cat-file call). */
export function readCommits(cwd, shas) {
  if (shas.length === 0) return [];
  const buf = gitOut(cwd, ["cat-file", "--batch"], `${shas.join("\n")}\n`);
  const commits = [];
  let at = 0;
  while (at < buf.length) {
    const eol = buf.indexOf(10, at);
    const [sha, type, size] = buf.subarray(at, eol).toString("utf8").split(" ");
    const end = eol + 1 + Number(size);
    const body = buf.subarray(eol + 1, end).toString("utf8");
    at = end + 1;
    if (type !== "commit") continue;
    const split = body.indexOf("\n\n");
    const headers = (split < 0 ? body : body.slice(0, split)).split("\n");
    const field = (key) => headers.find((h) => h.startsWith(`${key} `))?.slice(key.length + 1) ?? "";
    commits.push({
      sha,
      parents: headers.filter((h) => h.startsWith("parent ")).length,
      author: field("author"),
      committer: field("committer"),
      message: split < 0 ? "" : body.slice(split + 2),
    });
  }
  return commits;
}

/** Problems of the commits a push adds ("<sha> <subject>: <problem>"); `stdinText`: git's ref lines. */
export function pushProblems(cwd, stdinText, list) {
  const problems = [];
  for (const c of readCommits(cwd, pushedCommits(cwd, parsePushLines(stdinText)))) {
    const at = `${c.sha.slice(0, 10)} ${c.message.split("\n")[0].slice(0, 50)}`;
    const found = [
      ...checkMessage(c.message, { merge: c.parents > 1, list }).map((p) => `message: ${p}`),
      ...checkIdentity(c.author, list).map((p) => `author ${p}`),
      ...checkIdentity(c.committer, list).map((p) => `committer ${p}`),
    ];
    problems.push(...found.map((p) => `${at}: ${p}`));
  }
  return problems;
}

/** The merge commits a push adds ("<short sha> <subject>"), docs/adr/0005-linear-history.md. */
export function mergeCommits(cwd, stdinText) {
  return readCommits(cwd, pushedCommits(cwd, parsePushLines(stdinText)))
    .filter((c) => c.parents > 1)
    .map((c) => `${c.sha.slice(0, 10)} ${c.message.split("\n")[0]}`);
}

/** How to combine branches instead of merging them (printed by tools/check-linear.mjs). */
export const REBASE_HINT =
  "Combine branches by rebasing onto the target and fast-forwarding, never with a merge commit:\n" +
  "  git switch -C integrate origin/<branch> && git rebase <target> && git switch <target> && git merge --ff-only integrate\n" +
  "(docs/adr/0005-linear-history.md)";

/** Log file name of step `i` (`03-wildcard-binds.log`). */
export function logName(i, name) {
  const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  return `${String(i + 1).padStart(2, "0")}-${slug}.log`;
}

/**
 * Runs one step, teeing its output to the console and `logFile`; resolves to the exit code.
 * `input`: text for its stdin (else the step inherits this process's stdin).
 */
export function runStep(cmd, args, cwd, logFile, input) {
  return new Promise((resolve) => {
    const log = createWriteStream(logFile);
    // One command string (constant arguments): npm is a .cmd on Windows and needs a shell.
    const line = [cmd, ...args.map((a) => (/[*\s]/.test(a) && !a.startsWith('"') ? `"${a}"` : a))].join(" ");
    log.write(`$ ${line}   (in ${cwd}, ${new Date().toISOString()})\n`);
    const piped = input !== undefined;
    const child = spawn(line, { cwd, shell: true, stdio: [piped ? "pipe" : "inherit", "pipe", "pipe"] });
    if (piped) child.stdin.end(input);
    child.stdout.on("data", (b) => {
      process.stdout.write(b);
      log.write(b);
    });
    child.stderr.on("data", (b) => {
      process.stderr.write(b);
      log.write(b);
    });
    const done = (code) => log.end(`\n[exit code ${code}, ${new Date().toISOString()}]\n`, () => resolve(code));
    child.on("error", (e) => {
      log.write(`spawn error: ${e.message}\n`);
      done(-1);
    });
    child.on("close", (code) => done(code ?? -1));
  });
}
