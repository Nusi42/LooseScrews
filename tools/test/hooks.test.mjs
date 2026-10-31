// Keeps the hooks, guards and Claude Code settings wired (docs/adr/0001, 0002).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import { MAX_LINES, plan, runChecks, shortReport } from "../lib/run-checks.mjs";
import { afterEdit, cleanNote, isMemoryNote, withoutHarnessKeys } from "../lib/after-edit.mjs";
import { REPO_ROOT } from "../lib/work-folder.mjs";
import { AFTER_EDIT, checkoutOf, editedFile } from "../post-edit.mjs";
import { commitRaw, git, listFile, tempRepo } from "./git-fixture.mjs";
import { FIXTURE_LIST } from "./private-list-fixture.mjs";

const read = (rel) => fs.readFileSync(path.join(REPO_ROOT, rel), "utf8");

test("git hooks exist and call the node hooks", () => {
  assert.match(read(".githooks/pre-commit"), /tools\/hooks\/pre-commit\.mjs/);
  // pre-push and commit-msg run the tools of the checkout the hooks belong to.
  assert.ok(read(".githooks/pre-push").includes('exec node "$(dirname "$0")/../tools/hooks/pre-push.mjs" "$@"'));
  assert.ok(read(".githooks/commit-msg").includes('exec node "$(dirname "$0")/../tools/hooks/commit-msg.mjs" "$@"'));
  const pkg = JSON.parse(read("web/package.json"));
  assert.equal(pkg.scripts.prepare, "node ../tools/hooks/install.mjs");
  assert.match(pkg.scripts.dev, /work-folders\.mjs guard/);
});

test("Claude Code post-edit hook runs tools/post-edit.mjs on edits", () => {
  const s = JSON.parse(read(".claude/settings.json"));
  const entry = s.hooks.PostToolUse[0];
  assert.match(entry.matcher, /Edit/);
  assert.match(entry.matcher, /Write/);
  // The session may run in the master; the loader finds the edited checkout itself.
  assert.equal(entry.hooks[0].command, 'node "$CLAUDE_PROJECT_DIR/tools/post-edit.mjs"');
});

test("cargo includes the gitignored local config; build output is ignored", () => {
  assert.match(read(".cargo/config.toml"), /include = \[\{ path = "config\.local\.toml", optional = true \}\]/);
  const ignore = read(".gitignore");
  for (const p of [".cargo/config.local.toml", ".codegraph/", "/target/", "node_modules/", "web/dist/"]) {
    assert.ok(ignore.includes(p), p);
  }
});

