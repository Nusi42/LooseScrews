#!/usr/bin/env node
// Fails on wildcard binds (0.0.0.0, [::], "::", *::UNSPECIFIED, wtransport's
// with_bind_default) outside the allowlist below. Tests, bots, e2e and smoke runs
// bind 127.0.0.1 (port 0 where possible): a wildcard bind makes Windows ask to
// allow every new hashed test binary in the firewall (docs/adr/0001).
//   node tools/check-binds.mjs [files...]   (default: every tracked file; .md is prose)

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { REPO_ROOT } from "./lib/work-folder.mjs";

export const PATTERN = /0\.0\.0\.0|\[::\]|"::"|UNSPECIFIED|with_bind_default/;

/** Allowed hits: file -> substrings of the allowed lines, each with its reason. */
export const ALLOWLIST = {
  // Real hosting: the server listens on every interface by default.
  "crates/ls-server/src/lib.rs": ['pub const DEFAULT_BIND: &str = "0.0.0.0:7777";'],
  // Doc comments describing the hosting default.
  "crates/ls-server/src/main.rs": ["//! ls-server --bind 0.0.0.0:7777", "/// `0.0.0.0` is not browsable"],
  // Dual-stack WebTransport when the host asked for 0.0.0.0.
  "crates/ls-server/src/wt.rs": [
    "builder.with_bind_default(bind.port())",
    "/// Generates the certificate and binds the UDP socket. `0.0.0.0` binds",
    "let want_dual = addr.ip() == IpAddr::V4(Ipv4Addr::UNSPECIFIED);",
  ],
  // Directory hosting default and its doc comment.
  "crates/ls-directory/src/lib.rs": ['pub const DEFAULT_BIND: &str = "0.0.0.0:7780";'],
  "crates/ls-directory/src/main.rs": ["//! ls-directory --bind 0.0.0.0:7780"],
  // Bots against a remote server (loopback servers get loopback_bind).
  "crates/ls-bots/src/transport.rs": ["None => builder.with_bind_default(),"],
  // This checker.
  "tools/check-binds.mjs": ["*"],
  "tools/test/check-binds.test.mjs": ["*"],
};

/** Files the check reads (code and config; Markdown is prose). */
export const SCANNED = /\.(rs|mjs|js|ts|ps1|sh|toml|ron|json)$/;

/** Findings (`file:line: text`) for `file` with content `text`. */
export function findBinds(file, text) {
  const allowed = ALLOWLIST[file] ?? [];
  if (allowed.includes("*")) return [];
  const out = [];
  text.split("\n").forEach((line, i) => {
    if (PATTERN.test(line) && !allowed.some((a) => line.includes(a))) {
      out.push(`${file}:${i + 1}: ${line.trim()}`);
    }
  });
  return out;
}

function trackedFiles() {
  const r = spawnSync("git", ["ls-files", "-z"], { cwd: REPO_ROOT, encoding: "utf8" });
  return r.stdout.split("\0").filter(Boolean);
}

function main() {
  const args = process.argv.slice(2);
  const files = (args.length ? args.map((f) => path.relative(REPO_ROOT, path.resolve(f)).split(path.sep).join("/")) : trackedFiles())
    .filter((f) => SCANNED.test(f) && !/(^|\/)(target|node_modules|dist|pkg)\//.test(f));
  const findings = files.flatMap((f) => {
    const abs = path.join(REPO_ROOT, f);
    return fs.existsSync(abs) ? findBinds(f, fs.readFileSync(abs, "utf8")) : [];
  });
  if (findings.length === 0) return;
  console.error("wildcard bind outside the allowlist (bind 127.0.0.1, port 0 in tests; or allowlist it in tools/check-binds.mjs with a reason):");
  console.error(findings.join("\n"));
  process.exit(1);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
