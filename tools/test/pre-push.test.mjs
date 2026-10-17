// Tests of tools/hooks/pre-push.mjs: fast or full checks by remote URL, step logs (the output of every step is kept) and
// the checks of the pushed commits (throwaway repositories, a made-up private list).
import assert from "node:assert/strict";
import fs, { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { STEPS, fullCheck, logName, parsePushLines, pushProblems, pushedCommits, pushMode, readCommits, runStep } from "../hooks/pre-push.mjs";
import { commitRaw, git, tempRepo } from "./git-fixture.mjs";
import { FIXTURE_LIST } from "./private-list-fixture.mjs";

test("every step has a distinct log name", () => {
  const names = STEPS.map(([name], i) => logName(i, name));
  assert.equal(new Set(names).size, STEPS.length);
  assert.equal(logName(5, "cargo test --workspace"), "06-cargo-test-workspace.log");
});

test("a push to a network remote runs the full check, a push to a local path the fast subset", () => {
  for (const url of ["https://github.com/example/repo.git", "http://host/x", "ssh://git@host/x.git", "git://host/x", "git@github.com:example/repo.git", "host:repo", "  https://h/x\n"]) {
    assert.equal(pushMode(url), "full", url);
  }
  for (const url of ["C:\\work\\loosescrews", "c:/work/loosescrews", "D:", "/home/x/loosescrews", "../loosescrews", "loosescrews", "\\\\server\\share\\repo", "file:///C:/work/repo", "./a:b", "", undefined]) {
    assert.equal(pushMode(url), "fast", String(url));
  }
});

test("the full check is tools/ci.ps1 on Windows and tools/ci.sh elsewhere; the fast subset checks licenses", () => {
  assert.deepEqual(fullCheck("win32").slice(1, 3), ["pwsh", ["-NoProfile", "-File", "tools/ci.ps1"]]);
  assert.deepEqual(fullCheck("linux").slice(1, 3), ["bash", ["tools/ci.sh"]]);
  assert.ok(STEPS.some(([, cmd, args]) => cmd === "node" && args[0] === "tools/check-licenses.mjs"));
});

test("runStep keeps the output and the exit code of a failing step", async () => {
  const dir = mkdtempSync(join(tmpdir(), "pre-push-"));
  const log = join(dir, "step.log");
  const code = await runStep("node", ["-e", "\"console.log('out-line'); console.error('err-line'); process.exit(3)\""], dir, log);
  assert.equal(code, 3);
  const text = readFileSync(log, "utf8");
  assert.match(text, /out-line/);
  assert.match(text, /err-line/);
  assert.match(text, /\[exit code 3,/);
});

test("the pushed refs are parsed; deletions and commits already on the remote are skipped", () => {
  const refs = parsePushLines(`refs/heads/a ${"1".repeat(40)} refs/heads/a ${"0".repeat(40)}\r\n\nbad line\n`);
  assert.deepEqual(refs, [{ localRef: "refs/heads/a", localSha: "1".repeat(40), remoteRef: "refs/heads/a", remoteSha: "0".repeat(40) }]);
  const repo = tempRepo();
  const pub = `${FIXTURE_LIST.publicIdentity.name} <${FIXTURE_LIST.publicIdentity.email}> 1760000000 +0200`;
  const base = commitRaw(repo, { author: pub, message: "an old commit before the rules\n" });
  git(repo, ["update-ref", "refs/remotes/origin/main", base]);
  const good = commitRaw(repo, { parents: [base], author: pub, message: "feat(sim): add the tether\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n" });
  const badMsg = commitRaw(repo, { parents: [good], author: pub, message: "WIP on WIN3\n" });
  const badWho = commitRaw(repo, { parents: [badMsg], author: "Jane Roe <jane.roe@example.org> 1760000000 +0200", committer: pub, message: "fix: x\n" });
  const merge = commitRaw(repo, { parents: [badWho, base], author: pub, message: "Merge the tether gadget\n" });
  assert.deepEqual(new Set(pushedCommits(repo, "origin", parsePushLines(`refs/heads/x ${merge} refs/heads/x ${"0".repeat(40)}`))), new Set([good, badMsg, badWho, merge]));
  const commits = readCommits(repo, [merge]);
  assert.equal(commits[0].parents, 2);
  assert.equal(commits[0].message, "Merge the tether gadget\n");
  const line = (sha) => `refs/heads/x ${sha} refs/heads/x ${base}`;
  assert.deepEqual(pushProblems(repo, "origin", line(good), FIXTURE_LIST), []);
  assert.deepEqual(pushProblems(repo, "origin", `refs/heads/gone ${"0".repeat(40)} refs/heads/gone ${good}`, FIXTURE_LIST), []);
  const problems = pushProblems(repo, "origin", line(merge), FIXTURE_LIST);
  assert.ok(problems.some((p) => p.startsWith(badMsg.slice(0, 10)) && /message: the subject is not/.test(p)), problems.join("\n"));
  assert.ok(problems.some((p) => p.startsWith(badWho.slice(0, 10)) && /author name contains a name from the private list/.test(p)));
  assert.ok(!problems.some((p) => p.startsWith(merge.slice(0, 10)) || p.startsWith(good.slice(0, 10))));
  assert.ok(problems.every((p) => !/Jane|Roe/.test(p)));
  fs.rmSync(repo, { recursive: true, force: true });
});
