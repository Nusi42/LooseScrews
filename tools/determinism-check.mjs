#!/usr/bin/env node
// Determinism cross-check: runs the built ls-wasm package under Node and
// compares goldenHashes(code) for every share code in
// crates/ls-worldgen/tests/fixtures/golden_hashes.txt with the native fixture
// (sdf, surface_nets, dual_contouring, edited). Exits non-zero on any mismatch
// or if the package is missing.
//
// Usage: node tools/determinism-check.mjs [--pkg <dir>] [--fixture <file>]
//   --pkg      wasm-bindgen output directory (default web/src/wasm/pkg;
//              build it with `cd web && npm run build:wasm`)
//   --fixture  golden hash fixture (default: the ls-worldgen fixture)
//
// The ls-sim replay hash (REPLAY_GOLDEN_HASH) is not exported by ls-wasm; it is
// cross-checked by the wasm32 unit tests in tools/ci.* instead.

import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function parseArgs(argv) {
  const opts = {
    pkg: join(repoRoot, "web", "src", "wasm", "pkg"),
    fixture: join(repoRoot, "crates", "ls-worldgen", "tests", "fixtures", "golden_hashes.txt"),
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const next = () => {
      const v = argv[++i];
      if (v === undefined) fail(`missing value for ${a}`);
      return resolve(v);
    };
    if (a === "--pkg") opts.pkg = next();
    else if (a === "--fixture") opts.fixture = next();
    else if (a === "--help" || a === "-h") {
      console.log("usage: node tools/determinism-check.mjs [--pkg <dir>] [--fixture <file>]");
      process.exit(0);
    } else fail(`unknown argument ${a}`);
  }
  return opts;
}

function fail(msg) {
  console.error(`determinism-check: ${msg}`);
  process.exit(2);
}

/** Fixture lines: `code sdf surface_nets dual_contouring edited`. */
export function parseFixture(text) {
  const rows = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    const [code, ...hashes] = line.split(/\s+/);
    if (hashes.length !== 4) throw new Error(`malformed fixture line: ${line}`);
    rows.push({ code, hashes });
  }
  return rows;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const js = join(opts.pkg, "ls_wasm.js");
  const wasm = join(opts.pkg, "ls_wasm_bg.wasm");
  if (!existsSync(js) || !existsSync(wasm)) {
    fail(`ls-wasm package not found in ${opts.pkg} (run \`cd web && npm run build:wasm\`)`);
  }
  if (!existsSync(opts.fixture)) fail(`fixture not found: ${opts.fixture}`);

  const pkg = await import(pathToFileURL(js).href);
  pkg.initSync({ module: readFileSync(wasm) });
  if (typeof pkg.init === "function") pkg.init();
  if (typeof pkg.goldenHashes !== "function") fail("package has no goldenHashes export");

  const rows = parseFixture(readFileSync(opts.fixture, "utf8"));
  if (rows.length === 0) fail("fixture has no entries");
  const names = ["sdf", "surface_nets", "dual_contouring", "edited"];
  let mismatches = 0;
  for (const { code, hashes } of rows) {
    const t0 = performance.now();
    let got;
    try {
      got = pkg.goldenHashes(code);
    } catch (e) {
      console.error(`FAIL ${code}: goldenHashes threw: ${e?.message ?? e}`);
      mismatches++;
      continue;
    }
    const ms = (performance.now() - t0).toFixed(0);
    const bad = names.filter((_, i) => got[i] !== hashes[i]);
    if (bad.length === 0) {
      console.log(`ok   ${code} (${ms} ms)`);
    } else {
      mismatches++;
      console.error(`FAIL ${code} (${ms} ms)`);
      for (const n of bad) {
        const i = names.indexOf(n);
        console.error(`     ${n}: wasm ${got[i]} != native ${hashes[i]}`);
      }
    }
  }
  if (mismatches > 0) {
    console.error(`determinism-check: ${mismatches}/${rows.length} share codes differ between WASM and native`);
    process.exit(1);
  }
  console.log(`determinism-check: all ${rows.length} share codes match the native golden hashes`);
}

await main();
