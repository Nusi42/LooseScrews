# ADR 0004: Local verification, no hosted CI

Status: accepted · 2026-10-17

## Context

The repository is hosted on GitHub. Hosted CI (GitHub Actions) costs money on private
repositories, and the full check needs a GPU-capable browser for the smoke test, which hosted
runners do not have. The checks already run locally: `tools/ci.sh` / `tools/ci.ps1` and the git
hooks.

## Decisions

- **No hosted CI.** The repository has no `.github/workflows/`; Actions are switched off for the
  repository.
- **Verification is local.** The full check (`tools/ci.ps1` on Windows, `tools/ci.sh`
  elsewhere) runs in the lead work folder before `main` moves; `main` only ever moves to a commit
  that passed it. `docs/dev-guide.md` lists the full local test run.
- **Only the master publishes.** Work folders push only to the master checkout (the pre-push
  hook runs the fast subset); the master pushes `main` only to the public remote, and its
  pre-push hook runs the private-information and dependency-license checks (nothing is built in
  the master). The hook refuses a push from a work folder to the public remote.
- **Releases are local.** A release is built, signed and published locally; the GitHub release is
  created by hand.

## Consequences

- Publishing is quick: the full check already ran before `main` moved.
- Pull requests from contributors are verified by the maintainer running the full check locally.

## Alternatives

- **GitHub Actions:** costs money on private repositories and cannot run the WebGPU smoke test.
- **A self-hosted runner:** more setup and a machine that must stay online.
- **Publishing from the lead work folder** (its pre-push runs the full check): every work folder
  could then publish, and a push would take as long as the full check.
