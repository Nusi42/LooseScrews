// What the pre-push hook runs for a push (docs/adr/0004-local-verification.md,
// docs/adr/0005-linear-history.md): the decision is one function, `pushPlan`, from the
// pushing checkout's role, the remote's URL and the pushed refs. tools/hooks/pre-push.mjs
// runs the plan. Tests: tools/test/pre-push.test.mjs (every combination) and
// tools/test/hooks.test.mjs (the hook itself, through git).
//
// - The master (`LS_CHECKOUT_ROLE=master`) is the only checkout that publishes: it pushes
//   `main` (and release tags `v<semver>`) to a network remote and runs only checks that
//   build nothing (no merge commits, private information, dependency licenses). `main` only
//   moves to a commit that passed the full check in win1, so that check is not repeated.
// - A work folder (its `origin` is a master checkout) pushes only to a local path: no merge
//   commits, then the fast CI subset (STEPS).
// - A plain clone (no master) runs the fast CI subset for every push.
// - Every other combination is refused. Every push also checks the pushed commits'
//   messages and identities (tools/lib/push-checks.mjs) before the plan's steps.

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { isMaster } from "./work-folder.mjs";

/** The remote the master publishes to. */
export const PUBLIC_REMOTE = "github";

/** The branch the master publishes. */
export const PUBLISH_BRANCH = "main";

/** The command that publishes `main` from the master at `master` (named in refusals). */
export function publishCommand(master = "<master>") {
  return `git -C "${master}" push ${PUBLIC_REMOTE} ${PUBLISH_BRANCH}`;
}

/** A step: [name, command, arguments, folder relative to the checkout, options]; `stdin`: gets git's ref lines. */
const step = (name, cmd, args, dir = ".", options = {}) => [name, cmd, args, dir, options];

/** No merge commits in what the push adds (docs/adr/0005-linear-history.md). */
export const LINEAR_STEP = step("no merge commits", "node", ["tools/check-linear.mjs"], ".", { stdin: true });

