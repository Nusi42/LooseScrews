# ADR 0005: Linear history and the push plan

Status: accepted · 2026-10-31 · revises [ADR 0001](0001-master-repo-and-work-folders.md),
[ADR 0002](0002-public-repository.md) and [ADR 0004](0004-local-verification.md)

## Context

Branches were combined in win1 with `git merge -m "Merge <what it brought>" origin/WINn`, so
`main` grew merge commits. The public repository's ruleset on `main` is to require a linear
history, which a push with merge commits would break, and a straight `main` is easier to read,
bisect and revert.

The pre-push hook already decided who may push where (ADR 0004), but the decision only looked
at the role and the remote's URL: the master could have pushed any branch or tag to the public
remote. The hook file decided and ran only behind an "is this the main module" test, which
would let every push through silently if that test ever went wrong. Its tests called its
functions but never ran the hook through git.

The Claude Code post-edit hook runs from `$CLAUDE_PROJECT_DIR`, usually the master, whose
copy of the tools lags behind until `main` is fast-forwarded; any logic in that file runs in
its old version.

## Decisions

- **Rebase, never merge.** Branches are combined by rebasing onto the target and
  fast-forwarding, in win1:
  `git fetch && git switch -C integrate origin/WINn && git rebase WIN1 && git switch WIN1 && git merge --ff-only integrate`.
  Merge commits already in the history stay; it is not rewritten for this.
- **The pre-push hook refuses merge commits** in what a push adds (the commits on no
  remote-tracking branch and not under the remote's old value) on pushes to the master and
  from the master: `tools/check-linear.mjs`, the first step of both plans.
- **One decision function.** `pushPlan({ role, url, refs, master })` in
  `tools/lib/push-plan.mjs` returns the steps of a push or the reason it is refused:
  - master → network remote: only `main` to `main` and release tags `v<semver>` to
    themselves; steps: no merge commits, private information, dependency licenses (nothing is
    built);
  - master → local path: refused;
  - work folder (its `origin` is a master checkout) → local path: no merge commits, then the
    fast CI subset;
  - work folder → network remote: refused, naming the command that publishes from the master
    (`git -C "<master>" push github main`);
  - plain clone (no master): the fast CI subset for every push, no linear-history check (a
    contributor's pull request is squash-merged).
  Before the steps, every push checks the pushed commits' messages and identities (ADR 0002).
- **The hook runs on load.** `tools/hooks/pre-push.mjs` and `tools/check-linear.mjs` have no
  "is main module" test; the libraries hold everything the tests import. `.githooks/pre-push`
  and `.githooks/commit-msg` run the tools of the checkout the hooks belong to
  (`$(dirname "$0")/../tools/...`), so tests can point a throw-away repository's
  `core.hooksPath` at them and run the real hooks through git (`git push` to temporary
  repositories, `git hook run --to-stdin=<file> pre-push -- <name> <url>`).
- **The post-edit hook is a small, stable loader.** `tools/post-edit.mjs` finds the checkout
  from the edited file and loads that checkout's `tools/lib/after-edit.mjs`, which removes the
  front-matter keys Claude Code writes into memory notes (`originSessionId`, `modified`,
  `node_type`, at the top level or indented under `metadata`), skips a master checkout and
  runs the per-file checks. The private-information check refuses `originSessionId` in every
  tracked file.
- Docs name `Claude-Session` and `originSessionId` without a trailing colon, so a scan of the
  history for real trailers finds only real ones.

## Consequences

- `main` stays a straight line from here on; the GitHub ruleset can require it.
- A package branch whose base moved is rebased in win1; conflicts are resolved per commit.
- Changing what a push runs means changing `pushPlan` and its table test; the hook only runs
  the plan.
- A master whose `post-edit.mjs` is older still works with newer work folders: the old loader
  calls the per-file runner, the new one the after-edit module, and both exist.

## Alternatives considered

- **Squash every package into one commit:** loses the package's own history; rebasing keeps
  it.
- **Merge commits, linear history only on GitHub (squash on publish):** `main` in the master and
  on GitHub would differ, and every publish would rewrite history.
- **Check only on the master's publish:** a merge commit would reach the master's `main` first
  and could then only be removed by rewriting it; checking pushes to the master stops it
  earlier.
