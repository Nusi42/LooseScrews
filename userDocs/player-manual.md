# Loose Screws: player manual

This manual covers the current build (Milestone 2: the full arsenal, four ship types, gadgets
and a level that fights back). Features from the game design that this build doesn't have yet
are listed under [Not in this build yet](#not-in-this-build-yet). Every number in the
weapon, ship and gadget tables of the [reference pages](reference/) is generated from the
game's data files, so those pages always match the build.

## Contents

- [What Loose Screws is](#what-loose-screws-is)
- [What you need](#what-you-need)
- [Starting a game](#starting-a-game)
- [Controls](#controls)
- [The HUD](#the-hud)
- [Ship types](#ship-types)
- [Your ship: hull, energy, boost and scrap](#your-ship-hull-energy-boost-and-scrap)
- [Flying](#flying) ([will I fit?](#will-i-fit))
- [Weapons](#weapons)
- [Surviving a Tectonic Charge](#surviving-a-tectonic-charge)
- [Gadgets](#gadgets)
- [The level fights back](#the-level-fights-back)
- [Pickups and wrecks](#pickups-and-wrecks)
- [Paint the Void](#paint-the-void)
- [What the modes look like](#what-the-modes-look-like)
- [Share codes, rule set codes and invite links](#share-codes-rule-set-codes-and-invite-links)
  ([server list, parties and duels](#server-list-parties-and-duels))
- [Troubleshooting](#troubleshooting)
- [Not in this build yet](#not-in-this-build-yet)

Reference tables: [weapons](reference/weapons.md), [ships](reference/ships.md),
[gadgets](reference/gadgets.md).

## What Loose Screws is

You pilot a Scrapling, a palm-sized ship built from junk, through a procedurally generated
world of caverns, tunnels and floating rock. You can fly in any direction and turn any way you
like, with no fixed "up". The level is part of the fight: weapons blast craters, melt
channels, scorch and charge the rock, knock debris loose and even grow new rock, and all of it
stays for the rest of the match. In **Paint the Void**, the only mode in this build, two teams
race to cover as much of the level as they can in their own colour.

Every world comes from a **share code**, a short text such as
`LS0F-143C-ZQW8-CCHA-V4M6-8001-8FAE`. The same code always builds the same world.

## What you need

- A desktop browser with **WebGPU**: current Chrome or Edge (version 113 or newer), Firefox 141
  or newer on Windows, or Safari 26 or newer. On Linux, Chrome needs WebGPU turned on under
  `chrome://flags/#enable-unsafe-webgpu`.
- A mouse (with a wheel, for cycling weapons) and a keyboard. The game uses pointer lock, so
  the mouse steers the ship instead of moving a cursor.
- The address of a running game server for online play (the host gives it to you, for
  example `http://localhost:7777`). Practice mode needs no server once the page has loaded.

There is no sound in this build.

## Starting a game

Open the game page. The start screen has these fields:

| Field | What it does |
| --- | --- |
| **Pilot name** | Your name, up to 32 bytes. Other pilots see it above your ship, on the scoreboard and in the kill feed. |
| **Share code** | A world code. Leave it empty to use the server's world (online) or a random world (practice). |
| **Ship** | The ship type you fly first: Skitter, Tinker (default), Hauler or Tug (see [Ship types](#ship-types)). If the server's rules don't allow it, you get the Tinker or the first allowed type. You can change ships later in the ship bay (**B**). |
| **Horizon assist** | On by default. Slowly levels your roll so "up" stays up. You can toggle it in flight with **H**. |
| **View: Field of view** | How wide you see, 80 to 120 degrees across a 16:9 screen (default 100). Narrower screens keep that width; wider screens (21:9) see a little more at the sides, up to 115 degrees. Boosting widens the view by 4 degrees for a sense of speed. |
| **View: Cockpit frame** | On by default. Shows the nose, canopy pillars and side parts (pods, armour plates, grapple arms) of the ship you fly, so you can judge its size. |
| **View: Fit ring** | On by default. A thin ring around the crosshair that shows how big your ship is at the rock ahead (see [Will I fit?](#will-i-fit)). |

The page remembers your name, code, ship, horizon assist and view settings for next time. The
**Rule set editor** link in the bottom right corner opens the
[rule set editor](#the-rule-set-editor).

Then choose one of the two buttons:

- **Play online** joins the server that served the page. You always play **the server's world
  and rules**, whatever you typed in the share code field. If your code is different, a message
  in the kill feed reminds you that your code only works in practice. The server puts you on the
  team with fewer players.
- **Practice offline** builds a world on your own computer from the share code, or a random
  world if the field is empty, with the default Paint the Void rules. You fly for the Orange
  team against three target dummies ("Dummy 1" to "Dummy 3") on the Blue team. The dummies
  float in place and never move or shoot. Pickups spawn as in a real match, so practice is the
  place to try every weapon you find. A practice round lasts the full 5 minutes. When it ends,
  reload the page for another round.

Building a world takes a moment ("Generating world…" or "Connecting…"). When the game starts,
the HUD shows "Loading terrain (N chunks left)" while the level streams in around you.

**Click the game view to take control.** The browser locks the mouse to the game. Press **Esc**
to release it. While the mouse is released, the HUD shows "Click to fly · Esc releases the
mouse" and your ship doesn't respond to flight keys.

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

Tab, F3, Enter, T and O work even while the mouse is released. While you spectate, the flight
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
| Over ships | Pilot names: teammates within 90 u, enemies within 45 u |

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
match and names its mode (see [After the match](#after-the-match)).

**Lobby:** between matches you fly the next match's level with weapons off (warm-up). A card
on the left shows the next mode with a one-line summary, the world code and rules, every pilot
in the room with a tick when ready and their team pick, and what the room waits for. Press
Enter when you're ready, T to pick a team, O to spectate. The countdown starts when enough
pilots are there and everyone is ready, or when the warm-up timer runs out.

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

- On the start screen, pick your first ship in the **Ship** field.
- In game, press **B** to open the **ship bay**. It shows each type with its numbers and the
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
  through rock (thin walls are melted through) and shoves debris aside. *Tip:* charge behind
  cover, release as you peek out.
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
  **charged** for 15 s and drains 5 energy per second from ships that scrape it. *Tip:* the
  drain feeds your own weapons; stick to your target like glue.

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
  see you unless they are within 6 u, or you fired in the last half second. Teammates see a
  shimmer. *Tip:* sit still on a route the enemy uses and fire at point blank.
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

### Craters and rock strength

Explosions blast craters (up to 6 u across for a Heavy Rocket, a 35 u cave-in for a Tectonic
Charge) that stay for the rest of the match. Craters erase paint, and the fresh rock starts
unpainted. **Rock strength** varies by region: **weak rock** (lighter, sandy, crumbly) blasts
1.4 times wider, and kinetic rounds and the Magnet Sneeze knock chunks off it; **hard rock**
(darker, cooler, with glinting specks) only 0.6 times. Other marks: Fusion Charger channels,
Railspike and Polite Laser holes through thin walls, Corkscrew spirals, Plasma pits, Ricochet
chips, and new rock from Seed Grenades.

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

Each level has **12 pickup points** in open space, spread across the level and away from the team
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

## Paint the Void

Two teams, **Orange** and **Blue**, try to paint as much of the level as they can.

- The match lasts **5 minutes** (the server's rules can change this).
- Your team's score is the share of the **whole level's paintable surface** covered in your colour.
  The score updates twice a second, and the HUD bars and the scoreboard show it in percent.
- The paint's **effect colour** (red, blue, yellow) doesn't matter for the score; only the team
  does.
- When the timer runs out, the team with more painted surface wins. Equal coverage is a draw.
- **Kills don't score points.** They matter because a destroyed enemy stops painting for
  3 seconds and respawns far away.
- Painting over the other team's colour takes that surface from them.
- Craters erase paint, both theirs and yours.

Levels are large, so the percentages stay small: a close match can end with both teams
under 1%. Only the comparison between the two teams counts.

Tactics:

- Paint surfaces the other team has painted. That takes surface from them and adds it to your
  team at the same time.
- Keep energy for painting: boosting stops energy regeneration, and the Paint Thrower needs
  energy. A Leech Arc refills you from an enemy.
- A rocket into a freshly painted enemy wall wipes their paint. Then paint the crater.
- Use the effect colours: sticky yellow around your painted areas slows raiders down.
- Guard your team's painted areas and hunt the enemy painters.

### After the match

Online, the result stays on screen for about 10 seconds (the server can change this). Then you
move on to the next match of the server's rotation without leaving: the warm-up (the lobby
card, see [The HUD](#the-hud)), the countdown and the next match load in place, on the same
connection, with your name and settings. Servers with the lobby turned off start the next
match right away.

An older server instead closes every connection after the result and starts a new match on the
same level; your browser then rejoins automatically ("Match over. Joining the next match…") and
keeps your name, ship choice, horizon assist setting and debug overlay. If it can't rejoin
after about 12 tries, it shows an error. Reload the page to join again.

In practice mode, the result stays on screen. Reload the page for a new round.

## What the modes look like

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
- **Four teams:** Orange, Blue, Lime and Magenta show on the ships' liveries, the paint, the
  paint shots and the respawn flash.

## Share codes, rule set codes and invite links

### Share codes (worlds)

A share code describes a whole world: a random seed, the seven world sliders and the world
generator version, plus a checksum that catches typos. It always looks like this:

```
LS0F-143C-ZQW8-CCHA-V4M6-8001-8FAE
```

That's `LS` plus 26 letters and digits, 34 characters with the dashes. Codes of this build's
world generator (version 3) start with `LS0C` to `LS0F`.

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
  Milestone 1 codes starting `LS04` or `LS06`) is rejected with a message that names both
  versions.
- **Sliders:** the code stores all seven sliders from the design (Enclosure, Density,
  Verticality, Gravity Chaos, Palette, Absurdity, Instability). In this build **Enclosure** (open
  sky to dense mine shafts) and **Density** (few to many pillars and floating rocks) shape the
  world, **Instability** sets how far a Tectonic Charge's cracks spread, and **Palette** drains
  the colour from the picture (100 is nearly black and white). The seed also picks the sky:
  open worlds float in space with stars, a nebula, a sun and a planet. The edge of an open
  world is invisible; a faint blue grid shows it when you fly close. Every level also has
  a network of caverns and tunnels that links both teams' ends. There's no screen to set sliders
  yet: random practice worlds and server worlds pick them for you.

### Rule set codes

A server's **rule set** decides the match length, respawn time, which weapons, gadgets and ship
types are allowed, the starting loadout, the pickups, per-weapon tuning and friendly fire. A rule
set fits into a short **rule set code** that starts with `LR`, for example `LR8V-6097-G000-TMBG`
(the default Paint the Void rules). You don't need one to play: the server sends its rules when
you join. Codes are for hosts and for sharing rule ideas.

### The rule set editor

Click **Rule set editor** on the start screen (or open the game page with `?editor=rules`). You
can change every rule: weapon and gadget availability (loadout and pickup, pickup only, loadout
only, banned), the starting loadout, pickup weights, respawn times and ammo, the Tectonic
Charge's per-map limit, damage, fire rate and ammo multipliers per weapon, how each weapon meets a
Shield, friendly fire, the allowed ship types, scrap piles, match and respawn times. Problems the
server would refuse are listed in red. When the rules are valid, the **Share** section gives you:

- the **rule set code**,
- an optional **World** field for a share code,
- an **invite link** with both codes (`…/?world=LS…&rules=LR…`),
- the **host command** (for example `ls-server --code LS0F-… --ruleset LR8V-…`), which the host
  runs to play those rules.

The editor remembers the last code you made. Opening it with `?editor=rules&rules=LR…` loads that
code. **Close** returns to the start screen.

**Opening an invite link** shows the normal start screen; choose **Play online**. A link with a
world and/or rule set code (like the editor's) opens a private **custom match** with them on the
server that serves the page (or the one named by `join=`), and you play there; friends who open
the same link get their own custom match, so to play together use a party invite (below). To look
at an invite's rules, add `&editor=rules` to the link.

### Server list, parties and duels

- **Server list:** the **Server list** link on the start screen lists the servers registered
  with the server directory your host uses (name, region, mode, players, ping, world). **Join**
  opens that server. If no directory is set up, the list says so; you can still join a server
  by its address or an invite link.
- **Parties:** in game, press Esc (to free the mouse) and click **Party & duels**, then
  **Create party invite**. Send the link to your friends: it brings them into your room and, in
  team modes, onto your team when the teams stay even (otherwise the party is split). The party
  code (`P-…`) works only on that server.
- **Duels:** the same panel lists the other players on the server with a **Duel** button; the
  server list has one next to every listed player too. A duel is played on your current world
  (from the server list: the world you enter, or a random one). The other player sees a
  challenge with **Accept** and **Decline** for 30 seconds; when they accept, you both move to
  a private two-player match (first to 10 kills or 5 minutes). Players on another server get
  the challenge through the directory.
- Your browser remembers a random player id for this; there are no accounts.

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
- **"Could not rejoin" after a match:** the next match didn't accept you in time. Reload to join
  it.
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

## Not in this build yet

These are part of the game design but aren't in the Milestone 2 build:

- **Game modes:** Paint the Void and Team Brawl (team deathmatch with 2 to 4 teams, or
  free-for-all; the server's duel rule set plays free-for-all to 10 kills), plus the Instagib
  mutator (Railspike only, every enemy hit kills, instant respawn), on servers whose playlist
  includes them. Heist, Hot Reactor, The Squeeze, Last Color Standing, the co-op modes and The
  Architect come later.
- **World sliders:** Verticality, Gravity Chaos (gravity wells), Palette (black-and-white levels)
  and Absurdity don't change the world yet, and there's no screen to set sliders or save a level.
- **Comedy events** ("the level has moods"): not yet.
- **Sound:** none at all yet, so the Polite Laser's "excuse me" and the Tectonic Charge's
  rumble are banners only.
- **Scorch fading and cosmetic chips:** scorch marks stay for the whole match; there are no
  client-only decals or small cosmetic debris chips.
- **Accounts and friends lists:** parties live on one server; duel challenges reach players
  on servers listed in the same directory only.
- **Settings:** no mouse sensitivity, invert-look or key rebinding options.
- **Cosmetics, a WebGL2 fallback for browsers without WebGPU, and a desktop app.**
