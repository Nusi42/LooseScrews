#!/usr/bin/env node
// Git pre-push (.githooks/pre-push): the fast CI subset. The full suite
// (release bot tests, wasm32 tests, WASM build, determinism cross-check, browser
// smoke) is tools/ci.*, run in win1 before main moves. `--list` prints the steps.
//
// Every step's output is shown live and also kept in
// target/pre-push/<n>-<step>.log (overwritten by the next push), so a failure
// under load can be read afterwards even when the pushing terminal or agent
// cut the output short; a failed step names its log.
import { spawn } from "node:child_process";
import { createWriteStream, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { REPO_ROOT, assertWorkFolder } from "../lib/work-folder.mjs";

export const STEPS = [
  ["cargo fmt --check", "cargo", ["fmt", "--all", "--", "--check"], "."],
  ["generated reference docs", "node", ["tools/gen-weapon-docs.mjs", "--check"], "."],
  ["wildcard binds", "node", ["tools/check-binds.mjs"], "."],
  ["tools tests", "node", ["--test", "tools/test/*.test.mjs"], "."],
  ["cargo clippy -D warnings", "cargo", ["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"], "."],
  ["cargo test --workspace", "cargo", ["test", "--workspace"], "."],
  ["web typecheck", "npm", ["run", "typecheck"], "web"],
  ["web unit tests", "npm", ["test"], "web"],
];

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

// Run only as a script (tests import STEPS and logName).
if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1"))) {
  process.exitCode = await main();
}
