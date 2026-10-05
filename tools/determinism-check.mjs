#!/usr/bin/env node
// Determinism cross-check: runs the built ls-wasm package under Node and
// compares
// - goldenHashes(code) for every share code in
//   crates/ls-worldgen/tests/fixtures/golden_hashes.txt with the native fixture
//   (sdf, surface_nets, dual_contouring, edited), and
// - modeReplayHashes(): one scripted 40 s, 8-ship replay per built-in rule set
//   (every mode, Instagib, duel; ls_sim::golden) with the native fixture
//   crates/ls-sim/tests/fixtures/mode_replays.txt.
// Exits non-zero on any mismatch or if the package is missing.
//
// Usage: node tools/determinism-check.mjs [--pkg <dir>] [--fixture <file>] [--mode-fixture <file>]
//   --pkg           wasm-bindgen output directory (default web/src/wasm/pkg;
//                   build it with `cd web && npm run build:wasm`)
//   --fixture       worldgen golden hash fixture (default: the ls-worldgen fixture)
//   --mode-fixture  per-mode replay fixture (default: the ls-sim fixture)
//
// The ls-sim unit-test replay goldens (REPLAY_GOLDEN_HASH and the per-mode
// constants in modes/*) are cross-checked by the wasm32 unit tests in tools/ci.*.

import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function parseArgs(argv) {
  const opts = {
    pkg: join(repoRoot, "web", "src", "wasm", "pkg"),
    fixture: join(repoRoot, "crates", "ls-worldgen", "tests", "fixtures", "golden_hashes.txt"),
    modeFixture: join(repoRoot, "crates", "ls-sim", "tests", "fixtures", "mode_replays.txt"),
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
    else if (a === "--mode-fixture") opts.modeFixture = next();
    else if (a === "--help" || a === "-h") {
      console.log("usage: node tools/determinism-check.mjs [--pkg <dir>] [--fixture <file>] [--mode-fixture <file>]");
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

/** Mode replay fixture lines: `rulesets/<file>.ron 0x<16 hex>`. */
export function parseModeFixture(text) {
  const rows = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    const m = /^(\S+)\s+(0x[0-9a-f]{16})$/.exec(line);
    if (!m) throw new Error(`malformed mode replay fixture line: ${line}`);
    rows.push({ file: m[1], hash: m[2] });
  }
  return rows;
}

/** Compares `modeReplayHashes()` lines with the fixture rows; returns the problems. */
export function compareModeReplays(got, rows) {
  const problems = [];
  const byFile = new Map();
  for (const l of got) {
    const [file, hash] = String(l).trim().split(/\s+/);
    byFile.set(file, hash);
  }
  for (const { file, hash } of rows) {
    const g = byFile.get(file);
    if (g === undefined) problems.push(`${file}: missing in the WASM build`);
    else if (g !== hash) problems.push(`${file}: wasm ${g} != native ${hash}`);
    byFile.delete(file);
  }
  for (const file of byFile.keys()) problems.push(`${file}: not in the native fixture (re-bless it)`);
  return problems;
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
  } else {
    console.log(`determinism-check: all ${rows.length} share codes match the native golden hashes`);
  }

  // Per-mode replays.
  if (typeof pkg.modeReplayHashes !== "function") fail("package has no modeReplayHashes export (rebuild it: cd web && npm run build:wasm)");
  if (!existsSync(opts.modeFixture)) fail(`mode replay fixture not found: ${opts.modeFixture}`);
  const modeRows = parseModeFixture(readFileSync(opts.modeFixture, "utf8"));
  if (modeRows.length === 0) fail("mode replay fixture has no entries");
  const t0 = performance.now();
  const got = pkg.modeReplayHashes();
  const ms = (performance.now() - t0).toFixed(0);
  const problems = compareModeReplays(got, modeRows);
  for (const { file, hash } of modeRows) {
    if (!problems.some((p) => p.startsWith(`${file}:`))) console.log(`ok   ${file} ${hash}`);
  }
  for (const p of problems) console.error(`FAIL ${p}`);
  if (problems.length > 0) {
    console.error(`determinism-check: ${problems.length} mode replay(s) differ between WASM and native (${ms} ms)`);
  } else {
    console.log(`determinism-check: all ${modeRows.length} mode replays match the native hashes (${ms} ms)`);
  }
  if (mismatches > 0 || problems.length > 0) process.exit(1);
}

// Run only as a script (the parsers are imported by tools/test).
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
