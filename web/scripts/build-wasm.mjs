#!/usr/bin/env node
// Builds crates/ls-wasm for the browser:
//   cargo build -p ls-wasm --target wasm32-unknown-unknown --release
//   wasm-bindgen --target web --out-dir web/src/wasm/pkg <ls_wasm.wasm>
// The output directory is gitignored. Options:
//   --if-missing   skip everything when the package already exists (typecheck/test)
//   --debug        build the debug profile (faster compile, much slower game)
// Honors CARGO_TARGET_DIR (relative paths resolve against the repo root, like cargo
// run from there). Requires the wasm32-unknown-unknown target and wasm-bindgen CLI
// 0.2.126 (matching the crate pin in Cargo.toml).

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const webDir = resolve(here, "..");
const repoRoot = resolve(webDir, "..");
const outDir = join(webDir, "src", "wasm", "pkg");
const args = new Set(process.argv.slice(2));
const profile = args.has("--debug") ? "debug" : "release";

if (args.has("--if-missing") && existsSync(join(outDir, "ls_wasm.js")) && existsSync(join(outDir, "ls_wasm_bg.wasm"))) {
  console.log("build-wasm: package exists, skipping (run `npm run build:wasm` to rebuild)");
  process.exit(0);
}

function run(cmd, cmdArgs) {
  console.log(`> ${cmd} ${cmdArgs.join(" ")}`);
  const r = spawnSync(cmd, cmdArgs, { cwd: repoRoot, stdio: "inherit", shell: false });
  if (r.error) {
    console.error(`build-wasm: could not run ${cmd}: ${r.error.message}`);
    process.exit(1);
  }
  if (r.status !== 0) process.exit(r.status ?? 1);
}

const cargoArgs = ["build", "-p", "ls-wasm", "--target", "wasm32-unknown-unknown"];
if (profile === "release") cargoArgs.push("--release");
run("cargo", cargoArgs);

const targetEnv = process.env.CARGO_TARGET_DIR;
const targetDir = targetEnv ? (isAbsolute(targetEnv) ? targetEnv : resolve(repoRoot, targetEnv)) : join(repoRoot, "target");
const wasm = join(targetDir, "wasm32-unknown-unknown", profile, "ls_wasm.wasm");
if (!existsSync(wasm)) {
  console.error(`build-wasm: expected ${wasm} after cargo build`);
  process.exit(1);
}
run("wasm-bindgen", ["--target", "web", "--out-dir", outDir, wasm]);
console.log(`build-wasm: wrote ${outDir}`);
