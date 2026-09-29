# Loose Screws: hosting guide

How to run a Loose Screws game server (`ls-server`), fill it with bots (`ls-bots`) and keep it
healthy. It covers the current build (Milestone 1). Features from the design that the server
doesn't have yet are listed under [Not in this build yet](#not-in-this-build-yet).

## Contents

- [What the server does](#what-the-server-does)
- [Building](#building)
- [Quick start](#quick-start)
- [Command line reference](#command-line-reference)
- [Choosing the world: share codes](#choosing-the-world-share-codes)
- [Rule sets](#rule-sets)
- [Ports and connections](#ports-and-connections)
- [WebTransport certificate](#webtransport-certificate)
- [Playing across a network](#playing-across-a-network)
- [Firewall and router](#firewall-and-router)
- [Filling the match with bots](#filling-the-match-with-bots)
- [Logs and tick stats](#logs-and-tick-stats)
- [Match end, intermission and restart](#match-end-intermission-and-restart)
- [Stopping the server](#stopping-the-server)
- [Troubleshooting](#troubleshooting)
- [Not in this build yet](#not-in-this-build-yet)

## What the server does

One `ls-server` process hosts **one match** of Paint the Void for up to 8 players (32 at most,
see `--max-players`) on one world. It:

- runs the authoritative game simulation at 60 ticks per second and sends state updates to
  players 30 times per second
- serves the web client over HTTP, so players only need to open the server's address in a
  browser
- accepts players over **WebSocket** (TCP) and **WebTransport** (UDP). The browser picks
  WebTransport when it can and falls back to WebSocket.
- restarts the match on the same world when the timer runs out, and players rejoin
  automatically

There is no lobby, account system or server list: players join by opening your address.

## Building

You build the server, the bots and the web client from the source repository. You need:

- Rust (stable). The repository's `rust-toolchain.toml` installs the right toolchain and the
  `wasm32-unknown-unknown` target when you first run `cargo`. The server and bots need Rust
  1.88 or newer.
- `wasm-bindgen` command line tool, version **0.2.126** exactly:
  `cargo install wasm-bindgen-cli --version 0.2.126`
- Node.js with npm

From the repository root:

```sh
cd web
npm install
npm run build          # builds the game core for the browser, then the web client into web/dist
cd ..
cargo build --release -p ls-server -p ls-bots
```

The programs end up in `target/release/`: `ls-server` and `ls-bots` (`ls-server.exe` and
`ls-bots.exe` on Windows).

Always use **release** builds for hosting. A debug build works, but it takes several times
longer to build a match (about 7 seconds instead of under one) and runs the simulation much more
slowly.

**What the server needs at runtime:** only the executable and the built web client (`web/dist`).
The weapon stats and the default rule set are built into the executable, so you don't need the
`data/` folder unless you load a custom rule set file with `--ruleset`.

**Keep server and web client in sync.** The browser runs the same game code as the server to
predict movement. Rebuild both from the same source version, and after an update ask players
to reload the page.

## Quick start

From the repository root:

```sh
cargo run --release -p ls-server
```

or run the built program directly (`target/release/ls-server`, on Windows
`target\release\ls-server.exe`). The server builds a random world and prints:

```
INFO ls_server: share code LS06-WA9D-Z939-439D-V4M6-8001-8F70 - open http://localhost:7777 to play
```

Open `http://localhost:7777`, enter a pilot name and click **Play online**. To add six bots so
there's someone to play against, open a second terminal:

```sh
target/release/ls-bots --count 6
```

Press **Ctrl+C** in the server terminal to stop it.

Run the server **from the repository root** (or pass `--web-root` explicitly): the default web
client folder `web/dist` and any relative `--ruleset` path are resolved from the current
directory.

## Command line reference

Verified against `ls-server --help` of this build (`ls-server 0.1.0`).

| Flag | Default | What it does |
| --- | --- | --- |
| `--bind <ADDR>` | `0.0.0.0:7777` | TCP address and port for HTTP (web client, `/cert-hash`) and the WebSocket at `/ws`. `0.0.0.0` accepts connections on every network interface. Use `127.0.0.1:7777` to allow only this computer. `[::]:7777` listens on IPv6 (on Windows IPv6 only, so IPv4 players can't connect). |
| `--wt-bind <ADDR>` | same as `--bind` | UDP address and port for WebTransport. By default it uses the same port number as `--bind`, but over UDP. Can't be combined with `--no-webtransport`. |
| `--no-webtransport` | off | Don't start WebTransport. Everyone connects over WebSocket, and `/cert-hash` answers 404. |
| `--code <CODE>` | a random world | Share code of the world to host. Without it, the server picks a random seed with Enclosure 50 and Density 40. |
| `--ruleset <FILE>` | built-in Paint the Void rules | Rule set file in RON format, read once at startup (see [Rule sets](#rule-sets)). |
| `--max-players <N>` | `8` | Player limit, 1 to 32. Bots count as players. Players who try to join a full server get "room is full". |
| `--web-root <DIR>` | `web/dist` if that folder exists | Folder with the built web client, served over HTTP. If the folder doesn't exist, the server logs a warning and serves a placeholder page, but the game connection still works. |
| `--stats-interval <SECONDS>` | `10` | Seconds between two `tick stats` log lines, 0.1 to 3600 |
| `--intermission <SECONDS>` | `10` | Seconds the result of a finished match stays on screen before players are sent to the next match, 0.1 to 3600 |
| `--exit-when-empty` | off | Shut down about 2 seconds after the last player leaves (only after at least one player joined). Meant for scripted test runs (see the warning under [Match end](#match-end-intermission-and-restart)). |
| `-h`, `--help` | | Print help |
| `-V`, `--version` | | Print the version |

The environment variable `RUST_LOG` sets the log detail (default `info`), and `NO_COLOR=1` turns
off colored log output, which is useful when writing the log to a file. See
[Logs and tick stats](#logs-and-tick-stats).

Examples:

```sh
# A fixed world, 8 players, on the default port
ls-server --code LS06-WA9D-Z939-439D-V4M6-8001-8F70

# A custom rule set and a longer break between matches
ls-server --ruleset my_rules.ron --intermission 20

# Only reachable from this computer, WebSocket only, on another port
ls-server --bind 127.0.0.1:8080 --no-webtransport

# WebTransport on a separate UDP port
ls-server --bind 0.0.0.0:7777 --wt-bind 0.0.0.0:7778
```

## Choosing the world: share codes

A share code such as `LS06-WA9D-Z939-439D-V4M6-8001-8F70` rebuilds exactly the same world on the
server and in every browser. It stores a random seed, the seven world sliders and the world
generator version. Only the **Enclosure** and **Density** sliders affect the world in this
build.

- **Keep a level:** start with `--code <code>`. Without `--code`, every server start builds a
  new random world. The server logs the code at startup ("match ready share_code=…").
- **Find a level you like:** in the web client, click **Practice offline** with an empty share
  code to fly a random world, and press **F3**. The last line of the debug overlay shows the
  world's share code. Try as many worlds as you like, then start the server with the code of
  your favorite.
- **Invalid codes** stop the server at startup with a reason, for example
  `share code checksum mismatch (typo?)`,
  `share code must have 26 symbols after the prefix, found 5`, or a message that the code was
  made by a different generator version.
- Players always play the world the server runs. A code a player types on the start screen is
  only used for practice mode.

There's no tool yet for making a code with chosen slider values.

## Rule sets

A rule set decides the match length, respawn time, weapons, starting loadout, pickups and
friendly fire. The default is `data/rulesets/paint_the_void.ron` (built into the server). To
customize it, copy that file, edit the copy and start the server with `--ruleset <file>`. The
server sends the rule set to every browser when it joins, so players don't need to rebuild or
download anything.

The default rule set with explanations:

```ron
RuleSet(
    name: "Paint the Void",           // shown in the server log
    mode: PaintTheVoid((
        teams: 2,                     // must be 2
        match_secs: 300.0,            // match length in seconds
        respawn_secs: 3.0,            // wait after being destroyed
    )),
    weapons: {                        // weapons not listed here are banned
        "scrap_laser": LoadoutAndPickup,
        "rivet_gun": PickupOnly,
        "bolt_rocket": LoadoutAndPickup,
        "paint_thrower": LoadoutOnly,
    },
    starting_loadout: (
        primaries: ["scrap_laser", "paint_thrower"],
        secondaries: [("bolt_rocket", 3)],   // weapon and starting count
        gadget: None,                        // must be None
    ),
    pickups: {                        // spawn weight and respawn time per weapon pickup
        "scrap_laser": (weight: 1.0, respawn_secs: 30.0),
        "rivet_gun": (weight: 2.0, respawn_secs: 20.0),
        "bolt_rocket": (weight: 2.0, respawn_secs: 20.0, ammo: 3),
    },
    scrap_pickup: Some((weight: 3.0, respawn_secs: 15.0, amount: 30.0)),
    friendly_fire_percent: 50,        // 0 = off, otherwise 10 to 100
    allowed_ship_types: [Tinker],     // only Tinker exists
    absurdity: 20,                    // 0 to 100; no effect yet
)
```

Weapon ids: `scrap_laser`, `paint_thrower`, `rivet_gun` (primaries) and `bolt_rocket`
(secondary). Availability values:

| Value | Meaning |
| --- | --- |
| `LoadoutAndPickup` | Players spawn with it (if it's in the starting loadout) and can find it, ammo or upgrades as pickups |
| `PickupOnly` | Only found as a pickup |
| `LoadoutOnly` | Only given at spawn, never spawns as a pickup |
| `Banned` | Not available (same as leaving the weapon out) |

Rules the server checks at startup (it refuses to start and names the problem otherwise):

- A loadout weapon must allow the loadout (`LoadoutAndPickup` or `LoadoutOnly`) and be in the right
  slot. A pickup weapon must allow pickups (`LoadoutAndPickup` or `PickupOnly`).
- The Tinker has 2 secondary slots, and a starting count can't exceed the weapon's maximum
  (10 Bolt Rockets).
- `friendly_fire_percent` is 0 or 10 to 100, and `absurdity` is 0 to 100.
- `match_secs` must be above 0, and times and weights can't be negative.

Pickups spawn at 12 points per level. Each point rolls its content from the weights (with the
defaults: scrap 3, Rivet Gun 2, Bolt Rocket 2, Scrap Laser 1 out of 8). A point refills after its
`respawn_secs` once it has been taken or emptied.

Rule set changes take effect when the server starts. **Weapon stats** (damage, speed and so
on) are built into both the server and the web client. Changing `data/weapons/*.ron` means
rebuilding both and having players reload.

## Ports and connections

With the defaults, everything uses port **7777**:

| Protocol | Port | Used for |
| --- | --- | --- |
| TCP | `--bind` port (7777) | HTTP: the web client, `GET /cert-hash`, and the WebSocket game connection at `/ws` |
| UDP | `--wt-bind` port (7777 over UDP by default) | WebTransport game connection (`https://host:port/wt`) |

- Players only need the HTTP address, for example `http://192.168.1.20:7777`. The browser fetches
  `/cert-hash` to find the WebTransport port and certificate. It uses WebTransport if that
  works and WebSocket otherwise.
- If UDP is blocked anywhere along the way, players fall back to WebSocket automatically. It works,
  but it can feel less smooth on a bad connection.
- Players can force a connection type by adding `?transport=ws` or `?transport=wt` to the
  address. The F3 overlay in game shows which one is active.
- A connection that sends nothing for 10 seconds is dropped. The browser keeps the connection
  alive even while its tab is in the background.

## WebTransport certificate

WebTransport always runs over encrypted QUIC and needs a TLS certificate. You don't need to
provide one:

- At startup the server creates a **self-signed certificate** that is valid for 13 days (browsers
  accept at most 14 days for this method). Every 6 days it switches to a fresh one for new
  connections, and running connections continue undisturbed.
- The **certificate hash** (SHA-256) is published at `http://<server>:<port>/cert-hash` as JSON,
  for example:

  ```json
  {"algorithm":"sha-256","hash":"b304152f…ef995","port":7777,"path":"/wt","expires":1791779631}
  ```

  and logged at startup (`webtransport listening … cert_sha256=…`) and on every rotation
  (`webtransport certificate rotated`).
- Browsers fetch this hash right before they connect and trust the certificate by its hash. No
  certificate needs to be installed, and no browser warning appears.
- A restart creates a new certificate. That doesn't matter, because clients always fetch the
  current hash.
- Your own certificate (for example from Let's Encrypt) can't be used for WebTransport yet.

## Playing across a network

On the host computer itself, `http://localhost:7777` just works.

For players on other computers there's one catch: **browsers only allow WebGPU (and
WebTransport) on secure pages**, meaning `https://` addresses or `localhost`. A page opened as
`http://192.168.1.20:7777` is not secure, so the game can't draw and shows "WebGPU unavailable".
Ways around it:

1. **Browser flag (easiest, for friends on your network):** in Chrome or Edge, each player opens
   `chrome://flags/#unsafely-treat-insecure-origin-as-secure` (Edge: `edge://flags/…`), adds
   the server address (for example `http://192.168.1.20:7777`), enables the flag and restarts
   the browser. The page then counts as secure, and WebGPU and WebTransport both work.
2. **Serve the client locally:** a player who has the built web client can open it from their own
   computer on `localhost` and point it at your server with `?server=`. For example, they run
   their own `ls-server --bind 127.0.0.1:7780` and open
   `http://localhost:7780/?server=192.168.1.20:7777`. The page is then secure (localhost), and the
   game connects to your server. The browser may ask for permission to access devices on the
   local network: allow it.
3. **HTTPS in front of the server:** a reverse proxy with a real certificate that serves the page
   over `https://` and forwards WebSocket upgrades on `/ws` to the server would give players a
   secure page with no flags. **This setup is not tested with this build.**

For players on the internet, you also need port forwarding (see below). Option 1 or 2 applies
there too.

## Firewall and router

Open both **TCP and UDP** on the game port (7777 by default, plus the `--wt-bind` port if you use a
different one).

**Windows:** the first time `ls-server.exe` listens, Windows Defender Firewall asks whether to
allow it. Allow it for the networks you play on. To add rules by hand, run in an administrator
PowerShell:

```powershell
New-NetFirewallRule -DisplayName "Loose Screws (TCP)" -Direction Inbound -Protocol TCP -LocalPort 7777 -Action Allow
New-NetFirewallRule -DisplayName "Loose Screws (UDP)" -Direction Inbound -Protocol UDP -LocalPort 7777 -Action Allow
```

**Linux (ufw):**

```sh
sudo ufw allow 7777/tcp
sudo ufw allow 7777/udp
```

**Router:** for players outside your home network, forward TCP 7777 and UDP 7777 to the host
computer and give them your public address. If only TCP is forwarded, players still connect over
WebSocket.

## Filling the match with bots

`ls-bots` starts headless bot players that join over the network like real players. They fly
around the level avoiding walls, paint surfaces, fight enemies in range with the Scrap Laser
(or a Rivet Gun they picked up) and the occasional Bolt Rocket, and go scavenging when their hull
is low. They appear in the game as regular pilots and take up player slots.

```sh
# 7 bots on a local server until Ctrl+C
ls-bots

# 4 bots against another machine, for 10 minutes
ls-bots --server ws://192.168.1.20:7777/ws --count 4 --duration 600

# bots over WebTransport
ls-bots --transport wt
```

Verified against `ls-bots --help` of this build:

| Flag | Default | What it does |
| --- | --- | --- |
| `--server <URL>` | `ws://127.0.0.1:7777/ws` | WebSocket address of the server. With `--transport wt`, its host and port are used to fetch `/cert-hash`. |
| `--transport <ws\|wt>` | `ws` | Connect over WebSocket or WebTransport |
| `--count <N>` | `7` | Number of bots |
| `--duration <SECONDS>` | run until Ctrl+C | Stop after this many seconds |
| `--stagger-ms <MS>` | `750` | Delay between two bots joining |
| `--seed <N>` | `1` | Base seed for the bots' behavior (different seeds, different flying) |
| `--name-prefix <TEXT>` | `bot` | Bot names are this plus a number (`bot1`, `bot2`, …). Names show in the server log. |
| `--no-reconnect` | off | Don't reconnect after losing the connection |
| `--allow-disconnects` | off | A lost connection doesn't count as a failure (exit code 3) |
| `--verbose` | off | Log debug output |

Plan the numbers: `--count` plus the human players must fit in `--max-players`. Extra bots are
refused with "room is full", and the run ends with exit code 2. The bots rejoin by themselves
when a match restarts.

When the bots stop, they print a summary table and a result line. The exit code is:

| Code | Meaning |
| --- | --- |
| 0 | All good (match restarts are fine) |
| 1 | A bot saw a protocol error or lost sync with the server |
| 2 | A bot never managed to join (server not running, wrong address, server full) |
| 3 | A bot lost its connection (server stopped or crashed, kicked, timed out). `--allow-disconnects` turns this off. |

## Logs and tick stats

The server writes its log to the console. To keep it in a file:

```sh
NO_COLOR=1 ls-server --code <code> > server.log 2>&1
```

In PowerShell, set `$env:NO_COLOR = "1"` first. More or less detail comes from `RUST_LOG`, for
example `RUST_LOG=debug` (every HTTP request, handshake details), `RUST_LOG=warn` (problems
only) or `RUST_LOG=ls_server=debug`.

Important lines:

| Log line | Meaning |
| --- | --- |
| `serving the web client web_root=…` | The web client folder in use |
| `match ready share_code=… ruleset=… build_ms=…` | The world is built. Note the share code if you want to keep the level. |
| `webtransport listening … port=… cert_sha256=…` | WebTransport is up, with its certificate hash |
| `listening (http + websocket at /ws) …` | The server accepts players |
| `share code … - open http://… to play` | The address to open in a browser |
| `player joined conn=… player=… team=… name=… transport=… players=…` | A player joined (team 0 is Orange, 1 is Blue) |
| `join rejected … reason=ServerFull` | The server was full (other reasons: a version mismatch between client and server, or a bad handshake) |
| `player left … reason=…` | A player left, timed out or was removed |
| `tick stats …` | Performance and match counters, every `--stats-interval` seconds (see below) |
| `match over; sending players off to rejoin the next match` | The intermission ended |
| `previous match finished; started a new one build_ms=… matches=…` | The next match is built |
| `final stats …` | Totals at shutdown |

Warnings to watch:

| Warning | Meaning and what to do |
| --- | --- |
| `tick p99 over budget` | The slowest 1% of ticks took over 4 ms. Occasional warnings are harmless. Constant ones mean the machine is too slow or busy. Use a release build and close other heavy programs. |
| `sim fell behind; skipped ticks to catch up` | The simulation couldn't keep up (the computer was busy or asleep). Players may see a hitch. |
| `input starvation: clients will mispredict (desync risk)` | Players' inputs arrived too rarely, usually because of a bad connection. Their ships may jump or feel laggy. |
| `web root not found; serving a placeholder page` | The `--web-root` folder is missing. Build the web client or fix the path. |

### Tick stats fields

| Field | Meaning |
| --- | --- |
| `tick` | Tick of the current match (60 per second) |
| `total_ticks` | Ticks over all matches since startup |
| `players` | Connected players |
| `ticks` | Ticks simulated in this window |
| `p50_us`, `p99_us`, `max_us` | Median, 99th percentile and slowest tick time in microseconds. Budget: 4000 (4 ms). Healthy servers stay far below it. |
| `over_budget` | Ticks in this window over the budget |
| `skipped` | Ticks skipped to catch up in this window (should be 0) |
| `late_inputs` | Player inputs that arrived after their tick. Some are normal right after a join. |
| `missing_inputs` | Ticks where a player's input was missing and the last one was repeated |
| `duplicate_inputs` | Redundant copies of inputs (normal: each input packet repeats the latest commands) |
| `dropped_inputs` | Inputs that arrived too far ahead and were dropped |
| `compensated_hits` | Hits checked with lag compensation |
| `encode_errors` | Messages that couldn't be sent. Anything above 0 is a bug worth reporting. |
| `snapshot_kib_s` | Game state data sent to all players, in KiB per second |
| `unreliable_dropped` | Time-critical packets dropped because a player's connection was backed up |
| `edits`, `total_edits` | Terrain changes (paint and craters) in this match, and since startup |
| `craters` | Craters in this match |
| `paint_cells_0`, `paint_cells_1`, `surface_cells` | Surface painted by Orange and Blue, and the level's total paintable surface. The score is `paint_cells / surface_cells`. |
| `match_left_s` | Seconds left in the match |
| `match_over` | Whether the match has ended |
| `matches` | Matches played since startup, including the current one |
| `desync_warnings` | Input starvation warnings since startup |

The `final stats` line at shutdown adds totals: `connections`, `rejected`, `kicked`
(players removed for protocol errors or a connection that fell too far behind), `timeouts`,
`skipped_ticks`, `snapshots_sent`, `snapshot_bytes`, `edits`, `matches` and
`desync_warnings`.

## Match end, intermission and restart

1. When the match timer runs out (5 minutes by default, `match_secs` in the rule set), the team
   with more painted surface wins. Players see the result screen.
2. After `--intermission` seconds (default 10), the server disconnects every player with the
   message "match over; a new match is starting". This isn't counted as a kick.
3. The browsers and bots reconnect by themselves (after half a second, then with growing pauses,
   up to 12 attempts in the browser).
4. The **first join** after a finished match builds a fresh match on the **same share code and
   rule set**: the level is back to pristine (no craters, no paint), and teams are assigned
   again. Building takes under a second in a release build. Joins wait for it.

If a match ends while nobody is connected, the server waits and builds the next match when
someone joins. To change the world or the rules, restart the server with other flags.

> **Warning:** `--exit-when-empty` sees the moment between the intermission and the next match
> as "everyone left". If rebuilding the match and rejoining take longer than about 2 seconds
> (always with a debug build, and possible with a release build), the server shuts down
> instead of starting the next match. Don't use
> this flag for a server people play on.

## Stopping the server

Press **Ctrl+C**. The server sends every player a "server shutting down" message, closes the
connections, logs `final stats` and exits. Players see a "Disconnected" screen. A server
stopped with a hard kill (closing the terminal window, Task Manager, `kill -9`) doesn't write
`final stats`.

## Troubleshooting

| Problem | What to check |
| --- | --- |
| Server exits at startup with "invalid match configuration" | The reason follows: a bad share code, a rule set file that's missing or invalid (the message names the file and the problem). |
| "server failed to start", address in use | Another program (or another `ls-server`) already uses the port. Pick another port with `--bind` (and `--wt-bind`), or stop the other program. TCP and UDP both have to be free. |
| Browser shows "ls-server is running…" placeholder page | The web client wasn't found. Build it (`cd web && npm run build`) and start the server from the repository root, or pass `--web-root <path to web/dist>`. |
| Players elsewhere get "WebGPU unavailable" | The page isn't a secure context over `http://<IP>`. See [Playing across a network](#playing-across-a-network). |
| Players can't connect at all | Firewall or router (TCP port). Test from another computer with `http://<server-ip>:7777/cert-hash` in a browser: it should show JSON (or a 404 page with `--no-webtransport`). No answer at all means the connection is blocked. |
| Players only get WebSocket (F3 shows WebSocket) | UDP blocked by a firewall or router, `--no-webtransport` set, the page isn't a secure context, or the browser has no WebTransport. The game still works. |
| "room is full" | Raise `--max-players` (up to 32) or run fewer bots. |
| "Lost sync with the server" in browsers | The web client and the server come from different builds. Rebuild both, restart, and have players reload the page. |
| Players time out when their tab is hidden | Browsers slow down background tabs to about one update per second, which still keeps the connection alive. A computer going to sleep drops the connection. |
| Stuttering, `tick p99 over budget` warnings | Use a release build, and check CPU load on the host. With `RUST_LOG=debug` the log itself adds load. |
| Nobody gets back in after a match | See the log for "could not start a new match". Players can reload the page to rejoin. |

## Not in this build yet

- **One match per server process:** one world, one mode (Paint the Void, two teams). Run several
  servers on different ports for more matches. Other game modes and mutators come later.
- **No lobby, server list, invites, party codes or accounts.** Players need your address.
- **No admin commands** while running (kick, change world, end match). Restart the server with new
  flags instead.
- **No rule set editor, rule set codes or live balance list.** Rule sets are files loaded at
  startup. The `absurdity` value and weapon `shield` settings have no effect yet. Gadgets,
  per-weapon tuning multipliers and ship types other than the Tinker don't exist yet.
- **No slider tool:** you can't create a share code with chosen slider values yet, and only
  Enclosure and Density affect the world.
- **No custom TLS certificate** for WebTransport and no built-in HTTPS for the web page.
- **No WebRTC fallback, desktop app, regional hosting or orchestration** (the design mentions
  Agones or Nomad for later).
