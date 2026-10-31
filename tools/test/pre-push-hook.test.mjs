// The real pre-push hook (.githooks/pre-push → tools/hooks/pre-push.mjs) run by git: a
// throwaway master (marked by .env.local) and a work folder cloned from it, both using this
// checkout's hooks, a made-up private list. Pushes that would run the build steps are not
// made here; every refusal and the checks before the build steps are.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import { publishCommand } from "../lib/push-plan.mjs";
import { REPO_ROOT } from "../lib/work-folder.mjs";
import { git, listFile } from "./git-fixture.mjs";
import { FIXTURE_LIST } from "./private-list-fixture.mjs";

const ZERO = "0".repeat(40);
const NETWORK_URL = "https://github.com/example/repo.git";
const { name: PUBLIC_NAME, email: PUBLIC_EMAIL } = FIXTURE_LIST.publicIdentity;

/** A master and a work folder cloned from it, with this checkout's hooks; runs `body`, then cleans up. */
function withRepos(body) {
  const base = fs.mkdtempSync(path.join(os.tmpdir(), "ls-push-hook-"));
  const list = listFile(FIXTURE_LIST);
  try {
    const hooks = path.join(REPO_ROOT, ".githooks");
    const env = { ...process.env, LS_PRIVATE_LIST: list };
    delete env.LS_CHECKOUT_ROLE;
    const configure = (dir) => {
      for (const [key, value] of [["user.name", PUBLIC_NAME], ["user.email", PUBLIC_EMAIL], ["core.hooksPath", hooks], ["commit.gpgsign", "false"]]) {
        git(dir, ["config", key, value]);
      }
    };
    const master = path.join(base, "master");
    fs.mkdirSync(master);
    git(master, ["init", "--quiet", "--initial-branch=main"]);
    configure(master);
    fs.writeFileSync(path.join(master, ".env.local"), "LS_CHECKOUT_ROLE=master\n");
    fs.writeFileSync(path.join(master, ".gitignore"), ".env.local\n");
    git(master, ["add", ".gitignore"]);
    git(master, ["-c", "core.hooksPath=no-hooks", "commit", "--quiet", "-m", "chore: start the repository"]);
    const work = path.join(base, "work");
    git(base, ["clone", "--quiet", master, work]);
    configure(work);
    /** git with the hooks on; its result unchecked. */
    const hooked = (dir, args) => spawnSync("git", args, { cwd: dir, env, encoding: "utf8" });
    /** Commits file `name`, hooks off. */
    const commit = (dir, name, message) => {
      fs.writeFileSync(path.join(dir, name), `${name}\n`);
      git(dir, ["add", name]);
      git(dir, ["-c", "core.hooksPath=no-hooks", "commit", "--quiet", "-m", message]);
      return git(dir, ["rev-parse", "HEAD"]);
    };
    /** A merge commit on the current branch of `dir` (hooks off). */
    const merge = (dir) => {
      const branch = git(dir, ["branch", "--show-current"]);
      git(dir, ["switch", "--quiet", "-c", "side"]);
      commit(dir, "side.txt", "feat: add the side file");
      git(dir, ["switch", "--quiet", branch]);
      commit(dir, "main.txt", "feat: add the main file");
      git(dir, ["-c", "core.hooksPath=no-hooks", "merge", "--quiet", "--no-ff", "-m", "Merge the side file", "side"]);
      return git(dir, ["rev-parse", "HEAD"]);
    };
    /** Runs the pre-push hook as git would, with these ref lines on stdin. */
    const prePush = (dir, url, lines) => {
      const stdin = path.join(base, "push-lines.txt");
      fs.writeFileSync(stdin, lines);
      return hooked(dir, ["hook", "run", `--to-stdin=${stdin}`, "pre-push", "--", "github", url]);
    };
    body({ base, master, work, hooked, commit, merge, prePush });
  } finally {
    fs.rmSync(base, { recursive: true, force: true });
    fs.rmSync(path.dirname(list), { recursive: true, force: true });
  }
}

