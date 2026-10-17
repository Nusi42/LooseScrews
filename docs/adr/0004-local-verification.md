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
- **Verification is local.** The pre-push hook runs the full check (`tools/ci.ps1` on Windows,
  `tools/ci.sh` elsewhere) before every push to a network remote such as GitHub; pushes to a
  local checkout run the fast subset. `docs/dev-guide.md` lists the full local test run.
- **Releases are local.** A release is built, signed and published locally; the GitHub release is
  created by hand.

## Consequences

- A push to GitHub takes as long as the full check; a red check refuses the push.
- Pull requests from contributors are verified by the maintainer running the full check locally.

## Alternatives

- **GitHub Actions:** costs money on private repositories and cannot run the WebGPU smoke test.
- **A self-hosted runner:** more setup and a machine that must stay online.
