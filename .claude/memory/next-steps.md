---
name: next-steps
description: Where the project stands and what to do next — read first when continuing in a new session
metadata:
  type: project
---

**Where to work.** The master checkout `<master>` holds `main` and receives pushes and `git merge --ff-only WIN1`; it pushes `main` to `<public remote>` only when the maintainer says so. All edits, builds, tests and commits happen in the work folders `<work root>/loosescrews.win<N>`: win1 (`WIN1`) is the lead (integration, full check, notes, CodeGraph), win2 and up take one work package each. Live picture: `node tools/work-folders.mjs status`. See [[repo-docs-and-memory-layout]], [[orchestration-preferences]].

**Resume checklist:** 1) `node tools/work-folders.mjs status` from win1; 2) a winN with uncommitted changes is an interrupted package: finish it from its diff, never reset it; 3) agents of an earlier session are gone: restart unfinished ones from this file; 4) continue with "Next steps".

**Current state**
- Milestones 1–4 are closed; milestone 4 is done with exceptions (`docs/milestone-4-plan.md`, milestone summary): visual and listening review pending; 6 bot input-starvation warnings in the soak (bots stall 57–250 ms per think turn during Architect matches; cause in ls-bots not found); a 1.2–2 u rock blade in `cave-sunbeam` still shows steps; no cave-in remesh benchmark; mid-range GPU timings estimated.
- Accepted but not built: kill-cam after your own death and the grumpy mood's extra cave-ins; both are milestone 5 wave A items (package A1 and A2).
- Code-quality audit: `docs/code-quality-audit.md` (106 findings, 8 fix packages E, A, B, C, D, G, H, I); all 10 questions decided and folded in (section 5): the rule set editor requires WASM, Belt Up race progress exported from ls-wasm (new finding A15), data-hash change accepted, mode tuning into `data/modes/` with today's host settings unchanged, per-mode goldens into the fixture, manual numbers from generator placeholders, hosting-guide flag table generated, charged drain into a data file, decoys unchanged, one wave before milestone 5.
- Milestone 5 (platforms and ops): `docs/milestone-5-plan.md` decided (§8) and ready to start after the audit fix wave. Windows x64 Tauri app over QUIC datagrams with the WASM core, unsigned tester builds; Nomad deploy artifacts tested on a local dev agent plus the local fleet simulation, no real cloud deployment; WebGPU only. Dropped: WebRTC fallback, WebGL2 renderer (packages A5, A7, B1, C1, C3), rented reference GPU, signing and updater (the maintainer handles hosting, signing and the reference GPU separately).
- Public repository github.com/Nusi42/LooseScrews: prepared (`docs/adr/0002-public-repository.md`, `0003-licensing.md`, `0004-local-verification.md`); code AGPL-3.0-or-later, content CC BY-NC-SA 4.0, CLA in `CONTRIBUTING.md` (nothing commercial before a lawyer has reviewed it); no hosted CI (Actions are off for the repository). The history was rewritten once and `main` pushed (still a private repository); the old history is kept as bundles in the maintainer's config folder until the maintainer says they can go. Publishing: only from the master, `git -C <master> push github main`.
- The pre-push fast subset was flaky under load (failed three times on doc-only commits, passed on retry); keep failed step logs to find the test.

**Next steps, in order**
1. Write the follow-up prompt for the sibling project so that it, too, publishes only from its master.
2. Run the 8 audit fix packages as one wave (`docs/code-quality-audit.md` §4, merge order E, A, B, C, D, G, H, I), integrate, full check, fast-forward `main`.
3. Then milestone 5 wave A (`docs/milestone-5-plan.md` §5: A3, A2, A4, A9, A6, A8, A1).
4. Maintainer: review comments, audio check (`?audio=demo`).

**Maintainer to-do:** move the master's untracked `.codegraph/` out of the master once no session runs there (its CodeGraph server holds it open).

**Why:** a new session must know the state without re-deriving it; this file survives the session.
**How to apply:** read this first; update it after every finished step and before the maintainer closes a session; fold it into the work commit on WIN1 (or `chore(notes): …`) and fast-forward `main`. Keep it to the current state: no diary, no finished-work history.
