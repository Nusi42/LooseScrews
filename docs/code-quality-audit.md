# Code quality audit: SSOT, ODR, DRY

Rules: [`docs/code-quality.md`](code-quality.md) (mandatory for existing and new
code). Eight area auditors (math-worldgen, sim-core, sim-modes-events, net-game, server-bots-dir,
web-renderer, web-game-ui, tools-data-docs) reported raw findings; this file merges them, records
what verification removed, and splits the fixes into packages that run in parallel work folders.

Status of every finding: **open**. A package that fixes a finding marks it `fixed (<commit>)` in
the table; a finding the maintainer rejects is marked `won't fix: <reason>` (and, if it is a deliberate
exception, the code gets the one-line comment `code-quality.md` asks for).

The ten open questions are decided (section 5) and folded into the findings, packages and
acceptance criteria below. Decision (Q10): the eight packages run as one wave before milestone 5,
together with package J (remesh performance, section 3), which is not a code-quality finding.

## 1. Summary

| | Count |
|---|---|
| Raw findings from the 8 auditors | 146 |
| Merged away (same issue reported by two or more areas, or one issue split over Rust and TS reports) | 40 |
| Dropped as false or as an allowed exception | 0 (one dropped, then restored as A15 by Q2) |
| **Findings to fix** | **106** |

Raw findings per auditor area: math-worldgen 14, sim-core 21, sim-modes-events 18, net-game 21,
server-bots-dir 21, web-renderer 14, web-game-ui 18, tools-data-docs 19.

Findings by rule and package (package = owner area, see section 4):

| Package | SSOT | ODR | DRY | Total |
|---|---|---|---|---|
| A gen-contracts (Rust to TS, ls-wasm) | 13 | 2 | 0 | 15 |
| B codecs (share code, rule code, Crockford) | 1 | 2 | 0 | 3 |
| C sim-framework (ls-sim structure and test support) | 3 | 4 | 16 | 23 |
| D sim-data (gameplay numbers into `data/`) | 5 | 0 | 2 | 7 |
| E core-helpers (ls-math, ls-worldgen, ls-net, ls-game) | 7 | 6 | 9 | 22 |
| G server-bots-dir | 4 | 3 | 3 | 10 |
| H web-dry (renderer, shaders, UI, gallery) | 0 | 7 | 5 | 12 |
| I tools-docs | 10 | 1 | 3 | 14 |
| **Total** | **43** | **25** | **38** | **106** |

Determinism-relevant findings (touch code under the determinism rules or hashed data): 29. Rows with two rules (A06) count under the first.
Real behaviour bugs found (fixing them changes behaviour; stated in the packages): C01 (client
predictor drifted from `World::step`), H04 (cloak ignores the hull's Z stretch), I01 (e2e default
rotation lacks The Architect), I02 (smoke and e2e rule sets lack `pigment_pickup` and
`paperweight`), I03 (two smoke scripts cannot find a browser the others find). None of them changes
a sim golden hash.

### Verification

Spot-checked against the code with grep/read in `loosescrews.win1` at `fa94f38`: tick constants in TS
and ls-net, `tickHz()` unused outside `wasm.test.ts`, `fxFromMood` "older builds" fallback, duel
fallback (`duel.rs:338-351`, reachable only if the embedded `duel.ron` is missing), `message_name`,
`MAX_NAME_LEN` / `MAX_PLAYER_NAME`, e2e rotation (6 entries vs 7 in `DEFAULT_ROTATION`), missing
`pigment_pickup` / `paperweight` in the tool rule sets, ls-wasm `SURFACE_*` redefinitions with const
asserts, `NO_TEAM` / `NO_PAINT`, worldgen `segment_distance` / `lerp` / `floor_i32` /
`corner_index` copies, `ease` smoothstep copies, `random_dir` / `unit`, `hex`, Echo Cannon param
defaults, `CHARGED_DRAIN_PER_SEC`, `THICKNESS_STEP`, three `event_ids()`, `fn one()`, ls-wasm id
arrays and `num` helpers, Belt Up `% 8`, lagcomp `0.5` fallbacks, edit kind `== 3`, zigzag copies,
golden hash values in `determinism.md`, smoke constants, `decodeShareCode` + `crc8`, TS cue tables,
ship radii literal (12 files), `el()` helper (18 definitions), `quat_rotate` (4), `hash33` (4),
`MODE_NAMES`, `MAX_ENTRY_PLAYERS`, `wellPull`, FNV literal (8 files), `fn setup() -> MatchSetup`
(13 files), `struct Harness` (6 ls-game tests), `findBrowser` (4), `FRAME_UNIFORM_FLOATS` 60 vs 68,
missing `CONTROLS_OWNER` handling in `prediction.rs`, and the cloak vs objects instance transform.
The remaining findings are accepted on the auditors' cited evidence; each package re-checks its
findings before changing code.

Dropped or corrected during verification:
1. **Dropped, then restored as A15:** "Belt Up progress function mirrored in TypeScript"
   (`web/src/game/race/course.ts` mirrors `ls_sim::modes::belt_up::progress`). It is a deliberate,
   commented prediction mirror checked against a Rust fixture, the allowed exception class, so
   verification dropped it. Decision (Q2): ls-wasm exports the function and the TS copy goes, so
   it is finding A15 in package A. The constants half of that report (wedge count, gate count,
   tick rate) is merged into A08/A11.
2. Corrected: the toolchain-pin finding (I14) claimed the master's `CLAUDE.md` still says
   `0.2.126`; both copies say `0.2.129`. The finding stands (pins restated in 4-5 places), the
   drift claim does not.
3. Corrected: "Echo Cannon has two different defaults for one key": `echo_cannon.rs:289` is a test
   reading the value with a dummy default. Covered by D01 anyway.
4. Corrected: `ls-server/src/lobby.rs:147` "inline truncating seconds-to-ticks conversion": the
   line is the fixed constant `600 * TICK_HZ`, not a conversion. `lobby::ticks` remains in C14.
5. Corrected: "`fn one()` three times" is four times (`visual.rs:90` too); "two `num` JSON
   helpers in ls-wasm" is three (`level.rs:120` too); worldgen has a third `corner_index`
   (`chunk.rs:290`, chunk strides, a different formula, left alone).

## 2. Conventions used below

- **Det.** = yes when the fix touches code under the determinism rules (ls-math, ls-worldgen,
  ls-sim) or hashed `data/`. A "yes" refactor must keep every hash unchanged unless the row says
  otherwise.
- **Size**: S (under a day of agent work, a few files), M (one area, many files), L (cross-cutting).
- **Generated TS**: every new generated file is written by a blessed Rust test (same pattern as
  `LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode`, which writes
  `web/src/rules/catalog.ts`) and checked in CI without the bless variable (the test fails on
  a stale file), so `pwsh tools/ci.ps1` is the `--check`.

## 3. Findings

### Package A: gen-contracts (cross-language SSOT: Rust generated or exported into TS)

