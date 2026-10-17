// tools/check-commit.mjs: commit message and identity rules (docs/adr/0002).
// The private list is a made-up fixture.
import assert from "node:assert/strict";
import { test } from "node:test";

import { MAX_SUBJECT, checkIdentity, checkMessage, cleanMessage } from "../check-commit.mjs";
import { FIXTURE_LIST } from "./private-list-fixture.mjs";

const CO_AUTHOR = "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>";
const ok = (text, opts) => assert.deepEqual(checkMessage(text, opts), [], text);
const bad = (text, pattern, opts) => {
  const problems = checkMessage(text, opts);
  assert.ok(problems.some((p) => pattern.test(p)), `${text}\n-> ${JSON.stringify(problems)}`);
};

test("conventional subjects pass", () => {
  ok("feat(sim): add the tether gadget");
  ok("fix: keep f32 hashes stable across i64 timestamps");
  ok("refactor(ls-net)!: split the snapshot codec");
  ok(`docs(dev-guide): describe the dry run\n\nLonger body text.\nweapon slots and gadget slot stay.\n\n${CO_AUTHOR}`);
  ok("revert(web): undo the menu change");
  ok('Revert "feat(sim): add the tether gadget"');
  ok("fixup! fix(net): drop stale snapshots");
  ok("chore: bump vite\n# a comment line git strips\n");
});

test("malformed subjects are refused", () => {
  bad("Add the tether gadget", /not "<type>\(<scope>\): <subject>"/);
  bad("feature: add x", /not "<type>/);
  bad("feat(Sim): add x", /not "<type>/);
  bad("feat: Add x", /lowercase/);
  bad("feat: add x.", /period/);
  bad(`feat: ${"x".repeat(MAX_SUBJECT)}`, /at most 72/);
  bad("feat: add x\nno blank line", /second line/);
  bad("", /empty/);
  bad("# only a comment", /empty/);
});

test("merge commits say what they brought", () => {
  ok("Merge the tether gadget and its tests", { merge: true });
  bad("feat: add x", /merge commit/, { merge: true });
  bad("Merge branch 'WIN3' into WIN1", /"WIN3" \(logistics\)/, { merge: true });
  bad("Merge the gadget\n\nMerge-Notes: x", /trailer "Merge-Notes:"/, { merge: true });
});

test("only the Claude co-author and sign-off trailers are allowed", () => {
  ok(`fix: x\n\nbody\n\n${CO_AUTHOR}`);
  ok("fix: x\n\nco-authored-by: Claude Sonnet 4 <noreply@anthropic.com>");
  ok(`fix: x\n\nSigned-off-by: A B <a@b.c>\nSigned-off-by: C D <c@d.e>\n${CO_AUTHOR}`);
  bad("fix: x\n\nSigned-off-by: A B", /trailer "Signed-off-by:"/);
  bad("fix: x\n\nReviewed-by: A B <a@b.c>", /trailer "Reviewed-by:"/);
  bad("fix: x\n\nSigned-off-by: Jane Roe <x@example.org>", /a name from the private list/, { list: FIXTURE_LIST });
  bad("fix: x\n\nCo-Authored-By: Someone <a@b.c>", /trailer "Co-Authored-By:"/);
  bad("fix: x\n\nCo-Authored-By: Claude <noreply@anthropic.com>", /trailer/);
});

test("logistics, process jargon and local names are refused", () => {
  const terms = [
    "win2", "WIN11", "winN", "WINn", "work folder", "master", "lead", "fast-forward", "origin/main", "WIP",
    "work in progress", "wave", "waves", "work package", "WP17", "contract", "contracts", "slots", "M5", "C06", "I3",
    "CQ-12", "SSOT", "ODR", "DRY", "owner", "session", "next-steps", "orchestrator", ["co", "lent"].join(""),
    ["One", "Drive"].join(""), ["Self", "Made"].join(""), ["Claude", "Session"].join("-"),
  ];
  for (const t of terms) bad(`fix: x\n\nabout ${t} here`, /is not allowed in a commit message/);
});

test("technical uses of the restricted words pass", () => {
  for (const t of [
    "the projectile owner", "the key owner", "owner trust", "the file owner", "window owner", "entity owner",
    "weapon slots", "gadget slot", "a dry run", "f32 and i64", "e2e tests", "leading edge", "masterful",
  ]) {
    ok(`fix: x\n\nabout ${t} here`);
  }
});

test("URLs other than the public repository are refused", () => {
  ok("docs: link the tracker\n\nSee https://github.com/Nusi42/LooseScrews/issues/3.");
  bad("docs: x\n\nSee https://example.org/page", /a URL other than/);
  bad("docs: x\n\nSee github.com/Other/repo", /a URL other than/);
  bad(`docs: x\n\nSee https://${["claude", "ai"].join(".")}/code/x`, /claude\.ai link/);
});

test("private-list values are refused, masked", () => {
  const problems = checkMessage("fix: x\n\nThanks to Jane Roe.", { list: FIXTURE_LIST });
  assert.ok(problems.some((p) => /a name from the private list \(J\*{6}e\)/.test(p)), JSON.stringify(problems));
  assert.ok(problems.every((p) => !/Jane|Roe/.test(p)));
  bad("fix: x\n\nby Octo99", /GitHub handle/, { list: FIXTURE_LIST });
});

test("comments and the scissors part are not part of the message", () => {
  assert.equal(cleanMessage("feat: x\n\nbody  \n# comment\n# ------------------------ >8 ------------------------\ndiff"), "feat: x\n\nbody");
});

test("identities: the public identity of the private list, or any non-empty email without one", () => {
  const pub = `${FIXTURE_LIST.publicIdentity.name} <${FIXTURE_LIST.publicIdentity.email}> 1760000000 +0200`;
  assert.deepEqual(checkIdentity(pub, FIXTURE_LIST), []);
  const priv = checkIdentity("Jane Roe <jane.roe@example.org> 1760000000 +0200", FIXTURE_LIST);
  assert.ok(priv.some((p) => /^name contains a name from the private list/.test(p)));
  assert.ok(priv.some((p) => /^email contains an email address from the private list/.test(p)));
  assert.ok(priv.some((p) => /not the public identity/.test(p)));
  assert.ok(priv.every((p) => !/Jane|Roe|jane/.test(p)));
  assert.match(checkIdentity("Someone <x@example.org> 1 +0000", FIXTURE_LIST).join(), /not the public identity/);
  const { publicIdentity, ...withoutPublic } = FIXTURE_LIST;
  assert.ok(publicIdentity);
  assert.deepEqual(checkIdentity("Someone <x@example.org> 1 +0000", withoutPublic), []);
  assert.deepEqual(checkIdentity("Anyone <a@b.c> 1 +0000"), []);
  assert.match(checkIdentity("Anyone <> 1 +0000").join(), /empty email/);
  assert.match(checkIdentity("garbage").join(), /not a git identity/);
});
