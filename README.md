# Loose Screws

A 6DOF browser shooter in procedurally generated, destructible worlds. Palm-sized junk
ships fly through levels built from a short share code; every crater and paint splash is
part of the game. Rust core (simulation, world generation, server), compiled to WASM for
the TypeScript + WebGPU browser client.

Status: Milestone 1 (vertical slice) in progress.

## Quick start

Requirements: stable Rust with the `wasm32-unknown-unknown` target (`rust-toolchain.toml`
installs it), `wasm-bindgen` CLI 0.2.126, Node.js with npm.

```sh
cargo test --workspace                                        # Rust tests
cargo build -p ls-wasm --target wasm32-unknown-unknown --release
cd web && npm install && npm run dev                          # client on http://localhost:5173
```

Full check (fmt, clippy, tests native + wasm32, WASM build, web build):
`pwsh tools/ci.ps1` on Windows or `tools/ci.sh` elsewhere.

## Layout

| Path | Contents |
| --- | --- |
| `crates/` | `ls-math`, `ls-worldgen`, `ls-sim`, `ls-net`, `ls-server`, `ls-wasm`, `ls-bots` |
| `data/` | Weapon and rule set definitions (RON) |
| `web/` | Browser client (TypeScript, Vite, WebGPU) |
| `tools/` | CI and helper scripts |

## Documentation

- [docs/](docs/) — developer docs: [design doc](docs/design/game-design-doc.md),
  [implementation plan](docs/implementation-plan.md),
  [shared contracts](docs/architecture/contracts.md)
- [userDocs/](userDocs/) — player manual and hosting guides
- [CLAUDE.md](CLAUDE.md) — rules and commands for AI agents working on this repo
