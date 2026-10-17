// Throwaway git repositories for the hook tests. Commits are written with plumbing
// (`git hash-object`), so no hook or signing configuration of the machine applies.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

/** Runs git in `cwd`; returns trimmed stdout, throws on failure. */
export function git(cwd, args, input) {
  const r = spawnSync("git", args, { cwd, input, encoding: "utf8" });
  if (r.status !== 0) throw new Error(`git ${args.join(" ")}: ${r.stderr}`);
  return r.stdout.trim();
}

/** A new empty repository in the temp folder. */
export function tempRepo() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "hook-repo-"));
  git(dir, ["init", "--quiet"]);
  return dir;
}

/** Writes a commit object with `parents`, `author` / `committer` idents and `message`; returns its sha. */
export function commitRaw(repo, { parents = [], author, committer = author, message }) {
  const tree = git(repo, ["hash-object", "-t", "tree", "-w", "--stdin"], "");
  const raw = [`tree ${tree}`, ...parents.map((p) => `parent ${p}`), `author ${author}`, `committer ${committer}`, "", message].join("\n");
  return git(repo, ["hash-object", "-t", "commit", "-w", "--stdin"], raw);
}

/** Writes `list` as a private-list file and returns its path. */
export function listFile(list) {
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "private-list-")), "list.json");
  fs.writeFileSync(file, JSON.stringify(list));
  return file;
}
