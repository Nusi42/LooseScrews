# ADR 0002: Public repository

Status: accepted · 2026-10-17 · revises [ADR 0001](0001-master-repo-and-work-folders.md)

An ADR (architecture decision record) records one decision: its context, what was decided,
the consequences and the alternatives. ADRs are numbered; a later ADR supersedes or revises an
earlier one.

## Context

The repository goes public as `github.com/Nusi42/LooseScrews`. Until now it was a private
repository under a working title, developed by the maintainer with Claude Code agents. Its
files, history and tooling contained material that does not belong in a public repository:
personal names and addresses, user-home paths, the maintainer's folder layout (the master was
recognised by a path segment of a cloud-synced folder), names of other projects, links into the
Claude web app, `Claude-Session` commit trailers, session ids in memory notes, and commit
messages full of internal logistics and process codes.

A one-off cleanup is not enough: agents and people keep writing files and commits, so the rules
must be enforced by the tooling, without the tooling itself containing the private values.

## Decisions

- **Product name:** the project, the repository and the tooling use the name Loose Screws
  (identifiers `loosescrews`, `LooseScrews`, `loose-screws`). Work folders are
  `<work root>/loosescrews.win<N>`; the tooling derives the prefix from the current folder's
  name, so existing folders keep working.
- **Private list outside the repository:** the values that must never be published (names,
  name parts, emails, account names, other projects' names, case-sensitive words, the GitHub
  handle and the public commit identity) live in `$LS_PRIVATE_LIST` or
  `~/.config/loose-screws/private-list.json`, never in the repository. Format and matching
  rules: docs/dev-guide.md, "Private information". A missing file means only the built-in
  checks run, so contributors need no list.
- **Guards:** `tools/check-private.mjs` scans files: per file in pre-commit and the Claude Code
  post-edit hook, all tracked files in pre-push and `tools/ci.*`. Built in: the git user email,
  the OS user name as a path segment, absolute user-home paths, `claude.ai` links,
  `Claude-Session` lines and `originSessionId` lines. With the list: its values as whole words,
  and the handle outside its GitHub forms (the noreply address and `github.com/<handle>/...`
  URLs). Findings show the kind and a masked value, never the value. The post-edit hook removes
  the `originSessionId` line Claude Code writes into memory notes before checking them.
- **Public commit identity:** when the list names a `publicIdentity`, every commit's author and
  committer must be exactly that name and the GitHub noreply address; no identity may match the
  list. The commit-msg hook checks the commit being made, pre-push every pushed commit.
- **Commit messages:** Conventional Commits (`<type>(<scope>): <subject>`, lowercase subject,
  first line at most 72 characters, no trailing period); merges say `Merge <what it brought>`;
  git's `Revert "..."` and `fixup!` / `squash!` subjects pass. Trailers: only
  `Co-Authored-By: Claude <model> <noreply@anthropic.com>` and `Signed-off-by` lines
  (`git commit -s`; contributions come under the CLA in CONTRIBUTING.md, ADR 0003). Messages contain no internal logistics (work folders, branch numbers,
  remote-tracking refs), no process jargon (package and milestone codes, agent and session
  vocabulary), no URL except the repository's own, and nothing from the private list.
  `tools/check-commit.mjs` implements the rules (commit-msg, pre-push) and reuses the
  check-private matcher. Details: docs/dev-guide.md, "Commit messages and identity".
- **Checkout role instead of path detection:** the master is the checkout whose gitignored
  `.env.local` says `LS_CHECKOUT_ROLE=master` (the environment variable overrides it), written by
  `node tools/work-folders.mjs guard-master`. Everything else, including a plain clone without
  any configuration, builds, tests and commits normally. The work root is the parent of the
  current work folder and the master its `origin` remote. Refusals name placeholders
  (`<work root>/loosescrews.win1`), never personal paths.
- **No hosted CI:** verification is local. `tools/ci.*` is the gate for `main`, and the pre-push
  hook runs it on every push to the public (network) remote; pushes to the master checkout run
  the fast subset. A release is built, signed and published locally; the GitHub release is
  created by hand.
- **History rewrite (2026-10-17):** before the first push to GitHub, the history was rewritten
  once: authors and committers set to the public identity, messages reworded to the rules,
  private values removed from every revision. The pre-push check of all commits not yet on the
  remote verified the result on that first push.

## Consequences

- The tooling holds no private value; the same checks run for every contributor, with or
  without a private list.
- Commits made with git's default merge message are refused: merges need `-m "Merge ..."`.
- Every checkout needs the public identity in its git config once the private list names one;
  `create` copies it into new work folders.
- The master must be marked once with `guard-master`; until then it is an ordinary checkout.
- Findings never print the private values, so hook output can be shared safely.

## Alternatives considered

- **Keep the private values in the repository** (for example as hashes): hashes of short names
  are easy to reverse, and the list would still reveal which values matter.
- **A server-side check only** (GitHub push rules or a CI job): too late; a commit is already
  written and possibly pushed elsewhere, and a server-side job has no private list.
- **Detect the master by path:** ties the tooling to one machine's layout and puts that layout
  into public files.
- **Free-form commit messages:** the history becomes hard to read for outsiders, and internal
  vocabulary leaks into the public log.
