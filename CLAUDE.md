# Loose Screws — repo guide

6DOF browser shooter with procedurally generated, destructible worlds. Rust core + WASM + TypeScript/WebGPU client. Public repository: github.com/Nusi42/LooseScrews (code AGPL-3.0-or-later, content CC BY-NC-SA 4.0: `docs/adr/0003-licensing.md`).

- Docs index: `docs/README.md` (every doc, crate and folder map). Building, running, testing, CI, e2e, smoke test, gallery, README media, profiling, adding weapons/gadgets/rule sets: `docs/dev-guide.md`
- Design: `docs/design/game-design-doc.md` (exported from the living design doc; source of truth for game rules)
- Plan and scope: `docs/implementation-plan.md` (read the relevant module spec before working on a crate)
- Current milestone: M5 platforms and ops (draft `docs/milestone-5-plan.md`, pending decisions); previous: M4 done with exceptions (`docs/milestone-4-plan.md`), M3 (`docs/milestone-3-plan.md`); visual-quality track: `docs/visual-quality-plan.md`
- Cross-crate contracts: `docs/architecture/contracts.md`; types live in `ls_math::terrain` and `ls_math::level`. Changing them is a cross-crate change: update the doc and all users together
- Content work stays inside its module and data file: one module per weapon, gadget, mode, mutator and comedy event (`docs/architecture/arsenal-framework.md`, `modes-and-lobby.md`, `moods-and-world.md`). Registries and `ModeRules` field lists are part of the code formats: append only
- `userDocs/` is player-facing; `userDocs/reference/` is generated from `data/` (never edit by hand; `node tools/gen-weapon-docs.mjs` after data changes)
- Read `.claude/memory/next-steps.md` first (project notes, first line of `MEMORY.md`): state of every work package, open questions, next steps
- Machine-specific values (the concrete paths behind the placeholders below) live in the gitignored `CLAUDE.local.md` of the master checkout

## Where you work
- The master checkout `<master>` holds `main` and only receives pushes and `git merge --ff-only WIN1`; it pushes `main` to `<public remote>`. Never edit, build, test, npm or commit there: its role is set in the gitignored `.env.local` (`LS_CHECKOUT_ROLE=master`, `node tools/work-folders.mjs guard-master`) and the guards refuse. The maintainer's session runs in it and drives the work folders by absolute path (`cd <work folder> && ...` per command)
- Work folders: `<work root>/loosescrews.win1` (branch `WIN1`, the lead: integration, full CI, notes, CodeGraph index); `loosescrews.win2`+ (`WIN2`+, one per parallel work package). Without `.env.local` a plain clone builds, tests and commits normally. Setup, day-to-day commands, hooks and guards: `docs/dev-guide.md` "Work folders", `docs/adr/0001-master-repo-and-work-folders.md`
- Claude's post-edit hook runs the edited file's checkout's checks and reports findings (exit 2); it strips `originSessionId` lines from notes

## If you are the lead
- Resume prompt (every new session starts with it): "Continue where we left off: follow \"If you are the lead\" in CLAUDE.md and .claude/memory/next-steps.md". A session can be stopped at any moment: keep next-steps.md current after every step (what runs where, what is pending, what to do next), fold it into the work commit on WIN1 (or `chore(notes): …`) and fast-forward `main`
- On resume: `node tools/work-folders.mjs status`; a work folder with uncommitted package work belongs to an interrupted package — finish it from its diff, never `git reset` it; agents and workflows of an earlier session are gone, so restart unfinished ones from next-steps.md
- Start: read `.claude/memory/next-steps.md`; `git -C <win1> fetch`; check the `WINn` branches
- Integrate in win1: `git merge -m "Merge <what it brought>" origin/WINn`, run `pwsh tools/ci.ps1` there; only when green: `git push origin WIN1`, then `git -C <master> merge --ff-only WIN1`. `main` is always green. Push to `<public remote>` only when the maintainer says so
- CodeGraph's index lives in the win1 work folder's `.codegraph` (the master has none): pass that folder as `projectPath` to `codegraph_explore`, or run `codegraph explore` there
- Agents belong to the session that started them; a new session can't see or stop an older session's agents
- "Can I close this session?": report running background tasks, uncommitted/unpushed work per winN and `main` vs `WIN1` (`node tools/work-folders.mjs status`)