| Id | Rule | Title | Locations | Fix | Risk | Det. | Size |
|---|---|---|---|---|---|---|---|
| A01 | SSOT | `CueKind` numbers, `gadget_cue` sentinels and the client-only hit cue hand-copied into five TS tables and raw literals | `ls-sim/src/behavior.rs:73-136,170-184`; `ls-wasm/src/mechanics.rs:52`; TS `game/mechanics.ts:17-32`, `game/worldFx.ts:44`, `game/events/spectacleCommon.ts:17`, `game/events/visitorKit.ts:21`, `game/killcam.ts:27` (19 twice), `game/vfx.ts:75,628-630`, `audio/game.ts:304` (253), `gallery/spectacleRun.ts:263,427`, `gallery/vfxScenes.ts:184` | Generate `web/src/generated/cues.ts` (every `CueKind` name/value, `gadget_cue`, client `CUE_HIT` reserved on the Rust side); TS helper `gadgetCue(value)`; delete the five tables and literals | low | no | M |
| A02 | SSOT | Projectile and replicated state bits (`proj_flags`, Paperweight `LANDED`, decoy `RETURNING_BIT`, `CHICKEN`) copied into TS, the landed bit three times, plus a literal `256` in ls-wasm | `ls-sim/src/world.rs:222-248`; `weapons/specials/paperweight.rs:46`; `gadgets/decoy.rs:39`; `ls-wasm/src/lib.rs:753`; TS `game/views.ts:54-60`, `game/mechanics.ts:492`, `game/scene.ts:99,356`, `game/lightFx.ts:33,384`, `game/vfx.ts:72,914` | Generate a `stateBits` table into `web/src/generated/sim.ts`; ls-wasm uses `proj_flags::CHICKEN`; TS imports one name per bit | low | no | S |
| A03 | SSOT | Mode constants (objective kinds, player/view flags, heat phases, per-mode state enums, `SHELL_STEP`, mode ids and display names) mirrored in TS | `ls-sim/src/mode.rs:128-186`; `modes/belt_up.rs:90-98`; TS `game/modeState.ts:10-43,71,241-291`, `game/modeVisuals.ts:62,64,137,139`, `rules/types.ts:55`, `ui/serverList.ts:18-27` (`MODE_NAMES`) | Generate them into `web/src/generated/sim.ts`; add the snake_case id to the catalog's mode entries; `MODE_INFO` and `serverList` take names from the catalog; delete `MODE_NAMES` | low | no | M |
| A04 | SSOT | Button bits defined in ls-sim, again in ls-net (identity map in ls-game) and a third time in TS | `ls-sim/src/input.rs:12-31`; `ls-net/src/wire.rs:721-733`; `ls-game/src/convert.rs:278-287`; `web/src/game/controls.ts:10-21` | One Rust owner (ls-net wire `buttons`, ls-sim `Buttons` consts built from it, or both from ls-math); drop the identity `BUTTON_MAP`; generate TS `BUTTONS` | medium | no | S |
| A05 | SSOT | Entity kind bytes, event wire bytes, mood and phase id tables copied into TS and duplicated inside ls-wasm | `ls-sim/src/events/drone.rs:41-52`, `mimic_pickups.rs:37-44`, `pizza_delivery.rs:35`, `cat_paw.rs:48-49,454` (unnamed phases); `ls-sim/src/director/mod.rs:100-107`; `ls-wasm/src/modes.rs:119,153,155`, `events.rs:43` (`PHASE_IDS` = `MOOD_PHASE_IDS`), `sharecode.rs:68`; TS `game/events/visitorKit.ts:21-31`, `mimicPickups.ts:28-32`, `pizzaDelivery.ts:48`, `catPaw.ts:35`, `spectacleCommon.ts:20` (`PROP_KIND`), `game/modeState.ts:103` | Name the cat paw phases in Rust; add `EventPhase::id()`, heat phase ids, use `Mood::name()` / `Archetype::name()`; ls-wasm derives every list from them; generate the TS constants or resolve `PROP_KIND` via `entityKindNames()` | medium | no | M |
| A06 | SSOT/ODR | Comedy event ids, order and names hand-kept in TS; `event_ids()` defined three times in Rust | `ls-sim/src/events/mod.rs:88-127,640-664`; `ls-sim/src/rulecode.rs:279`; `ls-wasm/src/lib.rs:328-334`; TS `audio/announcer/lines.ts:17-36`, `game/events/info.ts:30-50`, consumers `visitorKit.ts:47`, `spectacles.ts:134`, `gallery/visitorRun.ts:115` | Keep only `ls_sim::events::event_ids()`; TS `EVENT_IDS` and names from the generated catalog (`as const` union); `info.ts` keeps kicker/accent/icon only | low | no | S |
| A07 | SSOT | ls-wasm flat-array strides and lengths hand-copied into TS (10 of 16 unchecked) | `ls-wasm/src/lib.rs:384,514-525,658-662`, `controls.rs:21`, `scene.rs:123`, `level.rs:25,85,87`; TS `wasm/api.ts:6-45`, `gallery/levelScenes.ts:66` | Generate `web/src/generated/layout.ts` from the Rust consts (strides, lengths, field names); `api.ts` field-order comment points to `lib.rs` | low | no | M |
| A08 | SSOT | Tick rate 60 redefined in 7 TS modules and inline `/60`, `*60` although ls-wasm exports `tickHz()` | `ls-math/src/tick.rs:14`; `ls-wasm/src/lib.rs:59-62`; TS `game/mechanics.ts:37`, `game/events/fx.ts:12`, `game/events/visitorKit.ts:27`, `game/race/course.ts:20`, `game/architect/model.ts:30`, `game/matchFlow.ts:13`, `gallery/visitorRun.ts:32`, `gallery/raceSceneRun.ts:257`; inline in `modeVisuals.ts:186`, `killcam.ts:138`, `lightFx.ts:415`, `chargeFx.ts:97`, `spectacleCommon.ts:135`, `vfx.ts:625`, visitor event files | One generated `TICK_HZ` with `ticksToSecs` / `secsToTicks` helpers; all others import it | low | no | M |
| A09 | SSOT | Ship type table (order, names, radii, thrust) hand-copied in 12+ TS places despite the generated `catalog.ts` `shipTypes` | `data/ships/*.ron:6,10`; `ls-sim/src/tuning.rs:40-45`; TS `game/views.ts:126,129`, `ui/startScreen.ts:46-55`, `attract/cinematic.ts:167`, `renderer/models/showcase.ts:34-36`, `renderer/models/index.ts:47-49`, `game/wrecks.ts:17`, `gallery/gallery.ts:226`, `gallery/{eventScenesRun,modeScenes,raceSceneRun,spectacleRun,vfxScenes,visitorRun,worldScenes}.ts` (radius literal), `gallery/vqpScenes.ts:26` (Fusion Charger charge time) | Extend the catalog with `ships: [{id, name, radius, thrustAccel}]` and weapon charge times; derive mesh kinds `ship_${id}`; every site reads it | low | no | M |
| A10 | SSOT | Gravity well force law and `gravity.ron` numbers re-implemented in TS for the HUD | `ls-sim/src/gravity.rs:37,121-145`; `data/world/gravity.ron:16,19`; `web/src/game/worldFx.ts:10,47-53,102-150` | Export `shipPullAt(pos, tick, shipType)` from ls-wasm using `ls_sim::gravity`; delete `wellPull` / `shipPull` and the constants | medium | no | M |
| A11 | SSOT | Worldgen facts (slider names, order, defaults, `BOX_SHAPES`, well count rule, belt ranges, `BELT_WEDGES`, `CHUNK_CELLS`, wall thickness, archetype ids, sky/cave/mine weights) hand-copied into TS and tool scripts | `ls-worldgen/src/sliders.rs:64-133`, `generator.rs:42,49-56,287-290`, `wells.rs:63-79`, `belt.rs:74-119`, `lib.rs:70`; `ls-math/src/level.rs:29,108-110`; TS `ui/workshop.ts:29-153`, `game/architect/model.ts:33`, `renderer/atmosphere.ts:29-37,48,113-126,211`, `renderer/terrainBlend.ts:40`, `renderer/sunShadow.ts:27`, `gallery/levelScenes.ts:15`, `gallery/scenes.ts:36`, `game/race/course.ts:14-18`; tools `browser-smoke/workshop.mjs:23`, `moods.mjs:25`, `race.mjs:39` | Generate `web/src/generated/world.ts` from ls-worldgen/ls-math (blessed by an ls-worldgen test) and export `sliderNames()` / `defaultSliders()` from ls-wasm for the tool scripts; one `SliderKey` type; fold `WorldSliders` into `SliderValues`; `SHADOW_REGION_SNAP = CHUNK_CELLS`; course.ts derives `WEDGE` and `LAP_GATES` | low | no | M |
| A12 | SSOT | Terrain id tables (`NO_TEAM`/`NO_PAINT`, `SURFACE_*`, `MATERIAL_*`, `PAINT_*`) hand-copied into the renderer | `ls-math/src/terrain.rs:50-76,100-115`; `web/src/renderer/mesh.ts:16-34` | Generate them into `web/src/generated/world.ts` after E07 settles the Rust owners | low | no | S |
| A13 | SSOT | Event and mode tuning numbers from `data/` copied into TS, plus a dead TS decoding fallback for "older" cores | `data/events/{director,tired_announcer,big_head_day,cat_paw,duck_parade,hat_rain,bored_level}.ron`, `data/rulesets/{belt_up,the_architect}.ron`; TS `game/events/fx.ts:4,12-14,100-127`, `game/killcam.ts:31`, `game/events/{bigHeadDay.ts:23,catPaw.ts:33,duckParade.ts:41,hatRain.ts:36,boredLevel.ts:45,info.ts:33}`, `game/race/raceFx.ts:49`, `ui/raceHud.ts:40`, `game/architect/model.ts:60-161,216-234`, `gallery/{eventCard.ts:13-24,eventScenesRun.ts:33-43,moodScenes.ts:38-39,spectacleRun.ts:127-130}`; `ls-wasm/src/events.rs:67-68` | `Game.eventFx()` carries the event's params map and announce end tick; `architectRules()` is required (no TS defaults); event phase lengths and client-relevant params go into the generated catalog for the gallery; delete `fxFromMood`, `unpackDir`, `unpackPos` | low | no | M |
| A14 | ODR | Three JSON number formatters in ls-wasm with different rounding | `ls-wasm/src/modes.rs:50-69`, `events.rs:30-36`, `level.rs:120` | One `json` module in ls-wasm (`num`, `opt`, `vec3`, `object`, `array`, `json_str`) used by all; choose one rounding and check the TS tests | low | no | S |
| A15 | ODR | Belt Up race progress function mirrored in TS (gates passed, then the position key in the sector), checked against a Rust fixture | `ls-sim/src/modes/belt_up.rs` (`progress`); `ls-sim/tests/race_progress.rs`, `tests/fixtures/race_progress.txt`; TS `game/race/course.ts:3-7,173-200`, `course.test.ts` | Decision (Q2): ls-wasm exports race progress from `ls_sim::modes::belt_up::progress`; `course.ts` calls it and keeps only the HUD maths built on it; delete the TS copy and its fixture comparison in `course.test.ts` (the Rust fixture test stays) | low | no | S |

### Package B: codecs (one codec per format)

| Id | Rule | Title | Locations | Fix | Risk | Det. | Size |
|---|---|---|---|---|---|---|---|
| B01 | SSOT | Second share-code decoder in TS (all bit layouts, CRC-8, base-101 sliders) although ls-wasm exports `checkShareCode` | `ls-worldgen/src/sharecode.rs:146-270`; `ls-wasm/src/sharecode.rs:28`, `lib.rs:88-114`; TS `renderer/atmosphere.ts:47-122`, users `attract/worlds.ts:19`, `game/architect/index.ts:134`, `renderer/atmosphere.ts:256`; `docs/architecture/contracts.md:349-351` | Extend ls-wasm `ShareCodeInfo` with seed and sliders; `decodeShareCode` becomes a thin wrapper or callers use ls-wasm directly; atmosphere tests use Rust-generated fixture codes; drop the "mirrors both layouts" sentence | medium | no | M |
| B02 | ODR | Rule set code format and `RuleSet::resolve_with` validation re-implemented in TS (about 1000 lines, "bit-exact mirror") | `ls-sim/src/rulecode.rs` (152-170 constants), `ls-sim/src/ruleset.rs` (`resolve_with`); TS `rules/code.ts` (whole file, 25-41, `MAX_NAME` 36), `rules/validate.ts` | Export `encodeRuleSet`, `decodeRuleCode`, `validateRuleSet` from ls-wasm; `code.ts` / `validate.ts` become thin wrappers over ls-wasm (decision Q1: the rule set editor requires the WASM core); keep the catalog vectors as tests | medium | no | L |
| B03 | ODR | Crockford alphabet, look-alike leniency (O to 0, I/L to 1) and bit packers defined in three Rust crates and three TS modules | `ls-sim/src/rulecode.rs:164,180-228,1090-1115`; `ls-worldgen/src/sharecode.rs:41,257-268`; `ls-server/src/rooms.rs:30` (+ `code_from_bits`, `parse_code`); TS `rules/code.ts:37,763`, `ui/shareCode.ts:11-60`, `renderer/atmosphere.ts:50` | `ls_math::code` (`ALPHABET`, `symbol_value`, `BitWriter`, `BitReader`) used by all three Rust codecs; one `web/src/util/crockford.ts`; outputs byte-identical (existing vector tests guard it) | low | yes | M |

