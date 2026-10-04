# Directory service (`ls-directory`)

The optional service behind the browser's server list (M3, [milestone-3-plan.md §3.1](../milestone-3-plan.md#31-framework-and-lobby)).
Game servers that opt in register with heartbeats; browsers read the list and ping the servers
themselves; the directory also relays short-lived invites (duel challenges across servers). A game
server never depends on it: without `--directory` it runs exactly as before and stays reachable by
link or address.

- Crate: `crates/ls-directory` — binary `ls-directory` plus a library: `api` (JSON types and
  validation), `registry` (in-memory state: TTL, limits, per-IP rate limit; every method takes
  `now`), `http` (the HTTP/1.1 service, same hand-written style as `ls-server`'s `http.rs`),
  `client` (`DirectoryClient` and `spawn_heartbeats`, used by `ls-server`).
- No database, no accounts: a restart forgets everything and servers re-register with their next
  heartbeat. No dependency on the game crates, so one directory can list servers of several builds;
  clients filter by protocol version and data hash.
- Plain HTTP only. For browsers on `https://` pages put a TLS reverse proxy in front (the client
  library speaks `http://` only; game servers can use the plain URL).

## Running

```sh
cargo run -p ls-directory --release -- --bind 127.0.0.1:7780
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--bind` | `0.0.0.0:7780` | TCP listen address |
| `--ttl` | 30 | Seconds after the last heartbeat until an entry disappears |
| `--heartbeat` | 10 | Heartbeat interval suggested to servers (seconds) |
| `--max-servers` | 1000 | Most entries; new servers get 503 beyond it |
| `--invite-ttl` | 30 | Seconds an invite stays available |
| `--max-invites` | 10000 | Most pending invites in total (8 per recipient, oldest dropped) |
| `--rate` / `--burst` | 10 / 40 | Per-IP token bucket (requests per second / burst); 429 beyond it |

Logging follows `RUST_LOG` (default `info`). Behind a reverse proxy every request comes from the
proxy's IP, so raise `--rate`/`--burst` there (forwarded-for handling is M5 hardening).

## API

Every reply carries `Access-Control-Allow-Origin: *`; `OPTIONS` answers CORS preflights (methods
`GET, POST, DELETE, OPTIONS`, header `Content-Type`). Bodies are JSON with `Content-Length` (no
chunked bodies), at most 16 KiB. Errors are `{"error":"..."}` with 400 (invalid JSON or field),
403 (wrong secret), 404, 405, 411, 413, 429 (rate limited) or 503 (full).

| Request | Body / query | Reply |
| --- | --- | --- |
| `POST /servers` | `Heartbeat` | 200 `{"id","ttl_secs","heartbeat_secs"}` |
| `DELETE /servers` | `{"address","secret"}` | 204 (server shutdown) |
| `GET /servers` | optional `?protocol=5&data_hash=<16 hex>` filters | 200 `{"servers":[{"id","age_ms","server":ServerInfo}]}`, sorted by address |
| `POST /invites` | `InviteRequest` | 200 `{"id","expires_in_secs"}` |
| `GET /invites` | `?player=<id>` | 200 `{"invites":[{"id","expires_in_ms","invite":InviteRequest}]}` |

`Heartbeat` = `{"secret": "<16..128 chars>", "server": ServerInfo}`. `ServerInfo`:

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | string, 1–48 chars | `--server-name` |
| `address` | `host:port` | Public game port (WebSocket `/ws`, `/cert-hash`); the entry's key |
| `cert_hash` | 64 hex, optional | WebTransport certificate SHA-256 |
| `region` | string ≤ 16 | `--region` tag, may be empty |
| `mode` | string ≤ 32 | Current mode id (or `lobby`) |
| `map_code` | `LS..` | Current world share code |
| `rules_code` | `LR..`, optional | Current rule set code |
| `players`, `max_players` | u16 | `players <= max_players` |
| `protocol` | u16 | `ls_net::PROTOCOL_VERSION` |
| `data_hash` | 16 lower-case hex | `ls_sim::data_hash()` (`api::data_hash_hex`); a string because JavaScript numbers can't hold a `u64` |
| `roster` | `[{"name","id"}]`, ≤ 64, optional | Players for challenges: display name and **public** player id (8–64 of `A-Za-z0-9-_.:[]`), never the browser's private token |

Rules:
- The first heartbeat for an address claims it with its secret; later heartbeats and `DELETE` must
  repeat the secret (403 otherwise). After the entry expires anyone may claim the address again.
  `ls_directory::new_secret()` makes a 32-hex-digit secret from the OS random source.
- Entries expire `ttl` (30 s) after their last heartbeat; heartbeats every 10 s survive two lost ones.
- Ids (`s1`, `s2`, ...) stay stable while an entry lives.
- The client lists only servers it can join: it passes its own `protocol` and `data_hash`.

`InviteRequest` = `{"to","kind","from_name","from_id","address","room"?,"world_code"?,"rules_code"?}`:
`to`/`from_id` are public player ids, `kind` is free-form (`duel`, `accept`, `decline`, ... up to
16 token characters), `address` is the server the invite points at. Invites are not consumed by
reading: clients poll `GET /invites?player=` and de-duplicate by `id` until `expires_in_ms` runs
out. Answers travel the same way (an invite back to `from_id`).

## Using it from `ls-server`

```rust
let client = ls_directory::DirectoryClient::new("http://dir.example.com:7780")?;
let task = ls_directory::spawn_heartbeats(client, ls_directory::new_secret(), move || current_info(), shutdown_rx);
```

`spawn_heartbeats` sends a heartbeat at once and then every `heartbeat_secs` the directory returns,
calling the closure for the current `ServerInfo`; failures are logged at `warn` and retried at the
next interval; when the shutdown watch turns true it sends `DELETE /servers` and ends. The server's
flags (`--directory`, `--server-name`, `--region`, `--public-address`) and the browser's server
list are M3 package B7.

## Tests

`cargo test -p ls-directory`: unit tests for validation, the registry (TTL, secrets, filters, caps,
invites, rate limit with explicit time), request parsing and routing; `tests/http.rs` runs the
service on `127.0.0.1:0` and covers register, heartbeat, expiry, list filters, CORS and
keep-alive, the invite relay, 429, and the heartbeat task (register, refresh, leave; survives a
missing directory).
