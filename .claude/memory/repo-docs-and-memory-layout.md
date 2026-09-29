---
name: repo-docs-and-memory-layout
description: Where docs, user manuals, CLAUDE.md and the project notes live in this repo
metadata:
  type: feedback
---
Developer docs go in `docs/`, player-facing docs and manuals in `userDocs/`. `CLAUDE.md` at the repo root is kept current whenever layout, commands or conventions change; machine-specific values go into the gitignored `CLAUDE.local.md`. The project notes live in the repo at `.claude/memory/` (edited in the lead work folder `<work root>/loosescrews.win1`; the harness memory folder points there, and changes reach the master checkout through `main`), never in temp or user folders.
**Why:** requirement: everything about the project, including the notes, is versioned in the repo; the notes are public, so they hold only rules, preferences and the current state, not a diary.
**How to apply:** update CLAUDE.md in the same change that alters structure or commands; write notes only under `.claude/memory/`; remove notes about finished work, move lasting pitfalls into `docs/dev-guide.md`. See [[orchestration-preferences]].