## Commits
- Identity: the public one in the local git config of every checkout (`work-folders.mjs create` copies it); the `commit-msg` hook and pre-push refuse private identities
- Messages: Conventional Commits, `<type>(<scope>): <subject>` (feat, fix, perf, refactor, test, docs, build, ci, chore, style, revert), imperative, lowercase start, first line ≤ 72 characters; body: what changed and why, in plain English. Merges: `Merge <what it brought>`. Notes-only commits: `chore(notes): …`
- The only trailers: `Co-Authored-By: Claude <model> <noreply@anthropic.com>` and `Signed-off-by:` (`git commit -s`). Never a `Claude-Session` line or a claude.ai link
- Never in a message: work-folder/branch logistics (winN, WINn, work folder, master, lead, fast-forward, branch names), process jargon (wave, work package, contract, internal milestone or package codes), the acronyms SSOT/ODR/DRY (describe the change), "owner", session talk, WIP. `tools/check-commit.mjs` enforces it ([docs/adr/0002-public-repository.md](docs/adr/0002-public-repository.md))

## Gotchas
- **No private information in git, ever** (names, e-mail addresses, the OS user name, user-home paths, names of other projects): write placeholders (`<master>`, `<work root>`, `<projects>`), `~` or use `os.homedir()`. `tools/check-private.mjs` reads the private list outside the repo (`~/.config/loose-screws/private-list.json`) and runs in pre-commit, commit-msg, pre-push and CI; never bypass it. Links to published claude.ai pages stay out of the repo ("a published page, not linked here")
- Professional voice in docs and notes: requirements, decisions, review findings; "the maintainer", never quoted casual remarks
- Licensing (`docs/adr/0003-licensing.md`): code AGPL-3.0-or-later, content (`data/`, `docs/media/`, the design doc) CC BY-NC-SA 4.0, contributions under the CLA in `CONTRIBUTING.md`; no GPL/LGPL/AGPL dependencies (`cargo deny check licenses`, `node tools/check-licenses.mjs`)
- Never kill processes you didn't start
- Shared machine: cargo runs with `-j 4`; several cargo runs in one folder set `CARGO_TARGET_DIR=target/<your-area>`
- Python on Windows writes CRLF: open files with `newline=''`; write scripts to files, not heredocs with backslashes (the shell eats `\\`)
- `git stash -u` in the master can trip on locked empty folders; don't stash there (nothing should be uncommitted there anyway)
- Automated browsers (smoke, gallery, README recordings, any puppeteer script, also throwaway ones) run headless only and inject `tools/browser-smoke/fake-pointer-lock.mjs` (`page.evaluateOnNewDocument(fakePointerLock)`): otherwise windows pop up and even headless Chrome captures the maintainer's mouse
- Tests, bots, e2e and smoke bind `127.0.0.1` (port 0 where possible); `tools/check-binds.mjs` fails on wildcard binds
- Stop any Vite dev server before CI (on Windows it locks `web/node_modules` and `npm ci` fails)

## Commands (full list and options: `docs/dev-guide.md`)
- Full check: `pwsh tools/ci.ps1` (Windows) or `tools/ci.sh`; no hosted CI, verification is local (pre-push runs the full check on pushes to the public remote)
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
- Don't commit; the orchestrator commits after each wave, with messages under "Commits"
- Versions: `0.<milestone>.<patch>` until 1.0 (workspace `Cargo.toml`, `web/package.json`); a closed milestone bumps the minor version

## Maintaining this file
Keep it to rules and pointers; documentation belongs in `docs/` (commands and options in `docs/dev-guide.md`). Update it in the same change when layout, rules or the pointers change. Concrete machine values go into `CLAUDE.local.md`, never here.
