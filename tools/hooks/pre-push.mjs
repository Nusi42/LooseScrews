#!/usr/bin/env node
// Git pre-push (.githooks/pre-push): first every pushed commit's message and
// author / committer identity (tools/check-commit.mjs; the pushed refs come on
// stdin, commits already on the remote are skipped), then the checks: a push to a
// local path remote (the master checkout) runs the fast CI subset (STEPS), a push
// to a network remote (the public repository) the full check (tools/ci.*: release
// bot tests, wasm32 tests, WASM build, determinism cross-check, browser smoke).
// Git passes the remote's name and URL as arguments. `--list` prints the fast steps.
//
// Every step's output is shown live and also kept in
// target/pre-push/<n>-<step>.log, the full check's in target/pre-push/full-check.log
// (overwritten by the next push), so a failure under load can be read afterwards
// even when the pushing terminal or agent cut the output short; a failed step
// names its log.
import { spawn, spawnSync } from "node:child_process";
import { createWriteStream, mkdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { IDENTITY_HINT, MESSAGE_HINT, checkIdentity, checkMessage } from "../check-commit.mjs";
import { loadPrivateList } from "../check-private.mjs";
import { REPO_ROOT, assertWorkFolder } from "../lib/work-folder.mjs";

export const STEPS = [
  ["cargo fmt --check", "cargo", ["fmt", "--all", "--", "--check"], "."],
  ["generated reference docs", "node", ["tools/gen-weapon-docs.mjs", "--check"], "."],
  ["wildcard binds", "node", ["tools/check-binds.mjs"], "."],
  ["private information", "node", ["tools/check-private.mjs"], "."],
  ["dependency licenses", "node", ["tools/check-licenses.mjs"], "."],
  ["tools tests", "node", ["--test", "tools/test/*.test.mjs"], "."],
  ["cargo clippy -D warnings", "cargo", ["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"], "."],
  ["cargo test --workspace", "cargo", ["test", "--workspace"], "."],
  ["web typecheck", "npm", ["run", "typecheck"], "web"],
  ["web unit tests", "npm", ["test"], "web"],
];

/** The full check of a push to the public remote: tools/ci.ps1 on Windows, tools/ci.sh elsewhere. */
export function fullCheck(platform = process.platform) {
  return platform === "win32"
    ? ["full check (tools/ci.ps1)", "pwsh", ["-NoProfile", "-File", "tools/ci.ps1"], "."]
    : ["full check (tools/ci.sh)", "bash", ["tools/ci.sh"], "."];
}

/**
 * Which checks a push to `url` runs: "fast" for a local path (the master checkout:
 * `C:\...`, `/...`, `../x`, `\\host\share`, `file://...`), "full" for a network
 * URL (`https://...`, `ssh://...`, scp-like `git@host:path`).
 */
export function pushMode(url) {
  const u = (url ?? "").trim();
  if (/^file:\/\//i.test(u)) return "fast";
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(u)) return "full";
  if (/^[a-z]:([\\/]|$)/i.test(u)) return "fast";
  const colon = u.indexOf(":");
  const slash = u.search(/[\\/]/);
  return colon > 0 && (slash < 0 || colon < slash) ? "full" : "fast";
}

/** The pushed refs git writes on stdin: `<local ref> <local sha> <remote ref> <remote sha>` per line. */
export function parsePushLines(text) {
  return text
    .split(/\r?\n/)
    .map((l) => l.trim().split(/\s+/))
    .filter((f) => f.length === 4)
    .map(([localRef, localSha, remoteRef, remoteSha]) => ({ localRef, localSha, remoteRef, remoteSha }));
}

function gitOut(cwd, args, input) {
  const r = spawnSync("git", args, { cwd, input, maxBuffer: 1 << 28 });
  if (r.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${r.stderr}`);
  return r.stdout;
}

/** The commits a push of `refs` to `remote` adds (deletions skipped, commits on any `remote` ref excluded). */
export function pushedCommits(cwd, remote, refs) {
  const shas = new Set();
  for (const { localSha } of refs) {
    if (/^0+$/.test(localSha)) continue;
    const out = gitOut(cwd, ["rev-list", localSha, "--not", `--remotes=${remote}`]).toString("utf8");
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

/** Problems of the commits a push adds ("<sha> <subject>: <problem>"). */
export function pushProblems(cwd, remote, stdinText, list) {
  const problems = [];
  for (const c of readCommits(cwd, pushedCommits(cwd, remote, parsePushLines(stdinText)))) {
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

/** Log file name of step `i` (`03-wildcard-binds.log`). */
export function logName(i, name) {
  const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  return `${String(i + 1).padStart(2, "0")}-${slug}.log`;
}

/** Runs one step, teeing its output to the console and `logFile`; resolves to the exit code. */
export function runStep(cmd, args, cwd, logFile) {
  return new Promise((resolve) => {
    const log = createWriteStream(logFile);
    // One command string (constant arguments): npm is a .cmd on Windows and needs a shell.
    const line = [cmd, ...args.map((a) => (a.includes("*") ? `"${a}"` : a))].join(" ");
    log.write(`$ ${line}   (in ${cwd}, ${new Date().toISOString()})\n`);
    const child = spawn(line, { cwd, shell: true, stdio: ["inherit", "pipe", "pipe"] });
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

async function main() {
  if (process.argv.includes("--list")) {
    for (const [name] of STEPS) console.log(name);
    return 0;
  }
  assertWorkFolder("git push");
  const stdin = process.stdin.isTTY ? "" : readFileSync(0, "utf8");
  const remote = process.argv[2] ?? "origin";
  const url = process.argv[3] ?? spawnSync("git", ["remote", "get-url", remote], { cwd: REPO_ROOT, encoding: "utf8" }).stdout;
  console.log("\n==> pre-push: commit messages and identities");
  const problems = pushProblems(REPO_ROOT, remote, stdin, loadPrivateList());
  if (problems.length) {
    console.error(`pre-push: push refused\n${problems.slice(0, 40).map((p) => `  ${p}`).join("\n")}`);
    console.error(`${MESSAGE_HINT}\n${IDENTITY_HINT}`);
    console.error("Reword with git commit --amend or git rebase; fix the identity with git commit --amend --reset-author.");
    return 1;
  }
  const logDir = path.join(REPO_ROOT, "target", "pre-push");
  mkdirSync(logDir, { recursive: true });
  for (const [i, [name, cmd, args, dir]] of STEPS.entries()) {
    console.log(`\n==> pre-push: ${name}`);
    const logFile = path.join(logDir, logName(i, name));
    const code = await runStep(cmd, args, path.join(REPO_ROOT, dir), logFile);
    if (code !== 0) {
      console.error(`pre-push: "${name}" failed; push refused (git push --no-verify skips it, don't)`);
      console.error(`pre-push: full output of the step: ${logFile}`);
      return 1;
    }
  }
  return 0;
}

// Run only as a script (tests import STEPS, logName, pushMode, fullCheck and the commit checks).
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = await main();
}
