# Loose Screws — repo guide

6DOF browser shooter with procedurally generated, destructible worlds. Rust core + WASM + TypeScript/WebGPU client.

- Docs index: `docs/README.md` (every doc, crate and folder map). Building, running, testing, CI, e2e, smoke test, gallery, profiling, adding weapons/gadgets/rule sets: `docs/dev-guide.md`
- Design: `docs/design/game-design-doc.md` (exported from the living doc; source of truth for game rules)
- Plan and scope: `docs/implementation-plan.md` (read the relevant module spec before working on a crate)
- Current milestone: M5 platforms and ops (draft `docs/milestone-5-plan.md`, awaiting the owner); previous: M4 done with exceptions (`docs/milestone-4-plan.md`), M3 (`docs/milestone-3-plan.md`); visual-quality track: `docs/visual-quality-plan.md`
- Cross-crate contracts: `docs/architecture/contracts.md`; types live in `ls_math::terrain` and `ls_math::level`. Changing them is a cross-crate change: update the doc and all users together
- Content work stays inside its module and data file: one module per weapon, gadget, mode, mutator and comedy event (`docs/architecture/arsenal-framework.md`, `modes-and-lobby.md`, `moods-and-world.md`). Registries and `ModeRules` field lists are part of the code formats: append only
- `userDocs/` is player-facing; `userDocs/reference/` is generated from `data/` (never edit by hand; `node tools/gen-weapon-docs.mjs` after data changes)
- Read `.claude/memory/next-steps.md` first (project memory, first line of `MEMORY.md`): state of every work package, open questions, next steps

## Where you work
- The master repo `<master>` holds `main` and only receives pushes and `git merge --ff-only WIN1`. Never edit, build, test, npm or commit there (the guards refuse). The maintainer's session runs in it and drives the clones by absolute path (`cd <clone> && ...` per command)
- Work folders: `<work root>/loosescrews.win1` (branch `WIN1`, the lead: integration, full CI, memory, CodeGraph index); `loosescrews.win2`+ (`WIN2`+, one per parallel work package). Setup, day-to-day commands, hooks and guards: `docs/dev-guide.md` "Work folders"
- Claude's post-edit hook runs the edited file's clone's checks and reports findings (exit 2)

## If you are the lead
- Resume prompt (the owner starts every new session with it): "Continue where we left off: follow \"If you are the lead\" in CLAUDE.md and .claude/memory/next-steps.md". A session can be stopped at any moment: keep next-steps.md current after every step (what runs where, what is pending from the owner, what to do next), commit it on WIN1 and fast-forward `main`
- On resume: `node tools/work-folders.mjs status`; a work folder with uncommitted package work belongs to an interrupted package — finish it from its diff, never `git reset` it; agents and workflows of an earlier session are gone, so restart unfinished ones from next-steps.md
- Start: read `.claude/memory/next-steps.md`; `git -C <win1> fetch`; check the `WINn` branches
- Integrate in win1: merge `origin/WINn` into `WIN1`, run `pwsh tools/ci.ps1` there; only when green: `git push origin WIN1`, then `git -C <master> merge --ff-only WIN1`. `main` is always green
- CodeGraph's index lives in `loosescrews.win1/.codegraph` (the master has none): pass `projectPath: <work root>/loosescrews.win1` to `codegraph_explore`, or run `codegraph explore` there
- Agents belong to the session that started them; a new session can't see or stop an older session's agents
- "Can I close this session?": report running background tasks, uncommitted/unpushed work per winN and `main` vs `WIN1` (`node tools/work-folders.mjs status`)

## Gotchas
- **No private information in git, ever** (email, OS user name, user-home paths, names): write `%USERPROFILE%`, `~` or use `os.homedir()`. `tools/check-private.mjs` enforces it; never bypass it
- Never kill processes you didn't start
- Shared machine: cargo runs with `-j 4`; several cargo runs in one folder set `CARGO_TARGET_DIR=target/<your-area>`
- Python on Windows writes CRLF: open files with `newline=''`; write scripts to files, not heredocs with backslashes (the shell eats `\\`)
- `git stash -u` in the master can trip on locked empty folders; don't stash there (nothing should be uncommitted there anyway)
- Automated browsers (smoke, gallery, any puppeteer script, also throwaway ones) run headless only and inject `tools/browser-smoke/fake-pointer-lock.mjs` (`page.evaluateOnNewDocument(fakePointerLock)`): otherwise windows pop up and even headless Chrome captures the maintainer's mouse
- Tests, bots, e2e and smoke bind `127.0.0.1` (port 0 where possible); `tools/check-binds.mjs` fails on wildcard binds
- Stop any Vite dev server before CI (on Windows it locks `web/node_modules` and `npm ci` fails)

## Commands (full list and options: `docs/dev-guide.md`)
- Full check: `pwsh tools/ci.ps1` (Windows) or `tools/ci.sh`
- `cargo test --workspace` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo fmt --all`; slow suites only in release: `cargo test -p ls-bots --release`, `cargo test -p ls-game --release`
- Web: `cd web && npm install && npm run typecheck && npm test && npm run build`; web unit tests run under `node --test` with type stripping: `.ts` import extensions, erasable-only syntax (no enums/parameter properties)
- Determinism: after a generator, sim behavior, tuning or `data/` change, re-bless as in `docs/architecture/determinism.md` "Re-blessing" (generator version, golden hashes, replay fixtures, `SIM_BEHAVIOR_VERSION`); `node tools/determinism-check.mjs` compares WASM with native
- After data changes touching weapon names, slots, costs, levels or the default rule set: `LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode`
- wasm-bindgen CLI 0.2.129 with crates pinned to match (bump all together; dev-guide "Toolchain")

## Determinism (ls-math, ls-worldgen, ls-sim)
Rules in `docs/architecture/determinism.md`: IEEE-exact float ops only, transcendentals via `ls_math::det`, no `mul_add`/fast-math, no HashMap iteration in outputs, no wall clock, threads or OS RNG; keep golden hashes passing.

## Conventions
- **SSOT, ODR, DRY are mandatory** for all code, existing and new (`docs/code-quality.md`); every package and review checks them
- Tests next to code; every module spec in the plan has acceptance criteria — meet them
- Don't commit; the orchestrator commits after each wave

## Maintaining this file
Keep it to rules and pointers; documentation belongs in `docs/` (commands and options in `docs/dev-guide.md`). Update it in the same change when layout, rules or the pointers change.
