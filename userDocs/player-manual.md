# Loose Screws: player manual

This manual covers the current build (Milestone 4, "the level has moods": comedy events and
the level's moods, all seven world sliders with gravity wells, cave-ins and black-and-white ink
levels with pigment, The Architect, the Belt Up race on asteroid belts, the Paperweight, the
world workshop and sound, on top of Milestone 3's modes, lobby, server list, invites and
parties and Milestone 2's full arsenal, four ship types and gadgets). Features from the game
design that this build doesn't have yet are listed under
[Not in this build yet](#not-in-this-build-yet). Every number in the
weapon, ship, gadget, mode and comedy event tables of the [reference pages](reference/) is
generated from the game's data files, so those pages always match the build.

## Contents

- [What Loose Screws is](#what-loose-screws-is)
- [What you need](#what-you-need)
- [Starting a game](#starting-a-game) ([the world workshop](#the-world-workshop))
- [Controls](#controls)
- [The HUD](#the-hud)
- [Ship types](#ship-types)
- [Your ship: hull, energy, boost and scrap](#your-ship-hull-energy-boost-and-scrap)
- [Flying](#flying) ([will I fit?](#will-i-fit))
- [Weapons](#weapons)
- [Surviving a Tectonic Charge](#surviving-a-tectonic-charge)
- [Gadgets](#gadgets)
- [The level fights back](#the-level-fights-back) ([comedy events](#comedy-events),
  [moods](#the-levels-mood), [gravity wells](#gravity-wells), [cave-ins](#instability-and-cave-ins),
  [black-and-white levels and pigment](#black-and-white-levels-and-pigment))
- [Pickups and wrecks](#pickups-and-wrecks)
- [Game modes](#game-modes): [Paint the Void](#paint-the-void), [Team Brawl](#team-brawl),
  [The Squeeze](#the-squeeze), [Heist](#heist), [Hot Reactor](#hot-reactor),
  [Last Color Standing](#last-color-standing), [The Architect](#the-architect), [Belt Up](#belt-up),
  [Instagib](#instagib), [duels](#duels)
- [Between matches: the lobby](#between-matches-the-lobby)
- [Share codes, rule set codes and the rule set editor](#share-codes-rule-set-codes-and-the-rule-set-editor)
- [Playing with friends: server list, invites, parties and duels](#playing-with-friends-server-list-invites-parties-and-duels)
- [Settings](#settings)
- [Troubleshooting](#troubleshooting)
- [Not in this build yet](#not-in-this-build-yet)

Reference tables: [weapons](reference/weapons.md), [ships](reference/ships.md),
[gadgets](reference/gadgets.md), [modes](reference/modes.md),
[comedy events](reference/events.md), [world physics](reference/world.md).

## What Loose Screws is

You pilot a Scrapling, a palm-sized ship built from junk, through a procedurally generated
world of caverns, tunnels and floating rock. You can fly in any direction and turn any way you
like, with no fixed "up". The level is part of the fight: weapons blast craters, melt
channels, scorch and charge the rock, knock debris loose and even grow new rock, and all of it
stays for the rest of the match. And the level has moods of its own: now and then it plays a
joke on everyone (a cat paw through the wall, a duck parade, a tax drone after the leader).
Servers rotate through seven modes: paint the level in your team's
colour (**Paint the Void**), team deathmatch (**Team Brawl**), survive the closing walls (**The
Squeeze**), dig out and steal the core (**Heist**), pass on a bomb (**Hot Reactor**), keep
your colour in a black-and-white world (**Last Color Standing**) or race for the exit while one
pilot reshapes the level against you (**The Architect**), plus one-hit-kill **Instagib**
matches and one-on-one **duels**. Hosts can also run **Belt Up**, a race around an asteroid
belt.

Every world comes from a **share code**, a short text such as
`LS0G-C4GD-KYZH-1HJ5-BC58-T2XM-57NT`. The same code always builds the same world.

## What you need

- A desktop browser with **WebGPU**: current Chrome or Edge (version 113 or newer), Firefox 141
  or newer on Windows, or Safari 26 or newer. On Linux, Chrome needs WebGPU turned on under
  `chrome://flags/#enable-unsafe-webgpu`.
- A mouse (with a wheel, for cycling weapons) and a keyboard. The game uses pointer lock, so
  the mouse steers the ship instead of moving a cursor.
- The address of a running game server for online play (the host gives it to you, for
  example `http://localhost:7777`). Practice mode needs no server once the page has loaded.

Sound is synthesized in the browser and starts with your first click or key press (see
[Settings](#settings)); headphones let you hear where things happen.

## Starting a game

Open the game page. Behind the start screen a camera flies through a generated world (caverns
that open to the sky) while a few ships fight ahead of it; the world's kind and share code show
in the bottom right corner. The start screen has these fields:

| Field | What it does |
| --- | --- |
| **Pilot name** | Your name, up to 32 bytes. Other pilots see it above your ship, on the scoreboard and in the kill feed. |
| **Share code** | A world code. Leave it empty to use the server's world (online) or a random world (practice). |
| **Ship** | The ship type you fly first: click one of the four cards (Skitter, Tinker (default), Hauler or Tug; each shows its hull turning). See [Ship types](#ship-types). If the server's rules don't allow it, you get the Tinker or the first allowed type. You can change ships later in the ship bay (**B**). |
| **Options: Horizon assist** | On by default. Slowly levels your roll so "up" stays up. You can toggle it in flight with **H**. |
| **View: Field of view** | How wide you see, 80 to 120 degrees across a 16:9 screen (default 100). Narrower screens keep that width; wider screens (21:9) see a little more at the sides, up to 115 degrees. Boosting widens the view by 4 degrees for a sense of speed. |
| **View: Cockpit frame** | On by default. Shows the nose, the canopy frame (pillars and the bow across the top) and side parts (pods, armour plates, grapple arms) of the ship you fly, so you can judge its size. |
| **View: Fit ring** | On by default. A thin ring around the crosshair that shows how big your ship is at the rock ahead (see [Will I fit?](#will-i-fit)). |
| **Sound** | Volume sliders for everything (master), effects, music and the announcer, and **Mute all sound**. Letting go of a slider plays a short sample. See [Settings](#settings). |

**Options** (horizon assist, the view and the sound settings) and **Controls** (every key) fold
open below the buttons. The page remembers your name, code, ship, horizon assist, view and sound
settings for next time. The **Rule set editor** link in the bottom right corner opens the
[rule set editor](#the-rule-set-editor). Without WebGPU the start screen still works, with a
plain background; the game then explains what is missing when you press a button.

Then choose one of the two buttons:

- **Play online** joins the server that served the page (or the one an
  [invite link](#invite-links) names). You play **the server's world, mode and rules**, whatever
  you typed in the share code field; if your code is different, a message in the kill feed
  reminds you that your code only works in practice. Between matches you stay connected and
  wait in the [lobby](#between-matches-the-lobby). In team modes the server puts you on the team
  with fewer players; in a mode with lives that is already running, you spectate until the next
  match.
- **Practice offline** builds a world on your own computer from the share code, or a random
  world if the field is empty, with the default Paint the Void rules. You fly for the Orange
  team against three target dummies ("Dummy 1" to "Dummy 3") on the Blue team. The dummies
  float in place and never move or shoot. Pickups spawn as in a real match, so practice is the
  place to try every weapon you find. The level's moods and comedy events run as in a real
  match, as often as the world's Absurdity says (random practice worlds have the default
  Absurdity of 20, about one event per round; make a livelier world in the
  [workshop](#the-world-workshop), or force one, see [Comedy events](#comedy-events)). A
  practice round lasts the full 5 minutes. When it ends, reload the page for another round.

Below the buttons, **Make a world** opens the [world workshop](#the-world-workshop).

Building a world takes a moment ("Generating world…" or "Connecting…"). When the game starts,
the HUD shows "Loading terrain (N chunks left)" while the level streams in around you.

**Click the game view to take control.** The browser locks the mouse to the game. Press **Esc**
to release it. While the mouse is released, the HUD shows "Click to fly · Esc releases the
mouse" and your ship doesn't respond to flight keys.


## The world workshop

**Make a world** on the start screen opens the workshop: you build a world from a seed and
seven sliders and see it at once. The flythrough behind the panel becomes your world; every
change rebuilds it about a third of a second after you stop moving a slider (the old view
blurs while the new one is built, "Building the preview… n %" in the top right corner, then
"Live preview").

| Part | What it does |
| --- | --- |
| **Classic / Asteroid belt** | The kind of level. Classic is the cave, mine and open-sky world of every mode. An asteroid belt is a ring of asteroids around an open centre with the EXIT in the middle, the same from all 8 start slots, built for the [Belt Up](#belt-up) race. The sliders below then describe the belt. |
| **Seed** | Any whole number. The same seed with the same sliders always makes the same world. The dice rolls a new one. |
| **Enclosure** | Open space and sky islands at 0, caverns in the middle, mine shafts at 100. On a belt: the size of the asteroids (the biggest are hollow from 60 up). |
| **Density** | How much rock fills the space: pillars, boulders and floating rock. On a belt: asteroids and drifting boulders per sector. |
| **Verticality** | The shape of the level, from flat and wide (320 × 64 × 320 u) to tall and narrow (192 × 224 × 192 u). On a belt: how thick the belt is. |
| **Gravity Chaos** | Gravity wells that pull you in, none at 0 up to seven strong ones at 100. On a belt: shepherd moons along the ring (from 20 up). |
| **Palette** | Full colour at 0, muted, then ink outlines, then a black-and-white ink world with pigment pickups from 80 up. |
| **Absurdity** | How often the level plays a joke on everyone (comedy events); 0 means none. A host's rules can cap it. |
| **Instability** | How easily the rock gives way and tunnels cave in; 0 means never. |

Each slider shows what its value means next to it (for example "Caverns open to the sky" or
"3 gravity wells, gentle pull"). Below the sliders:

- The **share code** of your world. **Save** copies it, so you can paste it into a chat, the
  start screen's share code field or a host's command line.
- **Fly it** starts [practice](#starting-a-game) on your world.
- **Host it** shows an **invite link**: opening it on this server starts a private match on
  your world (the server's rules), and anyone with the link can join. On an asteroid belt it
  proposes the **Belt Up** race ("Race it as Belt Up", on by default). **Pick rules** opens the
  [rule set editor](#the-rule-set-editor) on your world, and the line under "Your own server"
  is the command that hosts it on your own machine.

The back arrow (or **Esc**) returns to the start screen with your world's code in the share
code field. The workshop remembers your last world. A link with `?workshop` opens the
workshop directly, `?workshop=<share code>` on that world.

## Controls

Keys are bound to physical key positions, so on QWERTZ or AZERTY keyboards use the keys in the
same place as on a US keyboard (W, A, S, D and so on).

| Input | Action |
| --- | --- |
| Mouse | Pitch (nose up and down) and yaw (nose left and right). While you guide an Eyeball, the mouse steers the missile instead. |
| W / S | Thrust forward / backward |
| A / D | Strafe left / right |
| Space | Thrust up |
| Ctrl or C | Thrust down (C avoids browser shortcuts such as Ctrl+W, which closes the tab) |
| Q / E | Roll left / right |
| Shift (hold) | Boost |
| Left mouse button | Fire the selected primary weapon. Hold it to charge the Fusion Charger or the Polite Laser. |
| Right mouse button | Fire the selected secondary weapon |
| 1 to 9, 0 | Select your 1st to 10th carried primary weapon (in the order of the [weapon list](#weapons)). The HUD's weapon panel shows which key picks which weapon. |
| Mouse wheel | Cycle through your primary weapons (down: next, up: previous) |
| X | Cycle through your secondary weapons |
| G | Use the gadget: switch the Shield or Cloak on and off, launch the Decoy |
| V | Paint colour: while the Paint Thrower is your selected primary, switch its colour (red, blue, yellow, then red again) |
| B | Open or close the ship bay (the ship you fly after your next respawn). While it's open, **1** to **4** pick a ship instead of a weapon. It opens by itself while you're destroyed. |
| R (hold) | Scavenge: take scrap from wrecks, scrap piles and debris near you. On a Tug it also passes scrap to a teammate next to you. Next to your own Swap Harpoon spike it holds you in place. |
| F (hold) | Overdrive: pay for energy shots with hull when energy runs out |
| H | Toggle horizon assist |
| Tab (hold) | Show the scoreboard |
| F3 | Toggle the debug overlay |
| Enter | Lobby: ready (or not ready) for the next match |
| T | Lobby: pick your team for the next match (Auto, then each team in turn) |
| O | Spectate from the next match on, or play again |
| Esc | Release the mouse |

Tab, F3, Enter, T and O work even while the mouse is released, except that a focused button,
link or checkbox keeps Enter and Tab for itself (Enter presses it, Tab moves the focus) and a
text field keeps every key except F3. While you spectate, the flight
keys and the mouse fly the camera (Shift is faster, it passes through rock), the left mouse
button follows the next ship and the right mouse button goes back to the free camera. Even a very quick tap of G or V counts.
Mouse sensitivity is fixed in this build, and there are no settings for inverted look or key
rebinding yet.

## The HUD

| Where | What it shows |
| --- | --- |
| Center | Crosshair. Your shots fly straight out along it. Around it, within 12 u of rock ahead, the **fit ring** (see [Will I fit?](#will-i-fit)). |
| Top center | The **scoreline**: the mode, the match timer (minutes:seconds, red in the last 30 seconds), one score per team in the team colours (Orange, Blue, Lime, Magenta; in Paint the Void the team's share of the surface in percent) or, in free-for-all modes, the top three pilots and your own place, and the score that wins. Below it the mode's stage where it has one (The Squeeze's phase and its timer). |
| Left, above the meters | In modes with lives your **lives**, in Last Color Standing your **colour** meter |
| In the view | **Objective markers** with an icon, a label and the distance: the safe zone and the next zone (with the time until the walls move), the core ("dig it out", "take it", "dropped" with its return time, "Escort <name>" when a teammate carries it), extraction zones ("Your extraction", "Deliver here" while you carry the core), the reactor with its fuse and holder. Markers outside the view sit at the screen edge with an arrow pointing the way. |
| Left panel | **Hull**, **Energy** and **Boost** meters with their values. The hull bar changes colour below 25%. |
| Right panel | The selected primary and how many shots you can afford, the selected secondary with its count, then the **weapon panel**: every carried primary with its number key, every secondary (key X) and your gadget (key G), the selected ones highlighted. Weapons with levels show them (for example "L2/6" for a Scrap Laser at level 2). The gadget line shows "on", "off", "ready" or the cooldown in seconds. Below it, while the Paint Thrower is selected, the **paint colour** ("V paint: Red · bouncy (stripes)"). At the bottom your **Scrap**. |
| Bottom center | "Horizon assist: on (H)" or "off", and "Scavenging" while you hold R |
| Below the crosshair | The **charge meter** while you charge the Fusion Charger or the Polite Laser ("Fusion Charger: 60%", "full, release to fire", "OVERCHARGE, release!"), and "Weapons offline 2.4 s" after a Borrowed Time rewind |
| Upper middle, below the timer | **Warning banners** (see below) and mode alerts with an icon: "Walls close in 0:09: reach the next zone", "The wall is crushing you!", "You carry the core", "<name> has the core: stop them", "Core dropped", "You hold the reactor!", "Too close to the reactor", "Last of your colour" |
| Above the crosshair | Status messages: "Waiting for the server", "Syncing with the server", "Destroyed, respawning", "Match over: Orange wins (your team)" |
| Top right (kill feed) | The last few events, for example "bot3 destroyed You", "You self-destructed", or the match result. Lines fade after about 6 seconds. |
| Over ships | Pilot names: teammates within 90 u, enemies within 45 u (on black-and-white levels grey enemies only within 20 u, see [pigment](#black-and-white-levels-and-pigment)) |
| Top left | The **announcer** (a little broadcast robot with captions) and the level's **mood** badge, a small face (calm, bored, grumpy, giddy) that shows the mood's line for a moment when it changes |
| Under the scoreline | A **comedy event**'s card while it is announced (name, icon and the pilot it aims at), then a small pill with its name and a ring of the time left |
| Around the crosshair | The **pull indicator** near a gravity well: a violet arc pointing where you are pulled, thicker the harder, with "PULL n%" (of your thrust) under the crosshair |
| Belt Up | The race HUD and the radiation meter, see [Belt Up](#belt-up) |

**Warning banners** appear for a few seconds when something happens to you:

| Banner | Meaning |
| --- | --- |
| "Excuse me… (Polite Laser locked on you)" | Someone is charging a Polite Laser at you. It fires 1.5 s after they started. Get out of their aim. |
| "Tectonic Charge launched" | Someone fired a Tectonic Charge somewhere in the level (everyone sees this) |
| "Ground cracking: Tectonic Charge (you are in the damage zone)" with a countdown | A Tectonic Charge has stuck and its fuse is running. The banner names the zone you are in. See [Surviving a Tectonic Charge](#surviving-a-tectonic-charge). |
| "Overcharge! The Fusion Charger burns your hull" | You held the Fusion Charger past full charge. Release the trigger. |
| "Swapped places!" | A Swap Harpoon swapped you with another ship |
| "Rewound 3 s · weapons offline" with a countdown | Your Borrowed Time took you back 3 seconds |
| "Decoy snapping back" | Your Decoy's time is up and it is flying back to you |

Other screen effects: a Flash Pod turns your screen white for 2 seconds. Shockwaves shake the
view, the harder the closer you are. Inside a dust cloud the fog closes in. While you are
cloaked, the screen edges shimmer. While you guide an Eyeball, the view rides the missile with
a frame and the hint "Eyeball view · mouse steers the missile · your ship drifts without
thrust · the view returns when it hits".

**Scoreboard (hold Tab):** every team with its score (one list in free-for-all modes), and
every pilot with the mode score, kills, deaths and, in modes with lives, the lives left.
Online, kills and deaths count from the moment you joined. Pilots waiting to respawn are
dimmed, pilots who are out are struck through, and your own row is highlighted.

**Results:** when a match ends, a full-screen result shows "Victory", "Defeat" or "Draw" (or
who won, while you spectate), the team scores as bars or, in free-for-all modes, a podium of
the top three, and every pilot with score, kills and deaths. Online it counts down to the next
match and names its mode (see [Between matches: the lobby](#between-matches-the-lobby)).

**Lobby:** between matches you fly the next match's level with weapons off (warm-up); a card
on the left shows the next mode, the world, the rules and every pilot's ready tick and team
pick. Enter readies you, T picks a team. See [Between matches: the lobby](#between-matches-the-lobby).

**Debug overlay (F3):** frame rate and the slowest recent frame, ping, the connection type
(WebTransport or WebSocket), prediction corrections, shot counters, terrain chunk counts, your
speed in u/s and **the share code of the world you're in**. It's mainly for bug reports, but the
bottom line is the easiest way to copy a level's share code.

## Ship types

Four Scrapling hulls trade speed, damage and toughness, and heavier ships carry more secondary
weapons at once. No type is strictly best. All numbers: [ship reference](reference/ships.md).

| Ship | Hull | Damage dealt | Top speed (vs laser) | Secondary slots | Plays like |
| --- | --- | --- | --- | --- | --- |
| **Skitter** (light) | 70 | 0.85x | 98% | 1 | Hit and run: fastest, smallest hitbox, fastest turning, fragile |
| **Tinker** (standard) | 100 | 1x | 95% | 2 | The all-rounder. Every speed in this manual is for the Tinker. |
| **Hauler** (heavy) | 150 | 1.2x | 85% | 3 | Holds tunnels and soaks damage; largest hitbox, slowest turning |
| **Tug** (support) | 110 | 0.8x | 90% | 2 | Scavenges faster (30 scrap/s, 3.5 u reach) and passes scrap to teammates |

"Damage dealt" multiplies the damage and knockback of every weapon the ship fires. "Top speed"
is boosting diagonally on all three axes (see [Flying](#flying)). **Every type stays slower than
a laser bolt**, so running away works only with distance and the bolts' 3-second lifetime.

**Choosing a ship:**

- On the start screen, click your first ship's card in the **Ship** row.
- In game, press **B** to open the **ship bay**. It shows each type's hull turning, its numbers and the
  loadout you would spawn with. Press **1** to **4** (or click a card) to pick the ship for your
  **next respawn**; the card shows "(next)" and the bay closes. Your current ship is marked
  "(flying)". Press B again to close the bay without choosing.
- When you are destroyed, the bay opens by itself ("Destroyed · choose your next ship"). Pick a
  ship before the 3-second respawn, or keep flying the same type.
- A server's rules can ban ship types. Banned types show "(not allowed)" and can't be picked.

**Tug scrap passing:** hold **R** within 3.5 u of a teammate who is damaged or low on scrap to
give them 20 scrap per second from your store. Like scavenging, it repairs their hull first.

## Your ship: hull, energy, boost and scrap

Four resources are linked, so spending one puts pressure on another. The numbers below are the
same for every ship type except hull (see above) and the Tug's scavenging.

| Resource | Max | At spawn | Used for | How it refills |
| --- | --- | --- | --- | --- |
| **Hull** | 70 to 150 by ship | full | Your health. At 0 your ship is destroyed. | **Only** with scrap: scavenging (hold R) or a Tug passing scrap. It never regenerates on its own. |
| **Energy** | 100 | 100 | Energy weapons, gadgets | Regenerates 10 per second, **but only while you aren't boosting**. A Leech Arc steals it from others. |
| **Boost** | 100 | 50 | Higher top speed while you hold Shift | Hard braking and near-miss wall scrapes (see below). It doesn't regenerate on its own. |
| **Scrap** | 100 | 20 | Kinetic weapons (Rivet Gun, Railspike, Scrap Shotgun) | Scavenging, once your hull is fully repaired |

**Boost** drains 20 per second, but only while you hold Shift **and** thrust in some direction.
From full, that's 5 seconds of boost. Two ways to earn it back:

- **Hard braking:** thrust against your current motion while moving faster than 6 u/s (for
  example, press S while flying forward fast). The more speed you brake away, the more boost
  you get. Just letting go of the keys and drifting to a stop gives nothing.
- **Near-miss scrapes:** fly past a wall closer than about half a ship length without touching
  it, at 8 u/s or more. The closer you skim, the faster boost refills (up to 30 per second).
  Actually touching the wall gives nothing: you bounce off instead.

Walls don't damage you, so scraping is a free way to recharge. Watch out for **charged** rock
(blue sparks, left by the Leech Arc): touching it drains 5 energy per second.

**Scavenging (hold R):** takes scrap from wrecks, scrap piles and loose debris within 3 u of your
ship (Tug: 3.5 u), at 20 per second (Tug: 30). The scrap repairs your hull first, 1 hull per
scrap. Once the hull is full, the rest goes into your scrap store for the kinetic weapons. Stay
close while you do it. Debris you empty disappears.

**Overdrive (hold F):** when your energy is too low for an energy shot, holding F lets you fire
anyway. The missing energy is paid with **2 hull for every energy point**. Overdrive never
destroys your own ship: if a shot would take your hull below 1, it doesn't fire. This is
desperate but sometimes match-winning. Be careful with rapid-fire energy weapons such as the
Paint Thrower or the Plasma Repeater: in overdrive they burn hull fast, and even faster while you
boost (boosting stops energy regeneration).

**Destroyed:** when your hull reaches 0, you respawn after **3 seconds** at your team's side of
the level (as the ship you picked in the bay), with full hull and energy, half boost, 20 scrap
and the starting loadout. Weapon upgrades, picked-up weapons and a picked-up gadget are lost.
Your wreck stays where you died as a scrap pile (Skitter 20, Tinker 30, Hauler 45, Tug 35,
plus half the scrap you carried).

## Flying

The game uses **units (u)**: 1 u is one ship length. Speeds are in u per second.

Thrust is limited **per axis of your ship**, as in the classic game Descent. Forward/back,
left/right and up/down each have their own top speed. For the Tinker:

| How you fly | Top speed |
| --- | --- |
| Straight, cruising | 12 u/s |
| Straight, boosting | 16.5 u/s |
| Diagonal on all three axes, cruising | about 21 u/s |
| Diagonal on all three axes, boosting | about 28.6 u/s |
| For comparison: a Scrap Laser bolt | 30 u/s |

The other ship types: [ship reference, Flight](reference/ships.md#flight).

**Trichording** is the core movement skill: hold forward, a strafe key **and** up or down at
the same time (for example W + D + Space). You then fly diagonally at 1.73 times (the square
root of 3) your straight-line speed. Your nose points about 55 degrees away from the direction
you actually travel, so practice aiming while trichording. Two keys at once (W + D) already
give 1.41 times the speed.

More flight tips:

- **Nothing outruns a laser.** Even a boosting, trichording Skitter is a little slower than a
  laser bolt. Bolts expire after 3 seconds, so distance and dodging are what save you.
- **Your shots don't inherit your speed.** Every projectile leaves at its own fixed speed in the
  direction your nose points. Lead moving targets: aim where they will be.
- **Aim at what you see.** The server compensates for your connection delay when it checks hits,
  so you don't need to lead extra to make up for ping.
- **Let go to stop.** With no keys held, the ship slows down and stops by itself. After a boost
  ends, speed above the cruise limit bleeds off quickly.
- **Brake hard to recharge.** Reversing thrust at speed refills boost. A quick stop behind cover
  after a boosted run pays for the next one.
- **Turning has a limit.** Very fast mouse flicks are capped at a maximum turn rate (fastest on
  the Skitter, slowest on the Hauler), and rolling (Q/E) is slower than turning with the mouse.
- **Every hit pushes you.** Knockback grows with damage; a Heavy Rocket or a Tectonic shockwave
  throws you across a cavern. Shields don't reduce the push.
- **Mind the paint.** Red paint bounces you off walls, blue paint makes you slide, yellow paint
  slows you to 40% while you touch it. See [Paint colours](#paint-colours).
- **Horizon assist** slowly rolls you back level whenever you're not pressing Q or E. It pauses
  while your nose points almost straight up or down. Turn it off (H) if you want full control.
- **Leaving the tab** (switching tabs or windows) releases all keys. Online your ship stays in
  the match and drifts to a stop, so don't leave it somewhere dangerous.

### Will I fit?

Your ship is a sphere for collisions: 0.8 u across for the Skitter, 1 u for the Tinker, 1.1 u
for the Tug and 1.3 u for the Hauler. The **fit ring** around the crosshair is that sphere drawn
where your ship would first touch rock if you flew straight ahead:

- **Ring inside the opening**: you fit. The ring shows where you would stop (the end of a dead-end
  tunnel, or the wall you are aiming at; it is pale blue then).
- **Ring bigger than the opening** (amber): you won't fit. Your hull would touch the rim (or a
  wall you would scrape) before you reach the point you aim at.

The ring appears within 12 u of the rock and fades away when you are pressed against it. You
can switch it off on the start screen (**View: Fit ring**).

## Weapons

Weapons come in four families: **energy** primaries (slow bolts you can dodge, paid with
energy), **kinetic** primaries (very fast rounds paid with scrap), **ordnance** (missiles and
mines with a limited count) and **specials** (comedy weapons with a trick). Every weapon has a
bonus, a malus and a **lasting effect** on the level. The full numbers (damage, fire rate, cost,
speed, range, splash, crater size, how it meets a Shield) are in the
[weapon reference](reference/weapons.md).

### Carrying and selecting weapons

- **Primaries** (left mouse button): you can carry any number. Keys **1** to **9** and **0**
  select your 1st to 10th carried primary, counted in the order of the lists below (Scrap Laser
  first, then Scatter Laser, …, Polite Laser last). The weapon panel shows each weapon's key.
  The **mouse wheel** cycles through all of them.
- **Secondaries** (right mouse button): you carry as many kinds as your ship has slots
  (Skitter 1, Tinker and Tug 2, Hauler 3). **X** cycles through them. Picking up a new kind
  with every slot full **replaces the selected one**, so select the one you can spare first.
- **Pickups:** fly within about a ship length of a weapon pickup to take it. A weapon you
  already carry gives ammo instead (ordnance, up to its maximum) or a level (Scrap Laser, up to
  6). A pickup that would do nothing for you stays for someone else.
- You spawn with the rule set's loadout. With the default rules: Scrap Laser, Paint Thrower,
  3 Bolt Rockets and a Shield. A server's rules may give you other weapons.
- Online, the HUD's counts come from the server and are exact.

### Energy primaries

Paid with energy; the bolts fly at about laser speed, so good pilots dodge them.

- **Scrap Laser:** straight bolts at 30 u/s, 4 per second, 6 damage at level 1. Upgrade pickups
  raise it to level 6 (14 per hit); upgrades are lost when you die. Hit rock glows hot for 10 s.
  *Tip:* cheap enough to fire all day. Keep one eye on upgrade pickups.
- **Scatter Laser:** 3 bolts 8 degrees apart; the fan alternates between horizontal and vertical
  with every shot. Damage and push fall to 40% towards the end of a bolt's life. Leaves a fan of
  scorch marks. *Tip:* best at close to mid range, where the whole fan connects.
- **Plasma Repeater:** 10 bolts per second at 33 u/s, 3 damage each, 1.5 s reach (about 50 u).
  Burns 30 energy per second against 10 regeneration, so a full bar lasts about 5 seconds of
  fire. Every 2nd bolt melts a small pit into rock. *Tip:* fire in bursts and don't boost while
  you do.
- **Fusion Charger:** hold the trigger to charge for up to 2 s (the charge meter shows it), then
  release: 12 damage uncharged up to 48 at full charge, with matching knockback. Holding past
  full charge burns 10 hull per second (never below 1). The shot melts a channel 1 to 5 u deep
  through rock (thin walls are melted through) and shoves debris aside. While you charge, energy
  gathers in a glowing orb at your nose; it grows, pulses faster and crackles as the charge
  fills, flares at full charge and turns red and unstable when you overcharge. **Everyone sees
  it** (and the cave lights up around you), so a ship with a growing purple orb is about to
  fire a heavy shot. The channels it melts are drawn as wide as they really are, so you can see
  whether your ship fits through. *Tip:* charge behind cover, release as you peek out.
- **Corkscrew:** 5 bolts in a line that spins around your aim, widening as it flies. Only the
  centre bolt reliably connects at range. The spinning bolts etch spiral grooves into walls.
  *Tip:* point blank, every bolt hits.
- **Ricochet Arc:** bolts bounce off walls up to 3 times, chipping the rock at every bounce.
  After the first bounce they can hit **you**. *Tip:* shoot around corners into rooms you can't
  see; don't fire it into a wall in front of your face.
- **Leech Arc:** an instant arc (no projectile) to the nearest enemy within 12 u in front of you,
  10 arcs per second: 2 damage and 2 energy drained into your tank per arc. It stays locked on
  that target while it stays within 12 u and roughly in front of you, even if someone else
  comes closer. Beyond 12 u it does nothing (and still costs energy). Rock the arc touches stays
  **charged** for 15 s and drains 5 energy per second from ships that scrape it. Your own arc
  shows the moment you fire (online too, without waiting for the server) and follows its
  target. *Tip:* the drain feeds your own weapons; stick to your target like glue.

### Kinetic primaries

Very fast rounds (6 to 9 times laser speed) that fire your **scrap**, the same scrap that repairs
your hull, so every shot competes with healing.

- **Rivet Gun:** 180 u/s, 10 rivets per second, 3 damage each, half a scrap per rivet (5 scrap
  per second). Rivets embed in walls; on weak rock they sometimes knock a debris chunk loose.
- **Railspike:** 270 u/s, 35 damage, one shot every 1.5 s, 4 scrap. It pierces every ship and
  debris piece in its path and walls up to 2 u thick, drilling straight holes through thin
  rock. The muzzle flash reveals you (a cloaked shooter becomes visible for half a second).
  *Tip:* line up enemies in a tunnel, or shoot through a thin wall at someone behind it.
- **Scrap Shotgun:** 8 pellets at 200 u/s in a random cone, 3 damage each, 4 scrap per shot.
  Useless at range: pellets vanish after 30 u and damage falls to 30% by 24 u. Pockmarks walls
  and sprays fresh debris you can scavenge.

### Ordnance

Missiles and mines with a limited count. You find them as pickups (default rules: you start
with 3 Bolt Rockets). All of them explode (the splash also hurts **you**) and leave a crater,
except the Flash Pod.

- **Bolt Rocket:** dumbfire, 40 u/s, 30 damage direct plus splash within 2.5 u, 2 u crater.
  Carry up to 10.
- **Seeker:** homes on the nearest enemy (or enemy Decoy) in a 30 degree cone within 60 u, and
  keeps re-targeting. Slow (25 u/s): a boosting pilot outflies it, and a Decoy fools it.
- **Quicksilver:** the fastest missile, 45 u/s, but a small 1.5 u blast. Hard to dodge.
- **Eyeball:** a guided missile you steer with the mouse through its own camera (the view rides
  the missile). While you guide it, **your ship drifts with no thrust or turning** and you can't
  fire another secondary. The view returns when it hits. 4 u crater. *Tip:* launch it from a
  safe spot and fly it around corners.
- **Swarm Pod:** hits like a rocket, then releases 6 weak homing blobs that each chase a target
  within 30 u. Leaves a scatter of small craters.
- **Heavy Rocket:** 70 damage direct, a big 8 u blast, but slow (20 u/s). The 6 u crater can
  open a passage between tunnels. Its splash hurts you badly at close range.
- **Flash Pod:** bursts on impact or after 3 s and blinds every pilot within 15 u who can see it
  for 2 s (a white screen), **you and your teammates included**. No damage. Nearby surfaces
  glow for 30 s, lighting up dark caverns and anyone inside.
- **Prox Mine:** dropped behind you, arms after 1 s, sticks to walls and goes off when a ship
  comes within 2 u (40 damage plus a 5 u blast, 3 u crater). It hurts you and your teammates at
  **full** damage (unless the server has friendly fire off). *Tip:* drop them in tunnels you
  know the enemy uses, not the ones your team uses.
- **Stalker Mine:** arms after 1 s, then creeps at 4 u/s toward the nearest enemy it can see
  within 10 u and goes off within 1.5 u. It leaves a scorched trail. Slow: a pilot who notices
  it just flies away.
- **Tectonic Charge:** the earthshaker. See [Surviving a Tectonic Charge](#surviving-a-tectonic-charge).
  Only one exists on the map at a time (90 s respawn), you can carry only one, and it
  damages you like anyone else.

### Specials

The comedy weapons: rarer pickups, each with its own trick.

- **Echo Cannon (primary):** the shot flies out, then after 2 s flies back along the same path,
  so it can hit twice. The return leg can hit **you** (a Shield cuts that by 75%). Leaves scorch
  marks along the path. *Tip:* fire it past an enemy, then dodge your own echo.
- **Swap Harpoon (secondary):** on a hit you swap places **and velocities** with the ship you hit
  (any ship, teammates included): you inherit their momentum. 4 s between shots. A harpoon that
  hits a wall leaves a spike there; hold **R** within 1 u of your own spike to hold on to it like
  a grab point.
- **Paint Thrower (primary):** paints the rock in your team's colour (that's what scores in
  Paint the Void), 10 blobs per second, each painting a patch 4 u across. It deals no damage.
  Press **V** to switch its **effect colour**: red (bouncy), blue (slippery) or yellow
  (sticky). See [Paint colours](#paint-colours). Paint stays until someone paints over it.
- **Borrowed Time (secondary):** rewinds your position, speed and hull to where they were 3 s
  ago. Your weapons go offline for 3 s, and a ghost of your ship stays at the spot you left for
  3 s. Craters made meanwhile stay. *Tip:* fire it right after you took a big hit to get the
  hull back, or to escape a Tectonic zone you flew into.
- **Magnet Sneeze (secondary):** a sneeze that pulls everything in a 60 degree cone within 20 u
  toward you: debris, enemy ships, pickups and scrap piles (they jump up to 10 u closer). It also
  pulls in **every projectile** in the cone, mines stuck to walls included. On weak rock it rips
  chunks loose (debris full of scrap). *Tip:* great for scavenging and for grabbing a pickup from
  across a room.
- **Polite Laser (primary):** hold the trigger: after a 1.5 s charge it fires by itself, a
  75-damage bolt that burns clean holes through thin walls. The target gets an "Excuse me…"
  warning the moment you start charging, so a sharp pilot gets out of the way.
- **Seed Grenade (secondary):** grows a small rock structure where it hits: block a tunnel or
  build cover. In mid-air it grows floating cover. There's a 2% chance it grows a giant rubber
  duck instead. The rock is ordinary terrain and blocks your shots too.
- **Paperweight (secondary):** a lobbed stone (25 u/s, 3 per pickup, one every 2 s) that stops
  where it hits rock, or in mid-air after 1.2 s, and stays there. For 6 seconds it is a small
  **gravity well** (a violet glow): it pulls ships within 16 u (up to 30% of a Tinker's thrust),
  debris, mines and missiles toward it, **you included**; laser bolts and kinetic rounds fly
  straight. It does no damage. When it runs out, the debris it gathered fuses with it into a
  floating lump of weak rock. *Tip:* throw it into a rival's racing line or just past a
  corner they have to take. In the built-in rules it is a pickup only in Belt Up and the sandbox
  rules.

### Damage rules

- **Friendly fire** is 50% by default: shots from teammates deal half damage and half knockback.
  Your own splash always hurts you fully (a Shield cuts self-damage by 75%). Exceptions: Prox
  Mines hit teammates at full damage, and a Decoy's return hit is never reduced.
- Every hit pushes the target in the direction of the shot. The harder the hit, the bigger the
  push. Explosions push ships and debris outward.
- Your ship type multiplies the damage and push of everything you fire (Skitter 0.85x, Tinker
  1x, Hauler 1.2x, Tug 0.8x).
- A kill counts for whoever fired the killing shot. Destroying a teammate or yourself doesn't
  count as a kill, but still counts as a death for the victim.

## Surviving a Tectonic Charge

The Tectonic Charge devastates everything within 35 u and permanently reshapes that part of the
level. It flies slowly (15 u/s), and everyone is warned when one is launched.

1. **Launch:** everyone in the level gets the banner "Tectonic Charge launched".
2. **Impact:** the charge sticks to whatever it hits (rock, a ship, debris, a decoy: it rides
   along). The ground cracks, the view starts to shake, and a banner with a **1.5-second
   countdown** names the zone you are in.
3. **Detonation:** the main blast, then 6 sub-charges scatter up to 20 u and explode 0.5 s later
   (35 damage each within 6 u).
4. **Aftermath:** the collapse zone caves in, a dust cloud cuts visibility for 20 s, and a debris
   field full of scrap (24 pieces) is left behind. Loose debris everywhere in the level jostles.

| Zone | Radius | Effect | Time to clear from the impact at full boost |
| --- | --- | --- | --- |
| Core | 4 u | Instant kill, through any Shield | 0.14 s |
| Damage | 25 u | Heavy damage (up to 110), falling off with distance | 0.87 s |
| Collapse | 35 u | The rock caves in; cracks run further the higher the world's Instability | 1.22 s |
| Shockwave | 60 u | Knockback and screen shake, no damage | 2.1 s |

**How to survive:**

- **Boost straight away from the charge the moment the banner says "Ground cracking".** Don't
  turn to look; trichord away (forward + strafe + up/down) for the full diagonal speed. At full
  boost you clear the damage zone in under a second.
- **Keep boost in reserve** when someone on the other team is carrying one (the charge is a
  pickup: watch the "Tectonic Charge launched" banner).
- **Put rock between you and the blast** if you can't get far enough. Then fly out of the
  collapse zone: the cave-in and the sub-charges follow.
- **If it sticks to you**, you carry it until it goes off. Fly away from your teammates.
  **Borrowed Time** can take you back 3 s, out of a zone you flew into.
- **Afterwards**, the debris field is the richest scrap in the level. Scavenge it (hold R),
  but the dust cloud hides enemies doing the same.

## Gadgets

Every ship has one gadget slot; **G** uses it. With the default rules you start with a **Shield**;
the Cloak and the Decoy are pickups. Picking up a different gadget replaces yours (the new one
starts switched off). All numbers: [gadget reference](reference/gadgets.md).

- **Shield** (toggle, 5 energy to switch on, then 4 energy per second): mainly protects you from
  **your own** weapons: splash, ricochets, your returning Echo Cannon shots (self-damage -75%).
  Against enemy fire each weapon has its own setting: **blocked** (the Shield's energy pays,
  never the hull: Flash Pod, Paint Thrower, Seed Grenade and others), **wears it down** (energy
  pays, the hull takes what the energy can't cover: lasers and most missiles) or **partly passes
  through** (Railspike 75%, Rivet Gun and Scrap Shotgun 50%, Tectonic Charge 100%). The full list
  is in the [gadget reference](reference/gadgets.md#shield-and-weapons). Knockback isn't reduced,
  and the Shield drops when your energy runs out. *Tip:* switch it on for heavy fights and rocket
  duels, off to save energy for your weapons.
- **Cloak** (toggle, 5 energy, 3 energy per second, **3x drain while you move**): enemies don't
  see you unless they are within 6 u, or you fired in the last half second. Where a cloaked ship
  can be seen (teammates always, enemies up close), it shows only as a bending of the view
  behind it, like a lens, with a faint shimmer at its edges in its team colour. *Tip:* sit still
  on a route the enemy uses and fire at point blank.
- **Decoy** (instant, 30 energy, 10 s cooldown): launches a copy of your ship along your aim. It
  draws fire (projectiles hit it, Seekers and Swarm blobs chase it) and has as much integrity as
  your ship's max hull. After 6 s it snaps back to you like a rubber band, very fast, and hits
  **the first ship on its way back** for 99% of whatever integrity it has left. That's you if
  nobody is in the way, and it can be a teammate. A decoy shot down to half its integrity hits
  for about half. *Tip:* when the timer runs out, line yourself up so an enemy sits between you
  and your decoy.

The HUD's gadget line shows "on", "off", "ready" or the cooldown in seconds.

## The level fights back

Every hit leaves a mark that lasts, so a long fight visibly carves up the arena.

### Comedy events

The level has moods, and now and then it plays a joke on everyone: a cat paw sweeps through the
wall, a tax drone hunts the leader, rubber ducks parade through the tunnels, hats rain down,
the game turns into a side-scroller for ten seconds. How often depends on the world's
**Absurdity** slider (a server's rules can cap it; 0 means none). Every event is announced
3 seconds ahead with a banner, the announcer's line and a tell in the world, and nothing an
event does gives anyone kill credit. Nothing happens in a match's first 45 seconds or its last
20. The [comedy event reference](reference/events.md) lists all 18 with their numbers.

To try one without waiting, start practice with `?event=<id>` in the address, for example
`?event=duck_parade` (two seconds after you spawn), `?event=hat_rain@10` (after ten
seconds) or `?event=all` (every event in turn). The ids are in the reference.

### Comedy events: the level's own jokes

Every event is announced 3 seconds before it starts: a card under the score bar with its name and
icon (and the pilot it aims at, if any), the announcer's line and a fanfare. While it runs, a
small pill under the score bar shows its name and a ring of the time left. Eight of the jokes are
the level itself:

| Event | What happens | What you see and hear |
| --- | --- | --- |
| **Bored level** | The level's "up" turns 90 degrees over 5 s and stays turned | Sleepy Z's drift over the screen while it is announced, then a long yawn and the stone creaking; the sky, sun and shadows turn, horizon assist rolls you with them, grit slides toward the new down, and an arrow around the crosshair shows where "down" is now |
| **Level hiccups** | For 15 s, every 3 s or so, the level lifts everything about 2 u | "HIC!" pops on the screen, the view dips and nods, dust and pebbles shake off the ceilings and bubbles rise from every ship |
| **Gravity coffee break** | Every gravity well switches off for 20 s | A coffee mug drops onto each well's core stone, the wells dim and stop swirling, the mugs steam and take a sip now and then, lounge music plays; when the break is over the mugs lift off |
| **Wrong genre** | For 10 s every ship is locked to a flat plane, facing left or right | The game turns into an 80s side-scroller: the camera swings out to the side with the rock in front cut away (bricks), the picture drops to big pixels in 16 colours, everyone is drawn in your plane (shots hit by their position on the screen), a chiptune plays, then a 1-UP jingle |
| **Big head day** | For 15 s every ship is three times as easy to hit with shots | Every ship wears a balloon head of exactly that size with googly eyes that follow you; hits honk and make it wobble; at the end they all pop |
| **Rubber chickens** | For 12 s every new shot is a rubber chicken (same speed, damage and craters) | Shots fly as squeaking rubber chickens (the Tectonic Charge as a giant egg) and burst into feathers where they hit |
| **Moving day** | Every pickup moves to another pickup point; for 2 s nothing can be taken | The pickups are packed into taped cardboard boxes that hop to their new places, land with a thump and pop open |
| **Everything is cake** | For 20 s craters are bigger and debris repairs twice as much hull | The rock turns into a layer cake in a wave (sponge, cream, jam, pink frosting with sprinkles) after a party horn; shots throw cake crumbs |

### Comedy events: visitors

Some comedy events send a visitor into the level. Each one is announced 3 seconds ahead, and
the announcer names the pilot it is after.

| Visitor | What you see | What to do |
| --- | --- | --- |
| **Tax drone** | A beige box drone with a frowning visor, a red tie and a red-and-blue siren waits at a cavern entrance, then chases the leader | Every touch takes 10% of the leader's energy (you see it streaming into the drone's receipt printer; up to 5 touches, and the receipt gets longer). Shoot it down (40 hp) for a scrap pile of what it took |
| **Pizza delivery** | A red delivery drone with a steaming pizza box rings the doorbell next to a hurt pilot | The recipient holds still near it for 3 seconds in total: a pizza assembles slice by slice under the ship (and a ring fills around the crosshair), then the hull is full. Anyone else can shoot the drone (30 hp) and the pizza becomes 50 scrap |
| **Customer survey** | A teal call-centre drone holds a five-star panel right in front of the leader's view ("How was your match today?") | Shoot a star: the star the destroying shot goes through is the leader's rating, shown to everyone and listed in the match results. Unanswered, it floats off after 12 seconds |
| **Mimic pickups** | Some weapon and upgrade crates tremble and peek, then grow teeth, a tongue and googly eyes and hop at the nearest pilot | They bite (12 damage). Shoot one (25 hp) and its loot drops where it died, real again; the survivors hop home after 20 seconds |
| **Gossip wall** | A stone face pushes out of the rock nearest the pilots and starts whispering | For 15 seconds everyone is revealed to everyone, cloaked pilots too: pink eye markers with names show through the rock, and the face tells you where people are ("Psst. Ana is 40 u below you. Again.") |

Damage from visitors gives nobody a kill.

### Comedy events: spectacles

The last five are big shows:

| Event | What you see and hear | What to do |
| --- | --- | --- |
| **Duck parade** | Five giant rubber ducks waddle along the tunnels to a kazoo march, trailing soap bubbles. The first wears a top hat, the last wears sunglasses. In tight tunnels they squeeze thin and long, and they pop back into shape after. They squeak when they swallow a shot or shove a ship. | Get out of the way: ducks push ships aside and soak up shots. They do no damage. |
| **Hat rain** | Hats flutter down and land on the rock: top hats, cowboy hats, party cones, chef's hats, crowns, propeller beanies, fezzes and viking helmets. Fly through one and it pops onto your hull. Your stack sways when you turn, and the HUD shows your count. | Collect them. With three or more, a gold glint above your stack shows where you are, and the HUD says "Everyone sees you". Whoever wears the most hats at the end is named the **Hat Baron**. |
| **Cat paw** | The boundary wall near a group of ships glows with the paw's print, and three red-hot claws poke through. Then a giant ginger paw with pink toe beans bursts through and swipes along. Rock sprays where its claws rake, and ginger fur drifts down after it. | Leave when the wall starts to glow. The paw hits for 30 and throws you hard. |
| **Disco inferno** | The sun, sky and cave light drop to almost nothing, and your headlight dims too. A mirror ball comes down over the fight with eight coloured spotlights. Every ship leaves a glowing trail in its side's colour, even in black-and-white levels, and a four-on-the-floor beat plays. | Use the trails to find the others, and remember they can see yours. |
| **Tired announcer** | For 30 seconds, every kill plays as a 3-second replay from behind the killer in a small "Kill-cam" window at the top left, with a deadpan line from the announcer. While the event is being announced, the window shows static and "Zzz". | Nothing. Enjoy the replays. |

Some modes leave events out where they would break the mode: The Architect has no random
events (the architect starts them), The Squeeze has no Cat paw or Duck parade once the walls are
in phase 4, Belt Up has no Bored level, Wrong genre or Moving day, and Instagib has no Tax
drone or Pizza delivery. A host's rules can ban single events.

### The level's mood

The level is always in one of four **moods**: calm, bored, grumpy or giddy. The mood changes
every 60 to 90 seconds; a small face at the top left shows it (with a line for a moment when it
changes), and the level's light, the colour of the floating dust and the background hum follow
it. A mood never changes how you fly or fight, but it makes its own jokes twice as likely:

| Mood | Its events |
| --- | --- |
| Calm | none; the breaks between events are half again as long |
| Bored | Bored level, Level hiccups, Moving day |
| Grumpy | Cat paw, Tax drone, Customer survey |
| Giddy | Duck parade, Rubber chickens, Disco inferno, Hat rain, Pizza delivery |

### Gravity wells

Worlds with **Gravity Chaos** above 0 have up to seven gravity wells: a dark core stone floating
in a round hollow, glowing violet, with motes and rock chips swirling around it and the view
bending around it like a lens. A well pulls everything within 20 to 40 u toward its core
(stronger and farther the higher the slider): your ship, debris, mines and every missile and
grenade (the Seed Grenade and the Paperweight too). **Laser bolts and kinetic rounds fly
straight.** The pull on a ship is capped at 40% of its thrust, however many wells overlap, so you
can always thrust out; a ship that just drifts slowly falls in. Near a well the **pull indicator**
around the crosshair shows where and how hard it pulls.

*Tips:* curve rockets around a well into a target behind it; don't park next to one; a fight in
a well's swirl favours whoever keeps thrusting. During the **Gravity coffee break** event every
well rests for 20 s. The [Paperweight](#specials) and The Architect's Gravity Chaos stroke make
temporary wells, and asteroid belts can have **shepherd moons** that pull like wells. All
numbers, and which weapons wells bend: [world physics reference](reference/world.md#gravity-wells).

### Instability and cave-ins

**Instability** makes the rock give way: every crater is up to 40% wider (at Instability 100;
none at 0), and blasting the same spot again and again strains the rock until the tunnel
**caves in**. Roughly, at Instability 30 it takes about ten rocket craters in one place within
20 seconds, at 100 about four; at 0 tunnels never cave in.

1. **Warning (1.5 s):** glowing cracks open in the ceiling, dust and pebbles trickle down, the
   view shakes, the stone rumbles and the announcer may say something about it.
2. **Cave-in:** a hollow opens in the ceiling and its rock comes down as **rubble** in the tunnel
   below, with debris full of scrap. Ships within 3 u of the rubble take 20 damage (nobody gets
   the kill).

Rubble is **weak rock**: two or three rockets clear it. A cave-in never seals a ship in (rubble
that would is left out), never happens near the team spawns, the Heist core chamber or an
extraction zone, and at most once every 8 seconds per level. *Tip:* when the ceiling cracks,
leave the tunnel, then come back for the scrap. Numbers per Instability value:
[world physics reference](reference/world.md#instability-and-cave-ins).

### Black-and-white levels and pigment

The **Palette** slider drains the colour from a world: up to 49 the colours fade to half; from
50 ink outlines, banded light and paper grain fade in; from **80** the level is a black-and-white
**ink world** with hatched shadows and an ink-wash sky. Paint always keeps its colour (it is
Paint the Void's score).

Black-and-white levels (Palette 80 and above) have **pigment pickups**: three drops of paint
circling a glowing core, at the pickup points (with the built-in rules one per 4 players, at
least 2, back 30 s after they are taken).

- **Pigmented:** touching one paints your ship in your side's colour for **40 seconds** (another
  pickup restarts the time): every weapon does **25% more damage**, but you fly in full colour,
  drip paint and **everyone sees a marker on you through walls**. If you are destroyed, your
  pigment drops as a splash that anyone can take for 15 seconds.
- **Grey:** without pigment your ship is ink grey and blends in: enemies see your name only within
  20 u and never through walls. Teammates still see you in a tint of your team colour.

Pick your moment: pigment wins fights but tells everyone where you are. In **Last Color Standing**
pigment pickups give back **17 colour** instead (half a death's worth; no damage bonus there,
45 s respawn). Each rule set's pigment pickups: [world physics reference](reference/world.md#pigment).

### Craters and rock strength

Explosions blast craters (up to 6 u across for a Heavy Rocket, a 35 u cave-in for a Tectonic
Charge) that stay for the rest of the match. Craters erase paint, and the fresh rock starts
unpainted. **Rock strength** varies by region: **weak rock** (lighter, sandy, crumbly) blasts
1.4 times wider, and kinetic rounds and the Magnet Sneeze knock chunks off it; **hard rock**
(darker, cooler, with glinting specks) only 0.6 times. Other marks: Fusion Charger channels,
Railspike and Polite Laser holes through thin walls, Corkscrew spirals, Plasma pits, Ricochet
chips, new rock from Seed Grenades, and on worlds with **Instability** wider craters and
cave-ins (see [below](#instability-and-cave-ins)).

### Surface states

Some weapons change the surface of the rock for a while:

| State | Looks like | Caused by | Lasts | Effect |
| --- | --- | --- | --- | --- |
| **Hot** | Orange, pulsing glow | Scrap Laser and Plasma Repeater hits, Fusion Charger channels, fresh scorch | 10 s | Visual only |
| **Charged** | Dark blue with bright sparks | Leech Arc | 15 s | Drains 5 energy per second from a ship scraping it |
| **Glow** | Pale green light | Flash Pod | 30 s | Lights up dark caverns and anyone inside |
| **Scorch** | Charred, sooty black | Scatter Laser marks, Stalker Mine trail, Echo Cannon path and impacts | Rest of the match | Visual only |
| **Paint** | Your team's colour | Paint Thrower | Until painted over or blasted | Scores in Paint the Void; coloured paint changes how ships fly (below) |

### Paint colours

The Paint Thrower always claims the surface for your team. Its **effect colour** (switch with
**V** while the Paint Thrower is selected; the HUD shows "V paint: …") decides what the paint
does to **every** ship that touches it, yours included:

| Colour | Pattern | Effect on ships touching it |
| --- | --- | --- |
| **Red** (default) | Stripes | Bouncy: you rebound off it at 90% of your speed (plain rock: 20%) |
| **Blue** | Waves | Slippery: no slowdown while you touch it, and a wall hit keeps your speed, turned along the wall |
| **Yellow** | Dots | Sticky: your speed drops to 40% while you touch it |

The effect colour fills the paint, and the pattern is drawn in a dark shade of the owning
team's colour, so you can tell both the effect (by the pattern) and the owner (by the marks'
colour) without relying on hue alone. Painting over a patch replaces both its team and its
colour. Your colour goes back to red when you respawn.

*Tips:* yellow paint in a tunnel mouth slows chasers; red paint turns a cavern into a pinball
table; blue paint on the walls of a tunnel lets you scrape along at full speed.

### Debris and scavenging

- **Debris** are loose chunks of rock and scrap (0.3 to 1.5 u): knocked off weak rock, sprayed by
  the Scrap Shotgun, ripped loose by the Magnet Sneeze, and left in a big field by a Tectonic
  Charge. Debris drifts, bounces off walls and comes to rest.
- Debris **blocks shots** (it can be shot apart) and is **pushed** by ships ramming it, by
  explosions, by the Fusion Charger and by the Magnet Sneeze.
- Debris holds **scrap**: hold **R** next to it to scavenge, like a wreck. Emptied debris
  disappears. At most 64 pieces float in a level at once; the oldest goes first.
- **Wrecks** of destroyed ships and **scrap piles** at pickup points are the other scrap sources.
  Scrap is the only way to repair your hull.

### Dust clouds

A Tectonic Charge leaves a dust cloud 35 u in radius (70 u across) for 20 s. Inside it, the fog closes in and you
see much less, and so does everyone else inside it.

## Pickups and wrecks

Each classic level has **12 pickup points** in open space, spread across the level and away from the team
spawn points. Each point holds one item at a time: a weapon, a weapon upgrade, a gadget or a scrap
pile. Pickups float, bob and spin, tinted like their weapon's shots; scrap piles and wrecks are
heaps of junk that shrink as they are scavenged.

With the default rules every point rolls its item at random. About a third of the rolls are
scrap piles (30 scrap each), about 59% are weapons (most often Rivet Guns and Bolt Rockets,
rarest the Tectonic Charge, of which only one can be on the map), about 4% Scrap Laser upgrades
and about 4% gadgets (Shield, Cloak, Decoy). An item respawns 15 to 90 seconds after it was taken
(scrap piles after they are emptied). The exact weights and respawn times are in the rule set;
the [weapon reference](reference/weapons.md) lists how much ammo each pickup gives.

You collect a weapon, upgrade or gadget pickup by flying within about a ship length of it. A
pickup that would do nothing for you (a full ammo count, a Scrap Laser at level 6, the gadget you
already have) stays for someone else. Scrap piles and wrecks are scavenged with **R**.

On black-and-white levels (and in Last Color Standing) the pickup points also hold **pigment
pickups** (see [Black-and-white levels and pigment](#black-and-white-levels-and-pigment)). In
Belt Up every one of the belt's 8 sectors has 3 pickup points, and the same point in every sector
holds the same item (rolled again each heat), so no start slot is luckier than another.

## Game modes

A server plays its modes in a rotation (the host's playlist) and you stay connected from one
match to the next; see [Between matches: the lobby](#between-matches-the-lobby). The numbers
below are the built-in rules; a server can change them, and the
[mode reference](reference/modes.md) lists every built-in rule set with its numbers. The
scoreline at the top of the screen always names the mode and what wins it ("First to 15
kills", "Last ship flying wins", and so on).

**Teams or everyone for themselves.** Team modes have 2 to 4 teams: **Orange**, **Blue**,
**Lime** and **Magenta**. The server puts you on the team with fewer players (or on the team you
picked in the lobby, as long as the teams stay even). In free-for-all modes every other ship is
an enemy, each pilot has a colour of their own, and the scoreline shows the top three and your
place. Friendly fire (50% with the built-in team rules) only applies between teammates.

**Lives and spectating.** In The Squeeze, Hot Reactor and Last Color Standing you can run out:
once you are out, you **spectate** until the next match. Pilots who join one of these modes more
than 10 seconds after the start (Hot Reactor: after its first detonation) spectate too. While
you spectate, the flight keys and the mouse fly a free camera (Shift is faster, and it passes
through rock), the left mouse button follows the next ship and the right mouse button goes
back to the free camera.

### Paint the Void

Two to four teams try to paint as much of the level as they can.

- The match lasts **5 minutes**.
- Your team's score is the share of the **whole level's paintable surface** covered in your colour.
  The score updates twice a second, and the HUD and the scoreboard show it in percent.
- The paint's **effect colour** (red, blue, yellow) doesn't matter for the score; only the team
  does.
- When the timer runs out, the team with more painted surface wins. Equal coverage is a draw.
- **Kills don't score points.** They matter because a destroyed enemy stops painting for
  3 seconds and respawns far away.
- Painting over the other team's colour takes that surface from them.
- Craters erase paint, both theirs and yours.

Levels are large, so the percentages stay small: a close match can end with every team
under 1%. Only the comparison between the teams counts.

Tactics:

- Paint surfaces the other team has painted. That takes surface from them and adds it to your
  team at the same time.
- Keep energy for painting: boosting stops energy regeneration, and the Paint Thrower needs
  energy. A Leech Arc refills you from an enemy.
- A rocket into a freshly painted enemy wall wipes their paint. Then paint the crater.
- Use the effect colours: sticky yellow around your painted areas slows raiders down.
- Guard your team's painted areas and hunt the enemy painters.

### Team Brawl

Team deathmatch with respawns (2 to 4 teams), or every pilot for themselves.

- A kill scores **1 point** for you and your team. Destroying yourself (your own rocket, for
  example) or a teammate costs your team a point.
- The first team to the **kill target** wins at once. The target grows with the number of
  players: 5 kills per player in a team, at least 15 and at most 50 (8 players in 2 teams: 20).
  It never drops during a match, even when players leave.
- Otherwise the team with the most kills wins when the **10-minute** timer runs out; equal
  scores are a draw.
- You respawn after 3 seconds. You start with the Scrap Laser, 3 Bolt Rockets and a Shield
  (no Paint Thrower); everything else is a pickup.

### The Squeeze

Every pilot for themselves, **one life**, and the level shrinks.

- For the first minute nothing happens. Then the walls close in five **phases**: each one is
  announced 20 seconds ahead (the next safe zone appears and the walls glow cyan), then the walls
  grind inward for 30 seconds (amber, with glowing cracks) until only the next zone is left. The
  zones get smaller every time and their centre moves.
- A moving wall **crushes** you: 25 hull per second while you touch it, and the Shield doesn't
  help. A ship the walls swallow is destroyed.
- Closing walls shed **scrap debris** inward: something to scavenge on the way.
- Kills score a point, but only one thing wins: being the **last ship flying**. If the last two
  ships die in the same moment, it's a draw. After the fifth phase the walls keep closing slowly,
  so every match ends.
- The HUD shows the phase and its timer ("Phase 2 · walls close in 0:09", "walls closing") and
  warns you when you are outside the next zone ("Walls close in 0:09: reach the next zone") or
  being crushed.

Tips: head for the next zone as soon as it is announced; the walls stop exactly on its edge.
Fights next to a moving wall end badly for whoever gets pushed into it.

### Heist

Mine the **core** out of its vein at the bottom of the level and carry it to your team's
**extraction zone**. Two teams; with fewer than 4 players it is every pilot for themselves, and
any extraction zone counts.

- The core sits in a knot of **hard rock** in a chamber deep down. Blast it free: any weapon's
  craters count (4 to 5 Bolt Rockets do it). A gold outline on the rock shows where to dig.
- Once the core is free, the chamber's rock turns weak and its ceiling comes down for
  30 seconds: chunks of rock and debris fall.
- Fly into the free core to **take it**. The carrier is marked for everyone (a gold halo and a
  beam), flies at **75%** of its top speed and **can't boost**.
- Destroy the carrier and the core drops where it was: anyone can take it, and after
  20 seconds it returns to the vein.
- Fly the core into your **extraction zone** (at the level's surface, or your spawn cavern in
  fully enclosed levels) to score a delivery. The core is back in the vein, buried again, after
  15 seconds.
- First to **3 deliveries** wins, otherwise the most deliveries after **12 minutes** (equal: a
  draw). Kills don't score.

Tips: escort your carrier, it can't outrun anyone. Rockets dig fastest.

### Hot Reactor

Every pilot for themselves with **3 lives**, and a reactor nobody wants.

- A few seconds after the start the reactor appears on a random pilot. You can't drop it:
  **pass it** by being hit by another ship (the shooter gets it, even if your Shield took the
  hit) or by **ramming** someone. A new holder can't lose it again for 1 second.
- Its **fuse** runs 20 seconds, faster with every other ship within 15 u of the holder (two
  ships close by: twice as fast). The fuse keeps running when the reactor changes hands.
- When it **detonates**, the holder is destroyed and loses a life; the blast (10 u) hurts every
  ship near it. A new reactor appears on a living pilot 3 seconds later.
- Other deaths don't cost a life: you respawn after 3 seconds as usual. If someone destroys the
  holder, the reactor goes to them. Destroying yourself while holding it sets it off.
- The **last pilot with lives** wins. If the 15-minute timer runs out first, the most lives win.
- The HUD shows who holds it and the fuse ("Reactor 6.5 s · <name>") and warns you when you
  hold it or are too close.

Tips: holding it, fly at other ships and ram them; not holding it, stay out of the red fuse
sphere and don't shoot the holder.

### Last Color Standing

Every pilot for themselves on a **black-and-white level**.

- You start with **100 colour**. Every time you are destroyed (by anyone or anything) you lose
  **34**; damage alone takes nothing. At 0 colour you are out and spectate.
- Ships keep the share of their colour their meter has left, so a pale ship is one death from
  being out, and colourful ships are easy to spot. Your own meter is on the left of the HUD.
- **Pigment pickups** (paint drops at the pickup points, one per 4 pilots, at least 2) give back
  **17 colour**, up to the 100 you started with; they are back 45 seconds after they are taken.
- After **8 minutes** a stalemate drain starts: every pilot still in loses 1 colour every
  2 seconds, so the match always ends.
- The **last pilot with colour** wins; nobody left (the last two out at once) is a draw.

### The Architect

One pilot against everyone else, 2 to 8 players. A match is up to **three rounds of
4 minutes** (as many rounds as there are players, at most three), and the **architect** role
moves on every round in the order the pilots joined.

- **Runners** start at one end of the level and race for the **exit** at the far end (an open
  cavern). Flying into the exit scores **1 point**, and you are out of play until the next
  round. You have **2 lives** per round; runners **cannot hurt each other**. You start with
  the Scrap Laser and 4 Bolt Rockets (for digging through new rock); pickups are on.
- **The architect** has no ship. Instead they spend a **budget** (100 points at the round
  start, +4 per second, up to 250) on **strokes** that reshape a sphere of the level (12 to
  24 u across the radius) live:

  | Stroke | Cost | What it does |
  | --- | --- | --- |
  | Enclosure+ | 25 | grows rock in the region |
  | Enclosure- | 15 | opens rock in the region |
  | Density+ | 20 | pillars and floating rocks |
  | Gravity Chaos+ | 30 | a gravity well for 30 s |
  | Instability+ | 35 | a cave-in: weak rubble comes down from the ceiling |
  | Absurdity+ | 60 | a random comedy event |
  | Palette | 15 | an ink blackout of the region for 20 s |
  | Verticality | 80 | the level gets bored and turns (once per round) |

  Every stroke **glows for 2 seconds** before it acts, so runners see it coming; rock never
  grows over a ship or seals one in, and rock strokes stay clear of the start and the exit.
  New rock lands over a few frames, at most 32 pieces per stroke.
- Every runner who does **not** make it out by the end of the round scores **1 point for the
  architect**. The round ends early once no runner is left flying. The runners win the round
  if at least half of them got out.
- After the last round, the pilot with the **most points** (as runner and as architect) wins;
  equal points are a draw. The results list every round (who built, how many got out, who took
  it) and split each pilot's points into **Built** (as architect) and **Ran** (as runner).
- Bots play both roles: a bot architect places strokes in front of the runner closest to the
  exit, bot runners fly around glowing strokes and blast their way through new rock.

**Playing the architect.** A banner says "You are the architect" and you start in the **map
view**: high above the runners' start, looking down the level, with the rock above a cut taken
away so the tunnels below read like a floor plan. Your HUD is a drafting table:

| Key | What it does |
| --- | --- |
| 1-8 | pick a stroke (the cards along the bottom, in the order of the table above) |
| hold R | the stroke wheel: move the mouse toward a stroke, let go of R to pick it |
| mouse wheel | the stroke's size (12-24 u) |
| left click | place the stroke where the crosshair points |
| right click | put the stroke down (nothing picked) |
| V | switch between the map view and free flight |
| map view: WASD, Space/C, Q/E, mouse | pan over the level, move the cut up and down, turn, tilt |
| free flight: WASD, Space/C, Q/E, mouse, Shift | fly anywhere, through rock, fast with Shift |

- The picked stroke shows as a **wire sphere** where it would land: in its colour when it can
  go there, **red** when it cannot, with the reason under the crosshair (too close to the exit
  or the runners' start, not enough budget, recharging between strokes, four strokes already in
  flight, Verticality used, an event already running). A stroke the server still refuses (most
  often: Enclosure+ aimed where there is nothing to grow, Enclosure- where there is no rock to
  open) is named in a short note.
- The **budget bar** shows your points and, striped, what the picked stroke would cost; a card
  you cannot afford or use right now is dimmed with the reason.
- With the mouse free (Esc), the world's **sliders** appear on the right: nudge one to pick its
  stroke (Enclosure up or down, Density, Gravity Chaos, Instability, Absurdity, Palette,
  Verticality), then click the view to aim.
- You see every runner **through rock**: a marker with their name, distance and lives, and a
  thin pillar of light above their ship.

**Playing a runner.** The plate on the left names the architect and shows how much they have to
spend. Every stroke **glows** for 2 seconds as a sphere in its colour before it acts: its ring
fills up, it pulses faster, a beam of light stands over it so you can find it from far away,
sparks are drawn into it and you hear a rising whistle. A marker counts down to it, and when
your ship is inside or close to one the HUD warns you ("Enclosure+ in 1.2 s: rock grows here,
get out"). Then it acts: rock grows in a burst of dust, a gravity well opens, the ceiling comes
down, a comedy event pops in confetti, or the region fills with **squid ink** (and something in
it looks back). The **exit** is a green glowing gate with a beam over it and a marker that
always points to it. Through the exit, you watch the rest of the round with the spectator
camera; banners say who takes each round.

### Belt Up

A race for 2 to 8 pilots around an **asteroid belt**: a flat ring of asteroids around an open
centre with the **EXIT** in the middle. A match is **three heats** on the same belt.

- **Start.** Everyone waits in a start slot of their own (all 8 slots fly exactly the same
  course). Five **red lights** come on over your slot, one by one (also at the top of the
  screen); when they all go out, the announcer shouts **"Belt up!"** and you race. Your slot
  moves on by one each heat.
- **The lap.** Fly one lap **counter-clockwise** (seen from above) through the **8 gates**:
  glowing arches across the belt, numbered in *your* order (1 to 8; your own start gate is 8
  and wears a chequered band). The next gate breathes brightly, gates you passed dim to your
  colour. Gates count above and below the belt too, but not across the hot centre. Flying the
  wrong way flashes **WRONG WAY**.
- **The EXIT.** After the eighth gate the giant green **EXIT** sign and its swirling portal
  (visible from everywhere) are open for you: dive to the centre and fly into the portal to
  finish. Finished pilots watch the race: the camera follows the leader (left mouse button:
  the next ship, right: a free camera).
- **Radiation.** The belt shelters you; outside it radiation burns your **hull**, faster the
  farther out (see the table below), past any Shield, and nobody gets the kill. The sky
  above and below the belt glows with **aurora** where it is hot, your screen edges crackle
  with static, an irradiated ship sizzles, and the **radiation meter** above your weapons
  shows the dose and how long your hull lasts at it, with a Geiger counter ticking faster the
  hotter it gets. The open line is fast, the belt line is full of rock: pick yours. Shooting
  asteroids for scrap (hold R to scavenge) is how you repair on the way.
- **Dying** costs time, never helps: you come back at the last gate you passed after a 4 s
  hold, with the hull you had there (at least 30 %).
- **Rivals.** Ships **bump** each other (heavier ships push harder), **drifting boulders**
  orbit through the belt and push you around (a fast hit hurts), and every weapon works: Magnet
  Sneeze pulls the racer ahead back, Swap Harpoon trades places, mines in the racing line, and
  the **Paperweight** (a stone that becomes a small gravity well for 6 s) bends everyone's line.
- **The race HUD.** Your place (P3/8), the heat, your points, the race clock and the gaps to
  the racers ahead and behind (in seconds) at the top left; the distance to the EXIT along the
  course; a marker on your next gate (or the EXIT); a ring map of the belt with every racer as
  a dot in their colour at the right.
- **Points.** A heat ends when everyone finished, 30 s after the first finisher or after 3
  minutes. Places score 10, 8, 6, 5, 4, 3, 2 and 1 points (finishers by time, the rest by how
  far they got); the heat results show between heats, and after the third heat the podium.
  Most points wins; kills score nothing.

**Radiation** by where you fly (more rows in the
[world physics reference](reference/world.md#radiation-asteroid-belts)):

| Where | Hull per second |
| --- | --- |
| 4 u or deeper inside the belt (or in the dust round the EXIT) | 0 |
| at the belt's surface | 0.8 |
| skimming 4 u off the surface | 1.6 (a Tinker skimming a whole lap loses about one hull) |
| open space 6 u out | 2.8, then 0.6 more per u, at most 7 (13 u out) |

The belt is slow (rock and drifting boulders), open space is fast but hot: flying the whole lap
in open space costs several deaths and is slower than the belt; the fastest line is the belt with
skimming dashes where it is densest, as much as your hull allows. Keep about 20 hull and some
boost for the dive (it costs a Tinker about 20 hull at full boost). Hull never regenerates:
scavenge scrap (wrecks, scrap pickups) before the dive if you are short. Bots race too, each with
a line its hull can afford: belt huggers, skimmers and daredevils.

### Instagib

Instagib is a **mutator**: it changes the rules of any mode (the built-in **Instagib Brawl** is
Team Brawl with it).

- Everyone flies with the **Railspike** only, and it fires for free.
- **Any direct hit from an enemy destroys you**, Shield or not. Your own shots, splash and
  ramming work as usual.
- No other weapons, gadgets or pickups (in Paint the Void you keep the Paint Thrower).
- The HUD hides hull and energy; boost works as usual. Respawns are instant.
- The scoreline reads "<mode> · Instagib" and the shot count "∞ · one hit kills".

### Duels

A duel is a private match for two: free-for-all Team Brawl, **first to 10 kills or 5
minutes**, on the challenger's world (and with the challenger's rule set code, if they gave
one). How to challenge someone: [Duels](#challenging-someone-to-a-duel).

### What the modes look like

Every mode marks its objectives in the world, not only on the HUD:

- **The Squeeze:** the walls are rock, but they glow while they are about to move (a cyan
  energy curtain) and turn amber with glowing cracks while they grind inward, shedding sparks
  and dust. Where your ship scrapes a moving wall, sparks fly: get away, it crushes. The next
  safe zone is a faint holographic sphere; where it cuts the rock, a dashed yellow line shows
  its edge on the walls.
- **Heist:** the core is a glowing crystal with a gold beam rising from it. While it is still
  buried, a gold ring on the rock shows where to dig. The pilot carrying it has a gold halo, a
  beam and a trail of sparkles (and flies without boost). Extraction zones are spheres in their
  team's colour with a light pillar; your own is marked "Your extraction".
- **Hot Reactor:** the reactor floats over its holder, who glows red, pulsing faster as the fuse
  runs down. A red sphere shows the fuse radius (ships inside speed up the fuse); the ring
  around its middle shows how much fuse is left. A hit that passes the reactor throws sparks to
  the new holder; a detonation is a big red blast.
- **Last Color Standing:** the whole level is black and white. Ships keep the colour their
  meter has left, so a pale ship is one death from being out.
- **Instagib:** the scoreline says "Instagib", the hull and energy bars are hidden and the shot
  count reads "∞": the Railspike fires for free and any hit kills.
- **The Architect:** every stroke glows as a sphere in its colour for 2 seconds before it acts,
  with a beam of light over it; the exit is a green glowing gate with a beam. The architect sees
  the level from above with the rock cut away.
- **Belt Up:** a glowing dust band marks the belt, aurora curtains above and below it show where
  radiation burns, holographic arches are the gates, and a giant green EXIT sign over a swirling
  portal marks the finish in the middle.
- **Four teams:** Orange, Blue, Lime and Magenta show on the ships' liveries, the paint, the
  paint shots and the respawn flash. Three- and four-team matches start each team at its own end
  of the level.

## Between matches: the lobby

Online you never leave the server between matches. Every match runs through the same steps,
all on the same connection (no reload, no reconnect):

1. **Warm-up.** You fly the next match's level with weapons, gadgets and pickups off. A card on
   the left shows the next mode with a one-line summary, the world code and rules, every pilot
   in the room with a tick when ready and their team pick, and what the room waits for. While
   the mouse is free (Esc), the camera leaves your cockpit and flies a slow tour through the
   level; click into the game to fly your ship again.
2. **Countdown** (10 seconds by default). It starts when enough pilots for the next mode are
   there (Team Brawl needs 4, most modes 2) and either everyone pressed ready or the warm-up
   timer (30 seconds by default) ran out. If pilots leave and there are too few, it stops.
3. **The match**, on a fresh copy of the level.
4. **Results** (about 10 seconds): "Victory", "Defeat" or "Draw" (or who won, while you
   spectate), the team scores as bars or, in free-for-all modes, a podium of the top three, and
   every pilot with score, kills and deaths, plus "Next: <mode> in N s". Then the next warm-up.

In the warm-up and the countdown:

| Key | Button | What it does |
| --- | --- | --- |
| Enter | Ready | Ready (or not ready) for the next match |
| T | Team | Pick your team for the next match: Auto, then each team in turn. You get it as long as the teams stay even. |
| O | Spectate | Spectate from the next match on (or play again). The lobby shows your choice; from the next match on you fly the spectator camera without a ship until you press O again. Warm-ups between matches are open to everyone. |

If the room has too few or too many pilots for the next mode when the warm-up runs out, the
server switches to another mode that fits. Pilots who join in the middle of a match play at once
in modes with respawns, and spectate until the next match in modes with lives (see
[Game modes](#game-modes)). Servers with the lobby turned off start the next match right after
the results.

An older server instead closes every connection after the result and starts a new match on the
same level; your browser then rejoins automatically ("Match over. Joining the next match…") and
keeps your name, ship choice, horizon assist setting and debug overlay. If it can't rejoin
after about 12 tries, it shows an error. Reload the page to join again.

In practice mode (always Paint the Void against dummies), the result stays on screen. Reload the
page for a new round.

## Share codes, rule set codes and the rule set editor

### Share codes (worlds)

A share code describes a whole world: a random seed, the seven world sliders and the world
generator version, plus a checksum that catches typos. It always looks like this:

```
LS0G-C4GD-KYZH-1HJ5-BC58-T2XM-57NT
```

That's `LS` plus 26 letters and digits, 34 characters with the dashes. Codes of this build's
world generator (version 4) start with `LS0G`.

- **Typing codes:** upper or lower case both work, and dashes and spaces are optional. The letter
  O is read as zero, and I or L as one, so common typos still work.
- **Finding a level's code:** press **F3** in game. The bottom line of the overlay shows the code
  of the world you're in, including random practice worlds. Server hosts also see it in the
  server log.
- **Using a code:** type it on the start screen and press **Practice offline** to fly that world
  alone. To play it online, the server host starts the server with that code.
- **Sharing a level:** send the code to a friend. The same code builds exactly the same world on
  every computer.
- **Old codes:** a code made by an older version of the world generator (for example the
  Milestone 3 codes starting `LS0C` to `LS0F`) is rejected with a message that names both
  versions.
- **Sliders:** the code stores all seven sliders from the design (Enclosure, Density,
  Verticality, Gravity Chaos, Palette, Absurdity, Instability) and the kind of level: a
  classic level or an **asteroid belt** (a ring of asteroids around an open centre, the same
  from all of its 8 start slots, for the Belt Up racing mode). Every slider does something:
  **Enclosure** (open sky to dense mine shafts) and **Density** (few to many pillars and
  floating rocks) shape the world, **Verticality** sets its shape (flat and wide with low
  chambers at 0, the usual box at 50, tall and narrow with chimneys and islands stacked in tiers
  at 100), **Gravity Chaos** places up to seven [gravity wells](#gravity-wells), **Palette**
  turns the world from full colour to a [black-and-white ink world](#black-and-white-levels-and-pigment)
  with pigment pickups (80 and above), **Absurdity** sets how often [comedy events](#comedy-events)
  happen, and **Instability** makes craters wider, lets tunnels [cave in](#instability-and-cave-ins)
  and sets how far a Tectonic Charge's cracks spread. On an asteroid belt the first four mean
  other things (see [the world workshop](#the-world-workshop)). The seed also picks the sky:
  open worlds float in space with stars, a nebula, a sun and a planet. Rock blocks the sun:
  it casts shadows, and where caverns open to the sky, sunlight falls through the openings in
  visible shafts. Inside the rock, and in closed worlds (Enclosure 50 and above, which have no
  sky), caves are dark: your headlight, shots, explosions, engines and glowing rock light them.
  The edge of an open world is invisible; a faint blue grid shows it when you fly close. Every level also has
  a network of caverns and tunnels that links both teams' ends (and a cavern at each of the
  other two ends for teams 3 and 4). The
  [world workshop](#the-world-workshop) sets every slider and the seed; random practice worlds
  and server worlds pick them for you.

### Rule set codes

A server's **rule set** decides the mode and its settings (teams, kill target, lives and so
on), mutators such as Instagib, the match length, respawn time, which weapons, gadgets and ship
types are allowed, the starting loadout, the pickups, per-weapon tuning and friendly fire. A rule
set fits into a short **rule set code** that starts with `LR`, for example `LRAW-740M-8S00-01JD-4`
(the default Paint the Void rules). Rules also set the **Absurdity cap** (comedy events run at the
world's Absurdity, at most this; 0 means none), which comedy events are banned, and the pigment
pickups of black-and-white levels. Each mode's default rules have a short code of their own.
You don't need one to play: the server sends its rules when you join. Codes are for hosts, for
custom matches and duels, and for sharing rule ideas.

### The rule set editor

Click **Rule set editor** on the start screen (or open the game page with `?editor=rules`). You
can change every rule: weapon and gadget availability (loadout and pickup, pickup only, loadout
only, banned), the starting loadout, pickup weights, respawn times and ammo, the Tectonic
Charge's per-map limit, damage, fire rate and ammo multipliers per weapon, how each weapon meets a
Shield, friendly fire, the allowed ship types, scrap piles, match and respawn times, the
Absurdity cap, pigment pickups and comedy event bans (a box per event). It edits the
rules of the mode the loaded code has (open a code of another mode with `?editor=rules&rules=LR…`);
it has no mode picker and no fields for the mode's own settings (teams, kill target, lives and
so on) yet. Problems the server would refuse are listed in red. When the rules are valid, the **Share** section gives you:

- the **rule set code**,
- an optional **World** field for a share code,
- an **invite link** with both codes (`…/?world=LS…&rules=LR…`),
- the **host command** (for example `ls-server --code LS0G-… --ruleset LRAW-…`), which the host
  runs to play those rules.

The editor remembers the last code you made. Opening it with `?editor=rules&rules=LR…` loads that
code. **Close** returns to the start screen.

An invite link with a world and/or rule set code opens a private **custom match** with them
(see [Invite links](#invite-links)).

## Playing with friends: server list, invites, parties and duels

Your browser keeps a random player id for all of this; there are no accounts or friends lists.
It lives in the browser's storage, so another browser (or a private window) is a different
player.

### The server list

Click **Server list** in the bottom right corner of the start screen (or open the game page
with `?servers`). It lists the servers registered with the **server directory** your host uses:
name, region, mode, players, **ping** (measured by your browser) and world, fullest servers
first, and under each server the pilots on it. Only servers running the same build as the page
are listed. **Join** switches the page to that server (an invite link to it); **Back** returns
to the start screen. If the page's server uses no directory, the list says so; you can still join a server
by its address or an invite link. A host can also hand you a list link with
`?servers&directory=<directory address>`.

### Invite links

An invite link opens the game page and says where to play. Choose **Play online** after
opening it:

- with a **party code** (`party=P-…`): you join your friend's room on that server and, in team
  modes, their team when the teams stay even;
- with a **world and/or rule set code** but no party (for example the rule set editor's link,
  `…/?world=LS…&rules=LR…`): the server opens a private **custom match** with them and you play
  there. Friends who open the same link get a custom match of their own, so to play together use
  a party invite instead;
- with a **server address** (`join=host:port`): that server instead of the one that served the
  page;
- with a private **room code** (`room=R-…`): that private room.

To look at an invite's rules without joining, add `&editor=rules` to the link.

### Parties

In game, press Esc (to free the mouse) and click **Party & duels** in the top left corner, then
**Create party invite**. Send the link to your friends: it brings them into your room (the
public rotation, or your private room) and, in team modes, onto your team as long as the teams
stay within one player of each other (otherwise the party is split). A party code works only on
that server and lasts while one of you is connected, and 10 minutes after the last one left.

### Challenging someone to a duel

- **On the same server:** the **Party & duels** panel lists the other players on the server
  with a **Duel** button. The duel is played on your current world.
- **From the server list:** every listed pilot has a **Duel** button. Two fields on the list
  set the duel's world (a share code; empty: a random world) and an optional rule set code. The duel runs on the server that served your page, and a pilot on another listed server
  gets the challenge through the directory.

The other pilot sees a challenge at the top of the screen with **Accept** and **Decline** for
30 seconds. When they accept, both of you move to a private two-player match (first to 10
kills or 5 minutes, unless the challenge carried a rule set code). After the results the duel
room starts a rematch on the same world; to go back to the public rotation, open the server's
address again (or use the browser's back button). When all of the server's private rooms are in
use, the duel can't start.

## Settings

Everything is on the start screen under **Options**, and the page remembers it per browser:

| Setting | What it does |
| --- | --- |
| **Horizon assist** | On by default. Slowly levels your roll so "up" stays up. **H** toggles it in flight. |
| **Field of view** | How wide you see, 80 to 120 degrees across a 16:9 screen (default 100). Narrower screens keep that width; wider screens (21:9) see a little more at the sides, up to 115 degrees. Boosting widens the view by 4 degrees. |
| **Cockpit frame** | On by default. Shows the nose, the canopy frame and side parts of the ship you fly, so you can judge its size. |
| **Fit ring** | On by default. A ring around the crosshair that shows how big your ship is at the rock ahead (see [Will I fit?](#will-i-fit)). |
| **Sound** | Master volume (80), effects (80), music (60) and announcer (85), 0 to 100 each, and **Mute all sound**. Changes apply at once, also in a running game. |

**Sound** starts with your first click or key press on the page (browsers don't allow sound
before that). Every sound is made in your browser while you play: weapons by family, impacts,
explosions, the Tectonic Charge's rumble and blast, the Polite Laser's chime, boost, pickups,
gravity wells humming, cave-ins rumbling, pigment splashes, every comedy event's sounds (a
honk, a hiccup, a squeak, a "mrrp"), The Architect's strokes and Belt Up's start lights, gates
and Geiger counter. Sounds come from where they happen (headphones help). The **music** slider
sets the level's background hum (it follows the mood) and the events' tunes (the side-scroller's
chiptune, the lounge music, the kazoo march, the disco beat). The **announcer**, a little
broadcast robot at the top left, announces every comedy event, calls the match start, first
blood, multi-kills, cave-ins and "Belt up!", and narrates the Tired announcer's kill-cams,
in made-up robot speech with captions; each match gets one of three announcers: Doug (tired),
Sparky (overexcited) or Brenda (passive-aggressive). Volumes and mute are saved in the browser.
`?audio=demo` after the page address opens the sound check page, where you can play every sound
and hear every announcer.

The pilot name, share code and ship are remembered too. **Graphics quality** is picked
automatically after the first seconds of play (low, medium or high, by your GPU's speed); add
`?quality=low`, `?quality=medium` or `?quality=high` to the page address to force one. There is
no setting yet for mouse sensitivity, inverted look or key rebinding.

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
- **The ship doesn't respond to thrust or turning, and the view shows a missile's camera:** you
  are guiding an Eyeball. The view returns when it hits something.
- **Clicking doesn't lock the mouse right after pressing Esc:** browsers block pointer lock for a
  moment after you release it. Wait a second and click again.
- **Ctrl+W closed the tab:** browsers keep some Ctrl shortcuts for themselves. Use **C** to thrust
  down instead of Ctrl.
- **Movement keys are in the wrong place:** the game uses key positions, not letters. Use the keys
  where W, A, S and D are on a US keyboard.
- **Number keys pick a ship instead of a weapon:** the ship bay is open (B, or you're destroyed).
  Press B to close it.
- **V does nothing:** V only switches the colour while the Paint Thrower is your selected primary.
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
- **"game data mismatch":** the page and the server were built from different game data (weapon,
  ship, gadget or rule files). Reload the page with Ctrl+F5; if it persists, the host has to
  rebuild the web client and the server from the same version.
- **"Disconnected" during play:** the connection dropped or the server stopped. Reload to return
  to the start screen.
- **"Lost sync with the server":** the game received data it couldn't use, often because the page
  and the server come from different builds. Reload the page (a hard refresh with Ctrl+F5 helps
  after a server update).
- **"Could not rejoin" after a match:** the next match didn't accept you in time (older servers
  only). Reload to join it.
- **"unknown room", "room closed" or "unknown party":** the private room of an invite or duel has
  closed (rooms close when their last pilot leaves), the party ended (10 minutes after its last
  member left), or the link is for another server. Ask for a new invite, or open the server's
  plain address to join its rotation.
- **The server list says there is no directory, or stays empty:** the page's server isn't
  registered with a directory; ask the host for a list link (`?servers&directory=…`) or the
  server's address. On an `https://` page, the directory must be served over `https://` too.
- **A duel challenge never arrives:** the other pilot must have the game open (in a match or
  the lobby, not on the start screen) on a server of the same build; across servers both servers
  need the same directory. Challenges expire after 30 seconds.
- **You stay a spectator:** you joined a mode with lives after its start, or you are out. You
  play again in the next match.
- **Lag or rubber-banding:** press F3 and check ping and the connection type. WebTransport is
  usually smoother than WebSocket. The browser picks WebTransport when it can and falls back to
  WebSocket otherwise. Being swapped by a Swap Harpoon or rewound by Borrowed Time is a real
  jump, not lag.

Advanced page options, added to the address (combine them with `&`):

| Option | Effect |
| --- | --- |
| `?transport=ws` | Always use WebSocket |
| `?transport=wt` | Always use WebTransport (fails instead of falling back) |
| `?server=host:port` | Connect to a game server other than the one that served the page, for example `http://localhost:7777/?server=192.168.1.20:7777` |
| `?editor=rules` | Open the rule set editor instead of the game (`&rules=LR…` loads a code, `&world=LS…` fills the world field) |
| `?servers` | Open the server list (`&directory=<url>` names the directory) |
| `?join=host:port`, `&party=P-…`, `&room=R-…`, `&code=LS…`, `&rules=LR…` | Invite link parts (see [Invite links](#invite-links)) |
| `?quality=low`, `medium` or `high` | Force a graphics quality instead of the automatic pick |
| `?shipshadows=off` | Ships near you cast no shadows (the rock's shadows stay) |
| `?eventfx=off` | Comedy events show only their card and the announcer, without their looks |
| `?visitors=off`, `?worldfx=off` | Leave out the visitor events' drones, mimics and face, or the world effects (wells, cave-ins, pigment, moods); for bug reports |
| `?event=<id>`, `?event=<id>@<seconds>`, `?event=all` | Practice only: force comedy events (see [Comedy events](#comedy-events)) |
| `?workshop`, `?workshop=<share code>` | Open the [world workshop](#the-world-workshop) |
| `?audio=demo` | The sound check page: every sound and every announcer |

## Not in this build yet

These are part of the game design but aren't in the Milestone 4 build:

- **Co-op modes** (Deep Dig, Tethered, Escort the Intern), and a co-op Architect who helps the
  runners.
- **Belt Up variants:** team races, time trials against ghosts, and other modes on asteroid
  belts. Belt Up is not in the default rotation of servers yet (hosts add it).
- **Kill-cam after your own death:** the replay window only plays during the Tired announcer
  event.
- **Voting for the next mode:** servers play their rotation in order.
- **Sound:** no soundtrack and no recorded voices: the music bus carries the level's hum and the
  events' tunes, and the announcer speaks in made-up robot speech with captions.
- **Scorch fading and cosmetic chips:** scorch marks stay for the whole match; there are no
  client-only decals or small cosmetic debris chips.
- **Accounts, friends lists, ranked play and matchmaking by skill:** parties live on one
  server; duel challenges reach players on servers listed in the same directory only.
- **Settings:** no mouse sensitivity, invert-look or key rebinding options.
- **Cosmetics, a WebGL2 fallback for browsers without WebGPU, and a desktop app.**
