# Server, bots and end-to-end checks

`crates/ls-server` hosts the authoritative `ls_game::server::Room` (see
[game-glue.md](game-glue.md)); `crates/ls-bots` are headless clients built on
`ls_game::client::ClientSession`; `tools/e2e.*` runs both together. How to run them locally:
[dev-guide.md](../dev-guide.md); transport and netcode overview: [netcode.md](netcode.md).

## ls-server

```
tokio I/O tasks                                  ls-sim thread (60 Hz)
  HttpEndpoint (TCP): static files, /ws, /cert-hash
  WebTransportEndpoint (UDP): /wt  ---Join{hello, SimLink}-->  Sim: Room + clients
  ws / wt task per connection  <--PeerLink | SimLink-->          receive -> step -> dispatch
```

Endpoints on the `--bind` address (TCP) and the `--wt-bind` address (UDP, same port by default):

| Endpoint | What |
| --- | --- |
| `GET /`, static files | the web client from `--web-root` |
| `GET /ws` (WebSocket) | one binary message = one encoded `ls_net::Message`, both directions; unreliable messages travel reliably |
| `GET /cert-hash` | WebTransport certificate as JSON `{"algorithm":"sha-256","hash":"<hex>","port":N,"path":"/wt","expires":<unix s>}` with `Access-Control-Allow-Origin: *`; 404 with `--no-webtransport` |
| `https://host:port/wt` (WebTransport, UDP) | see below; `/` is accepted too |

- `link.rs`: `link_pair` creates the two halves of a connection. `SimLink` (sim thread)
  implements `ls_net::Transport` and never blocks; `PeerLink` (socket task) awaits outgoing
  frames. At most 8 unreliable frames wait (about 260 ms of snapshots plus pongs; older ones are
  dropped), reliable frames are never dropped but more than 16 MiB queued closes the link as too
  slow. Incoming packets use a bounded channel (256), so a flooding client stalls only its own
  socket.
- `timing.rs`: deadline scheduler, bursts of up to 4 ticks to catch up, at most 250 ms backlog.
- `endpoint.rs`: the transport seam (`Endpoint`, `Hub`, `CertHash`); `ws.rs` (WebSocket) and
  `wt.rs` (WebTransport) plug in here. Handshake and idle timeouts (`ConnSettings`, 10 s idle),
  shutdown grace and the timeout counters are the same for both. Incoming packets are tagged
  with the channel they arrived on; `PeerLink::try_push_datagram` drops a datagram instead of
  waiting when the sim's queue is full.

### WebTransport (`wt.rs`, `wtransport` 0.7 with the `ring` backend)

- Wire mapping: the client opens one bidirectional stream. Reliable packets are frames on it: a
  4-byte big-endian length, then the encoded message. The first client frame must be the
  `Hello`; `Welcome` / `Reject` come back on the same stream. Unreliable packets (inputs, pings,
  snapshots, pongs) are datagrams, one message each. The link's datagram limit is the session's
  `max_datagram_size` at accept time, so larger unreliable packets use the existing reliable
  fallback; a datagram the connection still refuses also goes on the stream.
- On a server-side close the stream is flushed (the final `Disconnect` arrives) before the
  connection closes.
- Certificate: self-signed ECDSA P-256, generated at startup, valid from 1 h ago for 13 days
  (browsers accept at most 14 for `serverCertificateHashes`). Every 6 days a new one replaces
  it for new sessions and `/cert-hash` follows; running sessions are unaffected. Clients fetch
  the hash right before connecting.
- `--wt-bind` defaults to the `--bind` address (same port number over UDP). For `0.0.0.0` the
  UDP socket is dual-stack (browsers may reach `localhost` over `::1`). `--no-webtransport`
  turns the endpoint off; the library default (`ServerConfig::webtransport = None`) is off.
- WebTransport needs a secure context in the browser: `localhost` works, a page opened as
  `http://<LAN-IP>:7777` has no WebTransport and falls back to WebSocket. The Vite dev server
  proxies `/ws` and `/cert-hash` to `127.0.0.1:7777`; WebTransport then goes straight to UDP 7777.