test("a work folder's push to the master refuses a merge commit", () => {
  withRepos(({ master, work, hooked, merge }) => {
    merge(work);
    const push = hooked(work, ["push", "origin", "HEAD:refs/heads/WIN2"]);
    assert.notEqual(push.status, 0);
    assert.match(push.stderr, /^merge commit [0-9a-f]{10} Merge the side file$/m);
    assert.match(push.stderr, /git rebase <target>/);
    assert.match(push.stderr, /"no merge commits" failed; push refused/);
    assert.equal(git(master, ["branch", "--list", "WIN2"]), "");
  });
});

test("a work folder's push to the master checks the pushed commits' messages first", () => {
  withRepos(({ master, work, hooked, commit }) => {
    commit(work, "a.txt", "WIP on WIN2");
    const push = hooked(work, ["push", "origin", "HEAD:refs/heads/WIN2"]);
    assert.notEqual(push.status, 0);
    assert.match(push.stderr, /pre-push: push refused/);
    assert.match(push.stderr, /message: /);
    assert.equal(git(master, ["branch", "--list", "WIN2"]), "");
  });
});

test("a work folder never pushes to a network remote; the refusal names the master's command", () => {
  withRepos(({ master, work, prePush }) => {
    const head = git(work, ["rev-parse", "HEAD"]);
    const run = prePush(work, NETWORK_URL, `refs/heads/main ${head} refs/heads/main ${ZERO}\n`);
    assert.equal(run.status, 1);
    assert.match(run.stderr, /a work folder pushes only to the master checkout/);
    const origin = git(work, ["remote", "get-url", "origin"]);
    assert.ok(run.stderr.includes(publishCommand(path.resolve(work, origin))), run.stderr);
    assert.ok(path.resolve(work, origin) === path.resolve(master));
  });
});

test("the master never pushes to a local path", () => {
  withRepos(({ base, master, hooked }) => {
    const bare = path.join(base, "bare.git");
    git(base, ["init", "--quiet", "--bare", bare]);
    const push = hooked(master, ["push", bare, "main"]);
    assert.notEqual(push.status, 0);
    assert.match(push.stderr, /the master checkout pushes only to the public remote/);
    assert.equal(git(bare, ["branch", "--list"]), "");
  });
});

test("the master publishes only main and release tags", () => {
  withRepos(({ master, prePush }) => {
    const head = git(master, ["rev-parse", "HEAD"]);
    for (const [local, remote] of [["refs/heads/WIN1", "refs/heads/WIN1"], ["refs/heads/main", "refs/heads/dev"], ["refs/tags/test", "refs/tags/test"]]) {
      const run = prePush(master, NETWORK_URL, `${local} ${head} ${remote} ${ZERO}\n`);
      assert.equal(run.status, 1, local);
      assert.match(run.stderr, /the master publishes only main \(and release tags v<semver>\)/);
    }
  });
});

test("the master's publish refuses merge commits and commits that break the rules", () => {
  withRepos(({ master, commit, merge, prePush }) => {
    const merged = merge(master);
    const linear = prePush(master, NETWORK_URL, `refs/heads/main ${merged} refs/heads/main ${ZERO}\n`);
    assert.equal(linear.status, 1);
    assert.match(linear.stderr, /^merge commit [0-9a-f]{10} Merge the side file$/m);
    assert.match(linear.stderr, /"no merge commits" failed; push refused/);
    // A merge already on the public remote stays; only what the push adds is checked.
    const good = commit(master, "a.txt", "fix: add the a file");
    const after = prePush(master, NETWORK_URL, `refs/heads/main ${good} refs/heads/main ${merged}\n`);
    // git hands a hook's stdout to stderr.
    assert.match(after.stderr, /check-linear: no merge commits to push/);
    assert.doesNotMatch(after.stderr, /^merge commit /m);
    const bad = commit(master, "b.txt", "Added stuff");
    const message = prePush(master, NETWORK_URL, `refs/heads/main ${bad} refs/heads/main ${good}\n`);
    assert.equal(message.status, 1);
    assert.match(message.stderr, /pre-push: push refused\n {2}[0-9a-f]{10} Added stuff: message: /);
  });
});
