---
name: orchestration-preferences
description: How implementation work is run — orchestrated Opus 5.5 agents at medium effort, in waves
metadata:
  type: feedback
---
Implementation is orchestrated through Opus 5.5 subagents at medium effort, in waves defined in the milestone plan (one plan file per milestone). The orchestrator commits after each wave once builds and tests pass.
**Why:** decision of the maintainer.
**How to apply:** use workflows or agents with `effort: 'medium'`; keep agents on disjoint directories per wave; commit between waves with messages under CLAUDE.md "Commits". Related: [[repo-docs-and-memory-layout]], [[disk-hygiene]].