test("every dev and CI entry point has the work-folder guard", () => {
  for (const f of ["tools/ci.ps1", "tools/ci.sh", "tools/e2e.ps1", "tools/e2e.sh", "tools/browser-smoke.ps1", "tools/browser-smoke.sh"]) {
    assert.match(read(f), /work-folders\.mjs['"]?\)? guard/, f);
  }
  for (const f of ["tools/e2e.mjs", "tools/browser-smoke/smoke.mjs", "web/scripts/build-wasm.mjs", "tools/hooks/install.mjs", "tools/hooks/pre-commit.mjs"]) {
    assert.match(read(f), /assertWorkFolder\(/, f);
  }
  // pre-push decides by checkout role, remote and refs (pushPlan): the master may publish, nothing else.
  assert.match(read("tools/hooks/pre-push.mjs"), /pushPlan\(\{ role: pushRole\(root\), url, refs: parsePushLines\(stdin\)/);
});

test("pre-push and the linear-history check run on load, never behind an is-main test", () => {
  for (const f of ["tools/hooks/pre-push.mjs", "tools/check-linear.mjs"]) {
    assert.doesNotMatch(read(f), /fileURLToPath\(import\.meta\.url\)/, f);
    assert.doesNotMatch(read(f), /process\.argv\[1\]/, f);
  }
});

test("vite does not use the default port", () => {
  const cfg = read("web/vite.config.ts");
  assert.doesNotMatch(cfg, /port: 5173/);
  assert.match(cfg, /strictPort: true/);
  assert.match(cfg, /LS_DEV_PORT/);
});

test("post-edit finds the checkout from the edited file", () => {
  assert.equal(editedFile('{"tool_input":{"file_path":"C:/a/b.rs"}}'), "C:/a/b.rs");
  assert.equal(editedFile("not json"), undefined);
  assert.equal(checkoutOf(path.join(REPO_ROOT, "tools", "lib", "run-checks.mjs")), REPO_ROOT);
  assert.ok(fs.existsSync(path.join(REPO_ROOT, AFTER_EDIT)));
  assert.equal(checkoutOf(path.join(os.tmpdir(), "x", "y.rs")), undefined);
});

test("checks skip build output; reports stay short", () => {
  const p = plan(["target/a.rs", "web/node_modules/x.ts", "web/dist/a.js", "crates/a/src/b.rs", "web/src/m.ts"]);
  assert.deepEqual(p.rs, ["crates/a/src/b.rs"]);
  assert.deepEqual(p.ts, ["web/src/m.ts"]);
  const long = shortReport([{ name: "x", code: 1, output: Array.from({ length: 50 }, (_, i) => `l${i}`).join("\n") }]);
  assert.equal(long.split("\n").length, MAX_LINES);
});

test("the per-file checks refuse private information (pre-commit, post-edit)", async () => {
  const key = ["origin", "SessionId"].join("");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ls-run-checks-"));
  try {
    fs.mkdirSync(path.join(root, ".claude", "memory"), { recursive: true });
    fs.writeFileSync(path.join(root, ".claude", "memory", "note.md"), `---\nname: note\nmetadata:\n  ${key}: 1234\n---\n`);
    fs.writeFileSync(path.join(root, "clean.md"), "# Clean\n");
    const check = async (rel) => (await runChecks(root, [rel])).find((r) => r.name === "private information");
    const bad = await check(".claude/memory/note.md");
    assert.equal(bad?.code, 1);
    assert.match(bad.output, new RegExp(key));
    assert.equal((await check("clean.md"))?.code, 0);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("every data folder the reference pages read triggers the generated-docs check", () => {
  const files = ["weapons", "ships", "gadgets", "rulesets", "events", "world"].map((d) => `data/${d}/x.ron`);
  assert.deepEqual(plan([...files, "data/other/x.ron"]).data, files);
});

test("post-edit strips Claude Code's keys from memory notes' front matter", () => {
  const key = ["origin", "SessionId"].join("");
  assert.ok(isMemoryNote(".claude/memory/next.md"));
  assert.ok(!isMemoryNote("docs/next.md"));
  assert.equal(withoutHarnessKeys(`---\nname: x\n${key}: abc\ntype: y\n---\nbody ${key}: kept\n`), `---\nname: x\ntype: y\n---\nbody ${key}: kept\n`);
  assert.equal(withoutHarnessKeys(`---\r\nname: x\r\n${key}: abc\r\n---\r\n`), "---\r\nname: x\r\n---\r\n");
  const harness = `---\nname: x\nmodified: 2000-01-01\nnode_type: memory\nmetadata:\n  node_type: memory\n  type: y\n  ${key}: abc\n\t${key}: tab\n  modified: 2000-01-01T00:00:00Z\n---\n`;
  assert.equal(withoutHarnessKeys(harness), "---\nname: x\nmetadata:\n  type: y\n---\n");
  // Keys that only start like one of them stay.
  const kept = "---\nmodified_by: x\nnode_types: y\n---\n";
  assert.equal(withoutHarnessKeys(kept), kept);
  const plain = "# no front matter\n";
  assert.equal(withoutHarnessKeys(plain), plain);
});

test("the after-edit module cleans a memory note in place, then runs the checks", async () => {
  const key = ["origin", "SessionId"].join("");
  const root = tempRepo();
  try {
    fs.mkdirSync(path.join(root, ".claude", "memory"), { recursive: true });
    const note = path.join(root, ".claude", "memory", "note.md");
    fs.writeFileSync(note, `---\nname: note\nmetadata:\n  ${key}: 1234\n  modified: 2000-01-01\n---\nbody\n`);
    assert.equal(await afterEdit(root, note, ".claude/memory/note.md"), undefined);
    assert.equal(fs.readFileSync(note, "utf8"), "---\nname: note\nmetadata:\n---\nbody\n");
    // Only memory notes are rewritten; the checks still report the session key elsewhere.
    const doc = path.join(root, "doc.md");
    fs.writeFileSync(doc, `---\n${key}: 1234\n---\n`);
    assert.equal(cleanNote(doc, "doc.md"), false);
    assert.match(await afterEdit(root, doc, "doc.md"), /private information: failed/);
    // Nothing runs for files of a master checkout.
    fs.writeFileSync(path.join(root, ".env.local"), "LS_CHECKOUT_ROLE=master\n");
    assert.equal(await afterEdit(root, doc, "doc.md"), undefined);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("the post-edit loader hands the file to the edited checkout's own after-edit module", () => {
  // The loader may come from an older checkout (the master): what runs is the module of the
  // checkout that holds the file.
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "ls-post-edit-"));
  const hook = path.join(REPO_ROOT, "tools", "post-edit.mjs");
  const run = (file) => spawnSync(process.execPath, [hook], { input: JSON.stringify({ tool_input: { file_path: file } }), encoding: "utf8" });
  try {
    fs.mkdirSync(path.join(work, ".git"));
    fs.mkdirSync(path.join(work, "tools", "lib"), { recursive: true });
    fs.writeFileSync(path.join(work, AFTER_EDIT), "export const afterEdit = async (root, file, rel) => `this checkout's code: ${rel}`;\n");
    fs.mkdirSync(path.join(work, "docs"));
    const file = path.join(work, "docs", "a.md");
    fs.writeFileSync(file, "x\n");
    const edited = run(file);
    assert.equal(edited.status, 2, edited.stderr);
    assert.equal(edited.stderr.trim(), "this checkout's code: docs/a.md");
    // A file outside any checkout: nothing runs.
    fs.rmSync(path.join(work, ".git"), { recursive: true });
    const outside = run(file);
    assert.equal(outside.status, 0, outside.stderr);
    assert.equal(outside.stderr, "");
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
});

/** Runs the commit-msg hook in `repo` on `message` as `ident`; resolves to { code, stderr }. */
function commitMsg(repo, message, ident, list) {
  const file = path.join(repo, "MSG");
  fs.writeFileSync(file, message);
  const [, name, email] = /^(.*) <(.*)>$/.exec(ident);
  const env = {
    ...process.env, LS_PRIVATE_LIST: list,
    GIT_AUTHOR_NAME: name, GIT_AUTHOR_EMAIL: email, GIT_COMMITTER_NAME: name, GIT_COMMITTER_EMAIL: email,
  };
  const r = spawnSync("node", [path.join(REPO_ROOT, "tools", "hooks", "commit-msg.mjs"), file], { cwd: repo, env, encoding: "utf8" });
  return { code: r.status, stderr: r.stderr };
}

test("commit-msg checks the message, merges and both identities", () => {
  const repo = tempRepo();
  const list = listFile(FIXTURE_LIST);
  const none = path.join(repo, "no-list.json");
  const pub = `${FIXTURE_LIST.publicIdentity.name} <${FIXTURE_LIST.publicIdentity.email}>`;
  assert.equal(commitMsg(repo, "feat(sim): add the tether\n# comment\n", pub, list).code, 0);
  const wrongWho = commitMsg(repo, "feat(sim): add the tether\n", "Jane Roe <jane.roe@example.org>", list);
  assert.equal(wrongWho.code, 1);
  assert.match(wrongWho.stderr, /author name contains a name from the private list/);
  assert.match(wrongWho.stderr, /committer is not the public identity/);
  assert.doesNotMatch(wrongWho.stderr, /Jane|Roe/);
  assert.equal(commitMsg(repo, "feat(sim): add the tether\n", "Any One <a@example.org>", none).code, 0);
  const badMsg = commitMsg(repo, "Added stuff\n", pub, list);
  assert.equal(badMsg.code, 1);
  assert.match(badMsg.stderr, /Conventional Commits/);
  // A merge in progress: git's default message is refused, with the hint.
  const head = commitRaw(repo, { author: `${pub} 1760000000 +0200`, message: "feat: base\n" });
  fs.writeFileSync(path.resolve(repo, git(repo, ["rev-parse", "--git-path", "MERGE_HEAD"])), `${head}
`);
  const merge = commitMsg(repo, "Merge branch 'WIN3' into WIN1\n", pub, list);
  assert.equal(merge.code, 1);
  assert.match(merge.stderr, /Combine branches by rebasing/);
  assert.equal(commitMsg(repo, "Merge the tether gadget\n", pub, list).code, 0);
  assert.equal(commitMsg(repo, "feat: not a merge subject\n", pub, list).code, 1);
  fs.rmSync(repo, { recursive: true, force: true });
  fs.rmSync(path.dirname(list), { recursive: true, force: true });
});