### Package C: sim-framework (ls-sim structure, shared hooks, sim test support)

| Id | Rule | Title | Locations | Fix | Risk | Det. | Size |
|---|---|---|---|---|---|---|---|
| C01 | DRY | Ship movement pipeline written three times (`World::step`, `preview_movement`, client `Predictor::step_ship`); the predictor has already drifted (no `CONTROLS_OWNER` thrust zeroing, `limits.apply` once, no empty-boulder filter) | `ls-sim/src/world.rs:1336-1440`; `ls-sim/src/world/preview.rs:22-127`; `ls-game/src/client/prediction.rs:581-663` | One `ship::move_ship(ship, cmd, tuning, limits, &MoveCtx, terrain) -> MoveReport` called by all three; same op order. **Behaviour fix:** the client predictor gains the missing steps (client prediction only; sim hashes unchanged) | high | yes | L |
| C02 | DRY | Detonation (Impact event, direct hit, data splash, crater near rock) copied outside `Ctx::detonate`; `Command::Splash` built by hand six times | `ls-sim/src/behavior.rs:615-635,860-893`; `weapons/ordnance/prox_mine.rs:41-81`, `stalker_mine.rs:42-75`, `tectonic_charge.rs:255-275`; `world.rs:2888-2915` | `Ctx::detonate_with(.., DetonateOpts)`; `Ctx::splash` gains `dir`; shared splash-command fn for `boulder_absorb`; event and command order unchanged | medium | yes | M |
| C03 | DRY | Def/table loading duplicated (`WeaponTable` vs `GadgetTable`, eight `ron::from_str` + `SimError::Parse` sites, four `fn one()`), data test hard-codes file counts | `ls-sim/src/weapon.rs:168-408`; `gadgets/mod.rs:77-210`; `events/mod.rs:198-240`; `director/mod.rs:206`; `tuning.rs:188`; `ruleset.rs:153,678,1142`; `visual.rs:90`; `data.rs:118-127` | `data::parse_ron<T>`, `check_snake_id`, generic `IdTable`, `ParamDef` trait, one `serde_defaults::one`; data test computes counts from the registries | low | no | M |
| C04 | DRY | Five world-settings loaders repeat OnceLock + parse + expect, and `WORLD_FILES` lists the paths again | `ls-sim/src/gravity.rs:65-70`, `instability.rs:97-103`, `obstacle.rs:194-200`, `pigment.rs:70-76`, `radiation.rs:61-67`, `data.rs:58-64` | One `world_settings!` macro (or generic `embedded<T>`) that also yields the `WORLD_FILES` entry | low | no | S |
| C05 | SSOT | Weapon and gadget registry entries repeat the id five times and the family twice | `ls-sim/src/weapons/registry.rs:26-222`; 46 `fn id()` in weapon/gadget modules; `gadgets/mod.rs` `MODULES`; `weapons/tests.rs:431-434` | `weapon_module!` / `gadget_module!` macro deriving file and `include_str!` from the id; drop `family` from the entry; `id()` from the registry | low | no | S |
| C06 | ODR | Mode registry split over parallel match tables in three files, plus seven identical rules extractors | `ls-sim/src/mode.rs:33-112`; `modes/mod.rs:411-461`; `ruleset.rs:508-532`; extractors `belt_up.rs:311`, `heist.rs:119`, `hot_reactor.rs:82`, `the_architect.rs:448`, `the_squeeze.rs:341`, `last_color_standing.rs:51`, `team_brawl.rs:56` | One `MODES` table macro (kind, id, name, rule set file, module, settings accessor) like events' `entry!` | low | no | M |
| C07 | DRY | Drone visitor events copy flight, outcome cue, spawn, shot-down/leaving and fly/pose blocks | `ls-sim/src/events/tax_drone.rs:40-170`, `pizza_delivery.rs:41-180`, `customer_survey.rs:59-160`, `mimic_pickups.rs:55-232`, `drone.rs` | `Flight::from_def`, `outcome_cue`, `spawn_drone`, `fly_home`, `fly_to` in `events/drone.rs` | medium | yes | M |
| C08 | DRY | Spectacle events re-implement `drone::remove_props` | `ls-sim/src/events/drone.rs:484-500`; `cat_paw.rs:390,491-503`; `duck_parade.rs:273,295-307`; `disco_inferno.rs:124,139-151`; `hat_rain.rs:181,286-296` | Move `props` / `first_prop` / `remove_props` to `events/mod.rs`; all events call them | low | no | S |
| C09 | DRY | Smoothstep written three times in events | `ls-sim/src/events/cat_paw.rs:99-102`, `disco_inferno.rs:31-34`, `bored_level.rs:43` | `ls_math::det::smoothstep01`; identical formula | low | yes | S |
| C10 | DRY | Two cone-lock target searches (Seeker `acquire` vs `Ctx::target_in_cone`), a third manual nearest-ship loop in Decoy | `ls-sim/src/weapons/ordnance/seeker.rs:42-89`; `behavior.rs:474-525`; `gadgets/decoy.rs:127-145` | `Ctx::acquire(origin, dir, range, half_angle, filter, include_decoys)`; `target_in_cone` wraps it with the same iteration order. Decision (Q9): decoys keep fooling only the Seeker; the Leech Arc and Polite Laser locks stay as they are | medium | yes | S |
| C11 | ODR | Client shot/beam prediction selects weapons by id strings in ls-wasm and repeats the modules' param defaults and the Leech Arc lock predicate | `ls-wasm/src/shots.rs:108-119,313-318`; `ls-wasm/src/beam.rs:33-120`; `ls-sim/src/weapons/energy/{corkscrew.rs:40,66,100,scatter_laser.rs:50,leech_arc.rs:55-102}`, `specials/borrowed_time.rs:41` | Typed `Params::of(def)` per module shared by sim and ls-wasm; `WeaponBehavior::client_shot_model()` hook; pure Leech Arc cone/keep predicate used by `lock()` and `aim_beam` | medium | yes | M |
| C12 | SSOT | Railspike copies `Ctx::thickness`'s private sampling step | `ls-sim/src/behavior.rs:533-536`; `weapons/kinetic/railspike.rs:28,80` | `pub const THICKNESS_STEP` in behavior.rs, imported by Railspike | low | no | S |
| C13 | DRY | Projectile and entity wire-id allocators repeat the free-id search | `ls-sim/src/world.rs:3396-3445` | `alloc_wire_id(counter, mask, live)`; projectile fast path kept in front | low | yes | S |
| C14 | ODR | Several seconds-to-ticks helpers next to `Tick::ticks_for_secs` | `ls-math/src/tick.rs:68-83`; `ls-sim/src/modes/mod.rs:697-699` (37 uses); `director/tests.rs:89`, `modes/hot_reactor/tests.rs:67`; `ls-server/src/lobby.rs:81-83` | Add `Tick::ticks_for_duration`; delete `modes::ticks`, the test `secs` wrappers and `lobby::ticks` (lobby keeps its rounding: check the warmup tests) | low | no | S |
| C15 | DRY | Lag compensation repeats the projectile-vs-ship hit radius and hard-codes a 0.5 fallback radius | `ls-sim/src/world.rs:2193`; `ls-game/src/server/lagcomp.rs:158-216` | `ls_sim::ship_hit_radius(..)` used by both; drop the fallbacks | low | yes | S |
| C16 | DRY | Client session builds the static well field by hand "as World::new builds them" | `ls-game/src/client/session.rs:201-206`; `ls-sim/src/world.rs:634-638` | `WellField::for_setup(&MatchSetup)` used by both | low | yes | S |
| C17 | DRY | Client re-implements the `radiation_scale` mode hook dispatch | `ls-game/src/client/session.rs:385-397`; `ls-sim/src/modes/belt_up.rs:922-927`, `modes/mod.rs:351` | Pure `modes::radiation_scale_for(rules, race)` called by the hook and the client | low | no | S |
| C18 | DRY | Belt Up next-gate formula repeated, once with a literal 8 in ls-wasm | `ls-sim/src/modes/belt_up.rs:279-283,818-819`; `ls-wasm/src/modes.rs:124-128`; `ls-net/src/mode.rs:144`; `docs/architecture/netcode.md:358` | `belt_up.rs:819` calls `next_gate()`; ls-wasm converts via `race_from_net`; docs name `RacerView::next_gate` | low | yes | S |
| C19 | SSOT | Instagib / Paint the Void weapon id literals repeated | `ls-sim/src/mutators/instagib.rs:29-31`; `ruleset.rs:726`; `ls-server/src/main.rs:341` | Public id consts per weapon module (from C05's macro), referenced by all three | low | no | S |
| C20 | DRY | Sim test fixtures (two-spawn setup, `world_with`, `place`, `equip`, `kill`, `hit`, `step`, force-event helpers, pickup clearing) copied into about 20 test modules | `ls-sim/src/tests.rs:21-60,694-696`; `tests_arsenal.rs:14-106,508-552`; `tests_world.rs:37-60,682-684`; `gadgets/test_util.rs`; `weapons/energy/scrap_laser.rs:88-210`; `weapons/{kinetic/rivet_gun,ordnance/heavy_rocket,specials/paint_thrower,tests}.rs`; `director/tests.rs:18-93`; `events/{world_events_tests.rs:25-107,spectacle_tests.rs:24-95,visitor_tests.rs:26-128}`; `modes/{tests.rs,hot_reactor/tests.rs,last_color_standing/tests.rs,heist/tests.rs,the_architect/tests.rs,belt_up/tests.rs}`; `mutators/instagib.rs:65-67,136` | One `#[cfg(test)] ls_sim::test_util` (fold `gadgets/test_util.rs` and the Scrap Laser helpers in); `RuleSet::clear_pickups()` also used by Instagib | low | no | M |
| C21 | DRY | Replay harnesses duplicated (FNV fold literal in 8 files, roster loop, built-in rule set lookup, fixture bless/rewrite) | `ls-sim/src/events/replays.rs:60-221`; `events/visitor_replays.rs:56-197`; `event_replays.rs:95-127`; `golden.rs:227-237`; `world_replays.rs:202-212`; `modes/belt_up/replay.rs:160-244`; `modes/the_architect/replay.rs:154-155`; `hash.rs:5-6` | `replay_util`: `HashFold` on `hash.rs` constants, `add_roster`, `builtin_rules`, `bless_or_check_fixture`; RNG streams and step order unchanged | medium | yes | M |
| C22 | DRY | Scripted "aim at nearest enemy" input generator copied across replays | `ls-sim/src/golden.rs:154-217`; `modes/team_brawl.rs:224-282`; `modes/last_color_standing/tests.rs:434-498`; `modes/hot_reactor/tests.rs:552-575`; `modes/the_squeeze/tests.rs:560-575`; `modes/the_architect/replay.rs:126`; `tests.rs:761-830`; `event_replays.rs:205`; `world_replays.rs:283` | `golden::aim` pub(crate) and `golden::scripted_inputs(rng, w, ids, tick, &InputProfile)`; each replay passes its current profile so draws stay identical | medium | yes | M |
| C23 | ODR | Two per-mode replay golden systems with two bless procedures (hand-edited module constants and `mode_replays.txt`) | `ls-sim/src/modes/*/tests.rs` (`*_REPLAY_GOLDEN*`); `ls-sim/src/golden.rs`; `ls-sim/tests/fixtures/mode_replays.txt`; `docs/architecture/determinism.md:136-146`; `tools/determinism-check.mjs` | Register the tailored module replays as named entries in the fixture, blessed by `LS_BLESS_MODE_REPLAYS` and cross-checked by determinism-check; delete the constants (decision Q5: one bless mechanism) | medium | yes | M |

### Package D: sim-data (gameplay numbers live only in `data/`)

| Id | Rule | Title | Locations | Fix | Risk | Det. | Size |
|---|---|---|---|---|---|---|---|
| D01 | SSOT | Every weapon, gadget and event param has a second default in code (`param(key, default)`, about 270 call sites; 240 weapon/gadget defaults equal the data value, 14 differ but never apply); `CHARGED_DRAIN_PER_SEC` repeats `leech_arc.ron`; ls-wasm repeats Bored Level's defaults | `ls-sim/src/weapon.rs:273`; `gadgets/mod.rs:122`; `events/mod.rs:233`; all `weapons/**`, `gadgets/*`, `events/*.rs` param calls (e.g. `echo_cannon.rs:43,51`, `seed_grenade.rs:94,156`, `tax_drone.rs:42-146`, `cat_paw.rs:128-461`); `world.rs:92,648-653`; `ls-wasm/src/events.rs:67-68` | Modules declare required keys; `validate` rejects defs missing one; `param(key)` without default plus `param_ticks(key)`; repeated reads into a typed params struct; `BoredLevel::turn(def)` for ls-wasm. Built-in data unchanged, so replay hashes unchanged. Decision (Q8): the charged-surface drain without a Leech Arc (today `CHARGED_DRAIN_PER_SEC`, 5/s) moves into a `data/world/` settings file read through C04's loader, same value; the data hash changes (accepted, Q3) | medium | yes | L |
| D02 | SSOT | Gameplay numbers kept as Rust constants: debris physics, pickup radius, wreck cap, knockback, paint contact margins, material crater scales, `DEFAULT_SECS` / `DEFAULT_TICKS`, monochrome palette (also in TS) | `ls-sim/src/entity.rs:32-43`; `world.rs:81,83,1446`; `weapon.rs:295`; `weapons/mod.rs:342`; `ship.rs:436-439`; `pigment.rs:41`; `ls-math/src/terrain.rs:58-64,119-121` (users `ls-sim/src/behavior.rs:650-672`); `web/src/renderer/ink.ts:28` | `data/world/debris.ron`, `data/world/contact.ron`, `data/world/materials.ron` through C04's loader; palette into `pigment.ron` and exported to TS; contract caps (`MAX_DEBRIS` if it is a wire limit) stay with a comment; `DEFAULT_SECS` / `DEFAULT_TICKS` replaced by a test helper with explicit ticks | medium | yes | M |
| D03 | SSOT | Mode and event tuning consts in mode/event modules (Heist shed, Squeeze radius fractions and scrap, Team Brawl kill target scaling, Architect stroke timings, drone/hat/mimic consts) restated in docs | `ls-sim/src/modes/heist.rs:56-80`; `the_squeeze.rs:84-99`; `team_brawl.rs:30-38`; `belt_up.rs:105-108`; `the_architect.rs:93-118`; `modes/mod.rs:71`; `events/drone.rs:55-70`; `hat_rain.rs:53-55`; `mimic_pickups.rs:53`; `ruleset.rs:263-264`; `docs/milestone-3-plan.md:74` | `data/modes/<id>.ron` (embedded, hashed) read through a `ModeTuning::builtin()`. Decision (Q4): today's host-editable `mode_settings` stay as they are and every other mode tuning number moves into `data/modes/`; no new `mode_settings` fields, so the rule-code format is unchanged; event consts into their event params; docs name the file | medium | yes | L |
| D04 | DRY | Built-in rule sets repeat the full arsenal availability and pickup tables (8+ files, about 30-80 lines each) | `data/rulesets/paint_the_void.ron:13-97`; `duel`, `team_brawl`, `heist`, `hot_reactor`, `last_color_standing`, `squeeze`, `the_architect`, `belt_up`, `sandbox_all_weapons` `.ron` | `base:` (or shared `data/rulesets/arsenal.ron`) merged by the loader; files state overrides only; resolved `RuleSet`s identical, so rule codes and `REPLAY_GOLDEN_HASH` hold; the data hash changes (accepted, Q3); re-bless the rules catalog and reference pages | medium | yes | M |
| D05 | SSOT | Mode setting defaults exist in `mode_settings!` literals, in the rule set files, in RON header comments and in TS (Architect) | `ls-sim/src/ruleset.rs:195-532,718-728`; `modes/hot_reactor/tests.rs:121`; mode fallbacks `heist.rs:122`, `hot_reactor.rs:85`, `belt_up.rs:314`; `data/rulesets/*.ron` mode blocks and headers (e.g. `hot_reactor.ron:1-21`); TS side covered by A13 | Rule set file is the only source; macro keeps steps and bits (code contract) but no defaults; `ModeRules::default_for(kind)` = `RuleSet::default_for(kind).mode`; serde fills from it; drop numeric header comments | medium | yes | M |
| D06 | DRY | Ship files repeat about 30 identical fields; game-wide paint constants live per ship | `data/ships/{skitter,tinker,hauler,tug}.ron:10-50` | `data/ships/common.ron` (or `base` merge) plus per-type overrides; paint bounce/sticky into a world data file; resolved tuning identical; data hash changes (accepted, Q3) | low | yes | M |
| D07 | SSOT | Per-mode minimum and maximum players are rule facts hard-coded in ls-server by rule set id | `ls-server/src/playlist.rs:60-80`; `userDocs/hosting-guide.md:118` | Optional `min_players` / `max_players` in the rule set RON; `DEFAULT_ROTATION` keeps ids only; docs point to the data; data hash changes (accepted, Q3) | medium | yes | M |

### Package E: core-helpers (ls-math, ls-worldgen, ls-net, ls-game production code and their tests)

| Id | Rule | Title | Locations | Fix | Risk | Det. | Size |
|---|---|---|---|---|---|---|---|
| E01 | DRY | Closest point on segment / segment distance written five times | `ls-worldgen/src/belt.rs:239-249`, `wells.rs:82-92`, `network.rs:141-147,570-578`; `ls-sim/src/events/drone.rs:376-384` | `Vec3::closest_on_segment`, `segment_param`, `segment_distance` in ls-math; same op order | low | yes | S |
| E02 | DRY | `lerp` redefined three times and the range-lerp spelled inline seven times | `ls-math/src/det.rs:128`; `ls-worldgen/src/mesh.rs:1508`, `noise.rs:85`, `belt.rs:223-225,357,452,552-556`, `wells.rs:79,197` | Use `det::lerp`; add `det::lerp_range` | low | yes | S |
| E03 | DRY | Central-difference SDF gradient written six times | `ls-math/src/terrain.rs:330-338`; `ls-sim/src/terrain.rs:578-584`; `ls-worldgen/src/mesh.rs:1367-1375,2145-2151`; `ls-worldgen/tests/mesh_quality.rs:41-49,189-193` | `ls_math::terrain::central_difference(f, p, e)` (raw vector, same op order) | low | yes | S |
| E04 | ODR | Two `floor_i32` and two halo `corner_index` in ls-worldgen | `ls-worldgen/src/generator.rs:1636-1643`, `mesh.rs:1483-1505`, `grid.rs:86-90` | mesh.rs uses `generator::floor_i32` (or move to `det`) and grid.rs's halo index; check range equality (the generator version saturates) | low | yes | S |
| E05 | DRY | Uniform random unit vector implemented four times, two byte-identical in ls-sim | `ls-sim/src/weapons/ordnance/tectonic_charge.rs:72-78`, `weapons/specials/seed_grenade.rs:61-67`; `ls-worldgen/src/belt.rs:251-264`; `ls-bots/src/steering.rs:241-253`; `ls-math/src/rng.rs:126-171` | `Pcg32::unit_vector()`, `SplitMix64::rotation()` / `unit_vector()` with the existing algorithms; bots switch to `Pcg32::unit_vector` | low | yes | S |
| E06 | DRY | Lowercase hex encoder written three times | `ls-worldgen/src/golden.rs:41-49`; `ls-server/src/endpoint.rs:74-76`; `ls-directory/src/lib.rs:52` | One `hex()` in a low-level crate every user depends on | low | no | S |
| E07 | ODR | Terrain ids defined twice in Rust: material enum literals, ls-wasm surface bits with const asserts, unpainted paint id as `NO_TEAM` (worldgen) and `NO_PAINT` (sim), plus an unrelated entity `NO_TEAM` | `ls-math/src/terrain.rs:50-54,100-115`; `ls-worldgen/src/material.rs:44-70`, `lib.rs:74`; `ls-wasm/src/game.rs:2375-2379,2423-2426`; `ls-sim/src/terrain.rs:12,25`; `ls-sim/src/entity.rs:45` | Enum discriminants from the ls-math consts; ls-wasm uses `SurfaceStates::*.bits()`; `NO_PAINT` once in `ls_math::terrain`; entity constant renamed `NO_ENTITY_TEAM` (or documented as separate) | low | yes | S |
| E08 | SSOT | World slider index order defined in ls-worldgen (three times) and again in ls-sim; `slider >= 7` literal in ls-game | `ls-worldgen/src/sliders.rs:89-133`; `ls-sim/src/terrain.rs:123-133`; `ls-game/src/terrain.rs:192` | `SLIDER_COUNT`, index consts and `SLIDER_NAMES` in ls-math, used by all three | low | yes | S |
| E09 | SSOT | Tick rate redefined in Rust: ls-net `TICKS_PER_SECOND`, server tests `/ 60` | `ls-math/src/tick.rs:14`; `ls-net/src/quant.rs:37-38,157`, `lib.rs:47`; `ls-server/tests/websocket.rs:32`, `tests/webtransport.rs:30` | Use `ls_math::TICK_HZ as f64` (same value); server tests use `timing::TICK_PERIOD` | low | no | S |
| E10 | SSOT | Well wire grid (0.25 u / 0.125 u/s^2 and clamps) copied into Paperweight and the Architect well | `ls-net/src/wire.rs:125-148`; `ls-sim/src/weapons/specials/paperweight.rs:48-68`; `ls-sim/src/world/architect.rs:161-163` | Grid and `quantize_range` / `quantize_peak` in ls-math, re-exported by ls-net; check clamp-then-round order keeps `REPLAY_GOLDEN_HASH` | medium | yes | S |
| E11 | SSOT | Mode wire constants (player flags, heat and mood phases, `NONE`, unlimited lives) defined in ls-sim and ls-net, only two cross-checked | `ls-net/src/mode.rs:97-128,171-177`; `ls-sim/src/mode.rs:25-31,155-172`; `ls-sim/src/modes/belt_up.rs:90-99`; `ls-game/src/server/tests.rs:1153-1163` | `ls_math::mode_wire` contract module re-exported by both (or const asserts for every pair in ls-game) | medium | no | M |
| E12 | SSOT | Team, ship and objective limits defined per crate | `ls-sim/src/mode.rs:21-25`; `ls-net/src/mode.rs:93-95`; `ls-net/src/wire.rs:24,42-43`; `ls-game/src/setup.rs:208-211`; `ls-server/src/playlist.rs:86` | One definition each; `MAX_ENTRY_PLAYERS = ls_net::MAX_SHIPS`; const asserts against wire field widths | low | no | S |
| E13 | ODR | Two constants for the 32-byte player name limit, two truncation algorithms, the number in an error string and in TS | `ls-net/src/message.rs:68,944-946`; `ls-net/src/wire.rs:332`; `ls-game/src/convert.rs:88-92`; `ls-game/src/mode.rs:212-214,268-276`; `web/src/ui/startScreen.ts:36` | Keep `MAX_PLAYER_NAME`; `ls_net::clamp_player_name` (char-boundary cut) used by both conversions; message from the constant; TS limit generated (A-file) | low | no | S |
| E14 | ODR | Wire "none" id 255 defined several times and written as raw `u8::MAX` | `ls-net/src/mode.rs:97-99`; `ls-game/src/convert.rs:28-30,97,330`; `ls-game/src/server/mod.rs:1094-1096`; TS `game/mechanics.ts:35`, `spectacleCommon.ts:23`, `visitorKit.ts:25`, `killcam.ts:39` | `ls_net::NONE_ID` with `wire_opt` / `from_wire_opt`; TS value generated | low | no | S |
| E15 | DRY | Ship-id presence mask and sorted-id validation written six to ten times in ls-net encoders | `ls-net/src/snapshot.rs:247-291,1396-1420,1643,1722-1723,1894-1902`; `ls-net/src/mode.rs:517-549,606-632,813,857-858,923-927` | `write_id_mask`, `read_id_mask`, `check_sorted_ids`, `check_sorted_unique`; wire bytes identical (round-trip tests) | low | no | M |
| E16 | ODR | Zigzag and raw Vec3 bit helpers defined in several ls-net files | `ls-net/src/bits.rs:275-283`; `snapshot.rs:317-324`; `mode.rs:491-508`; `wire.rs:899-911`; `message.rs:394-407,667` | zigzag16/32/64 and `write_vec3_f32` / `read_vec3_f32` in bits.rs only | low | no | S |
| E17 | SSOT | Terrain edit kind codes 0-3 used as magic numbers on both sides of the crate boundary (one feeds the desync state hash) | `ls-net/src/wire.rs:464-569`; `ls-game/src/terrain.rs:536` | `wire::edit_kind` consts or `TerrainEditBits::is_surface()` used everywhere | low | no | S |
| E18 | DRY | Two snapshot-frame buffers with the same bracket-and-lerp logic in the client | `ls-game/src/client/interp.rs:18-100`; `ls-game/src/client/session.rs:124,145,468-490,873-877` | Generic `TickFrames<T>` used by `InterpolationBuffer` and the entity history | low | no | S |
| E19 | SSOT | `QuantConfig::default` hard-codes the level box plus a margin owned by ls-game; doc comment restates weapon speeds | `ls-net/src/quant.rs:56-68`; `ls-game/src/convert.rs:25-44` | `QUANT_MARGIN` moves to ls-net; default bounds from a level-size constant; comment names the data files | low | no | S |
| E20 | ODR | `message_name` wrapper duplicates `Message::name` | `ls-game/src/server/mod.rs:1150-1152`; `ls-net/src/message.rs:421-440` | Delete the wrapper | low | no | S |
| E21 | DRY | Worldgen benchmarks and tests repeat surface-point ray fans, busiest-chunk search and best/median timing | `ls-worldgen/examples/{worldgen_bench.rs:30-56,mesh_bench.rs:27-55,mesh_timing.rs:30-58}`; `ls-worldgen/tests/timing.rs:15-60`; `ls-worldgen/src/chunk.rs:1842-1863` | `ls_worldgen::test_util` behind `cfg(any(test, feature = "test-util"))` | low | no | S |
| E22 | DRY | Netplay test harness (`Delayed`, `SimClient`, `Harness`, `lossy`) copied into seven ls-game integration tests | `ls-game/tests/{netplay,mood_netplay,spectacle_netplay,visitor_netplay,world_events_netplay,belt_up,transition}.rs` | `ls-game/tests/common/mod.rs` with one generic harness | low | no | M |

### Package G: server-bots-dir

| Id | Rule | Title | Locations | Fix | Risk | Det. | Size |
|---|---|---|---|---|---|---|---|
| G01 | DRY | Two hand-rolled HTTP/1.1 servers with copy-pasted request parsing, head/body readers, reason phrases (already diverged), CORS strings and body limit | `ls-server/src/http.rs:38-575`, `api.rs:44,59-80`; `ls-directory/src/http.rs:36-426` | Shared HTTP core in `ls_directory::http_core` (ls-server already depends on ls-directory); both keep only routing | medium | no | M |
| G02 | DRY | Three hand-rolled HTTP clients (directory client, bot social, bot cert-hash fetch); only one honours Content-Length | `ls-directory/src/client.rs:170-240`; `ls-bots/src/social.rs:118-176`; `ls-bots/src/transport.rs:353-384` | `ls_directory::client::http_request(..)` used by all | low | no | S |
| G03 | ODR | Lobby API DTOs defined in ls-server, rebuilt as untyped JSON in bots and by hand in TS | `ls-server/src/api.rs:137-235`; `ls-bots/src/social.rs:181-196`; `web/src/net/social.ts:29-80,146-174` | DTOs in `ls_directory::api`, used with serde by server and bots; optional TS generation with a CI check | low | no | S |
| G04 | ODR | WebTransport stream framing (length prefix, frame cap, `/wt` path) implemented three times | `ls-server/src/wt.rs:50-150`; `ls-bots/src/transport.rs:54-59,237-253,290-293,398`; `web/src/net/webtransport.ts:22-28,84-120` | Framing and constants in ls-net used by server and bots; constants exported to TS (generated or ls-wasm) | medium | no | M |
| G05 | SSOT | `/cert-hash` JSON written with `format!` and parsed by a hand-written scanner in bots | `ls-server/src/endpoint.rs:74-83`; `ls-bots/src/transport.rs:386-419`; `web/src/net/webtransport.ts:6` | One serde `CertHashInfo` in a shared crate; drop `json_field` | low | no | S |
| G06 | SSOT | Secret and name limits re-typed in the duel code next to ls-directory's constants | `ls-server/src/duel.rs:44,163`; `ls-directory/src/api.rs:12,22-24` | Duel uses `MIN_SECRET`, `MAX_SECRET`, `MAX_NAME` and a `validate_token` helper from ls-directory | low | no | S |
| G07 | SSOT | CLI defaults duplicated against config constants (`default_value_t` literals, directory `Args` copy of `Config::default`) | `ls-server/src/main.rs:96-125`, `lobby.rs:35,37`, `sim.rs:64`, `rooms.rs:25`, `server.rs:121`; `ls-game/src/server/mod.rs:64`; `ls-directory/src/main.rs:27-45`, `registry.rs:37-47` | clap defaults read the constants; add `DEFAULT_STATS_INTERVAL`; doc tables in I09 | low | no | S |
| G08 | SSOT | Dead duel fallback restates `duel.ron` (10 kills, 300 s) | `ls-server/src/duel.rs:48-50,338-351,511-517`; `data/rulesets/duel.ron:5-12` | Drop the constants and fallback; `builtin_rule_set("duel").expect(..)` covered by a test; test reads the values from the rule set | low | no | S |
| G09 | ODR | Bot arsenal re-lists every weapon id (`Role`) and hard-codes per-weapon distance bands and data values in comments | `ls-bots/src/arsenal.rs:49-80,236-290` | `data/bots/arsenal.ron` keyed by weapon id (not part of the hashed sim data), loaded by ls-bots; splash-safe distance from `WeaponDef.splash` | medium | no | M |
| G10 | DRY | Bot race line re-implements the segment free-space test | `ls-bots/src/race_line.rs:530-534`; `ls-bots/src/nav.rs:1083-1150` | `dive_clear` calls `nav::segment_clear` with a named margin | low | no | S |

### Package H: web-dry (renderer, shaders, UI, gallery)

| Id | Rule | Title | Locations | Fix | Risk | Det. | Size |
|---|---|---|---|---|---|---|---|
| H01 | ODR | WGSL hash, value noise, Voronoi, `ign`, `PI` / `TAU` pasted into several shaders; `sky.ts` has a different `hash33` under the same name | `renderer/shaders/belt.ts:60-103,629`; `modeFx.ts:77-133`; `wells.ts:83-134`; `post.ts:191-202`; `volumetric.ts:48`; `sky.ts:43-52`; `terrain.ts` | `shaders/noise.ts` (`HASH_WGSL`, `VALUE_NOISE_WGSL`) interpolated like `FRAME_WGSL`; rename sky's to `pcg_hash33`; test that each shader contains each routine once | low | no | M |
| H02 | ODR | Fullscreen-triangle vertex shader (10 copies, twice in `volumetric.ts`) and the depth binding type selector (8 copies) | `renderer/shaders/{belt.ts:112-121,615,642,ink.ts:55,77-78,modeFx.ts:40,69-70,post.ts:15-16,sky.ts:32-33,wells.ts:42,76-77,volumetric.ts:16,39-40,106-107,particles.ts:54,332-333,shadow.ts:162-163,cloak.ts:24}` | `shaders/fullscreen.ts`: `FULLSCREEN_VS_WGSL`, `fullscreen_ndc`, `sceneDepthType(ms)` | low | no | M |
| H03 | DRY | `linear_depth` (4 copies) and `fog_at` (3 copies) differ only in uniform field names | `renderer/shaders/belt.ts:152-162,465,657`; `modeFx.ts:170-180,206`; `wells.ts:137-155`; `particles.ts:286-295`; `cloak.ts:79` | Pure `linear_depth(d, near, far)` and `fog_at(t, density)` in the shared shader module | low | no | S |
| H04 | ODR | Instance transform and shading helpers pasted into four shaders and diverged: `shade` early-out missing in `hullFrame`, **cloak ignores the Z stretch** (stretched hulls get a mismatched cloak) | `renderer/shaders/objects.ts:35-70`; `renderer/hullFrame.ts:88-100`; `renderer/shaders/cloak.ts:50-65`; `renderer/shaders/shadow.ts:143-160` | `shaders/instance.ts` (`quat_rotate`, `instance_world`, `instance_normal`, `emissive_glow`, `shade`); **behaviour fix:** cloak honours stretch, `hullFrame` gets the early-out (visual only) | medium | no | M |
| H05 | ODR | Instanced-mesh vertex buffer layouts defined four times with literal strides | `renderer/renderer.ts:145,213-245`; `renderer/hullFrame.ts:148-159`; `renderer/cloak.ts:15-28`; `renderer/sunShadow.ts:51,396,413-421` | `renderer/vertexLayouts.ts` with the layouts, `INSTANCE_FLOATS` and an `instanceLayout(locations)` helper | low | no | S |
| H06 | DRY | sRGB-to-linear re-implemented in `post.ts` without the negative clamp; triple and WGSL-literal wrappers written 5 and 3 times; `Rgb` type three times | `renderer/shaders/frame.ts:134`; `renderer/post.ts:114`; `renderer/{belt.ts:65,modeFx.ts:88,sky.ts:46,wells.ts:68,renderer.ts:47,1052,atmosphere.ts:26}`; `game/vfxLibrary.ts:61`; `renderer/models/{shading.ts:54,spectacleShading.ts:40,toyPalette.ts:56}`; `renderer/shaders/{cake.ts:43,ink.ts:52}` | `color.ts`: `Rgb`, `srgbToLinear`, `srgbToLinearRgb`, `wgslVec3`, `wgslLinearVec3`; post.ts uses it (clamp only affects negative inputs) | low | no | S |
| H07 | DRY | Small vector, hash and random helpers duplicated across renderer and game (`V3` twice, `faceNormal`, `hash01` 7 variants, `mulberry32` 3, `cross`/`norm` 5, `dist` 4, `clamp01`/`smoothstep`, `secsTo`, `clampUnit`) | `renderer/primitives.ts:8,10,242`; `renderer/models/{kit.ts:18,78-82,428,eventProps.ts:17-24,props.ts:197-204,spectacleProps.ts:34-55,raceProps.ts:15}`; `renderer/{hullFrameModels.ts:55-60,spriteAtlas.ts:96-100,ink.ts:54-58,rockTextures.ts:47}`; `game/events/{spectacleCommon.ts:69,fx.ts:166,visitorKit.ts:179}`; `game/{worldFx.ts:250,race/raceFx.ts:137,288,mechanics.ts:192,architect/model.ts:378,441,modeVisuals.ts:185,controls.ts:79,spectator.ts:130}`; `audio/engine.ts:109`; `ui/modeHud.ts:111` | `models/v3.ts` (or kit.ts), `math/vec3.ts` gains `distance`, `math/random.ts` (`hash01`, `mulberry32`); tuned variants renamed (`hashSalted01`) with a comment | low | no | S |
| H08 | DRY | Gallery scene helpers (look-at in 13 places, camera basis, ship/entity/mood/objective builders, world presets, `SCENE_TICK`, `STEP`, `shapeId`, `UP`) copied across scene files | `gallery/{eventScenesRun.ts:98,levelScenes.ts:70,raceSceneRun.ts:29,visitorRun.ts:45,worldScenes.ts:32,spectacleRun.ts:38-45,210-220,gallery.ts:142-221,shadePose.ts:83,129,modeScenes.ts:31-56,vfxScenes.ts:28-51,vqpScenes.ts:28,architectScenes.ts:37,188,scenes.ts:150-154,eventScenes.ts:26-34,spectacleScenes.ts:31-35,moodScenes.ts:28-36,architectSceneList.ts:28}`; `game/events/spectacleCommon.ts:94` | `gallery/sceneKit.ts` (`lookAt` on `lookAlong`, `cameraBasis`, builders using A09's ship table, `WORLDS` presets, shared constants) | low | no | M |
| H09 | ODR | Three parallel per-event visual frameworks and four per-event fact tables | `game/events/index.ts:47-56`; `game/events/spectacles.ts:30`; `game/events/visitors.ts:29`; `game/events/info.ts`; `audio/announcer/lines.ts`; `gallery/visitorScenes.ts:12` | One event registry keyed by the generated event id (banner info, visual factory, sound ids); `index.ts` dispatches all 18 through it | medium | no | L |
| H10 | ODR | Per-shape tables for one concept spread over four modules | `game/scene.ts:67-96`; `game/vfx.ts:94-112`; `game/lightFx.ts:122-139`; `audio/game.ts:36-70` | One `SHAPES` registry keyed by `visualShapeNames()`; test that every exported shape has an entry | low | no | M |
| H11 | ODR | Two invite-link builders/parsers with different parameter names; `?server=` parsed twice | `ui/party.ts:56-112`; `rules/model.ts:165-196`; `net/transport.ts:25-33` | One `invite.ts` owning parameter names, build and parse | low | no | S |
| H12 | DRY | DOM `el()` helper defined 18 times, `esc()` twice | `ui/{architect,eventBanner,hud,lobby,modeHud,party,raceHud,results,scoreboard,visitorHud,workshop}.ts`; `rules/editor.ts:99`; `audio/demo.ts:44` (and the remaining `el` definitions found by grep) | `ui/dom.ts` (`el`, `elWith`, `escapeHtml`) | low | no | S |

### Package J: remesh performance (not a code-quality finding)

The full check fails on The Squeeze's shell remesh benchmark (`crates/ls-game/tests/squeeze.rs`,
`shell_remeshing_stays_within_the_frame_budget`): its 99th percentile chunk remesh must stay under
`P99_BUDGET` (4 ms), and on the development machine the eight seeds measure 3.4-4.0 ms (p50
1.4-1.7 ms), so it fails whenever the machine is slightly busy. No mesher code changed since the
benchmark last passed: the true-surface mesher (snap, smooth normals, baked occlusion) costs about
twice the old one, which milestone 4 accepted ("the cave-in remesh budget partly met"). Decision:
fix the cost, keep the budget.

| ID | Where | Problem | Fix |
|---|---|---|---|
| J01 | `ls-worldgen` chunk mesher (`mesh.rs`, `chunk.rs`), the client's remesh queue (`ls-game` terrain), `tests/squeeze.rs` | p99 chunk remesh at the 4 ms budget; the slowest chunks cost about the same with or without the shell, so the mesher itself is the cost | Profile the slow chunks first (`cargo test -p ls-game --release --test squeeze -- --nocapture` prints them), then make the busy-chunk path cheaper without changing its output: for example reuse the per-chunk SDF samples and occlusion between consecutive shell steps, skip unchanged cells, or move the shell's remeshes off the frame (background remesh with the same results). |

### Package I: tools-docs

| Id | Rule | Title | Locations | Fix | Risk | Det. | Size |
|---|---|---|---|---|---|---|---|
| I01 | SSOT | e2e default rotation is a hand copy of `DEFAULT_ROTATION` and **lacks The Architect** (run-time estimate and rotation check wrong) | `ls-server/src/playlist.rs:60-69`; `tools/e2e.mjs:380-397`; `userDocs/hosting-guide.md:105,870-873` | ls-server prints the rotation (ids, minimums, longest match from I07) as JSON (hidden flag or startup log); e2e reads it; hosting guide points to it | medium | no | S |
| I02 | SSOT | Smoke and e2e rule sets are full copies of built-in rule sets and **have drifted** (no `pigment_pickup`, no `paperweight`) | `tools/browser-smoke/{lobby,heist,instagib4,mechanics}.ron`; `tools/e2e-rulesets/sandbox_debris.ron`; `tools/e2e.mjs:314-330`; `tools/browser-smoke/race.mjs:22-35` | Derive at run time from the built-in file plus overrides via `tools/lib/ruleset.mjs` (shared `setRonField` / `quickRuleSet` on I06's parser); delete the copies. Behaviour fix: the tools test the current arsenal | medium | no | M |
| I03 | DRY | Browser discovery, launch flags, static server, wasm loader and process helpers pasted across smoke scripts and e2e; **`models.mjs` and `startup.mjs` miss two browser candidates** | `tools/browser-smoke/{smoke,gallery,models,startup,moods,workshop}.mjs`; `tools/determinism-check.mjs:108-116`; `tools/e2e.mjs:145,261-519` | `tools/browser-smoke/lib.mjs` and `tools/lib/proc.mjs`; every script imports them | low | no | M |
| I04 | DRY | CI step list written four times; build/target-dir logic in four wrapper scripts | `tools/ci.ps1:20-86`; `tools/ci.sh:13-72`; `tools/hooks/pre-push.mjs:17-26`; `docs/dev-guide.md:271-282`; `tools/{e2e,browser-smoke}.{ps1,sh}` | `tools/ci.mjs` with `STEPS` (`fast` tag for pre-push, `--list`); thin `.ps1` / `.sh` wrappers; `tools/lib/build.mjs` | low | no | M |
| I05 | SSOT | Reference generator hard-codes sim/worldgen constants, serde defaults and re-implements sim formulas (radiation, cave-in) in JS | `tools/gen-weapon-docs.mjs:382-390,457,565-566,1080-1129,1172-1175,1272,1302`; sources `ls-math/src/terrain.rs:60-61`, `ls-worldgen/src/{wells.rs:49-67,belt.rs:107}`, `ls-sim/src/{radiation.rs:70-85,instability.rs:120}`, `data/ships/*.ron:39`, `data/rulesets/paint_the_void.ron:98` | Generator reads A11's world catalog and a facts block emitted by the catalog generator (ship order, material multipliers, event vetoes, serde defaults, precomputed radiation cap distances and cave-in thresholds); sentences built from loaded data; JS formula copies removed. Depends on A, D02 | medium | no | M |
| I06 | ODR | Three RON readers in tools (full parser plus regex readers that match comments) | `tools/gen-weapon-docs.mjs:44-200`; `tools/e2e.mjs:314-350`; `tools/browser-smoke/{moods.mjs,race.mjs:24}` | `tools/lib/ron.mjs` (parser plus path setter) used by all | low | no | S |
| I07 | SSOT | e2e `matchLength` re-implements mode durations including `FINAL_CLOSE_FACTOR` | `tools/e2e.mjs:333-350`; `ls-sim/src/modes/the_squeeze.rs:88-89`; `the_architect.rs`; `belt_up.rs` | `ModeModule::max_match_secs(&RuleSet)` in ls-sim, printed by the server (I01's output); e2e reads it | low | no | S |
| I08 | SSOT | Smoke scripts hand-copy data and client constants | `tools/browser-smoke/moods.mjs:53,58`; `architect.mjs:16`; `steer.mjs:6` | Read `director.ron` / `the_architect.ron` via I06; import `DEFAULT_CONTROL_SETTINGS` from `controls.ts`; prop kinds from `entityKindNames()` | low | no | S |
| I09 | SSOT | Server CLI flags and defaults restated in hosting guide, dev guide and CLAUDE.md (dev guide already incomplete); built-in rule set ids hand-listed | `ls-server/src/main.rs:60-140`; `userDocs/hosting-guide.md:134-162,144,148,850-856,931,1011`; `docs/dev-guide.md:144-160`; `CLAUDE.md` Commands | Decision (Q7): generate the hosting-guide flag table from clap metadata with a CI check; dev guide and CLAUDE.md link to it; ids from the generated reference | low | no | M |
| I10 | SSOT | Player manual restates hundreds of `data/` numbers (ships, weapons, events) covered by the generated reference; Rust module docs and RON headers restate event values | `userDocs/player-manual.md:144,270-349,425-552,665-681`; `ls-sim/src/events/{tax_drone.rs:3-11,hat_rain.rs:3-12}`; `data/events/{tax_drone,cat_paw,pizza_delivery}.ron` headers | Decision (Q6): the manual's numbers become template placeholders expanded by gen-weapon-docs with `--check` (the prose stays readable); module docs and headers name the keys | medium | no | L |
| I11 | SSOT | Architecture docs restate values owned by code or data (worldgen boxes and belt, wire masks, netcode timings and limits, renderer constants with one already drifted, radiation and gravity tuning, golden hash values, `SIM_BEHAVIOR_VERSION`) | `docs/architecture/{contracts.md:355-372,423-435,507-509,556,moods-and-world.md:210-231,arsenal-framework.md:282-285,game-glue.md:86-321,netcode.md:59-378,renderer.md:12-210,determinism.md:128,140-146,322}`; `docs/visual-quality-plan.md:147,272`; `userDocs/hosting-guide.md:220-221`; `ls-sim/src/radiation.rs:9-14` | Replace numbers with constant names and files; state each fact in one doc and link from the others; determinism.md keeps names and the history table only; measured costs stay (the doc is their source) | low | no | M |
| I12 | SSOT | Reference-doc generator inputs listed in four places; `run-checks.mjs` header already stale | `tools/gen-weapon-docs.mjs:1290-1307`; `tools/lib/run-checks.mjs:7,32-34`; `CLAUDE.md`; `docs/dev-guide.md:288-292` | Export `INPUTS` from gen-weapon-docs; run-checks builds its matcher from it; docs point to the generator | low | no | S |
| I13 | DRY | Repo root and "run as script" detection recomputed in nine tools | `tools/lib/work-folder.mjs:57`; `tools/{e2e.mjs:106,1187,determinism-check.mjs:29,gen-weapon-docs.mjs:30,check-binds.mjs:74,post-edit.mjs:61,hooks/pre-push.mjs:81}`; `tools/browser-smoke/{gallery,moods,race,smoke,workshop}.mjs` | `REPO_ROOT` and `isMain(import.meta.url)` from one lib module | low | no | S |
| I14 | SSOT | wasm-bindgen / js-sys pins and bot test parameters restated in CLAUDE.md, dev guide and `build-wasm.mjs` | `Cargo.toml:20,59-61`; `CLAUDE.md` Commands; `docs/dev-guide.md:25,97-98,234`; `web/scripts/build-wasm.mjs:10`; `ls-bots/tests/arsenal_match.rs` | build-wasm reads the pin from `Cargo.toml` and checks `wasm-bindgen --version`; docs name the pin's location; CLAUDE.md test bullets shrink to commands plus a dev-guide link | low | no | S |

## 4. Fix packages

Nine packages (letters A-E, G-J; there is no F; J is remesh performance, not a code-quality
finding). Decision (Q10): they run as one wave before
milestone 5 starts, since every later change would otherwise add more hand copies; milestone 5
wave A starts after this wave is integrated. Each package runs in its own work folder
(`loosescrews.win2`+), starts from `origin/main`, and checks its own findings against the code
before changing anything. Packages run in parallel; the merge order below resolves the remaining
overlaps (later packages rebase on earlier ones).

| Pkg | Name | Owner area (files it may change) | Findings | Size | Merge order |
|---|---|---|---|---|---|
| E | core-helpers | `crates/ls-math`, `crates/ls-worldgen`, `crates/ls-net`, `crates/ls-game` (incl. `tests/`), small call-site edits in ls-sim (E01, E05, E07, E08, E10), ls-wasm (E07), ls-server tests and `playlist.rs:86`, `ls-bots/src/steering.rs` | 22 | L | 1 |
| A | gen-contracts | `crates/ls-wasm`, the catalog generator (`ls-sim` rulecode tests) plus a new ls-worldgen catalog test, `web/src/generated/` (new), `web/src/wasm/`, the TS consumers of each generated value, `.github`/`tools/ci.*` only if a check needs wiring, owner-names in ls-sim enums (`EventPhase::id`, cat paw phases), `web/src/game/race/course.ts` and its test (A15) | 15 | L | 2 |
| B | codecs | `ls-math::code` (new), `ls-sim/src/rulecode.rs`, `ls-worldgen/src/sharecode.rs`, `ls-server/src/rooms.rs`, `ls-wasm/src/sharecode.rs` and new rule-code exports, `web/src/rules/{code,validate}.ts`, `web/src/ui/shareCode.ts`, `web/src/renderer/atmosphere.ts` (decoder part), `web/src/util/crockford.ts` | 3 | L | 3 |
| C | sim-framework | `crates/ls-sim` (structure, events framework, mode registry, test support), `ls-game/src/client/{prediction,session}.rs`, `ls-game/src/server/lagcomp.rs`, `ls-wasm/src/{shots,beam,modes}.rs`, `ls-server/src/{lobby.rs,main.rs:341}` | 23 | L | 4 |
| D | sim-data | `data/**`, ls-sim data loading (`data.rs`, `ruleset.rs`, `tuning.rs`, `weapon.rs`/`gadgets`/`events` `param`), the param call sites in weapon/gadget/event modules, `ls-server/src/playlist.rs` (D07), re-blessed `catalog.ts` and `userDocs/reference/` | 7 | L | 5 |
| G | server-bots-dir | `crates/ls-server` (except `rooms.rs`, `playlist.rs`), `crates/ls-directory`, `crates/ls-bots`, `crates/ls-net` framing module (G04), `web/src/net/{social,webtransport}.ts` | 10 | M | 6 |
| H | web-dry | `web/src/renderer/**`, `web/src/ui/**` (except files A/B own), `web/src/gallery/**`, `web/src/game/**` registries and helpers, `web/src/audio/**`, `web/src/math/**` | 12 | L | 7 |
| I | tools-docs | `tools/**`, `docs/**` (except this file's status column, which every package updates), `userDocs/**` hand-written pages, `CLAUDE.md`, `web/scripts/build-wasm.mjs`, a small `max_match_secs` hook in ls-sim (I07) and the rotation print in ls-server (I01) | 14 | L | 8 |
| J | remesh-perf | `crates/ls-worldgen` mesher (`mesh.rs`, `chunk.rs`), the ls-game client remesh queue (`terrain.rs`), `crates/ls-game/tests/squeeze.rs` (reporting only, the budgets stay) | 1 | M | 9 |

Overlaps that the order handles: E adds the ls-math owners (terrain ids, slider indices, mode
wire, well grid) before A generates TS from them; A and H both touch TS consumers (A replaces
constants, H restructures helpers, so H rebases on A); C changes ls-sim module structure before D
rewrites every `param` call in the same modules; G's playlist-free changes do not touch D07's
`playlist.rs` edit; I05 needs A's facts block and D02's data files, so I merges after the others except J; J rebases
on E (both touch ls-worldgen and ls-game) and merges last. Packages
that edit the same file in the same lines should coordinate through the lead rather than resolve
conflicts silently.

### Acceptance criteria (all packages)

- `pwsh tools/ci.ps1` green in the package's work folder, including the native and wasm32 tests,
  `node tools/determinism-check.mjs`, `node tools/gen-weapon-docs.mjs --check`, and every new
  generated-file check.
- **Pure refactors leave every hash unchanged:** ls-worldgen golden hashes, `REPLAY_GOLDEN_HASH`,
  the per-mode and `mode_replays.txt` / event / world replay fixtures, and the handshake data hash
  (`ls_sim::data_hash`). No `LS_BLESS_*` re-bless and no `GENERATOR_VERSION` /
  `SIM_BEHAVIOR_VERSION` bump unless the package section below names it.
- Every finding the package owns is marked `fixed (<commit>)` or `won't fix: <reason>` in this
  file; any deliberate duplication that stays carries the one-line comment `code-quality.md`
  requires.
- The review checklist of `docs/code-quality.md` answered for the package's own diff (no new
  duplication introduced by the fix).

Per package:

- **E core-helpers.** All hashes unchanged (E01-E05, E07, E08, E10 are bit-identical moves; E10
  must show the clamp/round order is equal on the used range). Wire bytes unchanged (ls-net
  round-trip tests pass without edits to expected bytes). ls-game netplay tests use the shared
  `tests/common` harness.
- **A gen-contracts.** No hand-written copy of a Rust constant, enum order, layout or bit remains
  in `web/src` for the listed findings (grep for the old names returns only the generated files).
  Every new generated file has a bless variable and fails CI when stale; `docs/dev-guide.md` and
  `CLAUDE.md` list the new bless commands. A13 removes the TS fallback decoder; A10 takes the HUD
  pull from ls-wasm; A15: `course.ts` takes race progress from ls-wasm and holds no copy of
  `belt_up::progress` (the Rust `race_progress` fixture test still passes). Sim and data hashes
  unchanged.
- **B codecs.** Share codes and rule codes byte-identical: existing Rust vector tests, the
  catalog vectors and share-code tests pass unchanged; one Crockford implementation per language.
  The rule set editor runs on the ls-wasm codec and validation (Q1); `rules/code.ts` and
  `validate.ts` hold no format logic of their own.
  Worldgen golden hashes and `GENERATOR_VERSION` unchanged.
- **C sim-framework.** All sim hashes and replay fixtures unchanged (`preview equals step` tests
  kept). **Behaviour change, stated:** C01 brings the client predictor in line with `World::step`
  (`CONTROLS_OWNER` thrust zeroing, double `limits.apply`, boulder filter); this changes client
  prediction only, so prediction tests may need new expectations, which the package explains.
  C10 keeps today's decoy behaviour (Q9: no change). C23 moves the per-mode golden constants into
  the fixture (Q5); it keeps every hash value, only the storage moves.
- **D sim-data.** Resolved rule sets, ship tuning and sim behaviour identical:
  `REPLAY_GOLDEN_HASH` and all replay fixtures unchanged. **The data hash changes** (new or
  restructured hashed files in D01-D07): accepted under Q3, old clients become incompatible
  (handshake), recorded in `next-steps.md`. D03 adds no `mode_settings` field (Q4). Re-bless
  `web/src/rules/catalog.ts` (`LS_BLESS_RULES_CATALOG=1`) and regenerate `userDocs/reference/`; rule codes (`LR..`) for the
  built-in rule sets decode to the same rule sets. D01: a data set missing a required key fails at
  load with a named error (test). The charged-surface drain without a Leech Arc comes from the
  data file with today's value (Q8).
- **G server-bots-dir.** HTTP, lobby, duel and WebTransport tests pass; bots run against the
  server (`cargo test -p ls-bots --release`, `pwsh tools/e2e.ps1` once with `--transport wt`).
  No sim or data hash change (G09's bot data is outside the hashed set).
- **H web-dry.** `npm run typecheck`, `npm test`, `npm run build` green; browser smoke
  (`pwsh tools/browser-smoke.ps1`) and gallery screenshots compared before/after: identical except
  **H04, stated behaviour fix** (cloak follows the hull's Z stretch) and H06's clamp for negative
  inputs. Shader-routine-once test added.
- **I tools-docs.** e2e covers The Architect in the default rotation and the smoke tests run on the
  current built-in rule sets (stated behaviour change of the test tools, I01, I02); one step list
  drives CI and pre-push; docs name constants and files instead of values for every listed site;
  the hosting guide's flag table is generated from the server's clap definitions (Q7) and the
  player manual's numbers from placeholders (Q6); every new generated doc section has a `--check`
  in CI.
- **J remesh-perf.** The benchmark's p99 chunk remesh is under 3 ms on every seed on the
  development machine (a 25 % margin under the unchanged 4 ms `P99_BUDGET`), in three consecutive
  full checks; `CHUNK_BUDGET` and `P99_BUDGET` unchanged. Meshes identical: worldgen golden hashes
  and every mesh test unchanged, plus a test that the faster path produces the same vertices and
  indices as the plain mesher on the benchmark's slow chunks. Gallery screenshots identical. The
  cat-paw remesh benchmark (`tests/cat_paw_remesh.rs`) does not get slower. The finding is marked
  `fixed (<commit>)` with the measured numbers.

## 5. Decisions

All ten questions are decided; the findings, packages and acceptance criteria above follow them.

- **Q1 (B02):** may the rule set editor require the WASM core? Decision: yes (the game needs the
  WASM core anyway). `rules/code.ts` and `validate.ts` become thin wrappers over ls-wasm; no
  exception in `code-quality.md`.
- **Q2 (A15, the dropped finding):** keep `web/src/game/race/course.ts`'s TS mirror of
  `belt_up::progress` as a commented, fixture-checked exception, or export race progress from
  ls-wasm? Decision: export it from ls-wasm; the TS copy goes (finding A15, package A).
- **Q3 (D01, D02, D04, D06, D07):** accept a handshake data-hash change (old clients and servers
  stop matching) for restructuring `data/` without changing any resolved value? Decision: yes.
- **Q4 (D03):** which mode tuning numbers become host-editable `mode_settings` (they cost
  rule-code bits) and which go into fixed `data/modes/*.ron`? Decision: today's host-editable
  settings stay as they are; everything else moves into `data/modes/`.
- **Q5 (C23):** fold the hand-edited per-mode module golden constants into the
  `mode_replays.txt` fixture (one bless mechanism)? Decision: yes.
- **Q6 (I10):** player manual numbers: template placeholders expanded by gen-weapon-docs, or cut
  from the prose with links to `userDocs/reference/`? Decision: generator placeholders (the prose
  stays readable), checked by `--check`.
- **Q7 (I09):** generate the hosting guide's CLI table from clap, or keep it as the only
  hand-written table? Decision: generated from the server's clap definitions, with a CI check.
- **Q8 (D01):** when no Leech Arc is in the weapon table, charged surfaces drain at the code
  constant 5/s. Move that number to a world data file, or drain 0? Decision: a world data file
  (no behaviour change).
- **Q9 (C10):** should Leech Arc and Polite Laser locks be fooled by decoys like the Seeker?
  Decision: no change; only the Seeker is fooled (a design question for play-testing, not part of
  the refactor). No `SIM_BEHAVIOR_VERSION` bump.
- **Q10:** run the eight packages as one wave before milestone 5, or interleave? Decision: one
  wave before milestone 5, since every later change would otherwise add more hand copies.