/** The fast CI subset a work folder (or a plain clone) runs before its push. */
export const STEPS = [
  step("cargo fmt --check", "cargo", ["fmt", "--all", "--", "--check"]),
  step("generated reference docs", "node", ["tools/gen-weapon-docs.mjs", "--check"]),
  step("wildcard binds", "node", ["tools/check-binds.mjs"]),
  step("private information", "node", ["tools/check-private.mjs"]),
  step("dependency licenses", "node", ["tools/check-licenses.mjs"]),
  step("tools tests", "node", ["--test", "tools/test/*.test.mjs"]),
  step("cargo clippy -D warnings", "cargo", ["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]),
  step("cargo test --workspace", "cargo", ["test", "--workspace"]),
  step("web typecheck", "npm", ["run", "typecheck"], "web"),
  step("web unit tests", "npm", ["test"], "web"),
];

/**
 * The checks of a push from the master to the public remote: only those that build nothing.
 * Its `main` only ever moves to a WIN1 commit that passed the full check in win1.
 */
export const MASTER_STEPS = [
  LINEAR_STEP,
  ...STEPS.filter(([name]) => name === "private information" || name === "dependency licenses"),
];

/** The checks of a push from a work folder to the master. */
export const WORK_STEPS = [LINEAR_STEP, ...STEPS];

/** URLs that name a path on this machine; everything else is a network remote. */
const LOCAL_URL = [
  /^[a-z]:(?:[\\/]|$)/i, // C:\…, C:/…
  /^\\\\/, // \\host\share
  /^\//, // /…
  /^\.\.?(?:[\\/]|$)/, // ./x, ../x
  /^file:\/\//i,
];

/**
 * The kind of a remote URL: "local" for a path (`C:\...`, `/...`, `../x`, `\\host\share`,
 * `file://...`, a bare folder name), "network" for a URL (`https://...`, `ssh://...`,
 * scp-like `git@host:path`).
 */
export function remoteKind(url) {
  const u = (url ?? "").trim();
  if (LOCAL_URL.some((re) => re.test(u))) return "local";
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(u)) return "network";
  const colon = u.indexOf(":");
  const slash = u.search(/[\\/]/);
  return colon > 0 && (slash < 0 || colon < slash) ? "network" : "local";
}

/** The folder a local remote URL names, resolved against `root`. */
export function localPath(root, url) {
  const u = url.trim();
  return /^file:\/\//i.test(u) ? fileURLToPath(u) : path.resolve(root, u);
}

/** The `origin` URL of the checkout at `root`, or undefined. */
export function originUrl(root) {
  const r = spawnSync("git", ["remote", "get-url", "origin"], { cwd: root, encoding: "utf8" });
  return r.status === 0 ? r.stdout.trim() : undefined;
}

/** The master checkout the checkout at `root` pushes to (its local `origin`, marked master), or undefined. */
export function masterOf(root) {
  const url = originUrl(root);
  if (url === undefined || remoteKind(url) !== "local") return undefined;
  const dir = localPath(root, url);
  // Only the origin's own .env.local counts, not this process's environment.
  return isMaster(dir, {}) ? dir : undefined;
}

/** The push role of the checkout at `root`: "master", "work" (a work folder of a master) or "plain". */
export function pushRole(root, env = process.env) {
  if (isMaster(root, env)) return "master";
  return masterOf(root) === undefined ? "plain" : "work";
}

/** The pushed refs git writes on stdin: `<local ref> <local sha> <remote ref> <remote sha>` per line. */
export function parsePushLines(text) {
  return text
    .split(/\r?\n/)
    .map((l) => l.trim().split(/\s+/))
    .filter((f) => f.length === 4)
    .map(([localRef, localSha, remoteRef, remoteSha]) => ({ localRef, localSha, remoteRef, remoteSha }));
}

/** The object id git reports for "no object" (a deletion, a new ref). */
const NO_OBJECT = /^0+$/;

/** Whether the repository at `root` has the commit `sha`. */
export function hasCommit(root, sha) {
  return spawnSync("git", ["cat-file", "-e", `${sha}^{commit}`], { cwd: root, stdio: "ignore" }).status === 0;
}

/**
 * The `git rev-list` / `git log` arguments that select the commits a ref line adds: those on no
 * remote-tracking branch and not under the remote's old value. Undefined for a deletion.
 * `known(sha)`: whether the repository has that commit.
 */
export function pushedRange(ref, known) {
  if (NO_OBJECT.test(ref.localSha)) return undefined;
  const range = [ref.localSha, "--not", "--remotes"];
  if (!NO_OBJECT.test(ref.remoteSha) && known(ref.remoteSha)) range.push(ref.remoteSha);
  return range;
}

/** Whether `ref` is a release tag (`refs/tags/v<semver>`). */
export function isReleaseTagRef(ref) {
  return /^refs\/tags\/v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(ref);
}

/** Whether the master may push `ref`: `main` to `main`, or a release tag to itself. */
function publishable(ref) {
  const main = `refs/heads/${PUBLISH_BRANCH}`;
  return (ref.localRef === main && ref.remoteRef === main) || (ref.localRef === ref.remoteRef && isReleaseTagRef(ref.localRef));
}

/**
 * What a push runs, or why it is refused: `{ steps }` or `{ refused }`.
 * `role`: pushRole of the pushing checkout; `url`: the remote's URL; `refs`: parsePushLines of
 * git's stdin; `master`: the master's path (named in a work folder's refusal).
 */
export function pushPlan({ role, url, refs = [], master }) {
  const local = remoteKind(url) === "local";
  if (role === "plain") return { steps: STEPS };
  if (role === "work") {
    if (local) return { steps: WORK_STEPS };
    return {
      refused:
        `a work folder pushes only to the master checkout, not to ${String(url).trim()}.\n` +
        `Push the branch to origin, fast-forward the master's main, then publish from the master:\n` +
        `  ${publishCommand(master)}`,
    };
  }
  if (role !== "master") return { refused: `unknown checkout role "${role}"` };
  if (local) {
    return { refused: `the master checkout pushes only to the public remote, not to ${String(url).trim()}; work folders fetch from it instead` };
  }
  const other = refs.filter((ref) => !publishable(ref));
  if (other.length > 0) {
    return {
      refused: `the master publishes only ${PUBLISH_BRANCH} (and release tags v<semver>), not ${other.map((r) => `${r.localRef} -> ${r.remoteRef}`).join(", ")}`,
    };
  }
  return { steps: MASTER_STEPS };
}
