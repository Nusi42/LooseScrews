# ADR 0001: Master checkout and work folders

Status: accepted · 2026-10-04 · revised 2026-10-17 by [ADR 0002](0002-public-repository.md)
(the master is marked by a checkout role instead of being recognised by its path)

An ADR (architecture decision record) records one decision: its context, what was decided,
the consequences and the alternatives. ADRs are numbered; a later ADR supersedes or revises an
earlier one, and the revised one says so in its status line.

## Context

The repository first lived in a single checkout inside a cloud-synced folder, and everything
ran there: Cargo's `target/`, `web/node_modules`, `web/dist`, the WASM package, test output and
the CodeGraph index. The sync client uploaded gigabytes of build output and locked files during
builds, and parallel agents shared one tree. The checkout's folders had also become cloud-file
placeholders (Windows reparse points, `fsutil reparsepoint query crates` → `0x9000e01a`), which
Git for Windows cannot check out into (`unable to unlink 'crates'`, `cannot create directory`).

The maintainer already ran another project as one main checkout plus clones named
`<project>.winN` in a separate work root; Loose Screws adopted the same layout.

## Decisions

- **Master:** `<master>`, the maintainer's main checkout, holds `main`. It only receives pushes
  and `git merge --ff-only WIN1`. Nothing is built, tested, installed, run or committed there.
- **Checkout role:** the master is marked by `LS_CHECKOUT_ROLE=master` in its gitignored
  `.env.local` (the environment variable of the same name overrides the file), written by
  `node tools/work-folders.mjs guard-master` run inside the master. No tooling looks at path
  segments. A checkout without a role, such as a plain clone, works normally (ADR 0002).
- **Work folders** are clones of the master (`origin` = `<master>`) side by side in
  `<work root>`:
  - `<work root>/loosescrews.win1`, branch `WIN1`, integrates. Integration and the full CI run
    here.
  - `loosescrews.win2` and up (`WIN2`, …) take one parallel change each. Each one is reset to
    `origin/main`, commits its change and pushes its branch; win1 merges it into `WIN1`.
  - `main` moves only after `tools/ci.*` is green in win1: `git push origin WIN1`, then
    `git -C <master> merge --ff-only WIN1`.
  - The tooling derives the folder prefix from the current checkout's name (the part before
    `.win<N>`, default `loosescrews`), the work root from its parent folder and the master from
    its `origin` remote, so folders created under an earlier prefix keep working.
- **The maintainer's Claude session** may run in the master folder. It drives the clones by
  absolute path (`cd` per command) and never edits files in the master.
- **One-time reparse fix (2026-10-04):** every folder except `.git` moved out of the master,
  then `git checkout HEAD -- .` recreated plain folders (`fsutil` → error 4390). Never
  `reset --hard`, and never write files by hand in the master.
- **Build output stays in each work folder** (`target/`, `web/node_modules`, `web/dist`,
  `web/src/wasm/pkg`, `target/e2e-logs`, `target/browser-smoke`, `.codegraph/`).
  - The committed `.cargo/config.toml` sets `jobs = 4` (shared machine) and optionally includes
    the gitignored `.cargo/config.local.toml`.
  - In the master, `guard-master` writes that file with a `rustc-wrapper`
    (`.cargo/refuse-build.cmd`) that prints a refusal and exits 1, so Cargo cannot build there.
- **Guards:** `tools/lib/work-folder.mjs` (`assertWorkFolder`) refuses in a checkout whose role
  is master, with a neutral message that points to a work folder and to docs/dev-guide.md
  "Work folders". It is called by:
  - `tools/ci.*`, `tools/e2e.*` / `e2e.mjs`, `tools/browser-smoke.*` / `smoke.mjs`;
  - `npm run dev` and `web/scripts/build-wasm.mjs` (so also `build` and `typecheck`);
  - `npm ci`'s `prepare`;
  - the pre-commit and pre-push hooks.
- **Hooks:**
  - `npm ci` / `npm install` in `web/` runs `prepare` → `tools/hooks/install.mjs`, which sets
    `core.hooksPath=.githooks`; `create` sets it as well.
  - pre-commit runs the per-file checks on the staged files (`tools/lib/run-checks.mjs`:
    rustfmt, web `tsc`, wildcard binds, LF line endings, private information, generated
    reference docs for `data/`).
  - commit-msg checks the message and the commit identity (ADR 0002).
  - pre-push checks the pushed commits (ADR 0002), then runs the fast CI subset
    (`tools/hooks/pre-push.mjs --list`): fmt, generated docs, binds, private information, tools
    tests, clippy, `cargo test --workspace`, web typecheck and unit tests. The full
    `tools/ci.*` runs in win1 before `main` moves.
  - The Claude Code PostToolUse hook (`.claude/settings.json` → `tools/post-edit.mjs`):
    - finds the checkout from the **edited file**, i.e. the nearest folder above it with `.git`
      and the runner. It never uses `$CLAUDE_PROJECT_DIR`, which may be the master;
    - skips files of a checkout whose role is master;
    - removes the `originSessionId` line from memory notes' front matter;
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
  master through `main`. The master's gitignored `.claude/settings.local.json` sets
  `autoMemoryDirectory` to win1's memory folder. `codegraph init` runs in win1.
- **Handoff:** the project memory `.claude/memory/next-steps.md` (first line of `MEMORY.md`, so
  every new session loads it) is the committed state of every open change, the open questions
  and the next action. It is updated after every step and reaches the master through `main`.
- **Tooling:** `node tools/work-folders.mjs create <n> | guard-master [path] | status | guard <what>`.
  `create` also copies the local `user.name` / `user.email` into the new clone and sets
  `core.hooksPath`. `status` lists the sibling work folders and the master: uncommitted and
  unpushed work per folder, and `main` vs `WIN1`.

## Consequences

- Cloud sync, if the master lives in a synced folder, sees only sources and Git objects.
- Each work folder needs its own `npm ci`, WASM build and Cargo build (several GB each).
- Parallel changes never share `target/`, `node_modules` or a dev-server port.
- `CARGO_TARGET_DIR=target/<area>` is only needed for parallel cargo runs inside one work
  folder.
- With the public GitHub repository, GitHub becomes the place `main` is published; the master
  stays the integration target of the work folders.

## Alternatives considered

- **Keep one checkout and move only build output:** `node_modules` and the reparse-point
  problems would stay in the synced folder, and agents would still share one tree.
- **Git worktrees of the master:** Git state would live in the synced folder and be written by
  every agent at once.
- **Full `tools/ci` as pre-push:** too slow per push (release bot tests, wasm32, WASM build,
  browser smoke). The maintainer chose the fast subset; the full run gates `main` in win1.
- **Recognise the master by its path** (the original decision: a path segment naming the sync
  folder): replaced by the explicit role in ADR 0002, because path rules break for other
  machines and contributors and embedded the maintainer's folder layout in the tooling.
