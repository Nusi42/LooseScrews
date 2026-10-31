#!/usr/bin/env node
// No merge commits in what a push adds (docs/adr/0005-linear-history.md): branches are
// combined by rebasing, so `main` stays a straight line (the public repository's ruleset
// requires it). The pre-push hook runs it on pushes to and from the master
// (tools/lib/push-plan.mjs); it reads git's ref lines on stdin and checks the repository in
// the current directory. Merge commits already on a remote stay.
//
// Runs on load (no "is main module" test): a check that silently did nothing would let
// every push through.
import { readFileSync } from "node:fs";

import { REBASE_HINT, mergeCommits } from "./lib/push-checks.mjs";

try {
  const merges = mergeCommits(process.cwd(), readFileSync(0, "utf8"));
  if (merges.length === 0) {
    console.log("check-linear: no merge commits to push");
  } else {
    for (const merge of merges) console.error(`merge commit ${merge}`);
    console.error(`check-linear: ${merges.length} merge commit(s) in the push.\n${REBASE_HINT}`);
    process.exitCode = 1;
  }
} catch (e) {
  console.error(`check-linear: ${e instanceof Error ? e.message : String(e)}`);
  process.exitCode = 1;
}
