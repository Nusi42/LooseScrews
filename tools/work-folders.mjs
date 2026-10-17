#!/usr/bin/env node
// Work folders (docs/adr/0001-master-repo-and-work-folders.md). The work root is
// the parent folder of this checkout, the master is this checkout's `origin`
// remote, and the folder prefix comes from this checkout's name (`<prefix>.win<N>`,
// default `loosescrews`).
//   node tools/work-folders.mjs create <n>          clone the master into <work root>/<prefix>.win<n>
//                                                   (branch WIN<n>), copy the local git identity, set
//                                                   core.hooksPath, write .claude/settings.local.json,
//                                                   npm ci in web/
//   node tools/work-folders.mjs guard-master [path] mark a checkout as the master (default: the checkout
//                                                   containing the current directory): .env.local gets
//                                                   LS_CHECKOUT_ROLE=master, Cargo refuses every build
//   node tools/work-folders.mjs status              per work folder: branch, uncommitted / unpushed work;
//                                                   main vs WIN1 in the master
//   node tools/work-folders.mjs guard "<what>"      exit 1 with the refusal in the master (shell scripts)

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

import {
  ENV_LOCAL,
  MASTER_ROLE,
  MASTER_RUSTC_WRAPPER,
  REPO_ROOT,
  ROLE_KEY,
  assertWorkFolder,
  branchOf,
  claudeLocalSettings,
  folderOf,
  folderPrefix,
  masterCargoConfig,
  parseFolderNumber,
  withEnvValue,
  workFolderNumber,
} from "./lib/work-folder.mjs";

function git(cwd, args, { check = true } = {}) {
  const r = spawnSync("git", args, { cwd, encoding: "utf8" });
  if (check && r.status !== 0) throw new Error(`git ${args.join(" ")} failed:\n${r.stderr}`);
  return r.status === 0 ? r.stdout.trim() : undefined;
}

