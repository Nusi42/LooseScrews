#!/usr/bin/env node
// Git pre-push (.githooks/pre-push): the fast CI subset. The full suite
// (release bot tests, wasm32 tests, WASM build, determinism cross-check, browser
// smoke) is tools/ci.*, run in win1 before main moves. `--list` prints the steps.
import { spawnSync } from "node:child_process";
import path from "node:path";

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

if (process.argv.includes("--list")) {
  for (const [name] of STEPS) console.log(name);
  process.exit(0);
}
assertWorkFolder("git push");
for (const [name, cmd, args, dir] of STEPS) {
  console.log(`\n==> pre-push: ${name}`);
  // One command string (constant arguments): npm is a .cmd on Windows and needs a shell.
  const r = spawnSync([cmd, ...args.map((a) => (a.includes("*") ? `"${a}"` : a))].join(" "), { cwd: path.join(REPO_ROOT, dir), stdio: "inherit", shell: true });
  if (r.status !== 0) {
    console.error(`pre-push: "${name}" failed; push refused (git push --no-verify skips it, don't)`);
    process.exit(1);
  }
}
