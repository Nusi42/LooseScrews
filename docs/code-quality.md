# Code quality rules: SSOT, ODR, DRY

Mandatory for all code, existing and new (decision). Every package, every
integration and every review checks them. A violation is a defect like a failing test: fix it in
the same change, or record it with a reason in the milestone plan's open items.

## The three rules

**SSOT — single source of truth.** Every fact lives in exactly one place; everything else reads
or is generated from it.
- Gameplay numbers live in `data/` (RON), not in code. Code holds only structure and limits that
  are part of a contract.
- A value needed in Rust and in TypeScript is defined once and **generated** for the other side
  (as `web/src/rules/catalog.ts` and the `userDocs/reference/` pages are), or exported through
  `ls-wasm`. Never copy a Rust constant, enum order, bit layout or table into TypeScript by hand.
- Wire layouts, flag bits, cue numbers, entity kinds and mode ids have one owning definition
  (`ls-net` for the wire, `ls-sim` for sim enums); docs describe them and point to that
  definition instead of restating values that can drift.
- Docs state a number only when they are its source; otherwise they name the file it lives in.
  Generated docs (`gen-weapon-docs`) instead of hand-copied tables.

**ODR — one definition rule.** Every type, function, constant, shader routine and command has one
definition.
- No two commands, entity kinds, messages or helpers doing the same job (e.g. two "move this
  entity" commands, two boulder pose functions). When a second one appears, fold it into the
  first.
- Shared WGSL lives in one shader module and is included, not pasted into several shaders.
- One registry per concept (weapons, gadgets, modes, mutators, events, mesh kinds, sounds).

**DRY — don't repeat yourself.** The same logic is written once.
- Copy-pasted blocks (more than a few lines, or the same formula twice) become a function, a
  generic, a macro or a data table.
- Tests share fixtures and helpers (`test_util` modules) instead of rebuilding worlds by hand.
- Repetition across modes, events, weapons or UI panels goes into the shared framework hook or
  component, not into each instance.
- Allowed exceptions, each with a one-line comment saying why: deliberate duplication across a
  determinism boundary (e.g. a sim formula mirrored in a test as an independent oracle), and
  generated code.

## Review checklist

Every review (package self-check, integrator, wave D reviews) answers these for the change:
1. Does any new constant, table, enum order, bit layout or number duplicate one that already
   exists in Rust, TypeScript, WGSL, data or docs? (Search with `codegraph explore` and grep.)
2. Does any new type, command, message, helper or shader function do the same job as an existing
   one?
3. Is any block of logic or test setup written twice?
4. Is every cross-language value generated or exported, not hand-copied?
5. Do docs restate a value that lives elsewhere?

A "yes" is fixed before merging, or listed with its reason.

## Enforcement

- `CLAUDE.md` names this file under Conventions; agent prompts for packages and reviews require it.
- Generated artifacts have `--check` modes in CI (rules catalog, reference pages); every new
  generated file gets one.
- The existing codebase was audited against these rules; findings and their fixes
  are tracked in `docs/code-quality-audit.md`.
