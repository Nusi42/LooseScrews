// Keeps the hooks, guards and Claude Code settings wired (docs/adr/0001).
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import { MAX_LINES, plan, shortReport } from "../lib/run-checks.mjs";
import { REPO_ROOT } from "../lib/work-folder.mjs";
import { checkoutOf, editedFile } from "../post-edit.mjs";

const read = (rel) => fs.readFileSync(path.join(REPO_ROOT, rel), "utf8");

test("git hooks exist and call the node hooks", () => {
  assert.match(read(".githooks/pre-commit"), /tools\/hooks\/pre-commit\.mjs/);
  assert.match(read(".githooks/pre-push"), /tools\/hooks\/pre-push\.mjs/);
  const pkg = JSON.parse(read("web/package.json"));
  assert.equal(pkg.scripts.prepare, "node ../tools/hooks/install.mjs");
  assert.match(pkg.scripts.dev, /work-folders\.mjs guard/);
});

test("Claude Code post-edit hook runs tools/post-edit.mjs on edits", () => {
  const s = JSON.parse(read(".claude/settings.json"));
  const entry = s.hooks.PostToolUse[0];
  assert.match(entry.matcher, /Edit/);
  assert.match(entry.matcher, /Write/);
  assert.match(entry.hooks[0].command, /tools\/post-edit\.mjs/);
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
  for (const f of ["tools/e2e.mjs", "tools/browser-smoke/smoke.mjs", "web/scripts/build-wasm.mjs", "tools/hooks/install.mjs", "tools/hooks/pre-commit.mjs", "tools/hooks/pre-push.mjs"]) {
    assert.match(read(f), /assertWorkFolder\(/, f);
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
  assert.equal(checkoutOf(path.join(os.tmpdir(), "x", "y.rs")), undefined);
});

test("checks skip build output; reports stay short", () => {
  const p = plan(["target/a.rs", "web/node_modules/x.ts", "web/dist/a.js", "crates/a/src/b.rs", "web/src/m.ts"]);
  assert.deepEqual(p.rs, ["crates/a/src/b.rs"]);
  assert.deepEqual(p.ts, ["web/src/m.ts"]);
  const long = shortReport([{ name: "x", code: 1, output: Array.from({ length: 50 }, (_, i) => `l${i}`).join("\n") }]);
  assert.equal(long.split("\n").length, MAX_LINES);
});
