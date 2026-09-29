# Loose Screws: user documentation

Manuals for people who play Loose Screws or run a server for it. Developer
documentation (architecture, design, implementation plan) lives in [`../docs/`](../docs/).

| Document | For | Covers |
| --- | --- | --- |
| [Player manual](player-manual.md) | Players | Starting a game, controls, the HUD, ship resources, flying, weapons, Paint the Void rules, share codes, troubleshooting |
| [Hosting guide](hosting-guide.md) | Server hosts | Building and running `ls-server`, every command line flag, share codes and rule sets, ports, certificates, firewalls, bots, logs, match restarts, troubleshooting |

## What this build is

Loose Screws is a 6DOF (six degrees of freedom) browser shooter. You fly small ships built from
junk through worlds generated from a short share code, and every crater and splash of paint
changes the level. This build is **Milestone 1, a vertical slice**:

- one ship type (the Tinker)
- four weapons: Scrap Laser, Paint Thrower, Rivet Gun and Bolt Rocket
- one game mode (Paint the Void, two teams, up to 8 players per server)
- a web client that needs a WebGPU-capable browser
- a dedicated server (`ls-server`) and headless bots (`ls-bots`) to fill a match

A lot of the design (more ships, weapons, modes, gadgets, the comedy "moods" system, a lobby
and more) is **not in this build yet**. Both manuals list those features in a section of their
own so you know what to expect.

## Quick start

- **Play alone right now:** open the game page, click **Practice offline**. No server is needed
  once the page has loaded. See [Starting a game](player-manual.md#starting-a-game).
- **Play with others:** someone runs `ls-server` and everyone opens its address in the browser.
  See [Hosting guide: quick start](hosting-guide.md#quick-start).

The manuals describe the game as of the Milestone 1 build (waves 1 to 3). All numbers come from
the game data files (`data/`) and may change between builds.
