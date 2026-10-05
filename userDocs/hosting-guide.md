# Loose Screws: hosting guide

How to run a Loose Screws game server (`ls-server`), set its world and rules, fill it with bots
(`ls-bots`) and keep it healthy. It covers the current build (Milestone 2). Features from the
design that the server doesn't have yet are listed under
[Not in this build yet](#not-in-this-build-yet).

## Contents

- [What the server does](#what-the-server-does)
- [Building](#building)
- [Quick start](#quick-start)
- [Command line reference](#command-line-reference)
- [Choosing the world: share codes](#choosing-the-world-share-codes)
- [Rule sets](#rule-sets)
- [Rule set codes, the editor and invite links](#rule-set-codes-the-editor-and-invite-links)
- [Banning and balancing weapons](#banning-and-balancing-weapons)
- [Ports and connections](#ports-and-connections)
- [WebTransport](#webtransport)
- [Playing across a network](#playing-across-a-network)
- [Firewall and router](#firewall-and-router)
- [Filling the match with bots](#filling-the-match-with-bots)
- [Logs and tick stats](#logs-and-tick-stats)
- [Lobby, match end and rotation](#lobby-match-end-and-rotation)
- [Stopping the server](#stopping-the-server)
- [Troubleshooting](#troubleshooting)
- [Server directory (optional)](#server-directory-optional)
- [Not in this build yet](#not-in-this-build-yet)

## What the server does

One `ls-server` process hosts a **public room** that plays a rotation of matches (a playlist)
for up to 8 players (32 at most, see `--max-players`), plus a few **private rooms** for duels
and parties. It:

- runs the authoritative game simulation at 60 ticks per second and sends state updates to
  players 30 times per second
- serves the web client over HTTP, so players only need to open the server's address in a
  browser
- accepts players over **WebSocket** (TCP) and **WebTransport** (UDP). The browser picks
  WebTransport when it can and falls back to WebSocket.
- enforces its rule set: banned weapons, gadgets and ship types never spawn and can't be used
- runs a lobby between matches: warm-up flying with weapons off, a countdown, the match, its
  result; then the next match of the rotation, with players staying connected

There is no account system or server list yet: players join by opening your address.

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
longer to build a match (seconds instead of under one) and runs the simulation much more
slowly.

**What the server needs at runtime:** only the executable and the built web client (`web/dist`).
The weapon, ship and gadget data and the built-in rule sets are compiled into the executable, so
you don't need the `data/` folder unless you load a rule set or balance list file.

**Keep server and web client in sync.** The browser runs the same game code as the server to
predict movement, and both carry a hash of the game data (weapons, ships, gadgets, rule sets). A
browser whose data differs from the server's is refused with "game data mismatch". Rebuild both
from the same source version, and after an update ask players to reload the page (Ctrl+F5).

## Quick start

From the repository root:

```sh
cargo run --release -p ls-server
```

or run the built program directly (`target/release/ls-server`, on Windows
`target\release\ls-server.exe`). The server builds a random world with the default Paint the
Void rules and prints:

```
INFO ls_server::sim: rules: Paint the Void LR8V-6097-G000-TMBG (banned weapons: none)
INFO ls_server: share code LS0F-143C-ZQW8-CCHA-V4M6-8001-8FAE - open http://localhost:7777 to play
```

Open `http://localhost:7777`, enter a pilot name, pick a ship and click **Play online**. To add
six bots so there's someone to play against, open a second terminal:

```sh
target/release/ls-bots --count 6
```

Press **Ctrl+C** in the server terminal to stop it.

Run the server **from the repository root** (or pass `--web-root` explicitly): the default web
client folder `web/dist` and any relative `--ruleset` or `--balance-list` path are resolved
from the current directory.

## Command line reference

Verified against `ls-server --help` of this build (`ls-server 0.1.0`).

| Flag | Default | What it does |
| --- | --- | --- |
| `--bind <ADDR>` | `0.0.0.0:7777` | TCP address and port for HTTP (web client, `/cert-hash`) and the WebSocket at `/ws`. `0.0.0.0` accepts connections on every network interface. Use `127.0.0.1:7777` to allow only this computer. `[::]:7777` listens on IPv6 (on Windows IPv6 only, so IPv4 players can't connect). |
| `--wt-bind <ADDR>` | same as `--bind` | UDP address and port for WebTransport. By default it uses the same port number as `--bind`, but over UDP. Can't be combined with `--no-webtransport`. |
| `--no-webtransport` | off | Don't start WebTransport. Everyone connects over WebSocket, and `/cert-hash` answers 404. |
| `--code <CODE>` | a random world | Share code of the world to host; with the default rotation every match plays on it. Without it, the server picks a random seed with the default sliders (Enclosure 50, Density 40, Instability 30): once with `--ruleset`, for every match with the default rotation. |
| `--ruleset <FILE\|CODE\|ID>` | the default rotation | A rule set file (RON), a built-in rule set id (`paint_the_void`, `sandbox_all_weapons`) or an `LR…` rule set code (see [Rule sets](#rule-sets)), played in every match on one world. An existing file wins, then built-in ids; text that looks like a code is decoded. Read once at startup. Can't be combined with `--playlist`. |
| `--playlist <FILE>` | the default rotation | A playlist file: the matches to rotate through, each with a rule set, a world and a player range (see [Playlists](#playlists)). Can't be combined with `--ruleset` or `--code`. |
| `--ban <WEAPON_ID>` | none | Bans a weapon on top of the rule set. Repeat it for more weapons (`--ban railspike --ban tectonic_charge`). Weapon ids are listed in [Rule sets](#weapon-gadget-and-ship-ids). |
| `--balance-list <FILE>` | none | A balance list file: weapons and gadgets banned or retuned server-wide, applied on top of the rule set and **re-read before every match** (see [Banning and balancing weapons](#banning-and-balancing-weapons)). |
| `--max-players <N>` | `8` | Player limit, 1 to 32. Bots count as players. Players who try to join a full server get "room is full". |
| `--web-root <DIR>` | `web/dist` if that folder exists | Folder with the built web client, served over HTTP. If the folder doesn't exist, the server logs a warning and serves a placeholder page, but the game connection still works. |
| `--stats-interval <SECONDS>` | `10` | Seconds between two `tick stats` log lines, 0.1 to 3600 |
| `--intermission <SECONDS>` | `10` | Seconds the result of a finished match stays on screen before the next match's warm-up, 0.1 to 3600 |
| `--warmup <SECONDS>` | `30` | Longest warm-up (flying the next level with weapons off) before the countdown starts, once enough players are there; shorter when everyone is ready. 0 to 3600. `--warmup 0 --countdown 0` turns the lobby off: matches start right away. |
| `--countdown <SECONDS>` | `10` | Countdown before a match starts, 0 to 3600 |
| `--private-rooms <N>` | `4` | Most private rooms (duels, parties, custom matches) open at once next to the public room, 0 to 64; each runs on its own CPU core, so at most one per core beyond the public room's |
| `--directory <URL>` | none | Register with a server directory (`http://host:port`, see [Server directory](#server-directory-optional)) so the server shows up in the in-game server list. Without it the server is reachable by link or address only. |
| `--server-name <NAME>` | `Loose Screws server` | Name in the server list (up to 48 characters) |
| `--region <TAG>` | empty | Region tag in the server list, e.g. `eu-west` (up to 16 characters) |
| `--public-address <HOST:PORT>` | the `--bind` address | The address players reach this server at, as listed in the directory and put into invite links. Set it whenever `--bind` is `0.0.0.0` (then the default is `localhost:<port>`, which only works on this computer) or players come through a router or a domain name. |
| `--exit-when-empty` | off | Shut down about 2 seconds after the last player leaves (only after at least one player joined). Meant for scripted test runs. |
| `-h`, `--help` | | Print help |
| `-V`, `--version` | | Print the version |

The environment variable `RUST_LOG` sets the log detail (default `info`), and `NO_COLOR=1` turns
off coloured log output, which is useful when writing the log to a file. See
[Logs and tick stats](#logs-and-tick-stats).

Examples:

```sh
# A fixed world, 8 players, on the default port
ls-server --code LS0F-143C-ZQW8-CCHA-V4M6-8001-8FAE

# Every weapon and gadget as a pickup (the built-in sandbox rules)
ls-server --ruleset data/rulesets/sandbox_all_weapons.ron

# Rules from the rule set editor, without the Tectonic Charge
ls-server --ruleset LR8V-6097-G000-TMBG --ban tectonic_charge

# Your own rule set file, a server-wide balance list and a longer break between matches
ls-server --ruleset my_rules.ron --balance-list balance.ron --intermission 20

# Only reachable from this computer, WebSocket only, on another port
ls-server --bind 127.0.0.1:8080 --no-webtransport

# WebTransport on a separate UDP port
ls-server --bind 0.0.0.0:7777 --wt-bind 0.0.0.0:7778
```

## Choosing the world: share codes

A share code such as `LS0F-143C-ZQW8-CCHA-V4M6-8001-8FAE` rebuilds exactly the same world on the
server and in every browser. It stores a random seed, the seven world sliders and the world
generator version. This build's generator (version 3) makes codes starting `LS0C` to `LS0F`
and carves a network of caverns and tunnels that links both teams' ends into every level,
plus the Heist core vein at the bottom and the extraction caverns at the top (every level has
them; only Heist uses them). Of
the sliders, **Enclosure** and **Density** shape the world and **Instability** sets how far a
Tectonic Charge's cracks spread; **Palette** only changes the look in the browser (desaturated
colours); the others have no effect yet.

- **Keep a level:** start with `--code <code>`. Without `--code`, every server start builds a
  new random world. The server logs the code at startup ("match ready share_code=…").
- **Find a level you like:** in the web client, click **Practice offline** with an empty share
  code to fly a random world, and press **F3**. The last line of the debug overlay shows the
  world's share code. Try as many worlds as you like, then start the server with the code of
  your favourite.
- **Invalid codes** stop the server at startup with a reason, for example
  `share code checksum mismatch (typo?)`,
  `share code must have 26 symbols after the prefix, found 5`, or a message that the code was
  made by a different generator version. Milestone 1 codes (`LS04…`, `LS06…`) are from
  generator version 1 and no longer load.
- Players always play the world the server runs. A code a player types on the start screen is
  only used for practice mode.

There's no tool yet for making a code with chosen slider values.

## Rule sets

A rule set decides the match length, respawn time, which weapons, gadgets and ship types are
allowed, the starting loadout, the pickups, per-weapon tuning, each weapon's Shield setting and
friendly fire. The server sends its rules to every browser when it joins, so players don't need
to rebuild or download anything.

You can give the server its rules three ways:

1. **Nothing:** the built-in default, `data/rulesets/paint_the_void.ron`.
2. **A rule set code** made in the [rule set editor](#rule-set-codes-the-editor-and-invite-links):
   `--ruleset LR8V-6097-G000-TMBG`. Easiest, and nothing to copy around.
3. **A rule set file** (RON): `--ruleset my_rules.ron`. Copy one of the built-in files, edit
   the copy and pass its path.

Built-in rule set files:

| File | What it is |
| --- | --- |
| `data/rulesets/paint_the_void.ron` | The default: Scrap Laser, Paint Thrower, 3 Bolt Rockets and a Shield at spawn; every other weapon and gadget as a pickup; Tectonic Charge 1 per map with a 90 s respawn; friendly fire 50%; all four ship types |
| `data/rulesets/sandbox_all_weapons.ron` | Every weapon and gadget as a pickup with even weights and 15 s respawns (Tectonic Charge 60 s, 1 per map), so a 5-minute match sees the whole arsenal. Good for trying everything with friends or bots. |

### The rule set file

The default rule set, shortened, with explanations:

```ron
RuleSet(
    name: "Paint the Void",           // shown in the server log and the editor
    mode: PaintTheVoid((
        teams: 2,                     // must be 2
        match_secs: 300.0,            // match length in seconds
        respawn_secs: 3.0,            // wait after being destroyed
    )),
    weapons: {                        // availability per weapon; weapons not listed are banned
        "scrap_laser": LoadoutAndPickup,
        "scatter_laser": PickupOnly,
        // ... every other weapon ...
        "paint_thrower": LoadoutOnly,
        "tectonic_charge": PickupOnly,
    },
    gadgets: {                        // availability per gadget; gadgets not listed are banned
        "shield": LoadoutAndPickup,
        "cloak": PickupOnly,
        "decoy": PickupOnly,
    },
    starting_loadout: (
        primaries: ["scrap_laser", "paint_thrower"],
        secondaries: [("bolt_rocket", 3)],   // weapon and starting count
        gadget: Some("shield"),              // or None
    ),
    pickups: {                        // weapon pickups: spawn weight, respawn time, ammo given
        "scatter_laser": (weight: 1.0, respawn_secs: 30.0),
        "bolt_rocket": (weight: 2.0, respawn_secs: 20.0, ammo: 3),
        "tectonic_charge": (weight: 0.25, respawn_secs: 90.0, ammo: 1, max_on_map: 1),
        // ...
    },
    upgrade_pickups: {                // weapon level pickups (only the Scrap Laser has levels)
        "scrap_laser": (weight: 1.5, respawn_secs: 30.0),
    },
    gadget_pickups: {
        "shield": (weight: 0.5, respawn_secs: 45.0),
        "cloak": (weight: 0.5, respawn_secs: 45.0),
        "decoy": (weight: 0.5, respawn_secs: 45.0),
    },
    scrap_pickup: Some((weight: 12.0, respawn_secs: 15.0, amount: 30.0)),   // or None
    shield_overrides: {               // replace a weapon's Shield setting
        // "railspike": PassThrough(0.5),
    },
    tuning: {                         // multipliers per weapon (1.0 = unchanged)
        // "heavy_rocket": (damage: 0.8, fire_rate: 1.0, ammo: 1.0),
    },
    friendly_fire_percent: 50,        // 0 = off, otherwise 10 to 100
    allowed_ship_types: [Skitter, Tinker, Hauler, Tug],
    absurdity: 20,                    // 0 to 100; no effect yet
)
```

Availability values:

| Value | Meaning |
| --- | --- |
| `LoadoutAndPickup` | Players can spawn with it (if it's in the starting loadout) and find it, its ammo or upgrades as pickups |
| `PickupOnly` | Only found as a pickup |
| `LoadoutOnly` | Only given at spawn, never spawns as a pickup |
| `Banned` | Not available (same as leaving it out) |

Other fields:

- **Pickups:** each of the level's 12 pickup points rolls its content from all the weights
  together (weapon, upgrade and gadget pickups plus scrap piles). `respawn_secs` is the wait after
  the item was taken (scrap piles: after they are emptied). `ammo` is what an ordnance pickup
  gives. `max_on_map` caps how many of that pickup can be out at once (0 = no cap).
- **Shield settings:** `Blocked` (the Shield pays with energy, the hull never takes damage),
  `Wears(f)` (the Shield pays f × the damage in energy; what it can't cover reaches the hull) or
  `PassThrough(f)` (the share f, 0 to 1, reaches the hull). Each weapon's own setting is in the
  [gadget reference](reference/gadgets.md#shield-and-weapons).
- **Tuning:** `damage` multiplies damage and knockback, `fire_rate` multiplies shots per second,
  `ammo` multiplies ordnance counts (loadout, pickups and the carry limit). Rule set codes store
  multipliers in twentieths (0.05 steps).
- **Ship types:** `allowed_ship_types` lists which of `Skitter`, `Tinker`, `Hauler`, `Tug` players
  may fly. A player who picks another type gets the Tinker or the first allowed type. Secondaries
  in the loadout beyond a ship's slots (Skitter 1, Tinker and Tug 2, Hauler 3) are left out for
  that ship.

Rules the server checks at startup (it refuses to start and names the problem otherwise), for
example:

- A loadout weapon or gadget must allow the loadout (`LoadoutAndPickup` or `LoadoutOnly`) and a
  loadout weapon must be in the right slot (primary or secondary), listed once.
- A weapon, upgrade or gadget pickup must allow pickups (`LoadoutAndPickup` or `PickupOnly`), and
  upgrade pickups only exist for weapons with levels.
- Weights and respawn times can't be negative; tuning needs damage ≥ 0, fire rate > 0, ammo ≥ 0;
  Shield fractions are 0 to 1.
- `friendly_fire_percent` is 0 or 10 to 100, `absurdity` is 0 to 100, at least one ship type is
  allowed, the mode has 2 teams, `match_secs` is above 0. `mutators` must be empty (none exist
  yet).

### Weapon, gadget and ship ids

| Family | Ids |
| --- | --- |
| Energy primaries | `scrap_laser`, `scatter_laser`, `plasma_repeater`, `fusion_charger`, `corkscrew`, `ricochet_arc`, `leech_arc` |
| Kinetic primaries | `rivet_gun`, `railspike`, `scrap_shotgun` |
| Ordnance (secondaries) | `bolt_rocket`, `seeker`, `quicksilver`, `eyeball`, `swarm_pod`, `heavy_rocket`, `flash_pod`, `prox_mine`, `stalker_mine`, `tectonic_charge` |
| Specials | `echo_cannon` (primary), `swap_harpoon` (secondary), `paint_thrower` (primary), `borrowed_time` (secondary), `magnet_sneeze` (secondary), `polite_laser` (primary), `seed_grenade` (secondary) |
| Gadgets | `shield`, `cloak`, `decoy` |
| Ship types | `Skitter`, `Tinker`, `Hauler`, `Tug` |

What each weapon does and all its numbers: [player manual](player-manual.md#weapons) and
[weapon reference](reference/weapons.md). **Weapon stats** (damage, speed and so on) are built
into both the server and the web client; changing `data/weapons/*.ron` means rebuilding both and
having players reload. To change a weapon for one server, use `tuning` and `shield_overrides` in
the rule set or a balance list instead.

## Rule set codes, the editor and invite links

### Rule set codes

Every rule set fits into a **rule set code**: `LR` plus letters and digits in groups of four.
Codes store only what differs from the default rules of their mode, so they stay short: the
default Paint the Void rules are `LR8V-6097-G000-TMBG`, and a ban adds about three symbols.
Codes made by older versions (they start with `LR6`) still work. Codes are
case-insensitive, and dashes and spaces are optional. `--ruleset` accepts a code wherever it
accepts a file.

The server logs the code of the rules it actually plays (the rule set with the balance list and
bans applied) at startup, and again whenever a changed balance list changes them:

```
INFO ls_server::sim: rules: Paint the Void LR8V-6097-G084-VMY0-6DXC (banned weapons: eyeball, tectonic_charge)
```

Paste that code into `--ruleset` on another server, or into the editor, to get the same rules.

Code errors stop the server at startup: a typo fails the checksum; a code made against a
different default rule set asks for the full code; a code from a **newer** build (with weapons
this build doesn't know) is refused. A code from an **older** build still loads, with the
weapons added since then banned.

### The rule set editor

The web client has a rule set editor: open the game page and click **Rule set editor** on the
start screen, or open `http://<server>/?editor=rules` directly. It edits every rule described
above (availability, loadout, pickups, the Tectonic Charge's per-map limit, tuning, Shield
settings, friendly fire, ship types, scrap piles, match and respawn times), lists anything the
server would refuse in red, and, once the rules are valid, shows:

- the **rule set code**,
- a **World** field (an optional share code),
- an **invite link** (`http://<server>/?world=LS…&rules=LR…`),
- the **host command**, for example `ls-server --code LS0F-143C-ZQW8-CCHA-V4M6-8001-8FAE --ruleset LR8V-…`.

Copy the host command, add your other flags, and start the server. The editor needs no GPU
and no game connection, and it remembers the last code it made. `?editor=rules&rules=LR…`
opens it with a code loaded; `&world=LS…` fills the World field.

### Invite links

An invite link points at a server and may carry a world, a rule set and a party:
`http://<server>/?join=<host:port>&code=<LSâ€¦>&rules=<LRâ€¦>&party=<P-â€¦>` (every part optional;
`world=` works like `code=`, `room=<R-â€¦>` joins a private room). Opening one and choosing
**Play online** joins that server:

- with a **party code**: the party's room (the public room, or its private custom match) and,
  in team modes, the team of a party mate where team balance allows;
- with a **world and/or rule set code but no party**: the server opens a private custom match
  with them (one of its `--private-rooms`) and the player joins it;
- with neither: the server's normal rotation.

The rule set editor's links (`?world=â€¦&rules=â€¦`) therefore start a custom match on the server
that serves the page. Hosts can still use the codes for their own server (`--code`, `--ruleset`,
or open the link with `&editor=rules` added to see and copy them).

## Banning and balancing weapons

Two server flags change weapons on top of any rule set, without editing it:

- **`--ban <WEAPON_ID>`** (repeatable) bans a weapon for the server's whole run: it never spawns,
  is taken out of the starting loadout and can't be fired. An unknown id stops the server at
  startup ("unknown weapon `laser`").
- **`--balance-list <FILE>`** loads a balance list, meant for a community or a group of servers
  that share one list: if a weapon turns out overpowered, edit the list once and every server
  using it picks the change up **at its next match**, without a restart.

A balance list file:

```ron
BalanceList(
    bans: ["tectonic_charge"],        // weapons banned server-wide
    gadget_bans: [],                  // gadgets banned server-wide ("shield", "cloak", "decoy")
    // Multiplies the rule set's own multipliers for these weapons (rounded to 0.05 steps):
    // a list that weakens a weapon weakens it whatever the rule set says.
    tuning: { "railspike": (damage: 0.8, fire_rate: 1.0, ammo: 1.0) },
    // Replaces the rule set's Shield setting for these weapons.
    shield_overrides: { "heavy_rocket": Wears(1.5) },
)
```

The rules the server plays are: the rule set, then the balance list, then the `--ban` flags,
checked at startup. The balance list is read again before every match; if it can't be read or is
invalid then, the server logs `could not reload the rules; keeping the previous ones` and plays
on with the previous rules. At startup a bad list stops the server.

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

## WebTransport

WebTransport runs the game connection over UDP (QUIC): time-critical updates travel as
datagrams, so a lost packet doesn't hold up the ones behind it the way it does on a TCP
WebSocket. It is on by default; `--no-webtransport` turns it off and `--wt-bind` moves it to
another UDP address or port.

**Who uses it:** browsers with WebTransport support on a **secure page** (`localhost` or
`https://`). A page opened as `http://<LAN-IP>:7777` has no WebTransport and falls back to
WebSocket (unless the players use one of the workarounds in
[Playing across a network](#playing-across-a-network)). Bots use it with `ls-bots --transport wt`.

**Certificate:** WebTransport always runs over encrypted QUIC and needs a TLS certificate. You
don't need to provide one:

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

**Checking it:** the F3 overlay in the browser shows "WebTransport" or "WebSocket", and the
server's `player joined` log line has `transport=…`.

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
   local network: allow it. Their client must be built from the same version as your server.
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

`ls-bots` starts headless bot players that join over the network like real players. Each bot
picks a random ship type (among the ones the rules allow), navigates the whole level to find
enemies, fights with whatever it carries (primaries by range, secondaries by situation: rockets,
homing missiles at fleeing targets, mines when chased, a guided Eyeball, Borrowed Time when its
hull is low, the Tectonic Charge at groups of enemies), uses its gadget (Shield, Cloak, Decoy),
flees Tectonic Charge zones, collects pickups (also mid-fight when one is close), paints and goes
scavenging when its hull is low. Bots appear in the game as regular pilots and take up player
slots.

Bots play every mode for its objective: in The Squeeze they stay inside the safe zone (flying
to the next zone as soon as it is announced); in Heist about half of them head for the core and
dig it out with rockets, a free core is grabbed, the carrier flies it home while its team
escorts it, and the other team hunts the carrier; in Hot Reactor the holder rams the nearest
ship and everyone else keeps away from the holder and never shoots it; in Last Color Standing
they go for the most colourful players first. An eliminated bot spectates until the next match.
In the lobby the bots press Ready, so with only bots on the server the countdown starts at once.

```sh
# 7 bots on a local server until Ctrl+C
ls-bots

# 4 bots against another machine, for 10 minutes
ls-bots --server ws://192.168.1.20:7777/ws --count 4 --duration 600

# bots over WebTransport
ls-bots --transport wt

# bots that find a server through a directory and accept duel challenges
ls-bots --directory http://127.0.0.1:7780 --count 4 --accept-duels
```

Verified against `ls-bots --help` of this build:

| Flag | Default | What it does |
| --- | --- | --- |
| `--server <URL>` | `ws://127.0.0.1:7777/ws` | WebSocket address of the server. With `--transport wt`, its host and port are used to fetch `/cert-hash`. |
| `--transport <ws\|wt>` | `ws` | Connect over WebSocket or WebTransport |
| `--count <N>` | `7` | Number of bots |
| `--duration <SECONDS>` | run until Ctrl+C | Stop after this many seconds |
| `--stagger-ms <MS>` | `750` | Delay between two bots joining |
| `--seed <N>` | `1` | Base seed for the bots' behaviour (different seeds, different flying and ship choices) |
| `--name-prefix <TEXT>` | `bot` | Bot names are this plus a number (`bot1`, `bot2`, …). Players see the names in game. |
| `--directory <URL>` | none | Pick the server from this directory (`http://host:port`) instead of `--server`: among the servers of the same build with a free slot, the fullest |
| `--room <CODE>` | public room | Join this private room (`R-...`) |
| `--accept-duels` | off | Accept duel challenges: the bots appear in the server's player list, accept every challenge, play the duel in the challenger's private room and come back afterwards |
| `--no-reconnect` | off | Don't reconnect after losing the connection |
| `--allow-disconnects` | off | A lost connection doesn't count as a failure (exit code 3) |
| `--verbose` | off | Log debug output |

Plan the numbers: `--count` plus the human players must fit in `--max-players`. Extra bots are
refused with "room is full", and the run ends with exit code 2. The bots rejoin by themselves
when a match restarts. Bots built from a different version than the server are refused with a
data mismatch.

When the bots stop, they print a summary table (with shots and hits per weapon) and a result
line. The exit code is:

| Code | Meaning |
| --- | --- |
| 0 | All good (match restarts are fine) |
| 1 | A bot saw a protocol error or lost sync with the server |
| 2 | A bot never managed to join (server not running, wrong address, server full, different build) |
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
| `match ready room=public entry=… share_code=… ruleset=… warmup=… build_ms=…` | The first room is built (`warmup=true`: a warm-up room). Note the share code if you want to keep the level. |
| `rules: <name> LR… (banned weapons: …)` | The rules the server plays (rule set, balance list and bans applied), with their rule set code |
| `rules changed (balance list): <name> LR… (banned weapons: …)` | The balance list changed the rules for the next match |
| `webtransport listening … port=… cert_sha256=…` | WebTransport is up, with its certificate hash |
| `listening (http + websocket at /ws) … rules_code=… max_players=…` | The server accepts players |
| `share code … - open http://… to play` | The address to open in a browser |
| `player joined conn=… player=… team=… name=… transport=… players=…` | A player joined (team 0 is Orange, 1 is Blue) |
| `join rejected … reason=ServerFull` | The server was full. Other reasons: `DataMismatch` (the browser or bot was built from different game data), a version mismatch, or a bad handshake. |
| `player left … reason=…` | A player left, timed out or was removed |
| `tick stats …` | Performance and match counters, every `--stats-interval` seconds (see below) |
| `countdown started …` / `countdown cancelled …` | The lobby counts down to the match (or stopped because players left) |
| `player count does not fit; warm-up on another playlist entry` | Too few or too many players for the next playlist entry; the room switches to another one |
| `match over; building the next room … next=… next_share_code=…` | The match ended; the result shows while the next room is built |
| `next room swapped in room=… warmup=… entry=… share_code=… players=… matches=…` | Everyone moved to the next room (warm-up or match) without reconnecting |
| `private room open room=R-…` / `private room closed` | A private room started or closed (its last player left) |
| `final stats …` | Totals at shutdown |

Warnings to watch:

| Warning | Meaning and what to do |
| --- | --- |
| `tick p99 over budget` | The slowest 1% of ticks took over 4 ms. Occasional warnings are harmless (a Tectonic Charge collapse costs one slow tick). Constant ones mean the machine is too slow or busy: check `recv_max_us` and `send_max_us` in the tick stats. Use a release build and close other heavy programs. |
| `sim fell behind; skipped ticks to catch up` | The simulation couldn't keep up (the computer was busy or asleep). Players may see a hitch. |
| `input starvation: clients will mispredict (desync risk)` | Players' inputs arrived too rarely, usually because of a bad connection. Their ships may jump or feel laggy. |
| `could not reload the rules; keeping the previous ones` | The balance list couldn't be read or is invalid at a match start. Fix the file; the next match reads it again. |
| `player not accepted by the next room` | A player could not move to the next match (for example a smaller `max_players`); they are disconnected with the reason |
| `web root not found; serving a placeholder page` | The `--web-root` folder is missing. Build the web client or fix the path. |

### Tick stats fields

Every room writes its own `tick stats` line.

| Field | Meaning |
| --- | --- |
| `room` | `public` or the private room's code |
| `phase` | Lobby phase: `warmup`, `countdown`, `match` or `results` |
| `warmup` | Whether the room is a warm-up room (weapons off) |
| `tick` | Tick of the current match (60 per second) |
| `total_ticks` | Ticks over all matches since startup |
| `players` | Connected players |
| `ticks` | Ticks simulated in this window |
| `p50_us`, `p99_us`, `max_us` | Median, 99th percentile and slowest tick time in microseconds. Budget: 4000 (4 ms). Healthy servers stay far below it. |
| `recv_max_us`, `step_max_us`, `send_max_us` | Slowest time of each part of a tick in this window: reading commands and packets, simulating the game, sending. If a slow tick shows up in the reading or sending part (normally a few microseconds), the computer was busy with something else, not the game. |
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
| `edits`, `total_edits` | Terrain changes (paint, craters, surface states, grown rock) in this match, and since startup |
| `craters` | Craters in this match |
| `debris`, `debris_peak` | Loose debris pieces now and the most in this window (at most 64) |
| `paint_cells_0`, `paint_cells_1`, `surface_cells` | Surface painted by Orange and Blue, and the level's total paintable surface. The score is `paint_cells / surface_cells`. |
| `match_left_s` | Seconds left in the match |
| `match_over` | Whether the match has ended |
| `matches` | Matches started since startup, including the current one (warm-ups don't count) |
| `desync_warnings` | Input starvation warnings since startup |

The `final stats` line at shutdown adds totals: `connections`, `rejected`, `kicked`
(players removed for protocol errors or a connection that fell too far behind), `timeouts`,
`skipped_ticks`, `snapshots_sent`, `snapshot_bytes`, `edits`, `matches` and
`desync_warnings`.

**Bandwidth:** each player receives up to about 17 KB/s. In very busy fights with 32 players and
many shots in flight, single snapshots can run a little over that; `snapshot_kib_s` shows the
total for all players.

## Lobby, match end and rotation

Every match runs through the lobby:

1. **Warm-up.** Players fly the next match's level with weapons and gadgets off. When enough
   players for the next playlist entry are there and either everyone pressed ready or
   `--warmup` seconds (default 30) have passed, the countdown starts. If the warm-up time
   passes with too few or too many players for that entry, the server switches to another entry
   that fits (a new warm-up on its level); if none fits better, the entry is played anyway.
2. **Countdown** (`--countdown`, default 10 s). If players leave so the count no longer fits,
   the countdown stops and the warm-up continues.
3. **Match.** A fresh copy of the level (no warm-up craters), with the real rules.
4. **Result** (`--intermission`, default 10 s). Meanwhile the server builds the next
   playlist entry's level in the background (the balance list is read again here).
5. Everyone moves to the next warm-up **on the same connection**: nobody is disconnected and
   nobody waits for a level to build, players who join during the result join right away.

`--warmup 0 --countdown 0` turns the lobby off: a match starts as soon as its first player
joins, and after the result the next match starts straight away.

If a match ends while nobody is connected, the server waits. To change bans and tuning between
matches, edit the balance list; to change the rotation, restart the server with another
playlist.

### Playlists

Without `--playlist` and `--ruleset`, the server rotates through the default playlist: Team
Brawl (4 players or more), The Squeeze, Paint the Void, Hot Reactor, Heist and Last Color
Standing, each on a new random world (or on `--code`'s world). Modes whose rule set isn't in
this build yet are left out. `--ruleset` plays one rule set on one world in every match.
A team Heist with fewer than 4 players plays its free-for-all variant (every extraction zone
counts for everyone).

A playlist file lists the matches in order (RON, like rule sets):

```ron
Playlist(
    entries: [
        // A built-in rule set by id, a random world, 4 to 32 players.
        (rules: "team_brawl", min_players: 4),
        // Your own rule set file (relative to the playlist file) on a fixed world.
        (rules: "my_rules.ron", world: Code("LS0C-0000-0000-0000-8006-8001-8F9M")),
        // A rule set code from the editor, a random world with slider ranges, at most 8 players.
        (rules: "LR6V-69CM-G006-58R", world: RandomSliders(enclosure: (20, 60), density: (30, 70)), max_players: 8),
    ],
)
```

- `rules`: a built-in rule set id (the file name in `data/rulesets/` without `.ron`, for
  example `team_brawl`, `instagib_brawl` (Team Brawl with the Instagib mutator) or `duel`
  (free-for-all, first to 10 kills)), a rule set file or an `LR…` code. `--ban` and `--balance-list` apply to every entry.
- `world`: `Random` (default: a new random world with the default sliders every time),
  `RandomSliders(enclosure: (low, high), density: (low, high))` or `Code("LS…")`.
- `min_players` (default 1) and `max_players` (default 32): the entry is skipped while the
  player count is outside this range. If no entry fits, a Team Brawl entry is played (with 2
  teams from 4 players on, free-for-all below), otherwise the next entry anyway.

The server checks the whole playlist at startup and refuses to start if an entry is invalid.

### Private rooms

Next to the public room, the server runs up to `--private-rooms` private rooms (default 4) for
duels, parties and custom matches, each with its own code (`R-` and six characters) and its
own simulation thread. A private room closes when its last player leaves (or a minute after it
was opened if nobody came). Players reach one through an invite link, a party code or an
accepted duel; a client asking for an unknown or closed room code gets "unknown room". When all
private rooms are in use, new duels and custom matches are refused until one closes.

### Parties and duels

Players manage these in the browser ("Party & duels" button in game); the server needs no
setup beyond `--private-rooms`:

- **Party codes** (`P-` and six characters) are made by the server for a player and live on
  that server only, while a member is connected and for 10 minutes after the last one left.
  Members join the leader's room and, in team modes, the leader's team when team sizes stay
  within one of each other (otherwise the party is split).
- **Duels**: a player challenges another player on the server (or, through the directory, on
  another server). The challenged player has 30 seconds to accept; then the challenger's server
  opens a private two-player room on the challenger's world with the duel rules (free-for-all
  Team Brawl, first to 10 kills or 5 minutes, unless the challenger gave a rule set code) and
  both players move there.
- The browser talks to the server's small JSON API at `/api/â€¦` for this (and for the server
  list's player names). Players are known by a random id their browser keeps; there are no
  accounts.

## Stopping the server

Press **Ctrl+C**. The server sends every player a "server shutting down" message, closes the
connections, logs `final stats` and exits. Players see a "Disconnected" screen. A server
stopped with a hard kill (closing the terminal window, Task Manager, `kill -9`) doesn't write
`final stats`.

## Troubleshooting

| Problem | What to check |
| --- | --- |
| Server exits at startup with "invalid match configuration" | The reason follows: a bad share code, a rule set file that's missing or invalid, a rule set code with a typo ("rule set code: …"), an unknown weapon in `--ban` ("unknown weapon `laser`"), or a balance list problem. The message names the file and the problem. |
| `--ruleset LR…` says "I/O error reading …" | The text didn't look like a code, so the server looked for a file. Check the code (it starts with `LR`, letters and digits only apart from dashes). |
| "server failed to start", address in use | Another program (or another `ls-server`) already uses the port. Pick another port with `--bind` (and `--wt-bind`), or stop the other program. TCP and UDP both have to be free. |
| Browser shows "ls-server is running…" placeholder page | The web client wasn't found. Build it (`cd web && npm run build`) and start the server from the repository root, or pass `--web-root <path to web/dist>`. |
| Players get "game data mismatch" | The web client (or bots) and the server were built from different game data. Rebuild both from the same version, restart, and have players reload with Ctrl+F5. |
| Players elsewhere get "WebGPU unavailable" | The page isn't a secure context over `http://<IP>`. See [Playing across a network](#playing-across-a-network). |
| Players can't connect at all | Firewall or router (TCP port). Test from another computer with `http://<server-ip>:7777/cert-hash` in a browser: it should show JSON (or a 404 page with `--no-webtransport`). No answer at all means the connection is blocked. |
| Players only get WebSocket (F3 shows WebSocket) | UDP blocked by a firewall or router, `--no-webtransport` set, the page isn't a secure context, or the browser has no WebTransport. The game still works. |
| "room is full" | Raise `--max-players` (up to 32) or run fewer bots. |
| "Lost sync with the server" in browsers | The web client and the server come from different builds. Rebuild both, restart, and have players reload the page. |
| A weapon still shows up after a ban | Bans on the command line need a restart; balance list bans apply from the next match. The `rules:` line at startup (and `rules changed (balance list):` later) lists the banned weapons. |
| Players time out when their tab is hidden | Browsers slow down background tabs to about one update per second, which still keeps the connection alive. A computer going to sleep drops the connection. |
| Stuttering, `tick p99 over budget` warnings | Use a release build, and check CPU load on the host. With `RUST_LOG=debug` the log itself adds load. |
| Nobody gets to the next match | See the log for "could not build the next room; trying again". Players can reload the page to rejoin. |

## Server directory (optional)

`ls-directory` is a small, separate service that collects game servers for the in-game server
list. You don't need it: players can always join a server by its link or address. Run one if you
want a server list for your community:

```sh
cargo run -p ls-directory --release -- --bind 0.0.0.0:7780
```

It keeps everything in memory (a restart forgets the list; servers re-register within 10 s) and
speaks plain HTTP on TCP port 7780. Browsers on an `https://` page need it behind an HTTPS reverse
proxy.

| Flag | Default | Meaning |
| --- | --- | --- |
| `--bind` | `0.0.0.0:7780` | Address and TCP port to listen on |
| `--ttl` | 30 | Seconds a server stays listed after its last heartbeat |
| `--heartbeat` | 10 | How often servers are asked to check in (seconds) |
| `--max-servers` | 1000 | Most listed servers |
| `--invite-ttl` | 30 | Seconds a relayed duel invite stays available |
| `--max-invites` | 10000 | Most pending invites |
| `--rate` / `--burst` | 10 / 40 | Requests per second (and burst) per client IP; raise them behind a reverse proxy |

`http://<host>:7780/servers` shows the current list as JSON.

Game servers join the list with `--directory`:

```sh
ls-server --directory http://dir.example.com:7780 --server-name "Scrap Den" --region eu-west \
    --public-address play.example.com:7777
```

They check in every 10 seconds (a server that stops checking in disappears after 30 seconds)
and leave the list when stopped with Ctrl+C. The list shows the name, region, current mode,
world, player count and the players' names, and the browser measures the ping to each server
itself. Players open it with the **Server list** link on the start screen; the page finds the
directory through the server that served it (its `--directory`), or from `?directory=<url>` in
the page address. Only servers of the same game version are listed. A duel challenge to a
player on another listed server is passed on by the directory.

## Not in this build yet

- **Modes arrive one by one:** the default rotation only holds the modes whose rule sets are in
  this build; rule sets can't enable mutators yet.
- **No accounts or friends lists.** Players are known by a random id their browser keeps;
  parties exist on one server only. A public directory is not run for you: host your own
  `ls-directory` (behind an HTTPS proxy for `https://` pages).
- **No admin commands** while running (kick, change world or rule set, end match). Restart the
  server with new flags; bans and tuning can change between matches through the balance list.
- **No slider tool:** you can't create a share code with chosen slider values yet, and only
  Enclosure, Density, Instability and (visually) Palette have an effect. The rule set's `absurdity` value has no
  effect yet.
- **No custom TLS certificate** for WebTransport and no built-in HTTPS for the web page.
- **No WebRTC fallback, desktop app, regional hosting or orchestration** (the design mentions
  Agones or Nomad for later).
