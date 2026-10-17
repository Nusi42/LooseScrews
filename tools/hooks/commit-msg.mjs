#!/usr/bin/env node
// Git commit-msg (.githooks/commit-msg): the message being committed and the
// author and committer identity must follow the public-repository rules
// (tools/check-commit.mjs, docs/adr/0002-public-repository.md). `git merge` runs
// this hook too: a merge (MERGE_HEAD present) needs a "Merge <what it brought>"
// subject. Runs in the repository git runs it in (the current directory).
//   node tools/hooks/commit-msg.mjs <message file>
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { IDENTITY_HINT, MESSAGE_HINT, checkIdentity, checkMessage } from "../check-commit.mjs";
import { loadPrivateList } from "../check-private.mjs";

function git(cwd, args) {
  const r = spawnSync("git", args, { cwd, encoding: "utf8" });
  return r.status === 0 ? r.stdout.trim() : undefined;
}

/** Problems of the commit being made in `cwd` with the message in `file`. */
export function commitMsgProblems(cwd, file, list) {
  const mergeHead = git(cwd, ["rev-parse", "--git-path", "MERGE_HEAD"]);
  const merge = mergeHead !== undefined && fs.existsSync(path.resolve(cwd, mergeHead));
  const problems = checkMessage(fs.readFileSync(file, "utf8"), { merge, list }).map((p) => `message: ${p}`);
  for (const [who, variable] of [["author", "GIT_AUTHOR_IDENT"], ["committer", "GIT_COMMITTER_IDENT"]]) {
    const ident = git(cwd, ["var", variable]);
    if (ident === undefined) problems.push(`${who}: no identity (git config user.name / user.email)`);
    else problems.push(...checkIdentity(ident, list).map((p) => `${who} ${p}`));
  }
  return problems;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const file = process.argv[2];
  if (file === undefined) {
    console.error("usage: node tools/hooks/commit-msg.mjs <message file>");
    process.exit(2);
  }
  const problems = commitMsgProblems(process.cwd(), file, loadPrivateList());
  if (problems.length) {
    const hints = [problems.some((p) => p.startsWith("message")) && MESSAGE_HINT, problems.some((p) => !p.startsWith("message")) && IDENTITY_HINT];
    console.error(`commit-msg: commit refused\n${problems.map((p) => `  ${p}`).join("\n")}\n${hints.filter(Boolean).join("\n")}`);
    process.exit(1);
  }
}
