# Loose Screws: hosting guide

How to run a Loose Screws game server (`ls-server`), set its world and rules, fill it with bots
(`ls-bots`), run a server directory (`ls-directory`) and keep it all healthy. It covers the
current build (Milestone 3: every player-versus-player mode, a lobby and match rotation,
private rooms, parties, duels and the server list). Features from the design that the server
doesn't have yet are listed under [Not in this build yet](#not-in-this-build-yet).

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
- [Lobby, match end and rotation](#lobby-match-end-and-rotation) ([playlists](#playlists), [private rooms](#private-rooms), [parties and duels](#parties-and-duels))
- [Stopping the server](#stopping-the-server)
- [Troubleshooting](#troubleshooting)
- [Server directory (optional)](#server-directory-optional) ([HTTPS and a public directory](#https-and-a-public-directory))
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
  result; then the next match of the rotation (by default every mode in turn), with players
  staying connected
- opens private rooms for custom matches, parties and duels on request, and serves a small
  JSON API for them at `/api/…`
- optionally lists itself in a **server directory** (`--directory`) so players find it in the
  in-game server list

Players join by opening your address, by an invite link, or from the server list. There are no
accounts.

## Building

You build the server, the bots and the web client from the source repository. You need:

- Rust (stable). The repository's `rust-toolchain.toml` installs the right toolchain and the
  `wasm32-unknown-unknown` target when you first run `cargo`. The server and bots need Rust
  1.88 or newer.
- `wasm-bindgen` command line tool, version **0.2.129** exactly:
  `cargo install wasm-bindgen-cli --version 0.2.129`
- Node.js with npm

From the repository root:

```sh
cd web
npm install
npm run build          # builds the game core for the browser, then the web client into web/dist
cd ..
cargo build --release -p ls-server -p ls-bots -p ls-directory
```

The programs end up in `target/release/`: `ls-server`, `ls-bots` and `ls-directory`
(`ls-server.exe` and so on, on Windows). You only need `ls-directory` for a server list.

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
`target\release\ls-server.exe`). The server starts the default rotation of all six modes on
random worlds, beginning with a warm-up for Team Brawl, and prints (shortened):

```
INFO ls_server::sim: match ready room=public entry=team_brawl share_code=LS0G-APFE-9ZGE-BSJN-5C58-T2XM-57KF ruleset=Team Brawl warmup=true build_ms=496
INFO ls_server::sim: rules: Team Brawl LRAV-7439-JH00-07NT-8 (banned weapons: paint_thrower) room=public
INFO ls_server: share code LS0G-APFE-9ZGE-BSJN-5C58-T2XM-57KF - open http://localhost:7777 to play
```

("banned weapons: paint_thrower" is normal: only Paint the Void hands out the Paint Thrower.)

Open `http://localhost:7777`, enter a pilot name, pick a ship and click **Play online**. You
arrive in the warm-up. Team Brawl needs 4 players: alone, the warm-up runs its 30 seconds and
the server moves on to a mode that fits one player (Paint the Void). To add six bots so there's
someone to play against, open a second terminal:

```sh
target/release/ls-bots --count 6
```

With 7 players the rotation starts with Team Brawl. The bots press Ready at once, so the
countdown starts as soon as you press **Enter**. Press **Ctrl+C** in the server terminal to stop
it.

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
| `--ruleset <FILE\|CODE\|ID>` | the default rotation | A rule set file (RON), a built-in rule set id (`team_brawl`, `squeeze`, `heist`, `hot_reactor`, `last_color_standing`, `paint_the_void`, `instagib_brawl`, `duel`, `sandbox_all_weapons`) or an `LR…` rule set code (see [Rule sets](#rule-sets)), played in every match on one world. An existing file wins, then built-in ids; text that looks like a code is decoded. Read once at startup. Can't be combined with `--playlist`. |
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
| `--directory <URL>` | none | Register with a server directory (`http://host:port`, optionally with a path; plain `http://` only, see [Server directory](#server-directory-optional)) so the server shows up in the in-game server list. Without it the server is reachable by link or address only. |
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
ls-server --code LS0G-C4GD-KYZH-1HJ5-BC58-T2XM-57NT

# Every weapon and gadget as a pickup (the built-in sandbox rules)
ls-server --ruleset data/rulesets/sandbox_all_weapons.ron

# Rules from the rule set editor, without the Tectonic Charge
ls-server --ruleset LRAV-741Q-J900-01HQ-M --ban tectonic_charge

# Your own rule set file, a server-wide balance list and a longer break between matches
ls-server --ruleset my_rules.ron --balance-list balance.ron --intermission 20

# Only reachable from this computer, WebSocket only, on another port
ls-server --bind 127.0.0.1:8080 --no-webtransport

# WebTransport on a separate UDP port
ls-server --bind 0.0.0.0:7777 --wt-bind 0.0.0.0:7778

# One mode only: Instagib Brawl in every match, 16 players
ls-server --ruleset instagib_brawl --max-players 16

# Your own rotation with short warm-ups, and room for 8 duels or custom matches
ls-server --playlist playlist.ron --warmup 15 --countdown 5 --private-rooms 8

# The default rotation, listed in your directory under a name
ls-server --directory http://192.168.1.20:7780 --server-name "Scrap Den" --region eu-west \
    --public-address 192.168.1.20:7777
```

## Choosing the world: share codes

A share code such as `LS0G-C4GD-KYZH-1HJ5-BC58-T2XM-57NT` rebuilds exactly the same world on the
server and in every browser. It stores a random seed, the seven world sliders, the kind of
level (a classic level or an asteroid belt) and the world generator version. This build's
generator (version 4) makes codes starting `LS0G` and carves a network of caverns and tunnels
that links both teams' ends into every classic level, plus a cavern at each of the other two
ends for teams 3 and 4, the Heist core vein at the bottom and the extraction caverns at the
top (every classic level has them; only Heist uses them). Of the sliders, **Enclosure** and
**Density** shape the world, **Verticality** picks the level's shape (from 320 x 64 x 320 u,
flat and wide, to 192 x 224 x 192 u, tall and narrow, with the caverns stretched to match),
**Gravity Chaos** places up to seven gravity wells (rock cores in hollows; their pull arrives
in a later build), **Instability** sets how far a Tectonic Charge's cracks spread, and
**Palette** only changes the look in the browser (desaturated colours); Absurdity has no
effect yet. An **asteroid belt** level is a ring of asteroids around an open centre, the same
from every one of its 8 start slots, made for the Belt Up racing mode.

- **Keep a level:** start with `--code <code>`. Without `--code`, every server start builds a
  new random world. The server logs the code at startup ("match ready share_code=…").
- **Find a level you like:** in the web client, click **Practice offline** with an empty share
  code to fly a random world, and press **F3**. The last line of the debug overlay shows the
  world's share code. Try as many worlds as you like, then start the server with the code of
  your favourite.
- **Invalid codes** stop the server at startup with a reason, for example
  `share code checksum mismatch (typo?)`,
  `share code must have 26 symbols after the prefix, found 5`, or a message that the code was
  made by a different generator version. Codes of earlier generator versions (Milestone 1
  `LS04…`, Milestone 3 `LS0C…` to `LS0F…`) no longer load.
- Players always play the world the server runs. A code a player types on the start screen is
  only used for practice mode.

There's no tool yet for making a code with chosen slider values.

## Rule sets

A rule set decides the match length, respawn time, which weapons, gadgets and ship types are
allowed, the starting loadout, the pickups, per-weapon tuning, each weapon's Shield setting and
friendly fire. The server sends its rules to every browser when it joins, so players don't need
to rebuild or download anything.

The mode is part of the rule set: Team Brawl, The Squeeze, Heist, Hot Reactor, Last Color
Standing and Paint the Void each have their own. You can give the server its rules these ways:

1. **Nothing:** the default rotation of the six modes' built-in rule sets (see
   [Playlists](#playlists)).
2. **A built-in rule set by id:** `--ruleset team_brawl`, played in every match.
3. **A rule set code** made in the [rule set editor](#rule-set-codes-the-editor-and-invite-links):
   `--ruleset LRAV-741Q-J900-01HQ-M`. Nothing to copy around.
4. **A rule set file** (RON): `--ruleset my_rules.ron`. Copy one of the built-in files, edit
   the copy and pass its path.
5. **Several of them in turn:** a playlist file, `--playlist playlist.ron` (see
   [Playlists](#playlists)).

Built-in rule sets (ids are the file names in `data/rulesets/` without `.ron`; every number is in
the [mode reference](reference/modes.md)):

| Id | What it is |
| --- | --- |
| `team_brawl` | Team Brawl: 2 teams, kill target from the player count (15 to 50), 10 minutes |
| `squeeze` | The Squeeze: free-for-all, one life, five wall phases |
| `heist` | Heist: 2 teams (free-for-all for 1 to 3 players), first to 3 deliveries, 12 minutes |
| `hot_reactor` | Hot Reactor: free-for-all, 3 lives, 20 s fuse |
| `last_color_standing` | Last Color Standing: free-for-all, 100 colour, 34 per death, stalemate drain after 8 minutes |
| `paint_the_void` | Paint the Void: 2 teams, 5 minutes; the only rule set with the Paint Thrower |
| `instagib_brawl` | Team Brawl with the Instagib mutator: Railspike only, any hit kills |
| `duel` | The duel rules: free-for-all Team Brawl, first to 10 kills or 5 minutes |
| `sandbox_all_weapons` | Paint the Void with every weapon and gadget as a pickup at even weights and 15 s respawns (Tectonic Charge 60 s, 1 per map), so a 5-minute match sees the whole arsenal |

All but Instagib Brawl start with the Scrap Laser, 3 Bolt Rockets and a Shield, have every other
weapon and gadget as a pickup, the Tectonic Charge 1 per map with a 90 s respawn, all four ship
types, and friendly fire 50% (off in the free-for-all-only modes).

### The rule set file

The Paint the Void rule set, shortened, with explanations:

```ron
RuleSet(
    name: "Paint the Void",           // shown in the server log and the editor
    mode: PaintTheVoid((
        teams: 2,                     // 2 to 4
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
    // Pigment pickups on monochrome levels (Palette 80+): one per 4 players, at least 2; or None
    pigment_pickup: Some((respawn_secs: 30, players_per_pickup: 4, min_on_map: 2)),
    shield_overrides: {               // replace a weapon's Shield setting
        // "railspike": PassThrough(0.5),
    },
    tuning: {                         // multipliers per weapon (1.0 = unchanged)
        // "heavy_rocket": (damage: 0.8, fire_rate: 1.0, ammo: 1.0),
    },
    friendly_fire_percent: 50,        // 0 = off, otherwise 10 to 100
    allowed_ship_types: [Skitter, Tinker, Hauler, Tug],
    absurdity: 100,                   // cap of the comedy events: 0 = none, 100 = the world decides
    // mutators: [Instagib],          // optional: mutators on top of the mode
    // events: { "duck_parade": Banned, "cat_paw": Banned },   // optional: comedy events never rolled
)
```

Other modes have their own `mode:` block with the mode's settings, for example:

```ron
    mode: TeamBrawl((
        teams: 2,                     // 0 = free-for-all, or 2 to 4
        match_secs: 600.0,
        respawn_secs: 3.0,
        kill_target: 0,               // 0 = max(15, 5 x players / teams), at most 50
        team_kill_penalty: 1,         // points a suicide or team kill costs
    )),
```

The mode names are `PaintTheVoid`, `TeamBrawl`, `TheSqueeze`, `Heist`, `HotReactor` and
`LastColorStanding`; every setting of each is listed and explained in the
[mode reference](reference/modes.md). Leaving a setting out takes its default. The only mutator
is `Instagib`: it reduces any rule set to the Railspike (loadout only, free to fire), removes
secondaries, gadgets and pickups, makes any enemy hit kill and respawns instant.

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
  allowed, `match_secs` is above 0, `respawn_secs` and every mode setting at least 0, a mutator
  is listed once.
- The team count fits the mode: Paint the Void 2 to 4; Team Brawl 0 (free-for-all) or 2 to 4;
  Heist 0 or 2; The Squeeze, Hot Reactor and Last Color Standing 0 only.
- Mode settings make sense, for example Heist needs at least 1 delivery and a carrier speed of
  1 to 100%, Hot Reactor at least 1 life and a fuse above 0, The Squeeze at least one phase.

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
Codes store only what differs from the default rules of their mode, so they stay short, and a
ban adds about three symbols. The built-in rules of each mode:

| Mode | Code of its built-in rules |
| --- | --- |
| Paint the Void | `LRAV-741Q-J900-01HQ-M` |
| Team Brawl | `LRAV-7439-JH00-07NT-8` |
| The Squeeze | `LRAV-7453-NW00-04CR-W` |
| Heist | `LRAV-746S-WW00-03DB-4` |
| Hot Reactor | `LRAV-749E-S600-040N-C` |
| Last Color Standing | `LRAV-74AE-DC00-00PS-C` |

The editor has no mode picker yet: to edit another mode's rules, open it with that mode's code
(`?editor=rules&rules=LRAV-7439-JH00-07NT-8`); it edits weapons, pickups, tuning and times, not
the mode's own settings (write a rule set file for those).
Codes made by older versions (they start with `LR6` or `LR8`) still work; they mean what they
meant then, so an older code keeps the Absurdity cap of 20 the built-in rules had before comedy
events existed. Codes are
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
- the **host command**, for example `ls-server --code LS0G-C4GD-KYZH-1HJ5-BC58-T2XM-57NT --ruleset LR8V-…`.

Copy the host command, add your other flags, and start the server. The editor needs no GPU
and no game connection, and it remembers the last code it made. `?editor=rules&rules=LR…`
opens it with a code loaded; `&world=LS…` fills the World field.

### Invite links

An invite link points at a server and may carry a world, a rule set and a party:
`http://<server>/?join=<host:port>&code=<LS…>&rules=<LR…>&party=<P-…>` (every part optional;
`world=` works like `code=`, `room=<R-…>` joins a private room). Opening one and choosing
**Play online** joins that server:

- with a **party code**: the party's room (the public room, or its private custom match) and,
  in team modes, the team of a party mate where team balance allows;
- with a **world and/or rule set code but no party**: the server opens a private custom match
  with them (one of its `--private-rooms`) and the player joins it;
- with neither: the server's normal rotation.

Custom matches and duels play the rule set code they were given as it is: your `--ban` flags
and balance list do **not** apply in private rooms in this build (only in the public rotation).
Keep that in mind before you give out a server to people you don't know; `--private-rooms 0`
turns custom matches and duels off (party invites into the public room still work).

The rule set editor's links (`?world=…&rules=…`) therefore start a custom match on the server
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
checked at startup, for every entry of the playlist. They don't reach private rooms (custom
matches and duels, see [Invite links](#invite-links)). The balance list is read again before every match; if it can't be read or is
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
3. **HTTPS in front of the server:** a reverse proxy with a real certificate serves the page over
   `https://` and forwards everything (including the WebSocket upgrade on `/ws`) to the server,
   so players get a secure page with no flags. See
   [HTTPS and a public directory](#https-and-a-public-directory).

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
refused with "room is full", and the run ends with exit code 2. The bots stay connected from
match to match (and rejoin by themselves on older servers that restart matches). Bots built from a different version than the server are refused with a
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
| `player joined conn=… player=… team=… name=… transport=… players=…` | A player joined (team 0 is Orange, 1 Blue, 2 Lime, 3 Magenta; in free-for-all modes every player has a side of their own) |
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
Brawl (4 players or more), The Squeeze (2 or more), Paint the Void (1 or more), Hot Reactor,
Heist and Last Color Standing (2 or more each), each on a new random world (or on `--code`'s
world). `--ruleset` plays one rule set on one world in every match.
A team Heist with fewer than 4 players plays its free-for-all variant (every extraction zone
counts for everyone).

A playlist file lists the matches in order (RON, like rule sets):

```ron
Playlist(
    entries: [
        // A built-in rule set by id, a random world, 4 to 32 players.
        (rules: "team_brawl", min_players: 4),
        // Your own rule set file (relative to the playlist file) on a fixed world.
        (rules: "my_rules.ron", world: Code("LS0G-0000-0000-0000-1001-6RFX-250V")),
        // A rule set code (here: Team Brawl with 3 teams, no Railspike), a random world with
        // slider ranges, at most 8 players.
        (rules: "LR8V-63T6-XB09-C1W0-0GG1-3038-2C", world: RandomSliders(enclosure: (20, 60), density: (30, 70)), max_players: 8),
    ],
)
```

- `rules`: a built-in rule set id (the file name in `data/rulesets/` without `.ron`, for
  example `team_brawl`, `instagib_brawl` (Team Brawl with the Instagib mutator) or `duel`
  (free-for-all, first to 10 kills)), a rule set file or an `LR…` code. `--ban` and `--balance-list` apply to every entry.
- `world`: `Random` (default: a new random world with the default sliders every time),
  `RandomSliders(...)` with a `(low, high)` range for any of the seven sliders (`enclosure`,
  `density`, `verticality`, `gravity_chaos`, `palette`, `absurdity`, `instability`) and
  optionally `archetype: Belt` for asteroid belts, or `Code("LS…")`. Sliders you leave out
  take the default ranges: for classic worlds Enclosure 50 and Density 40 (as before),
  Verticality 20-80, Gravity Chaos 0-60, Palette 0-50, Absurdity 10-60, Instability 10-60; for
  belts Enclosure 30-70, Density 40-80, Verticality 30-70, Gravity Chaos 0-50, Palette 0-50,
  Absurdity 10-60, Instability 10-40. `Random` (the default) is `RandomSliders()` with the
  classic ranges.
- `min_players` (default 1) and `max_players` (default 32): the entry is skipped while the
  player count is outside this range. If no entry fits, the playlist's first `team_brawl` entry is played (with 2
  teams from 4 players on, free-for-all below), otherwise the next entry anyway.

The server checks the whole playlist at startup and refuses to start if an entry is invalid.

### Private rooms

Next to the public room, the server runs up to `--private-rooms` private rooms (default 4) for
duels, parties and custom matches, each with its own code (`R-` and six characters) and its
own simulation thread. A private room closes when its last player leaves (or a minute after it
was opened if nobody came). Players reach one through an invite link, a party code or an
accepted duel; a client asking for an unknown or closed room code gets "unknown room". When all
private rooms are in use, new duels and custom matches are refused until one closes. A private
room runs the same lobby as the public room (warm-up, countdown, results) and plays its one
match again and again until it is empty: a custom match on its world (a new random world each
time if the invite had none) with the invite's rules (the public room's current rules if it had
none), a duel on the challenger's world with at most 2 players. Each room has its own
simulation thread and its own `tick stats` line; `--private-rooms` is capped at one room per CPU
core beyond the public room's, so the tick budget holds per room.

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
- The browser talks to the server's small JSON API at `/api/…` for this (and for the server
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
| The match never starts, the warm-up keeps going | The next playlist entry needs more players (Team Brawl: 4). After `--warmup` seconds the server picks an entry that fits; with `min_players` too high everywhere it plays the entry anyway. Check the `player count does not fit` log line. |
| Server list empty or "No server directory" | The page's server has no `--directory`, or the directory is unreachable from the browser. On `https://` pages the directory must be `https://` too and named in the link (`?directory=`). `http://<directory>/servers` shows what is registered; servers of another build are hidden. |
| "unknown room" / "unknown party" for invites | Private rooms close when their last player leaves, parties 10 minutes after their last member left; codes only work on the server that made them. |
| Duels or custom matches are refused | All `--private-rooms` are in use (or set to 0). Raise it, within your CPU cores. |

## Server directory (optional)

`ls-directory` is a small, separate service that collects game servers for the in-game server
list. You don't need it: players can always join a server by its link or address. Run one if you
want a server list for your community:

```sh
cargo run -p ls-directory --release -- --bind 0.0.0.0:7780
```

It keeps everything in memory (a restart forgets the list; servers re-register within 10 s) and
speaks plain HTTP on TCP port 7780. Browsers on an `https://` page need it behind an HTTPS reverse
proxy (see [HTTPS and a public directory](#https-and-a-public-directory)).

Verified against `ls-directory --help` of this build:

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
player on another listed server is passed on by the directory. Hand players a list link such as
`http://<your server>/?servers&directory=http://<directory>:7780` to open the list directly; the
`directory=` part stays in the invite links the page makes.

Things to know:

- The directory has no accounts. Each server claims its address with a random secret on its
  first check-in, so nobody else can change its entry while it is listed.
- Behind a reverse proxy every request comes from the proxy's address, so raise `--rate` and
  `--burst` (every player in game asks it for relayed duel challenges every 4 seconds).
- `--public-address` is what the list shows and what invite links use: set it to the address
  players reach the server at (a domain name or your public IP and port).

### HTTPS and a public directory

Browsers treat a page from `https://` as secure, so players need no browser flags, but such a
page may only talk to `https://` and `wss://` addresses. For a public setup, put a reverse proxy
with a real certificate in front of both the game server's HTTP port and the directory. With
[Caddy](https://caddyserver.com/) (it gets certificates by itself and forwards WebSocket
upgrades) a `Caddyfile` for two domain names is:

```
play.example.com {
    reverse_proxy 127.0.0.1:7777
}
dir.example.com {
    reverse_proxy 127.0.0.1:7780
}
```

Then start both programs on the same machine:

```sh
ls-directory --bind 127.0.0.1:7780 --rate 100 --burst 400
ls-server --bind 127.0.0.1:7777 --wt-bind 0.0.0.0:7777 \
    --directory http://127.0.0.1:7780 --public-address play.example.com:443 \
    --server-name "Scrap Den" --region eu-west
```

- Players open `https://play.example.com/`; the server list is
  `https://play.example.com/?servers&directory=https://dir.example.com`. The page learns the
  directory from its server, which only knows the plain `http://` address it registers with, so
  on `https://` pages always give the `directory=` part.
- `--public-address play.example.com:443` makes the list and the invite links point at the
  proxy (`wss://play.example.com/ws` for the game, `https://play.example.com/api/…` for parties
  and duels).
- WebTransport doesn't go through the proxy: it uses UDP on `--wt-bind` (here 7777) with the
  server's own certificate, which browsers trust by its hash. Open UDP 7777 (and TCP 443 for the
  proxy) in the firewall; without UDP players still play over the proxied WebSocket.
- The game server talks to the directory over plain HTTP (`ls-server` has no TLS client). Keep
  that link on the same machine or a private network: the server's secret travels with every
  check-in.
- Other game servers on other machines need their own HTTPS name and proxy the same way, and a
  plain-HTTP route to the directory (a private network, or the proxy's HTTP side).

This setup follows from how the pieces work; the project's automated tests cover the directory,
the server list and invites over plain HTTP on one machine, not a TLS proxy.

## Not in this build yet

- **Co-op modes and The Architect**, and a vote for the next mode (the rotation plays in order).
- **No accounts, friends lists or ranked play.** Players are known by a random id their browser
  keeps; parties exist on one server only. A public directory is not run for you: host your own
  `ls-directory` (behind an HTTPS proxy for `https://` pages).
- **No admin commands** while running (kick, change world or rule set, end match). Restart the
  server with new flags; bans and tuning can change between matches through the balance list.
- **Server bans in private rooms:** custom matches and duels ignore `--ban` and the balance list.
- **No slider tool:** you can't create a share code with chosen slider values yet (playlists can
  pick random worlds within Enclosure and Density ranges), and only Enclosure, Density,
  Instability and (visually) Palette have an effect. The comedy director rolls events by the
  rule set's `absurdity` cap, but every event is still a placeholder that only announces itself.
- **No custom TLS certificate** for WebTransport and no built-in HTTPS for the web page or the
  directory (use a reverse proxy).
- **No WebRTC fallback, desktop app, regional hosting or orchestration** (the design mentions
  Agones or Nomad for later).
