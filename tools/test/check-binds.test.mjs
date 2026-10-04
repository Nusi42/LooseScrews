// The wildcard-bind check (tools/check-binds.mjs).
import assert from "node:assert/strict";
import { test } from "node:test";

import { findBinds } from "../check-binds.mjs";

test("wildcard binds outside the allowlist are found", () => {
  const src = 'let a = "0.0.0.0:0";\nlet b = "[::]:9";\nlet c = Ipv6Addr::UNSPECIFIED;\n.with_bind_default()\nlet ok = "127.0.0.1:0";\n';
  assert.deepEqual(findBinds("crates/x/tests/t.rs", src).map((l) => l.split(":")[1]), ["1", "2", "3", "4"]);
});

test("allowlisted lines pass, other lines in the same file do not", () => {
  const src = 'pub const DEFAULT_BIND: &str = "0.0.0.0:7777";\nlet t = "0.0.0.0:1";\n';
  assert.deepEqual(findBinds("crates/ls-server/src/lib.rs", src), ['crates/ls-server/src/lib.rs:2: let t = "0.0.0.0:1";']);
});

test("Markdown is prose and not scanned", async () => {
  const { SCANNED } = await import("../check-binds.mjs");
  assert.ok(!SCANNED.test("docs/dev-guide.md"));
  assert.ok(SCANNED.test("crates/a/src/b.rs"));
});
