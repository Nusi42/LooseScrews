---
name: disk-hygiene
description: Build output of parallel packages must not pile up — packages build into target/pkg, folders are cleaned after each green wave
metadata:
  type: feedback
---
Package agents build into `CARGO_TARGET_DIR=target/pkg` in their work folder (the same name every wave, so the next wave reuses it), never a per-package name. After a wave is integrated, green and on `main`, delete `target/` in the package folders (win2+); in win1 keep `target/debug` and `target/release` (the full check reuses them) and remove other subfolders. Keep `.codegraph/`, `.claude/memory/` and `node_modules` (git hooks).
**Why:** per-package target folders once grew to about 358 GB across eleven folders and nearly filled the disk.
**How to apply:** put `CARGO_TARGET_DIR=target/pkg` into every package prompt; run the cleanup as the last step of an integration. Details: `docs/dev-guide.md` "Work folders". See [[orchestration-preferences]], [[next-steps]].
