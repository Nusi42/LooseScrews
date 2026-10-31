// Tests of the push decision (tools/lib/push-plan.mjs, every combination of role, remote and
// refs), the checks of the pushed commits and the step runner (tools/lib/push-checks.mjs;
// throwaway repositories, a made-up private list). The hook itself runs through git in
// tools/test/hooks.test.mjs.
import assert from "node:assert/strict";
import fs, { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path, { join } from "node:path";
import { test } from "node:test";

import { logName, mergeCommits, pushProblems, pushedCommits, readCommits, runStep } from "../lib/push-checks.mjs";
import {
  LINEAR_STEP,
  MASTER_STEPS,
  STEPS,
  WORK_STEPS,
  isReleaseTagRef,
  localPath,
  parsePushLines,
  publishCommand,
  pushPlan,
  pushRole,
  pushedRange,
  remoteKind,
} from "../lib/push-plan.mjs";
import { commitRaw, git, tempRepo } from "./git-fixture.mjs";
import { FIXTURE_LIST } from "./private-list-fixture.mjs";

const ZERO = "0".repeat(40);
const GITHUB = "https://github.com/example/repo.git";
const MASTER = "C:\\work\\loosescrews";
const ref = (localRef, remoteRef = localRef) => ({ localRef, localSha: "1".repeat(40), remoteRef, remoteSha: ZERO });
const MAIN = ref("refs/heads/main");

test("every step has a distinct log name", () => {
  for (const steps of [STEPS, WORK_STEPS, MASTER_STEPS]) {
    const names = steps.map(([name], i) => logName(i, name));
    assert.equal(new Set(names).size, steps.length);
  }
  assert.equal(logName(5, "cargo test --workspace"), "06-cargo-test-workspace.log");
});

test("remote URLs are network URLs or local paths", () => {
  for (const url of ["https://github.com/example/repo.git", "http://host/x", "ssh://git@host/x.git", "git://host/x", "git@github.com:example/repo.git", "host:repo", "  https://h/x\n"]) {
    assert.equal(remoteKind(url), "network", url);
  }
  for (const url of ["C:\\work\\loosescrews", "c:/work/loosescrews", "D:", "/home/x/loosescrews", "../loosescrews", "./x", "loosescrews", "\\\\server\\share\\repo", "file:///C:/work/repo", "./a:b", "", undefined]) {
    assert.equal(remoteKind(url), "local", String(url));
  }
  assert.equal(localPath("/w/a", "../m"), path.resolve("/w/a", "../m"));
});

test("a work folder pushes only to the master, naming the publish command otherwise", () => {
  assert.deepEqual(pushPlan({ role: "work", url: MASTER, refs: [ref("refs/heads/WIN3")] }), { steps: WORK_STEPS });
  assert.deepEqual(pushPlan({ role: "work", url: "../loosescrews", refs: [] }), { steps: WORK_STEPS });
  for (const url of [GITHUB, "git@github.com:example/repo.git"]) {
    const plan = pushPlan({ role: "work", url, refs: [MAIN], master: MASTER });
    assert.match(plan.refused, /a work folder pushes only to the master checkout/);
    assert.ok(plan.refused.includes(publishCommand(MASTER)), plan.refused);
    assert.ok(plan.refused.includes(`git -C "${MASTER}" push github main`));
  }
  assert.ok(pushPlan({ role: "work", url: GITHUB }).refused.includes('git -C "<master>" push github main'));
});

test("the master publishes only main and release tags, only to a network remote", () => {
  assert.deepEqual(pushPlan({ role: "master", url: GITHUB, refs: [MAIN] }), { steps: MASTER_STEPS });
  assert.deepEqual(pushPlan({ role: "master", url: GITHUB, refs: [MAIN, ref("refs/tags/v0.5.0")] }), { steps: MASTER_STEPS });
  assert.deepEqual(pushPlan({ role: "master", url: GITHUB, refs: [] }), { steps: MASTER_STEPS });
  assert.match(pushPlan({ role: "master", url: MASTER, refs: [MAIN] }).refused, /the master checkout pushes only to the public remote/);
  for (const bad of [ref("refs/heads/WIN1"), ref("refs/heads/main", "refs/heads/other"), ref("refs/heads/x", "refs/heads/main"), ref("refs/tags/test"), ref("refs/tags/v1.0.0", "refs/tags/v1.0.1")]) {
    const plan = pushPlan({ role: "master", url: GITHUB, refs: [MAIN, bad] });
    assert.match(plan.refused, /the master publishes only main \(and release tags v<semver>\)/);
    assert.ok(plan.refused.includes(`${bad.localRef} -> ${bad.remoteRef}`));
  }
});

test("a plain clone keeps the fast checks for every push; unknown roles are refused", () => {
  assert.deepEqual(pushPlan({ role: "plain", url: GITHUB, refs: [MAIN] }), { steps: STEPS });
  assert.deepEqual(pushPlan({ role: "plain", url: MASTER, refs: [MAIN] }), { steps: STEPS });
  assert.match(pushPlan({ role: "other", url: GITHUB }).refused, /unknown checkout role/);
});

test("the plans: the master builds nothing; pushes to and from the master refuse merge commits first", () => {
  assert.deepEqual(MASTER_STEPS.map(([name]) => name), ["no merge commits", "private information", "dependency licenses"]);
  assert.ok(MASTER_STEPS.every(([, cmd]) => cmd === "node"));
  assert.equal(WORK_STEPS[0], LINEAR_STEP);
  assert.deepEqual(WORK_STEPS.slice(1), STEPS);
  assert.ok(!STEPS.includes(LINEAR_STEP));
  assert.deepEqual(LINEAR_STEP[2], ["tools/check-linear.mjs"]);
  assert.equal(LINEAR_STEP[4].stdin, true);
  for (const steps of [STEPS, MASTER_STEPS]) {
    assert.ok(steps.some(([, , args]) => args[0] === "tools/check-licenses.mjs"));
    assert.ok(steps.some(([, , args]) => args[0] === "tools/check-private.mjs"));
  }
});

test("release tags are v<semver>", () => {
  for (const t of ["v0.5.0", "v1.2.3-rc.1", "v10.0.0+build.5"]) assert.ok(isReleaseTagRef(`refs/tags/${t}`), t);
  for (const t of ["refs/tags/0.5.0", "refs/tags/v1.2", "refs/heads/v1.2.3", "refs/tags/vx.y.z"]) assert.ok(!isReleaseTagRef(t), t);
});

test("the checkout role comes from .env.local and the origin", () => {
  const master = tempRepo();
  const work = tempRepo();
  const plain = tempRepo();
  const none = tempRepo();
  try {
    fs.writeFileSync(join(master, ".env.local"), "LS_CHECKOUT_ROLE=master\n");
    git(work, ["remote", "add", "origin", master]);
    git(plain, ["remote", "add", "origin", GITHUB]);
    assert.equal(pushRole(master, {}), "master");
    assert.equal(pushRole(work, {}), "work");
    assert.equal(pushRole(plain, {}), "plain");
    assert.equal(pushRole(none, {}), "plain");
    assert.equal(pushRole(plain, { LS_CHECKOUT_ROLE: "master" }), "master");
  } finally {
    for (const d of [master, work, plain, none]) fs.rmSync(d, { recursive: true, force: true });
  }
});

test("a push adds the commits on no remote-tracking branch and not under the remote's old value", () => {
  const sha = "a".repeat(40);
  const old = "b".repeat(40);
  assert.equal(pushedRange({ localSha: ZERO, remoteSha: old }, () => true), undefined);
  assert.deepEqual(pushedRange({ localSha: sha, remoteSha: ZERO }, () => true), [sha, "--not", "--remotes"]);
  assert.deepEqual(pushedRange({ localSha: sha, remoteSha: old }, () => true), [sha, "--not", "--remotes", old]);
  assert.deepEqual(pushedRange({ localSha: sha, remoteSha: old }, () => false), [sha, "--not", "--remotes"]);
});

test("runStep keeps the output and the exit code of a failing step, and feeds stdin", async () => {
  const dir = mkdtempSync(join(tmpdir(), "pre-push-"));
  const log = join(dir, "step.log");
  const code = await runStep("node", ["-e", "\"console.log('out-line'); console.error('err-line'); process.exit(3)\""], dir, log);
  assert.equal(code, 3);
  const text = readFileSync(log, "utf8");
  assert.match(text, /out-line/);
  assert.match(text, /err-line/);
  assert.match(text, /\[exit code 3,/);
  const echo = await runStep("node", ["-e", "\"process.stdin.pipe(process.stdout)\""], dir, log, "ref-lines\n");
  assert.equal(echo, 0);
  assert.match(readFileSync(log, "utf8"), /ref-lines/);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("the pushed commits are checked; deletions and commits already on the remote are skipped", () => {
  const refs = parsePushLines(`refs/heads/a ${"1".repeat(40)} refs/heads/a ${ZERO}\r\n\nbad line\n`);
  assert.deepEqual(refs, [{ localRef: "refs/heads/a", localSha: "1".repeat(40), remoteRef: "refs/heads/a", remoteSha: ZERO }]);
  const repo = tempRepo();
  const pub = `${FIXTURE_LIST.publicIdentity.name} <${FIXTURE_LIST.publicIdentity.email}> 1760000000 +0200`;
  const base = commitRaw(repo, { author: pub, message: "an old commit before the rules\n" });
  git(repo, ["update-ref", "refs/remotes/origin/main", base]);
  const good = commitRaw(repo, { parents: [base], author: pub, message: "feat(sim): add the tether\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n" });
  const badMsg = commitRaw(repo, { parents: [good], author: pub, message: "WIP on WIN3\n" });
  const badWho = commitRaw(repo, { parents: [badMsg], author: "Jane Roe <jane.roe@example.org> 1760000000 +0200", committer: pub, message: "fix: x\n" });
  const merge = commitRaw(repo, { parents: [badWho, base], author: pub, message: "Merge the tether gadget\n" });
  assert.deepEqual(new Set(pushedCommits(repo, parsePushLines(`refs/heads/x ${merge} refs/heads/x ${ZERO}`))), new Set([good, badMsg, badWho, merge]));
  // Under the remote's old value: not pushed again, even without a remote-tracking ref.
  git(repo, ["update-ref", "-d", "refs/remotes/origin/main"]);
  assert.deepEqual(pushedCommits(repo, parsePushLines(`refs/heads/x ${badMsg} refs/heads/x ${good}`)), [badMsg]);
  git(repo, ["update-ref", "refs/remotes/origin/main", base]);
  const commits = readCommits(repo, [merge]);
  assert.equal(commits[0].parents, 2);
  assert.equal(commits[0].message, "Merge the tether gadget\n");
  const line = (sha) => `refs/heads/x ${sha} refs/heads/x ${base}`;
  assert.deepEqual(pushProblems(repo, line(good), FIXTURE_LIST), []);
  assert.deepEqual(pushProblems(repo, `refs/heads/gone ${ZERO} refs/heads/gone ${good}`, FIXTURE_LIST), []);
  const problems = pushProblems(repo, line(merge), FIXTURE_LIST);
  assert.ok(problems.some((p) => p.startsWith(badMsg.slice(0, 10)) && /message: the subject is not/.test(p)), problems.join("\n"));
  assert.ok(problems.some((p) => p.startsWith(badWho.slice(0, 10)) && /author name contains a name from the private list/.test(p)));
  assert.ok(!problems.some((p) => p.startsWith(merge.slice(0, 10)) || p.startsWith(good.slice(0, 10))));
  assert.ok(problems.every((p) => !/Jane|Roe/.test(p)));
  // The linear-history check: the merge is refused; an old merge already on the remote is not.
  assert.deepEqual(mergeCommits(repo, line(merge)), [`${merge.slice(0, 10)} Merge the tether gadget`]);
  assert.deepEqual(mergeCommits(repo, line(badWho)), []);
  git(repo, ["update-ref", "refs/remotes/origin/main", merge]);
  const after = commitRaw(repo, { parents: [merge], author: pub, message: "fix: y\n" });
  assert.deepEqual(mergeCommits(repo, `refs/heads/x ${after} refs/heads/x ${merge}`), []);
  fs.rmSync(repo, { recursive: true, force: true });
});
