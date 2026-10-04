# Handoff

The state a fresh lead session needs. Update it after every step, commit it in win1 and
fast-forward `main`, so the master's copy is current. Run
`node tools/work-folders.mjs status` from any work folder for the live picture.

Status: current

## Work packages

| Folder | Branch | Package | State |
| --- | --- | --- | --- |
| loosescrews.win1 | WIN1 | Work-folder setup (docs/adr/0001) | done: merged to main |
| loosescrews.win2 | WIN2 | none (verified a fresh clone passes pre-push) | idle, at main |

## Open questions

- None.

## Maintainer to-dos

- Move the master's untracked `.codegraph/` to `<work root>\colent.master-backup`
  once no Claude session runs in the master (its CodeGraph MCP server holds it open). The index
  now lives in loosescrews.win1.

## Next action

- Pick up the next milestone items from `docs/implementation-plan.md` and
  `docs/milestone-2-plan.md` (open M1/M2 items were scheduled into later milestones in f3efd55).
