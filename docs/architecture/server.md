# Server, bots and end-to-end checks

`crates/ls-server` hosts the authoritative `ls_game::server::Room` (see
[game-glue.md](game-glue.md)); `crates/ls-bots` are headless clients built on
`ls_game::client::ClientSession`; `tools/e2e.*` runs both together.

## ls-server

```
tokio I/O tasks                         ls-sim thread (60 Hz)
  HttpEndpoint: static files, /ws  ---Join{hello, SimLink}-->  Sim: Room + clients
  ws task per connection  <--PeerLink | SimLink-->               receive -> step -> dispatch
```

- `link.rs`: `link_pair` creates the two halves of a connection. `SimLink` (sim thread)
  implements `ls_net::Transport` and never blocks; `PeerLink` (socket task) awaits outgoing
  frames. At most 8 unreliable frames wait (about 260 ms of snapshots plus pongs; older ones are
  dropped), reliable frames are never dropped but more than 16 MiB queued closes the link as too
  slow. Incoming packets use a bounded channel (256), so a flooding client stalls only its own
  socket.
- `timing.rs`: deadline scheduler, bursts of up to 4 ticks to catch up, at most 250 ms backlog.
- `endpoint.rs`: the transport seam (`Endpoint`, `Hub`); WebTransport plugs in here in wave 4.
  Handshake and idle timeouts (`ConnSettings`, 10 s idle) apply per connection.

### Sim thread policies (`sim.rs`)

| Situation | Behaviour |
| --- | --- |
| Room full / bad version | `Reject`, link closed, `rejected` counted |
| More than 32 bad packets | `Disconnect { "too many protocol errors" }`, removed, `kicked` counted |
| Client sends `Disconnect` | removed; its remaining packets are ignored (not counted as errors) |
| Reliable queue overflow | removed as too slow, `kicked` counted |
| Unreliable packet over the transport's datagram limit | sent on the reliable channel instead |
| Match over, players connected | result shown for `--intermission` seconds (default 10), then every client gets `Disconnect { ls_game::MATCH_RESTART_REASON }` and is removed (not a kick); clients rejoin |
| First join after a finished match | the room is rebuilt (same share code and rules; ~1-2 s on the sim thread while nobody plays); the scheduler restarts instead of counting skipped ticks. An idle or exiting server never rebuilds |
| Stats window with starvation | more than `10 + 2 %` of the player ticks without input: `input starvation` warning, `desync_warnings` counted (missing inputs count only after a client's first input) |

Counters (`StatsSnapshot`) keep growing across matches: `total_ticks`, `total_edits`, snapshots,
inputs; `tick` and `edit_count` are the current match's. The `tick stats` log line (every
`--stats-interval` s) has p50/p99/max tick time, input counters, bandwidth, edits, craters,
paint cells per team, match time left, `match_over`, `matches` and `encode_errors`; `final stats`
is logged at shutdown.

### CLI

```
ls-server --bind 0.0.0.0:7777 --code <share code> --ruleset data/rulesets/paint_the_void.ron \
    --max-players 8 --web-root web/dist --stats-interval 10 --intermission 10 [--exit-when-empty]
```

`--web-root` defaults to `web/dist` if it exists, so `cargo run -p ls-server --release` after
`npm run build` in `web/` serves the client at `http://localhost:7777`. `--exit-when-empty` shuts
down 2 s after the last player left (scripted runs).

## Browser client lifecycle (`web/src/main.ts`)

- Every animation frame: drain packets, `Game::frame`, send inputs (droppable: skipped while the
  WebSocket's `bufferedAmount` exceeds 16 KiB; the `Hello` is never dropped).
- Hidden tab (no animation frames): a 250 ms timer (throttled to ~1 Hz by browsers) runs
  `GameSession::idleFrame`, which drains packets and sends idle input plus the periodic ping, so
  the server's idle timeout does not fire and held buttons are released at once.
- Session end (server `Disconnect`, closed socket, desync, GPU device lost): the session is
  disposed (socket closed, wasm game freed) and an error screen shown; a match restart shows
  "Match over" with a reload button.

## ls-bots

Each bot runs on its own thread with a current-thread tokio runtime and plays at 60 Hz.
Every socket write has a 5 s limit and a session in which nothing arrives for 5 s is lost
(`BotConfig::write_timeout`, `silence_timeout`), so a hung server cannot block `--duration`.

| Exit code | Meaning |
| --- | --- |
| 0 | all good (match restarts are fine: bots rejoin after 0.5 s and count `match_restarts`) |
| 1 | a protocol error or desync |
| 2 | a bot never joined |
| 3 | a bot lost its connection (crash, kick, timeout); `--allow-disconnects` turns this off |

## End-to-end run (`tools/e2e.mjs`)

`pwsh tools/e2e.ps1` / `tools/e2e.sh` build the release binaries and the web client, then start
the server on a free port with a fixed code, check that HTTP serves `web/dist`, run 7 bots for 90
s and check the bot summary (exit 0, all joined, no disconnects, protocol errors or desyncs,
snapshots flowing) and the server log (clean shutdown, no errors, no desync warnings, p99 under
4 ms in every window, no skipped ticks, edits, craters and paint growing within the first match,
match timer progressing). `--count 8 --duration 330` covers a full match including the restart.
