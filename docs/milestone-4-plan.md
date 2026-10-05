# Milestone 4 — The level has moods

Status: accepted; every design decision below is *Accepted*, including the proposed answers to the open questions (§8) · Design source: [design/game-design-doc.md](design/game-design-doc.md) (Comedy system, World generation, Weapon physics, Monochrome levels, Game modes: The Architect, Match rules) · Previous milestone: [milestone-3-plan.md](milestone-3-plan.md) · Visual track: [visual-quality-plan.md](visual-quality-plan.md) · Overall plan: [implementation-plan.md](implementation-plan.md)

Milestone 4 makes the level a participant. A deterministic comedy director rolls the 18 events of the design doc, weighted by Absurdity; the four inert sliders come alive (Verticality shapes the level, Gravity Chaos places gravity wells, Palette turns the world into black-and-white ink, Instability makes tunnels cave in); monochrome levels get pigment pickups (stealth versus power); and The Architect lets one player move the sliders live against everyone else. Players get a world workshop to make and share worlds with all seven sliders. M4 also takes over the two carried-over items from [implementation-plan.md §8](implementation-plan.md#carried-over-items) and the M3 leftovers that belong to world generation, meshing and rendering.

Input taken before planning:
- Requirement (review of the design doc's comedy section): many more fun events beyond the rubber chickens, at least a dozen random and creative ones. The event table grew to 18 events as a result; this plan builds **all 18** (open question 1 offers a smaller first cut).
- Standing requirement for everything visual: a strong first impression. Every mood, event, slider look and the Architect's view is judged by screenshots reviewed on a phone. Every package that draws something adds gallery scenes and sends before/after shots.
- Browser automation stays headless and injects `tools/browser-smoke/fake-pointer-lock.mjs`, also in agents' scratch scripts.

## 1. Scope

| Area | In scope |
| --- | --- |
| Comedy director | A deterministic director in `ls-sim` (own RNG stream) that rolls events weighted by the effective Absurdity, with a level "mood" that biases the weights, a telegraph before every event, per-mode vetoes, per-rule-set event bans, replicated mood state, and a practice/gallery trigger (`?event=<id>`) |
| Events (all 18 from the design doc) | One module per event (`crates/ls-sim/src/events/<id>.rs`) plus a data file (`data/events/<id>.ron`), behind a hook trait like weapons and modes: Bored level, Tax drone, Gravity coffee break, Rubber chickens, Cat paw, Tired announcer, Wrong genre, Gossip wall, Pizza delivery, Everything is cake, Duck parade, Hat rain, Customer survey, Disco inferno, Big head day, Moving day, Mimic pickups, Level hiccups. Each with its sound, its look, gallery scenes and bot reactions |
| Announcer and kill-cam | The announcer as a caption character with a synthesized "voice" and three rotating voice packs; a client-side kill-cam (replay of the last 3 s from the killer's view, from buffered snapshots) used by the Tired announcer event and, as a setting, after your own death |
| Audio foundation | The client has no audio today. A WebAudio engine with procedural (synthesized) sounds: every event's signature sound, announcer voice, Tectonic rumble, Polite chime, explosions, impacts, boost, a sound set per weapon family, ambient mood drones; spatial panning, mixer, volume settings, silent until the first user gesture (open question 2) |
| Verticality slider | Generator: the level box takes one of five shapes of about the same chunk count (flat and wide to tall and narrow), and the inner structure stretches with it (cavern noise, mine lattice, pillar and island tiers) |
| Gravity Chaos slider | Generator: gravity well sites (a dense rock core in a carved hollow), from none at 0 to seven strong wells at 100. Sim: wells pull ships (capped below thrust), ordnance, debris, mines and wrecks; energy bolts and kinetic rounds fly straight (design doc). Client prediction applies the same force. Looks: core stone, accretion swirl, lensing, pull indicator |
| Palette slider | Renderer: full colour to black-and-white ink: desaturation, then outlines and toon/hatched shading fading in, then the monochrome ink look; paint and pigment keep their colour |
| Instability slider | Sim: weaker rock overall (crater scale) and cave-ins: craters build strain per region; past a threshold the tunnel caves in after a 1.5 s warning (rubble of weak rock, a hollow ceiling, debris), never trapping a ship |
| Absurdity slider | The director's input; combined with the rule set's Absurdity cap (§3.1) |
| Monochrome levels and pigment | Palette 80+ makes a monochrome level: pigment pickups colour your ship (more damage, visible to everyone); grey ships blend in. Last Color Standing gets pigment pickups that restore colour (deferred from M3) |
| The Architect | New mode module (`modes/the_architect.rs`, `ModeKind` 6): one architect without a ship moves sliders live inside regions within a budget; runners must reach the exit; rounds rotate the architect; architect camera and stroke UI; architect and runner bots |
| World workshop | Start-screen page: seven sliders and a seed with a live preview flythrough, the share code ("Save"), play it in practice, host it (invite link). Its slider panel is reused by the Architect |
| Bots | Bots handle wells (counter-thrust, avoid cores), cave-ins and rubble (re-plan), pigment, every event (shoot drones and mimics, avoid ducks and the paw, hold still for pizza, collect hats), and play The Architect as runner and as architect |
| Carried over (from M2, via M3) | **Tectonic collapse tick**: the single exact 35 u crater's bake still costs 3.5-5.3 ms; make it cheaper in `ls-worldgen` (skip `room_open` for corners the edit leaves unchanged, or bake big craters lazily per chunk); target < 2 ms (`crates/ls-game/tests/tectonic_tick.rs` holds it to 8 ms today) |
| Carried over (from M1) | **Dual contouring in play**: the crisp mine look of the design doc within the per-frame remesh budget (today: dual contouring 4-13 ms per busy chunk against 1.6-2.2 ms for the VQ3a surface nets); approach in §3.6 (open question 10) |
| Carried over (from M3) | **Quiet-machine soak**: tick p99 < 4 ms per room and the unattributed 12:04 stall from the M3 soak (`milestone-3-plan.md` "Milestone soak"), with a per-tick log of slow steps. Runs before wave A integration (a run is in progress while this plan is drafted); its result closes the M3 exceptions or becomes a wave A task |
| M3 leftover (D5) | **Thin-rock jaggies**: rock blades thinner than a cell keep a jagged outline (`cave-sunbeam`); a thin-rock pass in the mesher like VQP's thin-air pass, with its own cost budget |
| M3 leftover (D5, VQP) | **Folded triangles at bore rims** (9 of 560 at the gallery bore mouth; the test allows 1 in 50) |
| M3 leftover (VQ track) | **Reference mid-GPU timings**: every VQ package and M3 C1 estimated its cost on the GTX 1660 / RX 6600 / M1 class from an RTX 5080; nobody measured (open question 11) |
| M3 leftover (D3) | **Teams 3 and 4 spawn zones**: 3- and 4-team matches give teams 2 and 3 no spawn zone of their own (they cycle through all spawn points) |

Out of scope (M5+): coop modes (Deep Dig, Tethered, Escort the Intern; M6) and The Architect's coop variant (open question 8); recorded voice packs and comedy event packs as cosmetics (M6 store); ranked playlists and the live balance list; WebGL2 fallback, Tauri, WebRTC (M5); the M3 security and robustness gaps (D1: tokens for duel answers and party tags, directory squatting, build backoff, private-room races) and D3 gaps (private rooms ignore `--ban`, plain-HTTP directory on `https://` pages), all M5 hardening; the D2 web items (party panel polls after a disconnect, server table and weapon panel at phone width, lobby card focus) go to M5 unless a package owns the file anyway; the remaining M5 carried-over items stay in M5.

## 2. Current state (start of M4)

- **Sliders.** `ls_worldgen::Sliders` has seven 0-100 values in the share code; the generator reads only Enclosure and Density. Verticality and Gravity Chaos are inert. Palette only lowers the client's grade saturation (up to 90 %, `worldGrade` in `web/src/renderer/atmosphere.ts`, VQ7). Instability only scales the Tectonic Charge's crack reach (`ctx.view.instability`, `MatchSetup::instability`). Absurdity exists twice: as a world slider (default 20) and as `RuleSet::absurdity` (20 in all nine built-in rule sets, in the rule set code), neither read by anything.
- **Level box.** Always 256 x 128 x 256 u (`DEFAULT_SIZE_CHUNKS` [8, 4, 8]); `Generator::with_size` exists for tests. `Welcome` already carries the quantization bounds, so a level of another size needs no protocol change.
- **No gravity anywhere.** Debris drifts until drag stops it; ships, projectiles and entities have no external forces. Horizon assist levels the roll against world +Y in `ls-sim` (`ship.rs`).
- **Generator version 3**: the connection network (8 caverns on a 4 x 2 grid, team ends at low and high x) and the Heist site. A share code of another generator version is refused (`WorldgenError` "made by generator version"). Spawn zones (`ls_game::setup::spawn_zone`) exist for two sides only.
- **Simulation.** Mode framework with hooks (`on_damage`, `on_hit`, `move_limits`, `material_at`, `terrain_shell`, ...), generic entities (`Debris`, `Decoy`, `Ghost`, `DustCloud`, `Marker`) with 4 state bytes, `CueKind` cues, Seed Grenade growth that leaves out spheres overlapping a ship, a runtime shell layer (The Squeeze). The Seed Grenade already grows a giant rubber duck (`Shape::Duck`).
- **Renderer.** The VQ pipeline is complete: HDR, bloom, AgX, clustered lights, particles, sun occlusion, sky, mode effects, cloak refraction, the colour grade hook with a monochrome blend and a per-pixel keep mask (Last Color Standing). There is no outline or ink pass. Gallery: 42 scenes, all within budget on an RTX 5080; no reference-GPU numbers.
- **Client.** No audio at all. No world-building UI: share-code entry only; practice mode and the FI backdrop flythrough can generate any code a chunk per frame (`practicePreview`, `generateNearest`). Spectator free camera exists.
- **Versions.** Protocol 5, `SIM_BEHAVIOR_VERSION` 5, rule set code format 4, generator version 3; `REPLAY_GOLDEN_HASH` `0x8321_c0ab_902b_8ee5`, nine per-mode replays (`crates/ls-sim/tests/fixtures/mode_replays.txt`).
- **Carried items.** Tectonic collapse tick 3.5-5.3 ms (crack ticks 0.5-0.9 ms). VQ3a's surface nets already snap to the exact field at creases (mine-shaft vertex error p95 0.354 -> 0.080 u) at 1.6-2.2 ms per busy chunk native, wasm p95 2.8 ms, max 3.7 ms against the client's 4 ms per-frame chunk budget. Dual contouring is golden-hashed but has no AO, material weights or thin passes.

## 3. Design decisions for gaps in the design doc

Every row is a *Accepted* for the owner. Numbers live in `data/` (events in `data/events/*.ron`, world physics in `data/world/*.ron`, mode and pigment settings in rule sets) and are tuning starting points. Rows rejected or changed in review are updated here before the wave that needs them starts.

### 3.1 Comedy director

| Topic | Decision |
| --- | --- |
| Framework | *Accepted.* `crates/ls-sim/src/director/` runs as a new last stage of `World::step` (after the mode). Events are modules (`crates/ls-sim/src/events/<id>.rs`, registered in `events/mod.rs` in `EventKind` order, append only) implementing `EventModule`: `eligible`, `on_announce`, `on_start`, `on_tick`, `on_end`, plus optional effect hooks the world consults while the event runs: `move_effects` (forces, plane lock, impulses; also applied by the client predictor), `hit_radius_scale`, `projectile_flags`, `crater_scale`, `scavenge_scale`, `reveal` (positions shown to everyone), `allows_pickup`. A hook an event does not override does nothing. Event entities (drones, ducks, hats, paw, face, disco ball) are generic entities with new `ReplicatedKind`s |
| Determinism | *Accepted.* New RNG stream `RNG_STREAM_MOOD` ("SIM_MOOD"): `Pcg32::new(mix64(match_seed ^ mix64(tick ^ stream)), ..)` per roll and per event start. Director and event state are part of the state hash |
| Effective Absurdity | *Accepted.* `min(world slider, rule set absurdity)`: the world's slider is its temperament, the rule set's value is the host's cap (0 = no events; the Serious playlist). The built-in rule sets go from 20 to 100 ("the world decides"), except `duel.ron` 50. Default world sliders keep Absurdity 20 (open question 3) |
| Cadence | *Accepted.* No events in warm-up, countdown or results, in the first 45 s of a match, or in the last 20 s of a timed one. A roll every 20 s; chance per roll 0.6 x absurdity / 100; at most one event at a time and at least 30 s between one event's end and the next announcement; an event does not repeat within a match while others are eligible. Expected per 5-minute match: about 1.4 events at Absurdity 20, 3.5 at 50, 5 at 100 |
| Telegraph | *Accepted.* Every event has a 3 s announce phase before anything changes: a banner with the event's name and icon, its sting, the announcer's line, and an in-world tell (the paw's shadow, the drone's siren, a rumble). Events aimed at a player name the player. "Funny, but fair" |
| Moods | *Accepted.* The level has one of four moods (calm, bored, grumpy, giddy) that drifts every 60-90 s (`SIM_MOOD`). The mood doubles the weight of its events (bored: Bored level, Level hiccups, Moving day; grumpy: Cat paw, Tax drone, Customer survey, more cave-ins; giddy: Duck parade, Rubber chickens, Disco inferno, Hat rain, Pizza delivery; calm: none, longer gaps) and drives presentation: a slow grade tint, dust mote colour, the ambient drone, the HUD mood icon. Mood has no direct gameplay effect |
| Vetoes and bans | *Accepted.* `ModeModule::allows_event(kind, &ModeState)` (default yes). Defaults: The Architect runs no director (the architect's Absurdity stroke triggers events instead); The Squeeze vetoes Cat paw and Duck parade from phase 4 on; Instagib vetoes Tax drone and Pizza delivery (energy and hull are hidden). Rule sets list banned events (`events: { "duck_parade": Banned }`, default all on), carried in rule set code format 5 and shown in the rule set editor |
| Eligibility | *Accepted.* An event's `eligible` checks its needs: Gravity coffee break needs wells; Pizza delivery a damaged player; Customer survey and Tax drone a clear leader (most mode score; team modes: best player of the leading side; ties: lowest id); Moving day and Mimic pickups at least 4 pickups in play; Cat paw an open boundary face with ships within 60 u |
| Replication | *Accepted.* `ModeView` gains a `MoodView` (protocol 6): mood, event id, phase (announce, active, ending), phase end tick, target player, 8 parameter bytes; sent on change. Everything an event does to movement is a pure function of `MoodView` and the match seed, so the predictor reproduces it (hiccup ticks, plane lock, duck paths, wells off) |
| Practice and tests | *Accepted.* The offline practice sim runs the director too. `?event=<id>` (practice and gallery) and the server's debug flag `--event-schedule <file>` (e2e and smoke only, logs a warning) force events at given times |

### 3.2 The 18 events

Durations exclude the 3 s announce phase. Damage from events never gives kill credit (killer: "the level"). Wave: the sim package (B) and the visual package (C) that build it.

| Event | Rule (sim) | Look and sound (client) | Wave |
| --- | --- | --- | --- |
| Bored level | The level's "up" turns 90 degrees about a seeded horizontal axis over 5 s and stays turned. Terrain and everything in it keep their sim coordinates (rotating the whole level with its contents changes nothing physically); what turns is `World::up`: horizon assist levels to it, Level hiccups and Hat rain follow it | The sky, sun, fog height and sun shadows turn; horizon assist rolls every pilot through the turn; a long bored yawn; the announcer sighs (open question 12) | B1, C1 |
| Tax drone | A drone enters from the nearest boundary cavern and chases the leader at 22 u/s along the network path with local avoidance; each touch takes 10 % of the leader's current energy (2 s apart, at most 5 touches, 25 s); 40 hp; shot down, it bursts into a scrap pile of 2 scrap per energy taken (max 20) | Boxy bureaucrat drone with a visor, receipt printer and siren; a "ka-ching" per touch; energy motes sucked out of the leader | B2, C2 |
| Gravity coffee break | Every well switches off for 20 s (fades over 1 s) | Wells dim and stop swirling, debris drifts free, a steaming mug icon; the announcer complains about union rules | B1, C1 |
| Rubber chickens | For 12 s every new projectile is flagged `CHICKEN`; speed, damage, craters unchanged | Projectiles drawn as rubber chickens scaled to the shot (ordnance: a big one, Tectonic Charge: a giant egg), squeak on fire, feather burst on impact | B1, C1 |
| Cat paw | After the tell (paw shadow and claw glow on the boundary wall nearest the densest group of ships), a paw (10 u pad) sweeps 90 u along an arc in 2 s and withdraws; ships touched take 30 damage and 25 u/s knockback; debris swept along; three claw grooves carved (chains of `SubtractSphere` r 2.5, about 36 edits spread over the sweep, scaled by rock strength), never in spawn zones or the Heist vein | Furry paw with pink pads, motion blur, rock spray along the grooves, a "mrrp" and a crash | B3, C3 |
| Tired announcer | For 30 s every kill plays a 3 s kill-cam for everyone (picture in picture) narrated by the announcer | Kill-cam window, deadpan captions, gibberish voice in the match's voice pack (tired, overexcited, passive-aggressive) | B3, C3 |
| Wrong genre | For 10 s every ship is locked to the plane through its position perpendicular to z (z velocity zeroed, facing snaps to +x or -x over 0.3 s, roll levelled); new projectiles fly in the plane; hits between ships and projectiles ignore depth (2D distance in x, y), terrain stays 3D | Side-on camera, the frame rendered at a quarter resolution with a 16-colour palette, chunky sprites for shots, chiptune loop, a "1-UP" jingle at the end (open question 14) | B1, C1 |
| Gossip wall | A face grows on the rock nearest the centroid of the living ships; for 15 s every ship's position is revealed to everyone (also cloaked ships) | Rock face with eyes and a moving mouth, through-wall markers with names, gossip captions ("Ana is 40 u below you, again"), mumbling voice | B2, C2 |
| Pizza delivery | A drone brings a pizza to a random living player weighted toward low hull (18 u/s along the network); staying under 1.5 u/s within 5 u for 3 s in total restores full hull; others can shoot the drone (30 hp), dropping the pizza as a 50-scrap pile anyone can scavenge; 30 s | Delivery drone with a pizza box and topping particles, doorbell chime, progress ring around the target | B2, C2 |
| Everything is cake | For 20 s every crater has 1.44x the radius (3x the volume, on top of rock strength); crater debris is cake: scavenging it repairs twice its scrap in hull | The terrain turns to cake in a wave from the tell's position: sponge layers, frosting on up-facing rock, sprinkles; cake crumbs instead of rock dust; a party horn | B1, C1 |
| Duck parade | Five giant rubber ducks (3.5 u) drift in a convoy along a network route from one end to the other at 5 u/s (45 s max); they follow the route's centre line and ignore rock; they absorb projectiles (no damage) and push ships aside (no damage). Positions are a function of start tick and seed, so the predictor knows them exactly | Glossy ducks that squash and stretch through narrow tunnels, squeak per hit, a marching kazoo tune | B3, C3 |
| Hat rain | For 25 s, 24 + 2 per player hats drift down along `up` at 3 u/s from the upper third of the level and rest where they touch rock; touch to wear one; a ship with 3 or more hats is revealed to everyone; hats are lost on death (they scatter as pickups for 10 s); the results name the "Hat Baron" (most hats at the end, ties to the earlier) | Hat stack on the hull (top hat, cowboy, party cone, chef, crown, ...), swaying; a pop per hat | B3, C3 |
| Customer survey | A drone parks 2.5 u in front of the leader's view, following its orientation, for 12 s or until shot (15 hp; anyone's shots); the star the destroying shot went through is the leader's rating, announced and shown in the results | Hovering drone with a five-star panel covering about a quarter of the view; "How was your match today?"; ratings sting | B2, C2 |
| Disco inferno | For 20 s the sun and ambient light drop to 8 %, a disco ball descends over the players' centroid, every ship leaves a light trail in its side colour. No other sim rule | Mirror ball with 8 sweeping coloured spot lights (VQ6), light trails as particles and lights, a four-on-the-floor loop; trails stay coloured in monochrome levels | B3, C3 |
| Big head day | For 15 s ship hit spheres against projectiles are 3x larger; terrain collision, ramming and pickups unchanged (tunnels still fit); lag compensation uses the scale at the rewound tick | Each ship wears a translucent cartoon balloon head at the hit size, honk on every hit | B1, C1 |
| Moving day | Every pickup not held moves to a new permutation of the pickup points; spawn timers kept; untouchable during the 2 s move | Pickups pack into cardboard boxes and arc to their new points; packing-tape sound; the announcer apologizes for the mess | B1, C1 |
| Mimic pickups | 3 + players / 4 (max 6) weapon or upgrade pickups become mimics for 20 s: they hop toward the nearest ship within 30 u at 12 u/s and bite (12 damage, knockback, 1 s apart); 25 hp; a killed mimic drops its content where it died; survivors return to their points | Crate grows teeth and a tongue, hops with squash and stretch, chomp sound | B2, C2 |
| Level hiccups | For 15 s, every 3 s (plus or minus 1, seeded) ships, debris and unstuck mines get an impulse along `up` that moves them about 2 u over 0.25 s with normal collisions; projectiles in flight are nudged by 2 u (swept) | Camera bump, dust shaken off ceilings, a "hic" | B1, C1 |

### 3.3 Sliders

| Slider | Decision |
| --- | --- |
| Verticality: box | *Accepted.* Five level shapes of about the same chunk count (load time): 0-19 [10, 2, 10] chunks (320 x 64 x 320 u), 20-39 [9, 3, 9], 40-59 [8, 4, 8] (today's box), 60-79 [7, 5, 7], 80-100 [6, 7, 6] (192 x 224 x 192 u). The team axis stays x in every shape (open question 4) |
| Verticality: structure | *Accepted.* Inside the box the cavern noise stretches vertically with the slider (flat: wide, low chambers; tall: chimneys), the mine lattice's vertical edge chance runs from 0.05 to 0.6, floating islands and rocks stack in tiers, network hubs gain a second layer above 60 |
| Gravity Chaos: sites | *Accepted.* Generator: 0 wells at 0, otherwise 1 + slider / 15 (max 7), placed by their own `SplitMix64` ("GRAVWELL", like the Heist site) in caverns and hubs, at least 40 u from spawn zones, 30 u apart, outside the Heist vein. Each is a hard-rock core (1.5-2.5 u) in a carved hollow (8 u radius); in open sky it is a floating stone. `Generator::gravity_wells()` lists them; `MatchSetup` carries them |
| Gravity Chaos: force | *Accepted.* Softened inverse-square pull toward the core, `a = G r / (r^2 + s^2)^1.5`, tapered smoothly to zero at the well's range (20-40 u, growing with the slider); the peak pull is at most 40 % of the Tinker's per-axis thrust acceleration at slider 100, so every ship can always thrust out (it costs boost). Applies to ships, debris, wrecks and scrap that moves, unstuck mines, ordnance and specials flagged `gravity: true` in their data file (missiles, Seed Grenade, Swarm blobs); energy bolts and kinetic rounds fly straight (design doc). IEEE-exact maths (`sqrt` only). The predictor applies the same force from `MatchSetup` (open question 5) |
| Palette | *Accepted.* 0-49: saturation falls to 50 %; 50-79: ink fades in (outlines from depth and normals, toon-banded light, paper grain); 80-100: a monochrome level (black-and-white ink, hatching from light and AO, ink-wash sky) with pigment pickups (§3.4). Paint keeps its colour in every level (it is Paint the Void's score); ship liveries follow the pigment rules. Render-only except the monochrome flag (`MatchSetup::monochrome`) |
| Instability: rock | *Accepted.* Every crater's radius is multiplied by `1 + 0.4 x instability / 100` on top of rock strength (exactly 1 at Instability 0, so existing replays keep their hashes) |
| Instability: cave-ins | *Accepted.* Craters add strain to a 16 u cell (removed volume x material factor: weak 1.5, rock 1, hard 0.5); strain decays 3 % per second. Past a threshold that falls with Instability (tuning target: at 30 about 10 Bolt Rocket craters in one spot within 20 s, at 100 about 4, at 0 never) the cell caves in: a 1.5 s warning (cue with position: rumble, dust trickling, glowing cracks), then 5-9 `AddSphere` rubble spheres (1.5-3 u) in the tunnel on the `up` side, a matching hollow in the ceiling above, 4-8 debris pieces with scrap, 20 damage to ships within 3 u. Rubble is weak rock (`World` keeps rubble regions that override the material, hashed). Spheres that would overlap a ship are left out (Seed Grenade rule). At most one cave-in per 8 s per level; none in spawn zones, the Heist vein or within 6 u of an extraction zone (open question 6) |
| Absurdity | §3.1 |
| Playlists | *Accepted.* `RandomSliders` takes ranges for all seven sliders; defaults: Enclosure and Density as today, Verticality 20-80, Gravity Chaos 0-60, Palette 0-50 (monochrome only when a playlist entry asks), Absurdity 10-60, Instability 10-60 |

### 3.4 Monochrome levels and pigment

| Topic | Decision |
| --- | --- |
| What is monochrome | *Accepted.* Palette 80 or more, or any Last Color Standing level |
| Pigment pickups | *Accepted.* New `PickupKind::Pigment` at pickup points, one per 4 players (at least 2), 30 s respawn; settings in the rule set (`pigment_pickup`, like `scrap_pickup`; rule set code format 5) |
| Pigmented | *Accepted.* Touching pigment colours your ship in your side's colour for 40 s (refreshed by another pickup): +25 % damage with every weapon (damage pipeline), drawn in full colour, and shown to everyone through walls with a marker; coloured drips trail behind. Death drops it as a splash pickup for 15 s |
| Grey | *Accepted.* Without pigment your ship is ink grey: enemies see your name tag only within 20 u and no through-wall indicator; teammates still see your team colour on the tag and outline. Visual only; relevance (what the server sends) is unchanged |
| Last Color Standing | *Accepted.* Pigment restores 17 colour (half a death, capped at 100), one per 4 players, 45 s respawn; no damage bonus there (colour already makes you visible) |

### 3.5 The Architect

| Topic | Decision |
| --- | --- |
| Shape | *Accepted.* PvP asymmetric for 2-8 players: one architect against 1-7 runners. A match is `min(players, 3)` rounds of 4 minutes; the architect role rotates by join order. Coop variant (the architect helping runners against the director) later (open question 8) |
| Runners | *Accepted.* Spawn at the team 0 end, must reach the exit gate at the team 1 end (an extraction-zone-style volume). Two lives per round; runners cannot damage each other; default loadout Scrap Laser + 4 Bolt Rockets (to dig through rubble), pickups on |
| Architect | *Accepted.* No ship: a god camera (free flight plus an overview cutaway), sees runners through rock. Budget 100 points, +4 per second; strokes act on a sphere of radius 12-24 u that the architect places: Enclosure+ (grow rock, 25), Enclosure- (open a region, 15), Density+ (pillars and floating rocks, 20), Gravity Chaos+ (a temporary well for 30 s, 30), Instability+ (a cave-in, 35), Absurdity+ (start a random eligible event, 60), Palette (ink blackout of the region for 20 s, 15), Verticality (Bored level, 80, once per round). Every stroke is telegraphed for 2 s (glowing outline the runners see) and never encloses a ship |
| Stroke edits | *Accepted.* A stroke samples the generator with the changed slider on a 4 u grid inside the sphere (new query `SimTerrain::reshape_sample`, answered by `GameTerrain` from its generator) and emits at most 32 existing `AddSphere` / `SubtractSphere` edits, spread over 8 ticks like the Tectonic cracks. No new `TerrainEdit` kind, so contracts, edit log and wire encoding stay |
| Scoring | *Accepted.* Each runner through the exit scores 1 for the runners' side; each runner who does not make it scores 1 for the architect. Runners win the round if at least half get out. Match: most points over the rounds; the results table shows per-player architect and runner points |
| Wire | *Accepted.* `ArchitectAction` message (reliable, protocol 6): stroke kind, centre, radius; the server validates budget, cooldown and range |

### 3.6 Carried-over items and leftovers

| Topic | Decision |
| --- | --- |
| Tectonic collapse tick | *Accepted.* In `ChunkStore::bake_edit`: evaluate only corners the edit can change (inside the sphere plus the snap margin), keep stored samples elsewhere; if that is not enough, bake edits above 8 u radius lazily per chunk (sim queries read the analytic edit until the chunk is baked). Target: collapse tick < 2 ms in `tests/tectonic_tick.rs` (assert tightened from 8 ms) |
| Dual contouring in play | *Accepted.* Keep the surface-nets topology and everything built on it (AO, material weights, thin passes, seams, the Squeeze shell mesher), and place the vertex of sharp-feature cells by the dual-contouring QEF (edge crossings and exact-field normals, clamped to the cell) where `SNAP_KINK` already flags a crease. Mine shafts get true right-angle edges and corners. Budget: at most +20 % mesh time (busy chunk native <= 2.6 ms, wasm p95 <= 3.4 ms), within the client's 4 ms per-frame chunk budget. The standalone `dual_contouring` mesher stays as a reference. This closes the item if accepted (open question 10) |
| Thin-rock jaggies | *Accepted.* Thin-rock pass: promote open corners whose half-cell box contains rock by the exact field (mirror of VQP's thin-air pass), at most +10 % mesh time; criterion: `cave-sunbeam` left edge smooth in the gallery, a unit test on a 0.4 u blade |
| Bore-rim folds | *Accepted.* With the QEF placement plus a rim-aware quad split: at most 1 in 100 triangles folded at the gallery bore mouth (`tests/thin_tunnels.rs` tightened from 1 in 50) |
| Reference mid-GPU timings | *Accepted.* The maintainer provides a GTX 1660 / RX 6600 / Apple M1 class machine for one gallery run after waves A, C and D (`gallery.mjs --label refgpu-<wave>`). Without one: a run on the development machine's integrated Radeon (`--power low`, far below the reference class) as a pessimistic floor, and the criterion stays "estimated" (open question 11) |
| Teams 3 and 4 | *Accepted.* Generator version 4 adds a cavern at each z end (centre x) linked to the hubs; `ls_game::setup` places team 2's zone at low z and team 3's at high z (slabs of `SPAWN_ZONE_FRACTION` of the z length). Two-team placement is unchanged |
| M3 soak | *Accepted.* The lead re-runs `node tools/e2e.mjs --count 8 --playlist default --duration 3600 --private-rooms duel:2,team_brawl:4` on a quiet machine with a per-tick log of slow steps before wave A integration, records it in milestone-3-plan.md, and opens an A task only if the 4 ms p99 fails there |

### 3.7 Client: workshop, audio, announcer

| Topic | Decision |
| --- | --- |
| World workshop | *Accepted.* "Make a world" on the start screen: seven sliders with one-line meanings, a seed with a dice button, a live preview (the FI backdrop flythrough, regenerated a chunk per frame on change, debounced 300 ms), the share code with copy ("Save"), "Fly it" (practice) and "Host it" (invite link with `code=`). The slider panel is a component the Architect's UI reuses |
| Audio | *Accepted.* `web/src/audio/`: a WebAudio engine, procedural sounds defined as parameter sets (sfxr-style oscillators, noise, envelopes, filters; FM for chiptune), positional sounds through a panner with distance falloff, a voice cap (32), mixer buses (master, effects, music, voice) with volumes in settings, nothing plays before the first user gesture. No recorded assets in M4 (open question 2) |
| Announcer | *Accepted.* A caption character (portrait icon, name, line) for event announcements, multi-kills, first blood, cave-ins and the Tired announcer, with a synthesized gibberish voice; three voice packs (tired, overexcited, passive-aggressive: line sets plus voice parameters), one per match from the match seed. Lines live in `web/src/audio/announcer/*.ts`, not in `data/` (client-only, not hashed) |
| Kill-cam | *Accepted.* The client keeps 3 s of snapshots (it already interpolates from them) and replays the killer's view in a window. Used by Tired announcer; also offered after your own death in modes with respawns (setting, default on, skipped by any key) (open question 13) |

## 4. Determinism and handshake impacts

| Item | Change | Owner of the change |
| --- | --- | --- |
| `GENERATOR_VERSION` | 3 -> 4 once (Verticality shapes and structure, gravity well sites and hollows, team 2/3 caverns). Worldgen goldens re-blessed (`LS_BLESS_GOLDEN=1`), share codes re-encoded in Rust tests, tools, docs and examples (codes carry the version; version 3 codes are refused as today), the setup sweep extended to all seven sliders. Mesher packages re-bless only the mesh goldens afterwards, without a version bump | A1 (version), A2 (mesh goldens) |
| `SIM_BEHAVIOR_VERSION` | 5 -> 6 at wave A integration (director stage, gravity in movement, Instability crater scale and cave-ins, pigment). Wave B and C changes do not bump it again unless a released behaviour changes (version 6 is not released before M4 closes) | A3 assigns, integrator decides |
| `REPLAY_GOLDEN_HASH` and the nine mode replays | Unchanged through wave A: the scripted replays run at Instability 0, without wells and with Absurdity 0 in their setup, which must leave the world bit-identical (that is the proof that the new systems are inert when off). New goldens: one 20 s replay per event (forced by the schedule), a gravity replay, a cave-in replay, a pigment replay, an Architect replay; fixture `crates/ls-sim/tests/fixtures/mood_replays.txt`, checked on native, wasm32 and by `tools/determinism-check.mjs` | A3, A4, each B package, C6 |
| Data hash | New embedded files `data/events/*.ron` and `data/world/{gravity,instability}.ron`; rule set absurdity values (20 -> 100), pigment settings, `the_architect.ron`; every one changes the hash automatically | A3, A4, B4 |
| Protocol | 5 -> 6 in wave A: `MoodView` in the mode state, event entity kinds, projectile flag `CHICKEN`, ship flags for pigmented and hats, `ArchitectAction`, the architect role in the mode view, new `CueKind`s (event announce, cave-in warning, stroke telegraph). Additions made before wave B integration stay in 6 | A5 |
| Rule set codes | Format 4 -> 5: event bans, pigment pickup settings, The Architect's mode settings (`ModeKind` 6). Formats 2-4 keep decoding. `LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode` regenerates `web/src/rules/catalog.ts` | A3 (format), B4 (Architect), integrator (catalog) |
| Contracts (contracts.md) | New RNG streams (`SIM_MOOD`; "GRAVWELL" in the generator; cave-in placement on `Ctx::rng`), gravity wells (sites, force law, who is pulled), `World::up`, strain cells and rubble regions, `SimTerrain::reshape_sample`, mood state ids (`EventKind` and mood indices, append only), `ModeKind` 6. `TerrainEdit` is unchanged | each owning package, D3 |
| Level bounds | Five box shapes: `Welcome` already sends the quantization bounds; mode objective quantization (bounds + 2 u) follows them; renderer extents (sun shadow region, boundary grid, atmosphere), nav grid size and The Squeeze's level radius must read the real bounds (A1 checks all of them) | A1 |
| WASM cross-check | `tools/determinism-check.mjs` gains the version 4 share codes (one per Verticality shape, one with seven wells) and the mood replays | A1, C6 |

## 5. Waves

Each wave is a set of packages that run in parallel in separate work folders (`loosescrews.win2`+), one package per folder, as in M3. Packages in one wave touch disjoint files except under "Shared files"; the integrator (win1) merges them in the listed order. A package may start before its dependency is merged only against the listed day-one contract. "As built" notes are added under each wave during execution, as in milestone-3-plan.md.

Rules for every package that draws or plays something:
- Gallery scenes for every look it adds (`web/src/gallery/`, new `moodScenes.ts`), before and after runs (`gallery.mjs --label <pkg>-before|after`), all within budget; for events, a strip of three frames (announce, peak, aftermath) per event. The lead sends the shots for phone review after each integrated wave.
- Browser runs headless only, with `tools/browser-smoke/fake-pointer-lock.mjs` on every page, scratch scripts included.
- Cost shares on the reference mid GPU (1080p, high): mood presentation 0.2 ms, wells 0.4 ms, ink pass 0.8 ms, event visuals 1.0 ms at peak (the gallery fails a pass 25 % over its share).
- Sounds: every new sound plays in `?audio=demo` (a page listing each sound with a play button, used for review).

### Wave A — foundations

| Package | Work | Owner areas | Depends on |
| --- | --- | --- | --- |
| A1 Generator version 4 | Verticality box shapes and structure, gravity well sites and hollows (`Generator::gravity_wells`), team 2/3 caverns, version 4, golden re-bless, share codes re-encoded everywhere; `ls_game::setup`: team 2/3 spawn zones, wells into `MatchSetup`; bounds audit (renderer extents, nav grid, Squeeze radius); playlist slider ranges | `crates/ls-worldgen` (`generator.rs`, `network.rs`, `sliders.rs`, `golden.rs`, `lib.rs`), `crates/ls-game/src/setup.rs`, `crates/ls-server/src/playlist.rs`, codes in tests, `tools/`, docs | — ; **day-one contract**: `WellSite` and `Generator::gravity_wells()` signatures and the five box shapes pushed first |
| A2 Mesher and bake | Tectonic collapse bake (carried over), QEF placement at sharp-feature cells (carried over), thin-rock pass, bore-rim folds; mesh goldens re-blessed; `worldgen_bench` and wasm timings before and after | `crates/ls-worldgen` (`chunk.rs`, `mesh.rs`, `grid.rs`, `connectivity.rs` if needed), `crates/ls-game/tests/tectonic_tick.rs` | — |
| A3 Director framework | `director/`, `EventModule` trait, `events/mod.rs` registry with 18 stub modules (announce only), moods, cadence, vetoes hook, `MoodView`, `data/events/*.ron`, effective Absurdity, rule set format 5 (event bans, pigment settings field), built-in rule sets' absurdity; `SIM_BEHAVIOR_VERSION` 6 | `crates/ls-sim` (`director/`, `events/`, `mode.rs` view types, `ruleset.rs`, `rulecode*`, `data.rs`), `data/events/`, `data/rulesets/` (absurdity lines) | — ; **day-one contract**: `MoodView`, `EventKind` list and the `EventModule` hook signatures pushed first |
| A4 World physics | Gravity force on ships (`simulate_movement`), entities and flagged projectiles; `World::up` (horizon assist reads it); Instability crater scale, strain cells, cave-ins, rubble regions; pigment pickups, pigmented state and damage bonus, Last Color Standing pigment; `data/world/*.ron`; scenario tests and replays | `crates/ls-sim` (`ship.rs`, new `gravity.rs`, `instability.rs`, `pigment.rs`, the force and strain calls in `world.rs`, `modes/last_color_standing.rs`), `data/world/`, `data/weapons/*.ron` (`gravity: true` lines) | A3 day-one contract (shared `world.rs`), A1 day-one contract (`WellSite`) |
| A5 Protocol 6 and glue | `MoodView` wire item, event entity kinds, `CHICKEN` and pigment/hat flags, `ArchitectAction`, new cues; `ClientSession` and `Room` carry mood state; the predictor applies wells, `up` and mood movement effects (`Predictor::set_mood`); `ls-wasm` exports mood view, wells, up, monochrome | `crates/ls-net`, `crates/ls-game` (`client/`, `server/`, `convert.rs`, `mode.rs`; not `setup.rs`), `crates/ls-wasm` | A3 and A4 day-one contracts |
| A6 Audio and announcer | `web/src/audio/` engine, synth library, spatial mixer, settings sliders; first sounds for existing cues (Tectonic rumble, Polite chime, explosions, impacts, boost, weapon families); announcer captions with three voice packs and gibberish voice; `?audio=demo` | `web/src/audio/` (new), `web/src/ui/announcer.ts` (new), one call each in `main.ts` and the settings panel | — |
| A7 Ink look (Palette) | Outline pass (depth and normals), toon banding, hatching, paper grain, monochrome-level look, keep mask for paint and pigment; `worldGrade` recalibrated to the Palette steps; gallery scenes `palette-{0,40,65,85,100}` on caves, mines and open sky | `web/src/renderer/` (new `ink.ts`, `shaders/ink.ts`; `post.ts`, `atmosphere.ts`, `renderer.ts` pass wiring) | — |
| A8 World workshop | Workshop page and slider panel component, preview flythrough on code change, save/copy, fly it, host it; gallery pages `workshop` and `workshop-tall`; smoke phase | `web/src/ui/workshop.ts` (new), `web/src/attract/` (preview hook), `web/src/ui/startScreen.ts` (one entry) | — (preview shows version 4 worlds once A1 is merged) |

Shared files: `crates/ls-sim/src/world.rs` (A3 adds the director stage, A4 the force and strain calls; both small, against the day-one contracts); `MatchSetup` (A4 owns the struct, A1 fills `wells`); `web/src/main.ts` (one call each from A6, A7, A8); `CLAUDE.md`, `docs/dev-guide.md` (integrator resolves).

Merge order: A1, A2, A3, A4, A5, A7, A6, A8.

Acceptance (wave A):
- A1: worldgen goldens re-blessed at version 4 and equal on native and wasm32; the setup sweep (8 seeds x 25 slider mixes, now over all seven sliders and all five shapes) passes: spawns, pickups, Heist sites, wells and team 2/3 zones reach the network with `CONNECT_MARGIN`, no well within 40 u of a spawn zone; a 4-team match spawns each team in its own zone (test); gallery `shape-flat`, `shape-tall`, `wells-cave`, `wells-sky` before and after; `node tools/determinism-check.mjs` passes.
- A2: collapse tick < 2 ms in `tectonic_tick.rs`; mine-shaft corner error at feature cells p95 <= 0.03 u (`mesh_quality`); mesh time within +20 % (busy chunk native, wasm p95), thin-rock and fold criteria from §3.6; The Squeeze remesh benchmark still within its 3 ms p99; gallery `mine-shaft`, `mine-corner` (new), `cave-sunbeam`, `fit-tunnel` (bore mouth) before and after.
- A3: with Absurdity 0, Instability 0 and no wells, `REPLAY_GOLDEN_HASH` and all nine mode replays unchanged; director statistics over 200 simulated 5-minute matches per Absurdity 0/20/50/100 within 20 % of the §3.1 expectations, 0 events at 0 or with a rule set cap of 0, none in warm-up; vetoes and bans tested; rule set code format 5 round-trips every built-in rule set and decodes format 4 codes.
- A4: a ship at full thrust escapes every well at Gravity Chaos 100 (test), bolts and kinetic rounds unbent, missiles bent; cave-ins follow the tuning targets at Instability 30 and 100, never at 0, never enclose a ship (property test over 8 seeds); pigment rules (damage bonus, expiry, drop on death, Last Color Standing restore); gravity, cave-in and pigment replays golden on native and wasm32.
- A5: protocol 6 round-trip tests for every new item; a client predicting through a well field and through mood movement effects keeps the reconciliation error under 0.05 u in an in-process netplay test.
- A6: no AudioContext before a user gesture (smoke check); the voice cap holds under a 64-sound burst (test); every sound listed in `?audio=demo`.
- A7: gallery palette scenes within the 0.8 ms ink share; paint keeps its colour at Palette 100; zero WebGPU validation errors; phone review of the five palette steps.
- A8: the smoke opens the workshop headless, changes two sliders, the preview regenerates, the copied code decodes to those sliders, "Fly it" starts practice on it.

### Wave B — events, The Architect, world visuals

| Package | Work | Owner areas | Depends on |
| --- | --- | --- | --- |
| B1 Events: world and movement | Bored level, Level hiccups, Gravity coffee break, Wrong genre, Big head day, Rubber chickens, Moving day, Everything is cake: modules, data, scenario tests, replays; the predictor side of their movement effects | `crates/ls-sim/src/events/` (these eight files), `data/events/` (their files), the `move_effects` application in `crates/ls-game` predictor | A3, A4, A5 |
| B2 Events: visitors | Tax drone, Pizza delivery, Customer survey (shared `events/drone.rs`: network-path following with local avoidance), Mimic pickups, Gossip wall | `crates/ls-sim/src/events/` (these files and `drone.rs`), `data/events/` | A3, A5 |
| B3 Events: spectacles | Duck parade (route and exact positions for the predictor), Hat rain (hats, Hat Baron in results), Cat paw (sweep, knockback, grooves spread over ticks), Disco inferno, Tired announcer (flags and kill events) | `crates/ls-sim/src/events/` (these files), `data/events/` | A3, A5 |
| B4 The Architect | `modes/the_architect.rs` (rounds, roles, exit, scoring, budget, strokes), `SimTerrain::reshape_sample` and its `GameTerrain` answer, `ArchitectAction` handling in `Room`, `the_architect.ron`, playlist and lobby support for the role (rotation by rounds), scenario tests and replay | `crates/ls-sim/src/modes/the_architect.rs`, `crates/ls-sim/src/terrain.rs` (one trait method), `crates/ls-game/src/terrain.rs` (reshape sampling), `crates/ls-game/src/server/` (action handling), `crates/ls-server` (`lobby.rs`, `playlist.rs` entries), `data/rulesets/the_architect.ron` | A1, A3, A4, A5 |
| B5 World visuals | Gravity wells (core stone, accretion swirl of motes and debris, lensing through the refraction pass, violet glow, pull indicator in the HUD), cave-in warning and rubble look, pigment pickups and pigmented ships (with A7's keep mask), grey-ship name tag rules, mood presentation (grade tint drift, motes, HUD mood icon, ambient drones through A6), sky and sun turning for Bored level (sun shadow redraw spread over frames) | `web/src/renderer/` (new `wells.ts`, `shaders/wells.ts`), `web/src/game/` (new `worldFx.ts`, `moodFx.ts`), `web/src/ui/` (pull indicator, mood icon in `hud.ts`), `web/src/gallery/moodScenes.ts` (new) | A4, A5, A6, A7 |

Shared files: `events/mod.rs` registry and `data/events` list (one line per event, filled by A3's stubs, so B packages only replace stub bodies); `web/src/rules/catalog.ts` regenerated after the merge; `BUILTIN_RULESETS` (B4 adds one line).

Merge order: B1, B2, B3, B4, B5.

Acceptance (wave B):
- B1-B3: each event has scenario tests for every rule in §3.2, a 20 s forced replay with a golden hash equal on native and wasm32, and an in-process netplay test with 4 clients and 0 desyncs while it runs; movement events (Bored level, Hiccups, Wrong genre, Coffee break, Duck parade pushes) keep the predictor's reconciliation error under 0.1 u; Cat paw's grooves and every cave-in stay within the client remesh budget in `ls-game`'s remesh benchmark (p99 per chunk < 3 ms, as The Squeeze's).
- B4: a full Architect match with in-process bots-in-sim reaches a result; budget, cooldown and range are enforced server-side (a forged action is rejected and counted); no stroke ever encloses a ship (property test); stroke edits <= 32 per stroke, spread over 8 ticks; replay golden.
- B5: gallery `wells-close`, `wells-debris`, `well-lensing`, `cave-in-warning`, `rubble`, `pigment-pickup`, `pigmented-vs-grey` (Palette 90), `mood-{calm,bored,grumpy,giddy}`, `bored-turn` (three frames) within budget; phone review.

### Wave C — event visuals, Architect client, bots, tests

| Package | Work | Owner areas | Depends on |
| --- | --- | --- | --- |
| C1 Event visuals I | Looks and sounds of B1's eight events: the turning sky, hiccup camera bump and ceiling dust, coffee-break well fade and mug, Wrong genre side camera, quarter-resolution 16-colour post and chiptune, balloon heads and honk, rubber chicken models and feathers, moving boxes, cake terrain material with its wave and crumbs | `web/src/game/events/` (new, one file per event), `web/src/renderer/` (new `pixelPost.ts`, cake branch in `shaders/terrain.ts`), `web/src/renderer/models/` (chicken, egg, box, balloon head), `web/src/audio/sounds/` (their sounds) | B1, A6 |
| C2 Event visuals II: visitors | Tax, pizza and survey drones (VQ4 kit models), mimic crates with teeth, the gossip wall face, through-wall reveal markers, captions | `web/src/game/events/` (their files), `web/src/renderer/models/` (drones, mimic, face), `web/src/audio/sounds/` | B2, A6 |
| C3 Event visuals III: spectacles | Ducks with squash and stretch, hats on hulls, the cat paw, disco ball with spot lights and light trails, kill-cam window and Tired announcer narration | `web/src/game/events/` (their files), `web/src/game/killcam.ts` (new), `web/src/renderer/models/` (ducks, hats, paw, ball), `web/src/audio/sounds/` | B3, A6 |
| C4 Architect client | Architect camera (free flight, overview cutaway, runners through rock), stroke placement with radial menu and the workshop slider panel, budget bar, telegraph visuals for runners, exit gate, round HUD and results | `web/src/game/architect/` (new), `web/src/ui/architect.ts` (new), `web/src/ui/modeHud.ts` and `results.ts` (Architect lines) | B4, A8 |
| C5 Bots in moods | Wells (counter-thrust, core avoidance), cave-ins and rubble (re-plan on edits), pigment (take it when strong, avoid when hunted), event reactions (shoot drones and mimics, dodge ducks and the paw, hold still for pizza, collect hats, pick a star), runner and architect bots; `tests/mood_matches.rs` (release): Absurdity 100 rotation with every event forced at least once, 8 bots, 0 desyncs; an Architect match with 6 bots | `crates/ls-bots` | B1-B4 |
| C6 End-to-end, smoke, determinism | Mood replays in `determinism-check.mjs` and `ls-wasm`; `tools/e2e.mjs --event-schedule` and a playlist entry with Absurdity 100 and The Architect; browser smoke phases `moods` (each event forced once in a match, screenshots, zero validation errors), `architect` (an architect page and a runner page in two headless contexts), `workshop`; generated `userDocs/reference/events.md` from `data/events` (`gen-weapon-docs.mjs`) | `tools/`, `tools/browser-smoke/`, `crates/ls-wasm` (replay export) | B1-B5, C5 |

Shared files: `web/src/main.ts` (one hook each from C1-C4 into `game/events/index.ts`, owned by C1); `web/src/gallery/moodScenes.ts` (B5 owns it; C packages add their scenes in `eventScenes.ts`, new, owned by C1, one entry per event from each package).

Merge order: C5, C1, C2, C3, C4, C6.

Acceptance (wave C):
- C1-C3: for each event, gallery strip `event-<id>` (announce, peak, aftermath) within budget, its sound in `?audio=demo`, unit tests of its timeline and packing, and phone review; events whose look is not accepted in review stay `enabled: false` in their data file until fixed.
- C4: the browser smoke plays an Architect round with an architect page and a runner page: a stroke placed, telegraphed on the runner's page, applied, budget spent.
- C5: `mood_matches.rs` and the Architect bot match pass in release; reconciliation error max 0.1 u near wells and ducks.
- C6: `determinism-check.mjs` passes with the mood replays; the smoke `moods` phase forces all 18 events with zero WebGPU validation errors; e2e quick playlist with Absurdity 100 and The Architect passes all checks.

### Wave D — review, maintainer review, docs, soak

| Package | Work |
| --- | --- |
| D1 Rust review | Review of A1-A5, B1-B4, C5 for determinism rules, hot paths, wire budget; fixes |
| D2 Web review | Review of A6-A8, B5, C1-C4; fixes; the D2 items of M3 if their files were touched |
| D3 Docs | New `docs/architecture/moods-and-world.md` (director, events and hooks, moods, sliders as built, gravity, instability, pigment, The Architect, workshop, audio); updates to contracts, determinism ("Milestone 4 record"), netcode (protocol 6), renderer (ink, wells, events, kill-cam), game-glue, modes-and-lobby, arsenal-framework (gravity flag); `userDocs/player-manual.md` (sliders, events, pigment, The Architect, workshop, audio settings), `userDocs/hosting-guide.md` (absurdity caps, event bans, `--event-schedule` as a test-only flag, Architect playlists); generated events reference; carried-over items closed in implementation-plan.md §8 |
| D4 Milestone soak | 60 minutes on a quiet machine: default rotation with The Architect and random worlds over all sliders, Absurdity 100 rule sets, 8 bots, plus a private duel; same checks as M3 plus events per match and remesh spikes |
| D5 Visual polish | The phone review list over the whole gallery (every mood, palette step, event strip, Architect views); grade and exposure tuning; the reference-GPU gallery run |

Acceptance (wave D): §6.

## 6. Acceptance criteria (milestone)

- The comedy director rolls events by the effective Absurdity as specified (statistical test), never outside a running match, always with a 3 s telegraph; moods drift and bias the weights; vetoes and rule set bans work.
- All 18 design-doc events exist as modules with data files, behave as in §3.2, and have scenario tests, a replay golden equal on native, wasm32 and the shipped WASM, a gallery strip, a sound, and bot reactions.
- Verticality, Gravity Chaos, Palette and Instability each change the level visibly and as specified; the setup sweep over all seven sliders and five shapes passes; worldgen goldens at version 4 match native, wasm32 and the WASM package.
- Ships can always escape a well; bolts and kinetic rounds fly straight; prediction stays within 0.1 u of the server near wells and during movement events.
- Cave-ins follow the tuning targets, never trap a ship, never occur at Instability 0, and stay within the remesh budget.
- Monochrome levels look like ink and carry pigment pickups with the §3.4 rules; Last Color Standing has pigment pickups that restore colour.
- The Architect plays a full match (bots in release tests and the browser smoke): strokes within budget, telegraphed, never enclosing a ship, results per round.
- The world workshop makes, previews, saves and plays a world from all seven sliders.
- Audio: every event and the listed cues have sounds; nothing plays before a user gesture; volume settings persist.
- Carried-over items closed with evidence: Tectonic collapse tick < 2 ms; sharp mine edges in play within the mesh budget; thin-rock and bore-fold criteria; teams 3 and 4 spawn in their own zones; reference mid-GPU gallery timings measured (or recorded as estimated if no hardware was available, with the maintainer's agreement); the M3 soak re-run recorded.
- A 60-minute soak on a quiet machine (D4) with Absurdity 100 and The Architect in the rotation: every match reaches a result, 0 disconnects at match end, 0 desyncs, 0 protocol errors, at most 1 full snapshot per join or transition, tick p99 < 4 ms per room.
- Visual sign-off: the gallery strips of every event, every palette step, the wells, the moods, the workshop and the Architect views are accepted in phone review; nothing ships with `enabled: false` unless agreed in review.
- `SIM_BEHAVIOR_VERSION`, `REPLAY_GOLDEN_HASH` (unchanged), the new goldens, protocol 6 and rule set code format 5 changed deliberately and are recorded in determinism.md.
- docs/ and userDocs/ describe everything as built.

## 7. Risks

| Risk | Mitigation |
| --- | --- |
| Scope: 18 events, four sliders, a new mode, audio, a workshop and six carried items make M4 the largest milestone | The director works with any subset: each event has `enabled` in its data file and ships on its own. Cut order if time runs short (pending confirmation): kill-cam outside the Tired announcer event, the Architect's Palette and Verticality strokes, moods' presentation, Wrong genre's depth-free hits (fall back to a plain plane lock) |
| Comedy that looks cheap hurts the first impression more than no comedy | Gallery strips and review per event before it is enabled; VQ kit models, particles and lights only, no placeholder primitives in a shipped event |
| Prediction errors from forces and moving obstacles (wells, hiccups, ducks, Wrong genre) | Everything that moves the own ship is a pure function of replicated state and the seed; netplay tests with reconciliation bounds per event (A5, B1-B3) |
| Remesh spikes from cave-ins, the cat paw and Architect strokes | Edits spread over ticks, bounded per stroke; remesh benchmark in `ls-game` per edit source; A2's cheaper bake helps all of them |
| Five box shapes break code that assumed 256 x 128 x 256 | A1 audits every bounds reader (renderer extents, sun shadow region, nav grid, Squeeze radius, camera far plane, atmosphere) and runs the gallery and setup sweep on all shapes |
| Generator version 4 churns share codes in tests, tools and docs | One package (A1) owns the bump and re-encodes every code in the same change, as in M2 and M3 |
| Gravity wells make 6DOF flight feel heavy or nauseating | Pull capped at 40 % of thrust, smooth taper, a HUD pull indicator; gallery review and a play-test before the default playlists use high Gravity Chaos |
| Audio quality from pure synthesis | Listening review through `?audio=demo` after A6 and C1-C3; recorded packs remain a later cosmetic option |
| Shared machine distorts timing tests and soaks | Timing assertions with the M3 margins; soaks only when the machine is confirmed quiet; the reference-GPU run on separate hardware |

## 8. Decisions

Decision: every proposed answer below is accepted as written. For question 11 no mid-range GPU machine was named: measure the integrated GPU of the development machine as a floor and keep the mid-GPU criterion "estimated" until a machine is available.

1. **All 18 events in M4, or a first dozen?** Proposed: all 18 (the design-doc review asked for more than a dozen); an event whose look is not accepted stays disabled by data until it is fixed.
2. **Audio in M4?** The game has no sound, and many events are built on sound (honk, hiccup, chiptune, announcer). Proposed: yes, a procedural audio foundation with synthesized sounds for every event and the main cues; recorded voice packs come later as cosmetics.
3. **How do the world's Absurdity and the rule set's Absurdity combine?** Proposed: the rule set is a cap, effective = min(world, rule set); built-in rule sets move to 100 (duel 50), the Serious playlist uses 0; new random worlds draw Absurdity 10-60.
4. **Does Verticality change the level box, not only its inside?** Proposed: yes, five shapes from 320 x 64 x 320 u to 192 x 224 x 192 u at about the same chunk count; teams stay at the x ends.
5. **Do gravity wells pull ships?** Proposed: yes, capped at 40 % of thrust so you can always escape; missiles, debris, mines and wrecks bend too; energy bolts and kinetic rounds fly straight (design doc).
6. **Instability cave-ins drop rubble that can block a tunnel?** Proposed: yes, after a 1.5 s warning, as weak rock that two or three rockets clear, never trapping a ship, never in spawn zones.
7. **Monochrome levels: Palette 80+ with pigment at +25 % damage, visible to everyone, for 40 s?** Proposed: yes; paint stays coloured in every level.
8. **The Architect: PvP only in M4?** Proposed: yes, one architect against up to 7 runners in rotating rounds; the coop variant joins the coop modes in M6.
9. **Moods (calm, bored, grumpy, giddy) on top of the event table?** Proposed: yes, as presentation and event weights only, no direct gameplay effect.
10. **Does "dual contouring in play" close with dual-contouring vertex placement inside the existing mesher?** Proposed: yes; it gives true mine edges and keeps AO, material blending and seams, within +20 % mesh time, while switching meshers would lose those.
11. **Is a reference mid-range GPU machine (GTX 1660, RX 6600 or an M1 Mac) available for one gallery run after waves A, C and D?** Proposed: yes if available; otherwise we measure the integrated GPU as a floor and keep the criterion "estimated".
12. **Bored level: turn the level's "up" (sky, sun, horizon assist) rather than the terrain?** Proposed: yes; turning the terrain itself would change nothing physically in zero gravity and cost a full remesh.
13. **Kill-cam after your own death, not only during the Tired announcer event?** Proposed: yes, as a setting on by default in respawn modes, skipped by any key.
14. **Wrong genre: hits ignore depth while the plane lock lasts?** Proposed: yes ("in 2D, depth does not exist"); the fallback is a plain plane lock with normal 3D hits.

## Milestone summary

Written when M4 closes, as in milestone-3-plan.md: integration notes per wave, every §6 criterion with its status and evidence, and what stays open.
