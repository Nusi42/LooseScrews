# Loose Screws developer documentation

Developer-facing docs for the Rust workspace, the browser client and the tooling.
Player manuals and hosting guides live in [`userDocs/`](../userDocs/); rules for AI agents
working on the repo are in [`CLAUDE.md`](../CLAUDE.md).

## Start here

| Document | Read it for |
| --- | --- |
| [dev-guide.md](dev-guide.md) | Toolchain, building, running server + client + bots, tests, CI, e2e, profiling, adding a weapon or a rule set |
| [implementation-plan.md](implementation-plan.md) | Milestone 1 scope, module specs with acceptance criteria, work waves and current status, later milestones |
| [milestone-2-plan.md](milestone-2-plan.md) | Milestone 2 (arsenal and physics): scope, design decisions, waves, acceptance criteria |
| [design/game-design-doc.md](design/game-design-doc.md) | The game: rules, ships, weapons, modes (exported from the living design doc; source of truth for game rules) |

## Architecture

| Document | Contents |
| --- | --- |
| [architecture/overview.md](architecture/overview.md) | Crates and their responsibilities, runtime components (diagram), data flow from share code to rendered frame, threads |
| [architecture/netcode.md](architecture/netcode.md) | Tick model, input redundancy, prediction and reconciliation, interpolation, snapshots and deltas, terrain edit replication, lag compensation, WebSocket and WebTransport |
| [architecture/determinism.md](architecture/determinism.md) | Determinism rules, golden hashes, re-blessing, the WASM cross-check |
| [architecture/contracts.md](architecture/contracts.md) | Cross-crate contracts: axes, units, time, RNG streams, `TerrainEdit` / `TerrainQuery`, chunk meshes, share codes, quantization |
| [architecture/game-glue.md](architecture/game-glue.md) | `ls-game` in detail: `GameTerrain`, match setup, wire conventions, `ClientSession`, `Room`, lag compensation, tests |
| [architecture/arsenal-framework.md](architecture/arsenal-framework.md) | M2 arsenal framework: weapon and gadget modules and their hooks, hook order per tick, ship types, entities and debris, generic entity replication, data hash, rule set v2 and codes |
| [architecture/server.md](architecture/server.md) | `ls-server` internals (endpoints, links, WebTransport mapping, sim thread policies, CLI), browser client lifecycle, `ls-bots` |

## Crate and folder map

| Path | Docs |
| --- | --- |
| `crates/ls-math` | [contracts.md](architecture/contracts.md), [determinism.md](architecture/determinism.md) |
| `crates/ls-worldgen` | [contracts.md](architecture/contracts.md), [determinism.md](architecture/determinism.md), plan section 4.2 |
| `crates/ls-sim`, `data/` | plan section 4.3, [arsenal-framework.md](architecture/arsenal-framework.md), [`data/README.md`](../data/README.md), [dev-guide.md](dev-guide.md#8-adding-a-weapon) |
| `crates/ls-net` | [netcode.md](architecture/netcode.md), plan section 4.4 |
| `crates/ls-game` | [game-glue.md](architecture/game-glue.md), [netcode.md](architecture/netcode.md) |
| `crates/ls-server`, `crates/ls-bots`, `tools/` | [server.md](architecture/server.md), [dev-guide.md](dev-guide.md) |
| `crates/ls-wasm`, `web/` | [overview.md](architecture/overview.md), [game-glue.md](architecture/game-glue.md#browser-core-extras-ls-wasm), [server.md](architecture/server.md#browser-client-lifecycle-websrcmaints) |

Crate-level rustdoc (`//!` at the top of each `lib.rs`) summarises every crate's modules;
`cargo doc --workspace --no-deps --open` renders it.

## Keeping docs current

- Update the page that owns a topic instead of repeating it elsewhere; link to it.
- Changing a cross-crate contract means updating [contracts.md](architecture/contracts.md) in the
  same change; changing commands or layout means updating [`CLAUDE.md`](../CLAUDE.md) and
  [dev-guide.md](dev-guide.md).
- `docs/design/` is an export of the living design doc; change the living doc, then re-export.
