// Fast per-file checks shared by the Git pre-commit hook (staged files) and the
// Claude Code post-edit hook (the edited file); docs/adr/0001. Each check gets
// repo-relative paths ("/" separators) and returns { name, code, output }.
//   rustfmt --check            *.rs
//   tsc --noEmit (web)         web/**/*.ts (whole project; tsc 7 is fast)
//   check-binds                code and config files
//   line endings               no CR except *.ps1 (.gitattributes: LF)
//   private information        no git email, OS user name, user-home paths, session links or private-list values (tools/check-private.mjs)
//   generated reference docs   data/{weapons,ships,gadgets,rulesets}/*, web/src/rules/catalog.ts, the generator
// Build output (target/, node_modules/, dist/, the wasm pkg) is never checked.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

import { SCANNED, findBinds } from "../check-binds.mjs";
import { SCANNED as PRIVATE_SCANNED, findPrivate, privateValues } from "../check-private.mjs";

/** Paths no check looks at. */
export const IGNORED = /(^|\/)(target|node_modules|dist|pkg|\.codegraph|\.git)\//;

/** Maximum report lines handed back to an agent or printed by the hook. */
export const MAX_LINES = 20;

function run(root, cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { cwd: root, encoding: "utf8", ...opts });
  return { code: r.status ?? 1, output: `${r.stdout ?? ""}${r.stderr ?? ""}${r.error ? String(r.error) : ""}`.trim() };
}

/** The checks that apply to `files` (repo-relative). */
export function plan(files) {
  const f = files.filter((x) => !IGNORED.test(x));
  const rs = f.filter((x) => x.endsWith(".rs"));
  const ts = f.filter((x) => /^web\/.*\.ts$/.test(x));
  const text = f.filter((x) => !/\.(png|wasm|ico|jpg|gif|ps1|cmd|bat)$/.test(x));
  const data = f.filter(
    (x) => /^data\/(weapons|ships|gadgets|rulesets|events|world)\//.test(x) || x === "web/src/rules/catalog.ts" || x === "tools/gen-weapon-docs.mjs",
  );
  return { rs, ts, text, data };
}

/** Runs the per-file checks for `files` in the checkout at `root`. */
export async function runChecks(root, files) {
  const { rs, ts, text, data } = plan(files);
  const results = [];
  if (rs.length) {
    results.push({ name: "rustfmt", ...run(root, "rustfmt", ["--check", "--color", "never", "--edition", "2021", ...rs]) });
  }
  if (ts.length) {
    const tsc = path.join(root, "web", "node_modules", "typescript", "bin", "tsc");
    if (fs.existsSync(tsc)) {
      const web = path.join(root, "web");
      const a = run(web, "node", [tsc, "--noEmit"]);
      const b = a.code === 0 ? run(web, "node", [tsc, "--noEmit", "-p", "tsconfig.test.json"]) : { code: 0, output: "" };
      results.push({ name: "tsc (web)", code: a.code || b.code, output: [a.output, b.output].filter(Boolean).join("\n") });
    } else {
      results.push({ name: "tsc (web)", code: 1, output: "web/node_modules missing: run `npm ci` in web/" });
    }
  }
  const binds = [];
  const crlf = [];
  const priv = [];
  const who = text.length ? privateValues(root) : {};
  for (const rel of text) {
    const abs = path.join(root, rel);
    if (!fs.existsSync(abs) || fs.statSync(abs).isDirectory()) continue;
    const content = fs.readFileSync(abs, "utf8");
    if (SCANNED.test(rel)) binds.push(...findBinds(rel, content));
    if (PRIVATE_SCANNED.test(rel)) priv.push(...findPrivate(rel, content, who));
    if (content.includes("\r")) crlf.push(`${rel}: CRLF line endings (the repo uses LF; Python: open(..., newline=''))`);
  }
  if (text.length) {
    results.push({ name: "wildcard binds", code: binds.length ? 1 : 0, output: binds.join("\n") });
    results.push({ name: "line endings", code: crlf.length ? 1 : 0, output: crlf.join("\n") });
    results.push({ name: "private information", code: priv.length ? 1 : 0, output: priv.join("\n") });
  }
  if (data.length) {
    results.push({ name: "generated reference docs", ...run(root, "node", ["tools/gen-weapon-docs.mjs", "--check"]) });
  }
  return results;
}

/** At most MAX_LINES lines naming each failed check and its first output lines. */
export function shortReport(results) {
  const lines = [];
  for (const r of results.filter((x) => x.code !== 0)) {
    lines.push(`${r.name}: failed`);
    lines.push(...r.output.split("\n").filter(Boolean).map((l) => `  ${l}`));
  }
  if (lines.length <= MAX_LINES) return lines.join("\n");
  return [...lines.slice(0, MAX_LINES - 1), `  ... ${lines.length - MAX_LINES + 1} more lines`].join("\n");
}
