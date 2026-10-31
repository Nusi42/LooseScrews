#!/usr/bin/env node
// Git pre-push (.githooks/pre-push): decides from the pushing checkout's role, the remote's
// URL and the pushed refs what the push runs (`pushPlan`, tools/lib/push-plan.mjs;
// docs/adr/0004-local-verification.md, docs/adr/0005-linear-history.md). A refusal names
// the command to use instead. Otherwise it checks every pushed commit's message and
// author / committer identity (tools/check-commit.mjs), then runs the plan's steps in the
// pushing checkout. Git passes the remote's name and URL as arguments and the ref lines on
// stdin. `--list` prints the steps of a work folder's push.
//
// Every step's output is shown live and also kept in target/pre-push/<n>-<step>.log
// (overwritten by the next push), so a failure under load can be read afterwards even when
// the pushing terminal or agent cut the output short; a failed step names its log.
//
// Runs on load (no "is main module" test): a hook that silently did nothing would let every
// push through. Never bypass it with `git push --no-verify`: fix the finding instead.
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync } from "node:fs";
import path from "node:path";

import { IDENTITY_HINT, MESSAGE_HINT } from "../check-commit.mjs";
import { loadPrivateList } from "../check-private.mjs";
import { logName, pushProblems, runStep } from "../lib/push-checks.mjs";
import { WORK_STEPS, masterOf, parsePushLines, pushPlan, pushRole } from "../lib/push-plan.mjs";
import { REPO_ROOT } from "../lib/work-folder.mjs";

/** Node steps name a script of the hook's own checkout (REPO_ROOT); they check the pushing one. */
function stepArgs(cmd, args) {
  return cmd === "node" && args[0]?.startsWith("tools/") ? [path.join(REPO_ROOT, ...args[0].split("/")), ...args.slice(1)] : args;
}

async function main(argv) {
  if (argv.includes("--list")) {
    for (const [name] of WORK_STEPS) console.log(name);
    return 0;
  }
  const [remote = "origin", url = remote] = argv;
  const top = spawnSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" });
  if (top.status !== 0) throw new Error(`not in a git checkout: ${top.stderr.trim()}`);
  const root = path.resolve(top.stdout.trim());
  const stdin = readFileSync(0, "utf8");
  const plan = pushPlan({ role: pushRole(root), url, refs: parsePushLines(stdin), master: masterOf(root) });
  if (plan.refused) {
    console.error(`pre-push: push to ${remote} refused: ${plan.refused}\n(docs/dev-guide.md "Work folders")`);
    return 1;
  }
  console.log("\n==> pre-push: commit messages and identities");
  const problems = pushProblems(root, stdin, loadPrivateList());
  if (problems.length) {
    console.error(`pre-push: push refused\n${problems.slice(0, 40).map((p) => `  ${p}`).join("\n")}`);
    console.error(`${MESSAGE_HINT}\n${IDENTITY_HINT}`);
    console.error("Reword with git commit --amend or git rebase; fix the identity with git commit --amend --reset-author.");
    return 1;
  }
  const logDir = path.join(root, "target", "pre-push");
  mkdirSync(logDir, { recursive: true });
  for (const [i, [name, cmd, args, dir, options]] of plan.steps.entries()) {
    console.log(`\n==> pre-push: ${name}`);
    const logFile = path.join(logDir, logName(i, name));
    const code = await runStep(cmd, stepArgs(cmd, args), path.join(root, dir), logFile, options.stdin ? stdin : undefined);
    if (code !== 0) {
      console.error(`pre-push: "${name}" failed; push refused (git push --no-verify skips it, don't)`);
      console.error(`pre-push: full output of the step: ${logFile}`);
      return 1;
    }
  }
  return 0;
}

try {
  process.exitCode = await main(process.argv.slice(2));
} catch (e) {
  console.error(`pre-push: ${e instanceof Error ? e.message : String(e)}; push refused`);
  process.exitCode = 1;
}
