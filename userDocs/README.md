# Loose Screws: user documentation

Manuals for people who play Loose Screws or run a server for it. Developer
documentation (architecture, design, implementation plan) lives in [`../docs/`](../docs/).

| Document | For | Covers |
| --- | --- | --- |
| [Player manual](player-manual.md) | Players | Starting a game, ship types and the ship bay, controls, the HUD, ship resources, flying, every weapon with tips, surviving a Tectonic Charge, gadgets, paint colours, surface states, debris and scavenging, every game mode (Paint the Void, Team Brawl, The Squeeze, Heist, Hot Reactor, Last Color Standing, Instagib, duels), the lobby between matches, share codes, rule set codes, the rule set editor, the server list, invite links, parties, duels, settings, troubleshooting |
| [Hosting guide](hosting-guide.md) | Server hosts | Building and running `ls-server`, every command line flag, share codes, rule sets for every mode (files, ids and `LR` codes), bans and balance lists, the rule set editor and invite links, ports, WebTransport, firewalls, bots, logs, the lobby, playlists, private rooms, parties and duels, the server directory (`ls-directory`) and HTTPS, troubleshooting |
| [Weapon reference](reference/weapons.md) | Everyone | Every weapon's numbers: damage, fire rate, cost, speed, range, splash, knockback, craters, Shield setting, default rules, parameters |
| [Ship reference](reference/ships.md) | Everyone | The four ship types' hull, damage, speeds, turn rates, resources, scavenging, paint effects |
| [Gadget reference](reference/gadgets.md) | Everyone | Shield, Cloak and Decoy numbers, and how every weapon meets a Shield |
| [Mode reference](reference/modes.md) | Everyone | Every built-in rule set: mode, teams, match length, the mode's settings, mutators, starting loadout and pickups |

The four reference pages are generated from the game data (`data/`) by
`node tools/gen-weapon-docs.mjs`, so their numbers always match the build; CI fails when they
are stale.

## What this build is

Loose Screws is a 6DOF (six degrees of freedom) browser shooter. You fly small ships built from
junk through worlds generated from a short share code, and every crater, scorch mark, splash of
paint and chunk of debris changes the level. This build is **Milestone 3, modes and lobby**, on
top of Milestone 2's arsenal and physics:

- four ship types (Skitter, Tinker, Hauler, Tug), chosen on the start screen or in the ship bay
- the full arsenal of 27 weapons: 7 energy primaries, 3 kinetic primaries, 10 ordnance
  secondaries including the Tectonic Charge, and 7 comedy specials
- three gadgets (Shield, Cloak, Decoy)
- lasting physics: knockback, rock strength, surface states (hot, charged, glow, scorch),
  paint colours that change how ships fly, debris you can push and scavenge, dust clouds
- configurable rules: rule set files and short `LR` rule set codes, an in-browser rule set
  editor, server-side bans and balance lists
- six game modes: Paint the Void, Team Brawl, The Squeeze, Heist, Hot Reactor and Last Color
  Standing, 2 to 4 teams or free-for-all, plus the Instagib mutator and one-on-one duels (up to
  8 players per server by default, 32 at most)
- a lobby between matches (warm-up, ready, team pick, countdown, results) and a match rotation,
  with players staying connected from match to match
- private rooms for custom matches and duels, party codes and invite links, and an optional
  server directory (`ls-directory`) behind the in-game server list
- a web client that needs a WebGPU-capable browser
- a dedicated server (`ls-server`) and headless bots (`ls-bots`) that use the whole arsenal and
  play every mode for its objective

A lot of the design (co-op modes, The Architect, most world sliders, the comedy "moods"
system, accounts, music and more) is **not in this build yet**. Both manuals list those features
in a section of their own so you know what to expect.

## Quick start

- **Play alone right now:** open the game page, click **Practice offline**. No server is needed
  once the page has loaded. See [Starting a game](player-manual.md#starting-a-game).
- **Play with others:** someone runs `ls-server` and everyone opens its address in the browser,
  or an invite link. See [Hosting guide: quick start](hosting-guide.md#quick-start) and
  [Playing with friends](player-manual.md#playing-with-friends-server-list-invites-parties-and-duels).
- **Make your own rules:** click **Rule set editor** on the start screen and hand the code to
  your host. See [The rule set editor](player-manual.md#the-rule-set-editor).

The manuals describe the game as of the Milestone 3 build. All numbers come from the game data
files (`data/`) and may change between builds.