/** The `origin` remote of the checkout at `dir`: { url, dir } (dir only when it is a local folder). */
function originOf(dir) {
  const url = git(dir, ["remote", "get-url", "origin"], { check: false });
  if (!url) return { url: undefined, dir: undefined };
  const local = path.resolve(dir, url.replace(/^file:\/\//, ""));
  return { url, dir: /^[a-z][a-z0-9+.-]+:\/\//i.test(url) || !fs.existsSync(local) ? undefined : local };
}

function create(n) {
  assertWorkFolder("work-folders create");
  const workRoot = path.dirname(REPO_ROOT);
  const dir = path.join(workRoot, folderOf(n, folderPrefix(REPO_ROOT)));
  if (fs.existsSync(dir)) throw new Error(`${dir} exists already`);
  const { url } = originOf(REPO_ROOT);
  if (!url) throw new Error("this checkout has no origin remote (the master)");
  const branch = branchOf(n);
  console.log(`> git clone <origin> ${path.basename(dir)}`);
  git(workRoot, ["clone", url, dir]);
  const remote = git(dir, ["ls-remote", "--heads", "origin", branch]);
  git(dir, remote ? ["checkout", "-b", branch, "--track", `origin/${branch}`] : ["checkout", "-b", branch]);
  for (const key of ["user.name", "user.email"]) {
    const value = git(REPO_ROOT, ["config", "--local", "--get", key], { check: false });
    if (value) git(dir, ["config", "--local", key, value]);
  }
  git(dir, ["config", "--local", "core.hooksPath", ".githooks"]);
  fs.writeFileSync(path.join(dir, ".claude", "settings.local.json"), claudeLocalSettings(dir));
  console.log("> npm ci (web/)");
  // One command string: npm is a .cmd on Windows and needs a shell.
  const npm = spawnSync("npm ci", { cwd: path.join(dir, "web"), stdio: "inherit", shell: true });
  if (npm.status !== 0) throw new Error("npm ci failed");
  console.log(`\n${path.basename(dir)} is ready on ${branch}. Next: cd web && npm run build (WASM package + client).`);
}

function guardMaster(target) {
  const master = path.resolve(target ?? git(process.cwd(), ["rev-parse", "--show-toplevel"], { check: false }) ?? process.cwd());
  if (workFolderNumber(path.basename(master), folderPrefix(master)) !== undefined) {
    throw new Error(`${path.basename(master)} is named like a work folder; run guard-master inside the master`);
  }
  const envFile = path.join(master, ENV_LOCAL);
  const before = fs.existsSync(envFile) ? fs.readFileSync(envFile, "utf8") : "";
  fs.writeFileSync(envFile, withEnvValue(before, ROLE_KEY, MASTER_ROLE));
  const cargoDir = path.join(master, ".cargo");
  fs.mkdirSync(cargoDir, { recursive: true });
  fs.writeFileSync(path.join(cargoDir, "refuse-build.cmd"), MASTER_RUSTC_WRAPPER);
  fs.writeFileSync(path.join(cargoDir, "config.local.toml"), masterCargoConfig(".cargo/refuse-build.cmd"));
  console.log(`${path.basename(master)}: ${ENV_LOCAL} sets ${ROLE_KEY}=${MASTER_ROLE}; cargo refuses to build (.cargo/config.local.toml)`);
}

function dirtyCount(dir) {
  return (git(dir, ["status", "--porcelain"], { check: false }) ?? "").split("\n").filter(Boolean).length;
}

function status() {
  const lines = [];
  const workRoot = path.dirname(REPO_ROOT);
  const prefix = folderPrefix(REPO_ROOT);
  const folders = fs
    .readdirSync(workRoot)
    .map((name) => ({ name, n: workFolderNumber(name, prefix) }))
    .filter((f) => f.n !== undefined && fs.existsSync(path.join(workRoot, f.name, ".git")))
    .sort((a, b) => a.n - b.n);
  const width = Math.max(12, ...folders.map((f) => f.name.length + 1));
  for (const { name } of folders) {
    const dir = path.join(workRoot, name);
    git(dir, ["fetch", "--quiet", "origin"], { check: false });
    const branch = git(dir, ["branch", "--show-current"], { check: false }) ?? "?";
    const upstream = git(dir, ["rev-list", "--count", `origin/${branch}..HEAD`], { check: false });
    const unpushed = upstream ?? `${git(dir, ["rev-list", "--count", "origin/main..HEAD"], { check: false })} (branch not on origin; vs main)`;
    lines.push(`${name.padEnd(width)} ${branch.padEnd(6)} uncommitted files: ${dirtyCount(dir)}, unpushed commits: ${unpushed}`);
  }
  const master = originOf(REPO_ROOT).dir;
  if (master === undefined) {
    lines.push("master: origin is not a local folder (nothing to compare)");
  } else {
    const main = git(master, ["rev-parse", "main"]);
    const win1 = git(master, ["rev-parse", "--verify", "--quiet", branchOf(1)], { check: false });
    const ahead = win1 ? git(master, ["rev-list", "--count", `main..${branchOf(1)}`]) : "-";
    const behind = win1 ? git(master, ["rev-list", "--count", `${branchOf(1)}..main`]) : "-";
    lines.push(`master main ${main.slice(0, 8)}, uncommitted files: ${dirtyCount(master)}; ` +
      `${branchOf(1)} is ${ahead} ahead, ${behind} behind main` +
      (ahead === "0" || ahead === "-" ? "" : ` (after green CI: git -C <master> merge --ff-only ${branchOf(1)})`));
  }
  console.log(lines.join("\n"));
}

const [cmd, arg] = process.argv.slice(2);
try {
  if (cmd === "create") create(parseFolderNumber(arg));
  else if (cmd === "guard-master") guardMaster(arg);
  else if (cmd === "status") status();
  else if (cmd === "guard") assertWorkFolder(arg ?? "this command");
  else {
    console.error("usage: node tools/work-folders.mjs create <n> | guard-master [path] | status | guard <what>");
    process.exit(2);
  }
} catch (e) {
  console.error(`work-folders: ${e.message}`);
  process.exit(1);
}
