# Loose Screws: player manual

This manual covers the current build (Milestone 1). Features from the full game design that
this build doesn't have yet are listed under [Not in this build yet](#not-in-this-build-yet).

## Contents

- [What Loose Screws is](#what-loose-screws-is)
- [What you need](#what-you-need)
- [Starting a game](#starting-a-game)
- [Controls](#controls)
- [The HUD](#the-hud)
- [Your ship: hull, energy, boost and scrap](#your-ship-hull-energy-boost-and-scrap)
- [Flying](#flying)
- [Weapons](#weapons)
- [Pickups and wrecks](#pickups-and-wrecks)
- [Paint the Void](#paint-the-void)
- [Share codes](#share-codes)
- [Troubleshooting](#troubleshooting)
- [Not in this build yet](#not-in-this-build-yet)

## What Loose Screws is

You pilot a Scrapling, a palm-sized ship built from junk, through a procedurally generated
world of caves, tunnels and floating rock. You can fly in any direction and turn any way you
like, with no fixed "up". The level is part of the fight: rockets blast craters into the rock,
and in **Paint the Void**, the only mode in this build, two teams race to cover as much of
the level as they can in their own color.

Every world comes from a **share code**, a short text such as
`LS06-WA9D-Z939-439D-V4M6-8001-8F70`. The same code always builds the same world.

## What you need

- A desktop browser with **WebGPU**: current Chrome or Edge (version 113 or newer), Firefox 141
  or newer on Windows, or Safari 26 or newer. On Linux, Chrome needs WebGPU turned on under
  `chrome://flags/#enable-unsafe-webgpu`.
- A mouse and keyboard. The game uses pointer lock, so the mouse steers the ship instead of
  moving a cursor.
- The address of a running game server for online play (the host gives it to you, for
  example `http://localhost:7777`). Practice mode needs no server once the page has loaded.

There is no sound in this build.

## Starting a game

Open the game page. The start screen has these fields:

| Field | What it does |
| --- | --- |
| **Pilot name** | Your name, up to 32 bytes. The server uses it in its log. In the game, other pilots currently appear as "Player 1", "Player 2" and so on, and you appear as "You". |
| **Share code** | A world code. Leave it empty to use the server's world (online) or a random world (practice). |
| **Horizon assist** | On by default. Slowly levels your roll so "up" stays up. You can toggle it in flight with **H**. |

The page remembers your name, code and horizon assist setting for next time.

Then choose one of the two buttons:

- **Play online** joins the server that served the page. You always play **the server's world**,
  whatever you typed in the share code field. If your code is different, a message in the
  kill feed reminds you that your code only works in practice. The server puts you on the team
  with fewer players.
- **Practice offline** builds a world on your own computer from the share code, or a random
  world if the field is empty. You fly for the Orange team against three target dummies on the
  Blue team. The dummies float in place and never move or shoot. A practice round lasts the
  full 5 minutes. When it ends, reload the page for another round.

Building a world takes a moment ("Generating world…" or "Connecting…"). When the game starts,
the HUD shows "Loading terrain (N chunks left)" while the level streams in around you.

**Click the game view to take control.** The browser locks the mouse to the game. Press **Esc**
to release it. While the mouse is released, the HUD shows "Click to fly · Esc releases the
mouse" and your ship doesn't respond to flight keys.

## Controls

Keys are bound to physical key positions, so on QWERTZ or AZERTY keyboards use the keys in the
same place as W, A, S, D on a US keyboard.

| Input | Action |
| --- | --- |
| Mouse | Pitch (nose up and down) and yaw (nose left and right) |
| W / S | Thrust forward / backward |
| A / D | Strafe left / right |
| Space | Thrust up |
| Ctrl or C | Thrust down (C avoids browser shortcuts such as Ctrl+W, which closes the tab) |
| Q / E | Roll left / right |
| Shift (hold) | Boost |
| Left mouse button | Fire the selected primary weapon |
| Right mouse button | Fire the secondary weapon (Bolt Rocket) |
| 1 | Select Scrap Laser |
| 2 | Select Paint Thrower |
| 3 | Select Rivet Gun (only once you have picked one up) |
| 4 | Select Bolt Rocket as your secondary weapon |
| R (hold) | Scavenge scrap near you |
| F (hold) | Overdrive: pay for energy shots with hull when energy runs out |
| H | Toggle horizon assist |
| Tab (hold) | Show the scoreboard |
| F3 | Toggle the debug overlay |
| Esc | Release the mouse |

Tab and F3 work even while the mouse is released. Mouse sensitivity is fixed in this build,
and there are no settings for inverted look or key rebinding yet.

## The HUD

| Where | What it shows |
| --- | --- |
| Center | Crosshair. Your shots fly straight out along it. |
| Top center | Match timer (minutes:seconds) and one bar per team with that team's share of the level's surface in percent: Orange on top, Blue below. |
| Left panel | **Hull**, **Energy** and **Boost** meters with their values. The hull bar changes color below 25%. |
| Right panel | The selected primary weapon and how many shots you can afford ("33 shots" for energy and scrap weapons), your secondary weapon with its count (for example "Bolt Rocket ×3"), and your **Scrap**. |
| Bottom center | "Horizon assist: on (H)" or "off", and "Scavenging" while you hold R. |
| Above the crosshair | Status messages: "Waiting for the server", "Syncing with the server", "Destroyed, respawning", "Match over: Orange wins (your team)". |
| Top right (kill feed) | The last few events, for example "Player 3 destroyed You", "You self-destructed", or the match result. Lines fade after about 6 seconds. |

Online, the Bolt Rocket count is an estimate made by your browser. It's correct after each
respawn but can be off after you pick up rockets.

**Scoreboard (hold Tab):** both teams with their paint share, and every pilot with kills and
deaths. Online, kills and deaths count from the moment you joined. Pilots waiting to respawn
are dimmed, and your own row is highlighted.

**End of match:** when the timer runs out, a full-screen result shows the winning team (or a
draw), each team's paint share and the scoreboard. Online it counts down to the next match (see
[After the match](#after-the-match)).

**Debug overlay (F3):** frame rate and the slowest recent frame, ping, the connection type
(WebTransport or WebSocket), prediction corrections, shot counters, terrain chunk counts, your
speed in u/s and **the share code of the world you're in**. It's mainly for bug reports, but the
bottom line is the easiest way to copy a level's share code.

## Your ship: hull, energy, boost and scrap

Every ship in this build is a **Tinker**, the standard Scrapling. Its four resources are
linked, so spending one puts pressure on another.

| Resource | Max | At spawn | Used for | How it refills |
| --- | --- | --- | --- | --- |
| **Hull** | 100 | 100 | Your health. At 0 your ship is destroyed. | **Only** by scavenging scrap (hold R). It never regenerates on its own. |
| **Energy** | 100 | 100 | Scrap Laser and Paint Thrower shots | Regenerates 10 per second, **but only while you aren't boosting** |
| **Boost** | 100 | 50 | Higher top speed while you hold Shift | Hard braking and near-miss wall scrapes (see below). It doesn't regenerate on its own. |
| **Scrap** | 100 | 20 | Rivet Gun ammunition | Scavenging, once your hull is fully repaired |

**Boost** drains 20 per second, but only while you hold Shift **and** thrust in some direction.
From full, that's 5 seconds of boost. Two ways to earn it back:

- **Hard braking:** thrust against your current motion while moving faster than 6 u/s (for
  example, press S while flying forward fast). The more speed you brake away, the more boost
  you get. Just letting go of the keys and drifting to a stop gives nothing.
- **Near-miss scrapes:** fly past a wall closer than about half a ship length without touching
  it, at 8 u/s or more. The closer you skim, the faster boost refills (up to 30 per second).
  Actually touching the wall gives nothing: you bounce off instead.

Walls don't damage you in this build, so scraping is a free way to recharge.

**Scavenging (hold R):** takes scrap from any wreck or scrap pile within 3 u of your ship, at
20 per second. The scrap repairs your hull first, 1 hull per scrap. Once the hull is full, the
rest goes into your scrap store for the Rivet Gun. Stay close and hold still while you do it.

**Overdrive (hold F):** when your energy is too low for an energy shot, holding F lets you fire
anyway. The missing energy is paid with **2 hull for every energy point**. Overdrive never
destroys your own ship: if a shot would take your hull below 1, it doesn't fire. This is
desperate but sometimes match-winning. Be careful with the Paint Thrower, which fires 10 times
a second: in overdrive it burns about 10 hull per second, and about 30 while you boost (boosting
stops energy regeneration).

**Destroyed:** when your hull reaches 0, you respawn after **3 seconds** at your team's side of
the level, with full hull and energy, half boost, 20 scrap and the starting loadout. Weapon
upgrades and picked-up weapons are lost. Your wreck stays where you died as a scrap pile of
30 plus half the scrap you carried.

## Flying

The game uses **units (u)**: 1 u is one ship length. Speeds are in u per second.

Thrust is limited **per axis of your ship**, as in the classic game Descent. Forward/back,
left/right and up/down each have their own top speed:

| How you fly | Top speed |
| --- | --- |
| Straight, cruising | 12 u/s |
| Straight, boosting | 16.5 u/s |
| Diagonal on all three axes, cruising | about 21 u/s |
| Diagonal on all three axes, boosting | about 28.5 u/s |
| For comparison: a Scrap Laser bolt | 30 u/s |

**Trichording** is the core movement skill: hold forward, a strafe key **and** up or down at
the same time (for example W + D + Space). You then fly diagonally at 1.73 times (the square
root of 3) your straight-line speed. Your nose points about 55 degrees away from the direction
you actually travel, so practice aiming while trichording. Two keys at once (W + D) already
give 1.41 times the speed.

More flight tips:

- **Nothing outruns a laser.** Even a boosting, trichording ship is a little slower than a laser
  bolt. Bolts expire after 3 seconds, so distance and dodging are what save you.
- **Your shots don't inherit your speed.** Every projectile leaves at its own fixed speed in the
  direction your nose points. Lead moving targets: aim where they will be.
- **Aim at what you see.** The server compensates for your connection delay when it checks hits,
  so you don't need to lead extra to make up for ping.
- **Let go to stop.** With no keys held, the ship slows down and stops by itself. After a boost
  ends, speed above the cruise limit bleeds off quickly.
- **Brake hard to recharge.** Reversing thrust at speed refills boost. A quick stop behind cover
  after a boosted run pays for the next one.
- **Turning has a limit.** Very fast mouse flicks are capped at a maximum turn rate, and rolling
  (Q/E) is slower than turning with the mouse.
- **Horizon assist** slowly rolls you back level whenever you're not pressing Q or E. It pauses
  while your nose points almost straight up or down. Turn it off (H) if you want full control.
- **Leaving the tab** (switching tabs or windows) releases all keys. Online your ship stays in
  the match and drifts to a stop, so don't leave it somewhere dangerous.

## Weapons

You carry up to three primary weapons (fired with the left mouse button) and one secondary
weapon (right mouse button). Select a primary with **1**, **2** or **3**. You spawn with the
Scrap Laser selected.

| Key | Weapon | Type | You start with it? |
| --- | --- | --- | --- |
| 1 | Scrap Laser | Energy primary | Yes |
| 2 | Paint Thrower | Special primary | Yes |
| 3 | Rivet Gun | Kinetic primary | No, pickup only |
| 4 / right mouse | Bolt Rocket | Ordnance secondary | Yes, 3 rockets |

The numbers below are for the default Paint the Void rules. A server can use different rules
(see the hosting guide), so a server may give you other starting weapons.

### Scrap Laser

The dependable all-rounder. Straight, visible bolts you can dodge and that your enemies can
dodge too.

- Bolt speed 30 u/s, lasts 3 seconds (range about 90 u)
- 8 damage per hit, 4 shots per second
- Costs 3 energy per shot: about 33 shots from full energy. Continuous fire drains energy only
  a little faster than it regenerates, as long as you don't boost.
- **Upgrades:** picking up another Scrap Laser raises its level by one, up to level 6. Each level
  adds 30% of the base damage, so level 6 deals 20 per hit. Upgrades are lost when you're
  destroyed.
- Weakness: weak at level 1. It takes 13 hits to destroy a full-hull ship.

### Paint Thrower

The tool that wins Paint the Void. It deals **no damage**.

- Fires blobs of paint in your team's color: 20 u/s, lasts 1.5 seconds (range about 30 u)
- 10 blobs per second, 1.5 energy each (15 energy per second). With normal regeneration, a full
  energy bar lasts about 20 seconds of continuous painting.
- Each blob that hits rock paints a patch about 4 u across. Paint covers whatever was there
  before, including the other team's color.
- Blobs that hit a ship do nothing.

### Rivet Gun

A very fast kinetic gun that fires your scrap. You don't spawn with it: fly through a Rivet Gun
pickup to get one, then press **3** to select it.

- Rivets fly at 180 u/s (6 times laser speed), last 1 second (range about 180 u)
- 3 damage per hit, about 12 shots per second
- Costs **1 scrap per shot**. You spawn with 20 scrap, so that's 20 rivets until you scavenge
  more. Scrap is also what repairs your hull, so every rivet competes with healing.
- Only a small push on hit. No upgrade levels.

### Bolt Rocket

A dumbfire rocket for burst damage and for blasting paint off walls.

- Flies at 40 u/s, lasts 4 seconds (range about 160 u)
- 35 damage on a direct hit. The explosion also deals up to 20 damage to other ships within
  2.5 u, less the farther they are. **The explosion hurts you too.**
- Strong knockback, one rocket every 0.75 seconds
- You start with 3 rockets and can carry up to 10. Each Bolt Rocket pickup adds 3.
- On rock it blasts a **crater 2 u across** that stays for the rest of the match. **Paint on the
  blasted surface is destroyed**, and the fresh rock of the crater starts unpainted.

### Damage rules

- **Friendly fire** is 50% by default: shots from teammates deal half damage and half knockback.
  Your own rocket blast always hurts you fully.
- Every hit pushes the target in the direction of the shot. The harder the hit, the bigger the
  push.
- A kill counts for whoever fired the killing shot. Destroying a teammate or yourself doesn't
  count as a kill, but still counts as a death for the victim.

## Pickups and wrecks

Each level has **12 pickup points** in open space, spread across the level and at least 10 u
away from the team spawn points. Each point holds one item at a time. With the default rules, an item is picked at
random, with these chances and respawn times:

| Pickup | Chance | Respawns after | Effect |
| --- | --- | --- | --- |
| Scrap pile | 3 in 8 | 15 s after it's emptied | 30 scrap to scavenge (hold R) |
| Rivet Gun | 2 in 8 | 20 s | Gives you the Rivet Gun |
| Bolt Rocket | 2 in 8 | 20 s | +3 rockets (up to 10) |
| Scrap Laser | 1 in 8 | 30 s | +1 Scrap Laser level (up to 6) |

You collect a weapon pickup by flying within about a ship length of it. A pickup that would do
nothing for you (a Scrap Laser at level 6, rockets when you already have 10, a second Rivet Gun)
stays for someone else.

**In this build, pickups, scrap piles and wrecks are not drawn yet.** They exist and work, but
you can't see them. Look at where ships were destroyed and hold R there to scavenge the wreck,
and fly through open spaces in the level to find pickups by chance.

## Paint the Void

Two teams, **Orange** and **Blue**, try to paint as much of the level as they can.

- The match lasts **5 minutes** (the server can change this).
- Your team's score is the share of the **whole level's paintable surface** covered in your color.
  The score updates twice a second, and the HUD bars and the scoreboard show it in percent.
- When the timer runs out, the team with more painted surface wins. Equal coverage is a draw.
- **Kills don't score points.** They matter because a destroyed enemy stops painting for
  3 seconds and respawns far away.
- Painting over the other team's color takes that surface from them.
- Bolt Rocket craters erase paint, both theirs and yours.

Levels are large, so the percentages stay small: a close match can end with both teams
under 1%. Only the comparison between the two teams counts.

Tactics:

- Paint surfaces the other team has painted. That takes surface from them and adds it to your
  team at the same time.
- Keep energy for painting: boosting stops energy regeneration, and the Paint Thrower needs
  energy.
- A rocket into a freshly painted enemy wall wipes their paint. Then paint the crater.
- Guard your team's painted areas and hunt the enemy painters.

### After the match

Online, the result stays on screen for about 10 seconds (the server can change this). Then the
server closes every connection and starts a new match **on the same level**. The craters and
paint are gone, and teams are assigned again. Your browser rejoins automatically ("Match over.
Joining the next match…") and keeps your name, horizon assist setting and debug overlay. If it
can't rejoin after about 12 tries, it shows an error. Reload the page to join again.

In practice mode, the result stays on screen. Reload the page for a new round.

## Share codes

A share code describes a whole world: a random seed, the seven world sliders and the world
generator version, plus a checksum that catches typos. It always looks like this:

```
LS06-WA9D-Z939-439D-V4M6-8001-8F70
```

That's `LS` plus 26 letters and digits, 34 characters with the dashes.

- **Typing codes:** upper or lower case both work, and dashes and spaces are optional. The letter
  O is read as zero, and I or L as one, so common typos still work.
- **Finding a level's code:** press **F3** in game. The bottom line of the overlay shows the code
  of the world you're in, including random practice worlds. Server hosts also see it in the
  server log.
- **Using a code:** type it on the start screen and press **Practice offline** to fly that world
  alone. To play it online, the server host starts the server with that code.
- **Sharing a level:** send the code to a friend. The same code builds exactly the same world on
  every computer.
- **Old codes:** a code made by an older version of the world generator is rejected with a
  message that names both versions.
- **Sliders:** the code stores all seven sliders from the design (Enclosure, Density,
  Verticality, Gravity Chaos, Palette, Absurdity, Instability). In this build only **Enclosure**
  (open sky to dense mine shafts) and **Density** (few to many pillars and floating rocks)
  change the world. There's no screen to set sliders yet: random practice worlds and server
  worlds pick them for you.

## Troubleshooting

### "WebGPU unavailable"

The game can't draw without WebGPU. The error screen says which step failed:

- **"This browser does not support WebGPU":** update your browser, or use Chrome or Edge 113+,
  Firefox 141+ on Windows, or Safari 26+. On Linux, turn on
  `chrome://flags/#enable-unsafe-webgpu` in Chrome.
- **WebGPU also needs a secure page.** `http://localhost:…` and any `https://` address work, but a
  plain `http://` address with a network IP (for example `http://192.168.1.20:7777`) does not.
  Ask your host for another way to reach the game (see the hosting guide, section
  "Playing across a network"). For a quick fix in Chrome, add the address under
  `chrome://flags/#unsafely-treat-insecure-origin-as-secure` and restart the browser.
- **"no suitable GPU adapter was found":** update your graphics driver, make sure hardware acceleration is
  turned on in the browser settings, and check `chrome://gpu` for GPU blocklisting.
- **"Could not create a GPU device":** close other GPU-heavy tabs and programs, then reload.
- **"GPU device lost":** the graphics driver reset or the system took the GPU back. Reload the
  page.

### Mouse and keyboard

- **The ship doesn't respond:** click inside the game view. The prompt "Click to fly" means the
  mouse isn't locked.
- **Clicking doesn't lock the mouse right after pressing Esc:** browsers block pointer lock for a
  moment after you release it. Wait a second and click again.
- **Ctrl+W closed the tab:** browsers keep some Ctrl shortcuts for themselves. Use **C** to thrust
  down instead of Ctrl.
- **Movement keys are in the wrong place:** the game uses key positions, not letters. Use the keys
  where W, A, S and D are on a US keyboard.
- **The mouse feels too fast or too slow:** there's no sensitivity setting yet. Change the pointer
  speed in your operating system, or the DPI of your mouse.
- **Low frame rate:** close other tabs, make sure the browser uses your dedicated GPU, and press
  F3 to check the frame rate and chunk counts. Stutter right after joining is normal while the
  terrain is still loading.

### Connection problems

- **"Could not join the server":** the server isn't running or isn't reachable at the address you
  opened. The page connects to the server that served it, at `/ws` on the same address. Check the
  address with your host. Practice offline still works: reload to return to the start screen.
- **"room is full":** the server has reached its player limit. Try again later.
- **"Disconnected" during play:** the connection dropped or the server stopped. Reload to return
  to the start screen.
- **"Lost sync with the server":** the game received data it couldn't use, often because the page
  and the server come from different builds. Reload the page (a hard refresh with Ctrl+F5 helps
  after a server update).
- **"Could not rejoin" after a match:** the next match didn't accept you in time. Reload to join
  it.
- **Lag or rubber-banding:** press F3 and check ping and the connection type. WebTransport is
  usually smoother than WebSocket. The browser picks WebTransport when it can and falls back to
  WebSocket otherwise.

Advanced page options, added to the address (combine them with `&`):

| Option | Effect |
| --- | --- |
| `?transport=ws` | Always use WebSocket |
| `?transport=wt` | Always use WebTransport (fails instead of falling back) |
| `?server=host:port` | Connect to a game server other than the one that served the page, for example `http://localhost:7777/?server=192.168.1.20:7777` |

## Not in this build yet

These are part of the game design but aren't in the Milestone 1 build:

- **Ships:** only the Tinker. The Skitter (light), Hauler (heavy) and Tug (support) come later.
- **Weapons:** only the four above. The rest of the arsenal (Scatter Laser, Plasma Repeater,
  Fusion Charger, Railspike, Seeker, Heavy Rocket, Prox Mine, Tectonic Charge and others) and the
  other comedy specials come later.
- **Gadgets:** no Shield, Cloak or Decoy yet.
- **Paint effects:** paint only claims territory. Bouncy, slippery or sticky paint colors come
  later.
- **Other lasting effects:** no scorch marks, glowing metal, debris physics, dust clouds or
  collapsing tunnels. Only craters and paint change the level.
- **Visible pickups, wrecks and scrap piles:** they work but aren't drawn yet.
- **Pilot names in game:** others appear as "Player N".
- **World sliders:** only Enclosure and Density have an effect. No Verticality, gravity wells,
  black-and-white palette, Instability or comedy events ("the level has moods") yet, and no
  screen to set sliders or save a level.
- **Game modes:** only Paint the Void with two teams. Heist, Hot Reactor, Team Brawl, The Squeeze,
  the co-op modes, The Architect and mutators such as Instagib come later.
- **Online features:** no lobby, server list, invite links, party codes or rule set codes. You
  join by opening the server's address.
- **Settings:** no mouse sensitivity, invert-look or key rebinding options.
- **Sound, cosmetics, a WebGL2 fallback for browsers without WebGPU, and a desktop app.**