- `ls-server` and `ls-bots` need Rust 1.88 (`wtransport`); the rest of the workspace keeps
  `rust-version = "1.85"` (which also sets clippy's MSRV).

### Sim thread policies (`sim.rs`)

| Situation | Behaviour |
| --- | --- |
| Room full / bad version | `Reject`, link closed, `rejected` counted |
| More than 32 bad packets | `Disconnect { "too many protocol errors" }`, removed, `kicked` counted |
| Client sends `Disconnect` | removed; its remaining packets are ignored (not counted as errors) |
| Reliable queue overflow | removed as too slow, `kicked` counted |
| Unreliable packet over the transport's datagram limit | sent on the reliable channel instead |
| Match over, players connected | result shown for `--intermission` seconds (default 10), then every client gets `Disconnect { ls_game::MATCH_RESTART_REASON }` and is removed (not a kick); clients rejoin |
| First join after a finished match | the room is rebuilt (same share code and rules; under a second in release builds, on the sim thread while nobody plays); the scheduler restarts instead of counting skipped ticks. An idle or exiting server never rebuilds |
| Stats window with starvation | more than `10 + 2 %` of the player ticks without input: `input starvation` warning, `desync_warnings` counted (missing inputs count only after a client's first input) |

Counters (`StatsSnapshot`) keep growing across matches: `total_ticks`, `total_edits`, snapshots,
inputs; `tick` and `edit_count` are the current match's. The `tick stats` log line (every
`--stats-interval` s) has p50/p99/max tick time, input counters, bandwidth, edits, craters,
paint cells per team, match time left, `match_over`, `matches` and `encode_errors`; `final stats`
is logged at shutdown.

### CLI

```
ls-server --bind 0.0.0.0:7777 [--wt-bind <addr> | --no-webtransport] --code <share code> \
    --ruleset data/rulesets/paint_the_void.ron --max-players 8 --web-root web/dist \
    --stats-interval 10 --intermission 10 [--exit-when-empty]
```

`--web-root` defaults to `web/dist` if it exists, so `cargo run -p ls-server --release` after
`npm run build` in `web/` serves the client at `http://localhost:7777`. `--exit-when-empty` shuts
down 2 s after the last player left (scripted runs).

## Browser client lifecycle (`web/src/main.ts`)

- Transport (`web/src/net/connect.ts`, `connectTransport`): WebTransport first when the
  browser has it and `GET /cert-hash` offers it (the certificate is trusted by hash), WebSocket
  otherwise or when WebTransport fails to connect; `?transport=ws|wt` forces one. The F3
  overlay shows the active one (`transportLabel`). `droppable` packets are only ls-net
  `Channel::Unreliable` messages (Input, Ping): WebTransport sends them as datagrams.
- Every animation frame: drain packets, `Game::frame`, send inputs (droppable: skipped while the
  send buffer is backed up, e.g. 16 KiB of WebSocket `bufferedAmount`; the `Hello` is never
  dropped; the F3 overlay counts skipped sends).
- Hidden tab (no animation frames): a 250 ms timer (throttled to ~1 Hz by browsers) runs
  `GameSession::idleFrame`, which drains packets and sends idle input plus the periodic ping, so
  the server's idle timeout does not fire and held buttons are released at once.
- Session end (server `Disconnect`, closed socket, desync, GPU device lost): the session is
  disposed (socket closed, wasm game freed) and an error screen shown.
- Match restart (`Disconnect` with `MATCH_RESTART_REASON`): the game is cleaned up, the renderer
  kept, and the client rejoins on its own (`web/src/game/rejoin.ts`: 0.5 s, then 1, 2, 3, 4, 4,
  ... s, up to 12 attempts, then an error screen). Name, horizon assist, look settings and the
  F3 overlay state carry over.
- Overlays: Tab holds the scoreboard (players by team, paint %); the end-of-match screen shows
  the winner, paint per team and the scoreboard; F3 toggles the debug overlay (FPS, ping,
  transport, prediction corrections, shot prediction counters, chunks, skipped sends).

## ls-bots

Each bot runs on its own thread with a current-thread tokio runtime and plays at 60 Hz.
`--transport ws|wt` (default `ws`) picks WebSocket or WebTransport (`src/transport.rs`, one
`Link` type): for `wt` the bot fetches `/cert-hash` from the host and port of `--server` and
trusts the certificate by that hash only. The `joined` log line names the transport.
Every socket write has a 5 s limit and a session in which nothing arrives for 5 s is lost
(`BotConfig::write_timeout`, `silence_timeout`), so a hung server cannot block `--duration`.

| Exit code | Meaning |
| --- | --- |
| 0 | all good (match restarts are fine: bots rejoin after 0.5 s and count `match_restarts`) |
| 1 | a protocol error or desync |
| 2 | a bot never joined |
| 3 | a bot lost its connection (crash, kick, timeout); `--allow-disconnects` turns this off |

## End-to-end run (`tools/e2e.mjs`)

A script, not a Rust test: it runs the real `ls-server` and `ls-bots` binaries over real
sockets and checks their logs (server started with `--exit-when-empty` and a short
`--stats-interval`). Commands, options and the full list of checks:
[dev-guide.md, End-to-end run](../dev-guide.md#6-end-to-end-run).
