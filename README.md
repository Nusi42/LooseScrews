# Loose Screws

A 6DOF browser shooter in procedurally generated, destructible worlds. Palm-sized junk
ships fly through levels built from a short share code; every crater and paint splash is
part of the game. Rust core (simulation, world generation, server), compiled to WASM for
the TypeScript + WebGPU browser client.

Status: Milestone 1 (vertical slice) — playable end to end: Paint the Void for up to 8
players over WebTransport or WebSocket, with headless bots. See
[docs/implementation-plan.md](docs/implementation-plan.md) for what remains.

## Quick start

Requirements: stable Rust with the `wasm32-unknown-unknown` target (`rust-toolchain.toml`
installs it), `wasm-bindgen` CLI 0.2.126, Node.js with npm, and a WebGPU browser (Chrome or Edge).

```sh
cd web && npm install && npm run build && cd ..               # builds the WASM package and web/dist
cargo run -p ls-server --release -- --web-root web/dist       # open http://127.0.0.1:7777
cargo run -p ls-bots --release -- --server ws://127.0.0.1:7777/ws --count 3   # optional opponents
```

For client development with hot reload use `cd web && npm run dev` (http://localhost:5173).
Full check (fmt, clippy, tests native + wasm32, WASM build, web build, determinism check):
`pwsh tools/ci.ps1` on Windows or `tools/ci.sh` elsewhere. End-to-end with bots: `tools/e2e.ps1` / `tools/e2e.sh`.

## Layout

| Path | Contents |
| --- | --- |
| `crates/` | `ls-math`, `ls-worldgen`, `ls-sim`, `ls-net`, `ls-game` (shared client/server glue), `ls-server`, `ls-wasm`, `ls-bots` |
| `data/` | Ship, weapon and rule set definitions (RON) |
| `web/` | Browser client (TypeScript, Vite, WebGPU) |
| `tools/` | CI, determinism check and end-to-end scripts |
| `docs/` | Developer documentation |
| `userDocs/` | Player manual and hosting guide |

## Documentation

- [docs/README.md](docs/README.md) — developer docs index: design doc, implementation plan,
  architecture, netcode, determinism, dev guide
- [userDocs/README.md](userDocs/README.md) — player manual and hosting guide
- [CLAUDE.md](CLAUDE.md) — rules and commands for AI agents working on this repo
