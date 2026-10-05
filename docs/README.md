# Loose Screws developer documentation

Developer-facing docs for the Rust workspace, the browser client and the tooling.
Player manuals and hosting guides live in [`userDocs/`](../userDocs/); rules for AI agents
working on the repo are in [`CLAUDE.md`](../CLAUDE.md).

## Start here

| Document | Read it for |
| --- | --- |
| [dev-guide.md](dev-guide.md) | Toolchain, building, running server + client + bots, tests, CI, e2e, profiling; adding a [weapon](dev-guide.md#8-adding-a-weapon) (data, hooks, commands, registry, tests, client, bots), a [rule set](dev-guide.md#9-adding-a-rule-set) ([codes and the editor](dev-guide.md#rule-set-codes-and-the-editor), [balance lists](dev-guide.md#balance-lists)), a [gadget](dev-guide.md#10-adding-a-gadget); [ship types](dev-guide.md#11-ship-types); the [browser smoke test](dev-guide.md#12-browser-smoke-test) and the [visual gallery](dev-guide.md#visual-gallery-and-gpu-budget); the [generated player reference](dev-guide.md#generated-player-reference); [work folders](dev-guide.md#work-folders) (master repo, clones, hooks, guards) |
| [adr/](adr/) | Architecture decision records: [0001 master repo and work folders](adr/0001-master-repo-and-work-folders.md) |
| [implementation-plan.md](implementation-plan.md) | Milestone 1 scope, module specs with acceptance criteria, work waves and current status, later milestones |
| [milestone-2-plan.md](milestone-2-plan.md) | Milestone 2 (arsenal and physics): scope, design decisions (numbers the design doc leaves open), waves and their status, acceptance criteria with evidence |
| [milestone-3-plan.md](milestone-3-plan.md) | Milestone 3 (modes and lobby): scope incl. carried-over items, design decisions, waves and packages with their "as built" notes, determinism and handshake impacts, acceptance criteria |
| [milestone-4-plan.md](milestone-4-plan.md) | Milestone 4 (the level has moods; draft, pending review): comedy director and the 18 events, the remaining sliders, monochrome levels and pigment, The Architect, world workshop, audio; carried-over items, proposals for design gaps, determinism and handshake impacts, waves and packages, acceptance criteria, open questions |
| [design/game-design-doc.md](design/game-design-doc.md) | The game: rules, ships, weapons, modes (exported from the living design doc; source of truth for game rules) |

## Architecture

| Document | Contents |
| --- | --- |
| [architecture/overview.md](architecture/overview.md) | Crates and their responsibilities, runtime components (diagram), data flow from share code to rendered frame, threads |
| [architecture/netcode.md](architecture/netcode.md) | Tick model, input redundancy, prediction and reconciliation (ship and shots), interpolation, snapshots and deltas (ship, projectile and entity deltas, settling, bandwidth budget), per-client relevance (Cloak, far debris), events and cues, protocol 5 (mode state, match transitions with epochs, lobby and results messages, room and party codes), terrain edit replication, lag compensation, WebSocket and WebTransport |
| [architecture/modes-and-lobby.md](architecture/modes-and-lobby.md) | M3: the mode framework (hooks, mode state, teams, lives, spectators, every mode and the Instagib mutator as built, adding a mode), the lobby state machine and match rotation, rooms, identity, parties, invite links, duels, the directory, the browser side, tests, known gaps |
| [architecture/determinism.md](architecture/determinism.md) | Determinism rules (incl. weapon and gadget modules), golden hashes, the data hash and `SIM_BEHAVIOR_VERSION`, re-blessing, the WASM cross-check |
| [architecture/contracts.md](architecture/contracts.md) | Cross-crate contracts: axes, units, time, RNG streams, `TerrainEdit` / `TerrainQuery`, surface states and paint colours, rock strength, chunk meshes and layers, share codes, quantization |
| [architecture/game-glue.md](architecture/game-glue.md) | `ls-game` in detail: `GameTerrain`, match setup, wire conventions, `ClientSession`, `Room`, lag compensation, tests |
| [architecture/arsenal-framework.md](architecture/arsenal-framework.md) | M2 arsenal framework: weapon and gadget modules, their hooks and `Ctx` commands, terrain layers for weapons, hook order per tick, gadgets, ship types and paint effects, entities and debris, replication (protocol 4 wire items), data hash, rule set v2, codes (format 4 with every mode) and balance lists, and the arsenal as built (every weapon's bonus, malus and lasting effect) |
| [architecture/directory.md](architecture/directory.md) | `ls-directory`: the optional server list service (flags, JSON API, heartbeat and secret rules, invite relay, the client library `ls-server` uses) |
| [architecture/audio.md](architecture/audio.md) | M4: the browser's sound: synthesized sounds, the WebAudio engine (gesture unlock, mixer, voice cap, positional voices, loops), game sounds per cue and shape, the announcer (voice packs, triggers, gibberish voice), `?audio=demo`, adding sounds |
| [architecture/renderer.md](architecture/renderer.md) | Browser renderer: HDR frame (scene, bloom, composite), colour units and emissive scale, post settings and the colour-grade hook, quality presets, GPU pass timing, shader files |
| [architecture/server.md](architecture/server.md) | `ls-server` internals (endpoints, links, WebTransport mapping, rooms, lobby and rotation, the JSON API, parties, duels and the directory client, sim thread policies, CLI incl. rule sets, playlists, bans and balance lists), browser client lifecycle (match transitions, invites, server list, mode HUD) and data-driven drawing, `ls-bots` (navigation and its cache, arsenal use, mode objectives, duels), the e2e run |

## Crate and folder map

| Path | Docs |
| --- | --- |
| `crates/ls-math` | [contracts.md](architecture/contracts.md), [determinism.md](architecture/determinism.md) |
| `crates/ls-worldgen` | [contracts.md](architecture/contracts.md), [determinism.md](architecture/determinism.md), plan section 4.2 |
| `crates/ls-sim`, `data/` | plan section 4.3, [arsenal-framework.md](architecture/arsenal-framework.md), [modes-and-lobby.md](architecture/modes-and-lobby.md), [`data/README.md`](../data/README.md), [dev-guide.md](dev-guide.md#8-adding-a-weapon) (weapons, rule sets, gadgets, ship types) |
| `crates/ls-net` | [netcode.md](architecture/netcode.md), [arsenal-framework.md](architecture/arsenal-framework.md#replication-ls-net-protocol-4), plan section 4.4 |
| `crates/ls-game` | [game-glue.md](architecture/game-glue.md), [netcode.md](architecture/netcode.md) |
| `crates/ls-server`, `crates/ls-bots`, `tools/` | [server.md](architecture/server.md), [modes-and-lobby.md](architecture/modes-and-lobby.md), [dev-guide.md](dev-guide.md) (CI, e2e, [browser smoke test](dev-guide.md#12-browser-smoke-test)) |
| `crates/ls-directory` | [directory.md](architecture/directory.md) |
| `crates/ls-wasm`, `web/` | [overview.md](architecture/overview.md), [renderer.md](architecture/renderer.md), [audio.md](architecture/audio.md), [game-glue.md](architecture/game-glue.md#browser-core-extras-ls-wasm), [server.md](architecture/server.md#browser-client-lifecycle-websrcmaints), rule set editor in [dev-guide.md](dev-guide.md#rule-set-codes-and-the-editor) |

Crate-level rustdoc (`//!` at the top of each `lib.rs`) summarises every crate's modules;
`cargo doc --workspace --no-deps --open` renders it.

## Keeping docs current

- Update the page that owns a topic instead of repeating it elsewhere; link to it.
- Changing a cross-crate contract means updating [contracts.md](architecture/contracts.md) in the
  same change; changing commands or layout means updating [`CLAUDE.md`](../CLAUDE.md) and
  [dev-guide.md](dev-guide.md).
- `docs/design/` is an export of the living design doc; change the living doc, then re-export.
- `userDocs/reference/*.md` are generated from `data/` by `node tools/gen-weapon-docs.mjs`
  ([dev-guide](dev-guide.md#generated-player-reference)); CI fails while they are stale.
  Player-facing prose (the manual, the hosting guide) is written by hand in `userDocs/`;
  developer detail stays here.
