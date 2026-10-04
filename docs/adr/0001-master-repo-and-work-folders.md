# ADR 0001: Master repository in OneDrive, work folders outside it

Status: accepted · 2026-10-04

An ADR (architecture decision record) records one decision: its context, what was decided,
the consequences and the alternatives. ADRs are numbered and never rewritten; a later ADR
supersedes an earlier one.

## Context

The repository lived in OneDrive and everything ran there: Cargo's `target/`, `web/node_modules`,
`web/dist`, the WASM package, test output and the CodeGraph index. OneDrive synced gigabytes of
build output and locked files during builds, and parallel agents shared one checkout. The
master's folders had also become OneDrive cloud-file reparse points (`fsutil reparsepoint query
crates` → `0x9000e01a`). Git for Windows cannot check out into those (`unable to unlink 'crates'`,
`cannot create directory`).

Other projects of the maintainer run as a master checkout plus clones in
`<work root>\<project>.winN` (another project's ADR 0132, another project's ADR-0024). colent now
works the same way.

## Decisions

- **Master:** `<master>` holds `main`. It only receives pushes and
  `git merge --ff-only WIN1`. Nothing is built, tested, installed, run or committed there.
- **Work folders** are clones of the master (`origin` = the master path):
  - `<work root>/loosescrews.win1`, branch `WIN1`, is the lead. Integration and
    CI run here.
  - `loosescrews.win2` and up (`WIN2`, …) take one parallel work package each. Each one is reset to
    `origin/main`, commits its package and pushes its branch; the lead merges it into `WIN1`.
  - `main` moves only after `tools/ci.*` is green in win1: `git push origin WIN1`, then
    `git -C <master> merge --ff-only WIN1`.
- **The maintainer's Claude session** keeps running in the master folder. It drives the clones by
  absolute path (`cd` per command) and never edits files in the master.
- **One-time reparse fix:** every folder except `.git` moves to
  `<work root>\colent.master-backup`. Then `git checkout HEAD -- .` recreates
  plain folders (`fsutil` → error 4390). Never `reset --hard`, and never write files by hand in
  the master.
- **Build output stays in each work folder** (`target/`, `web/node_modules`, `web/dist`,
  `web/src/wasm/pkg`, `target/e2e-logs`, `target/browser-smoke`, `.codegraph/`).
  - The committed `.cargo/config.toml` sets `jobs = 4` (shared machine) and optionally includes
    the gitignored `.cargo/config.local.toml`.
  - In the master, `node tools/work-folders.mjs guard-master` writes that file with a
    `rustc-wrapper` (`.cargo/refuse-build.cmd`) that prints a refusal and exits 1. Cargo
    therefore cannot build there.
- **Guards:** `tools/lib/work-folder.mjs` (`assertWorkFolder`) refuses when the checkout path has
  a segment starting with "OneDrive", with a message naming the work folders. It is called by:
  - `tools/ci.*`, `tools/e2e.*` / `e2e.mjs`, `tools/browser-smoke.*` / `smoke.mjs`;
  - `npm run dev` and `web/scripts/build-wasm.mjs` (so also `build` and `typecheck`);
  - `npm ci`'s `prepare`;
  - both Git hooks.
- **Hooks:**
  - `npm ci` / `npm install` in `web/` runs `prepare` → `tools/hooks/install.mjs`, which sets
    `core.hooksPath=.githooks`.
  - pre-commit runs the per-file checks on the staged files (`tools/lib/run-checks.mjs`:
    rustfmt, web `tsc`, wildcard binds, LF line endings, generated reference docs for `data/`).
  - pre-push runs the fast CI subset (`tools/hooks/pre-push.mjs --list`): fmt, generated
    docs, binds, tools tests, clippy, `cargo test --workspace`, web typecheck and unit tests.
    The full `tools/ci.*` runs in win1 before `main` moves.
  - The Claude Code PostToolUse hook (`.claude/settings.json` → `tools/post-edit.mjs`):
    - finds the checkout from the **edited file**, i.e. the nearest folder above it with `.git`
      and the runner. It never uses `$CLAUDE_PROJECT_DIR`, which is the master.
    - skips files inside OneDrive;
    - runs that clone's own checks;
    - on findings, exits 2 with at most 20 lines.
  - Every check ignores `target/`, `node_modules/`, `dist/` and the WASM `pkg/`.
- **Ports:**
  - The Vite dev server prefers port 41873. It falls back to any free port, `LS_DEV_PORT`
    overrides it, and it uses `strictPort` on the chosen port and binds 127.0.0.1.
  - Tests, bots, e2e and smoke runs bind `127.0.0.1`, port 0 where possible. The bot's
    WebTransport client binds loopback for a loopback server (`loopback_bind`).
  - `tools/check-binds.mjs` (CI, pre-push, per-file) fails on `0.0.0.0`, `[::]`, `"::"`,
    `UNSPECIFIED` or `with_bind_default` outside its allowlist. The allowlist covers
    `DEFAULT_BIND` for real hosting and the dual-stack WebTransport path. This stops Windows
    from asking to allow every new hashed test binary in the firewall.
- **Memory and index:** project memory (`.claude/memory/`) is edited in win1 and reaches the
  master through `main`. Claude Code's memory directory for the master session is a junction to
  `loosescrews.win1\.claude\memory`. The master's gitignored `.claude/settings.local.json` also sets
  `autoMemoryDirectory` to it. `codegraph init` runs in win1.
- **Handoff:** `docs/handoff.md` is the committed state of every work package, the open questions
  and the next action. It is updated after every step and reaches the master through `main`.
- **Tooling:** `node tools/work-folders.mjs create <n> | guard-master [path] | status | guard <what>`.
  `status` answers "can I close this session?": uncommitted and unpushed work per winN, and
  `main` vs `WIN1`.

## Consequences

- OneDrive syncs only sources and Git objects.
- Each work folder needs its own `npm ci`, WASM build and Cargo build (several GB each).
- Parallel packages never share `target/`, `node_modules` or a dev-server port.
- `CARGO_TARGET_DIR=target/<area>` is now only needed for parallel cargo runs inside one work
  folder.
- When the repository moves to GitHub, GitHub becomes `origin` and the master becomes one more
  clone.

## Alternatives considered

- **Keep one checkout and move only build output:** `node_modules` and the reparse-point
  problems would stay in OneDrive, and agents would still share one tree.
- **Git worktrees of the master:** Git state would live in OneDrive and be written by every
  agent at once.
- **Full `tools/ci` as pre-push:** too slow per push (release bot tests, wasm32, WASM build,
  browser smoke). Decision: the fast subset; the full run gates `main` in win1.
