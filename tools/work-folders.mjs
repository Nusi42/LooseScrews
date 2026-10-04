#!/usr/bin/env node
// Work folders (docs/adr/0001-master-repo-and-work-folders.md):
//   node tools/work-folders.mjs create <n>          clone the master into loosescrews.win<n> (branch WIN<n>),
//                                                   write .claude/settings.local.json, npm ci in web/
//   node tools/work-folders.mjs guard-master [path] make Cargo refuse every build in the master
//   node tools/work-folders.mjs status              per work folder: branch, uncommitted / unpushed work;
//                                                   main vs WIN1 (the "can I close this session?" report)
//   node tools/work-folders.mjs guard "<what>"      exit 1 with the refusal inside OneDrive (shell scripts)

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

import {
  MASTER,
  MASTER_RUSTC_WRAPPER,
  WORK_BASE,
  assertWorkFolder,
  branchOf,
  claudeLocalSettings,
  folderOf,
  masterCargoConfig,
  parseFolderNumber,
} from "./lib/work-folder.mjs";

function git(cwd, args, { check = true } = {}) {
  const r = spawnSync("git", args, { cwd, encoding: "utf8" });
  if (check && r.status !== 0) throw new Error(`git ${args.join(" ")} (in ${cwd}) failed:\n${r.stderr}`);
  return r.status === 0 ? r.stdout.trim() : undefined;
}

function create(n) {
  assertWorkFolder("work-folders create");
  const dir = path.join(WORK_BASE, folderOf(n));
  if (fs.existsSync(dir)) throw new Error(`${dir} exists already`);
  const branch = branchOf(n);
  console.log(`> git clone ${MASTER} ${dir}`);
  git(WORK_BASE, ["clone", MASTER, dir]);
  const remote = git(dir, ["ls-remote", "--heads", "origin", branch]);
  git(dir, remote ? ["checkout", "-b", branch, "--track", `origin/${branch}`] : ["checkout", "-b", branch]);
  fs.writeFileSync(path.join(dir, ".claude", "settings.local.json"), claudeLocalSettings(dir));
  console.log("> npm ci (web/; its prepare installs the Git hooks)");
  const npm = spawnSync("npm", ["ci"], { cwd: path.join(dir, "web"), stdio: "inherit", shell: true });
  if (npm.status !== 0) throw new Error("npm ci failed");
  console.log(`\n${dir} is ready on ${branch}. Next: cd web && npm run build (WASM package + client).`);
}

function guardMaster(master) {
  const cargoDir = path.join(master, ".cargo");
  fs.mkdirSync(cargoDir, { recursive: true });
  fs.writeFileSync(path.join(cargoDir, "refuse-build.cmd"), MASTER_RUSTC_WRAPPER);
  fs.writeFileSync(path.join(cargoDir, "config.local.toml"), masterCargoConfig(".cargo/refuse-build.cmd"));
  console.log(`${master}: cargo now refuses to build (.cargo/config.local.toml -> refuse-build.cmd)`);
}

function status() {
  const lines = [];
  const folders = fs
    .readdirSync(WORK_BASE)
    .filter((d) => /^colent\.win\d+$/.test(d))
    .sort((a, b) => Number(a.slice(10)) - Number(b.slice(10)));
  for (const name of folders) {
    const dir = path.join(WORK_BASE, name);
    git(dir, ["fetch", "--quiet", "origin"], { check: false });
    const branch = git(dir, ["branch", "--show-current"], { check: false }) ?? "?";
    const dirty = (git(dir, ["status", "--porcelain"], { check: false }) ?? "").split("\n").filter(Boolean).length;
    const upstream = git(dir, ["rev-list", "--count", `origin/${branch}..HEAD`], { check: false });
    const unpushed = upstream ?? `${git(dir, ["rev-list", "--count", "origin/main..HEAD"], { check: false })} (branch not on origin; vs main)`;
    lines.push(`${name.padEnd(12)} ${branch.padEnd(6)} uncommitted files: ${dirty}, unpushed commits: ${unpushed}`);
  }
  const main = git(MASTER, ["rev-parse", "main"]);
  const win1 = git(MASTER, ["rev-parse", "--verify", "--quiet", "WIN1"], { check: false });
  const ahead = win1 ? git(MASTER, ["rev-list", "--count", "main..WIN1"]) : "-";
  const behind = win1 ? git(MASTER, ["rev-list", "--count", "WIN1..main"]) : "-";
  lines.push(`master main ${main.slice(0, 8)}; WIN1 is ${ahead} ahead, ${behind} behind main` +
    (ahead === "0" ? "" : " (fast-forward after green CI: git -C <master> merge --ff-only WIN1)"));
  console.log(lines.join("\n"));
}

const [cmd, arg] = process.argv.slice(2);
try {
  if (cmd === "create") create(parseFolderNumber(arg));
  else if (cmd === "guard-master") guardMaster(arg ?? MASTER);
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
