//! wasm-bindgen bindings for the Loose Screws browser client
//! (`docs/implementation-plan.md` section 4.6).
//!
//! The logic lives in plain Rust modules that are tested natively:
//!
//! - [`sharecode`]: share-code decode/validation and random practice codes;
//! - [`controls`]: browser controls to per-tick `InputCmd`s, fixed 60 Hz step;
//! - [`stream`]: which chunk to mesh next around the camera, dirty first;
//! - [`game`]: [`game::GameCore`], online (wrapping
//!   `ls_game::client::ClientSession`, connect-agnostic: bytes in, bytes out)
//!   or offline practice (a local `ls_sim::World`);
//! - [`scene`]: generic entities, the data-driven visual table and player
//!   names;
//! - [`inventory`]: carried weapons for the weapon HUD and selection by
//!   number key, wheel and X;
//! - [`mechanics`]: game cues for effects, the charge meter, the gadget
//!   state and ship type choice with its loadout preview (M2 wave C);
//! - [`modes`]: mode state, lobby state and match results as JSON (M3);
//! - [`probe`]: ray cast and ship-sized sphere sweep for the HUD's fit ring
//!   (visual-quality-plan.md VQ1).
//!
//! This file only converts to JavaScript types. Flat arrays keep the
//! boundary cheap; their layouts are documented on each method and mirrored
//! by `web/src/wasm/api.ts`.

// wasm-bindgen's `getter_with_clone` getters clone every field, `Copy` ones
// too; clippy 1.99 flags that generated code.
#![allow(clippy::clone_on_copy)]

use wasm_bindgen::prelude::*;

pub mod beam;
pub mod controls;
pub mod game;
pub mod inventory;
pub mod level;
pub mod mechanics;
pub mod modes;
pub mod probe;
pub mod scene;
pub mod sharecode;
pub mod shots;
pub mod stream;

use crate::controls::Controls;
use crate::game::GameCore;
use crate::sharecode::CodeCheck;
use ls_math::Vec3;

/// Installs a panic hook that forwards Rust panics to `console.error`.
/// Call once from JavaScript right after the module is initialised.
#[wasm_bindgen]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Simulation tick rate in Hz, so the client and server agree.
#[wasm_bindgen(js_name = tickHz)]
pub fn tick_hz() -> u32 {
    ls_math::TICK_HZ
}

/// Generator version of this build (the second share-code field).
#[wasm_bindgen(js_name = generatorVersion)]
pub fn generator_version() -> u8 {
    ls_worldgen::GENERATOR_VERSION
}

/// Result of [`check_share_code`].
#[wasm_bindgen(getter_with_clone)]
pub struct ShareCodeInfo {
    pub ok: bool,
    /// Printed code (`LSab-cdef-…`) when valid.
    pub code: String,
    /// Reason when invalid.
    pub error: String,
    /// Seed as a decimal string (u64 does not fit a JS number).
    pub seed: String,
    /// Sliders in share-code order (enclosure, density, verticality, gravity
    /// chaos, palette, absurdity, instability).
    pub sliders: Vec<u8>,
    /// Level archetype index (0 classic, 1 asteroid belt).
    pub archetype: u8,
}

/// Full share-code validation (checksum and generator version included).
#[wasm_bindgen(js_name = checkShareCode)]
pub fn check_share_code(text: &str) -> ShareCodeInfo {
    match sharecode::check_share_code(text) {
        CodeCheck::Valid {
            printed,
            seed,
            sliders,
            archetype,
            ..
        } => ShareCodeInfo {
            ok: true,
            code: printed,
            error: String::new(),
            seed: seed.to_string(),
            sliders: sliders.to_vec(),
            archetype,
        },
        CodeCheck::Invalid(error) => ShareCodeInfo {
            ok: false,
            code: String::new(),
            error,
            seed: String::new(),
            sliders: Vec::new(),
            archetype: 0,
        },
    }
}

/// A new share code for practice from 64 bits of caller-supplied entropy.
#[wasm_bindgen(js_name = randomShareCode)]
pub fn random_share_code(
    entropy_lo: u32,
    entropy_hi: u32,
    enclosure: u8,
    density: u8,
) -> Result<String, JsError> {
    let entropy = (entropy_hi as u64) << 32 | entropy_lo as u64;
    sharecode::random_share_code(entropy, enclosure, density).map_err(|e| JsError::new(&e))
}

/// A share code like [`random_share_code`] (the same seed for the same
/// entropy) with all seven sliders (share-code order, each clamped to
/// 0..=100) and a level archetype index (0 classic, 1 asteroid belt); the
/// visual gallery's worlds.
#[wasm_bindgen(js_name = worldShareCode)]
pub fn world_share_code(
    entropy_lo: u32,
    entropy_hi: u32,
    sliders: &[u8],
    archetype: u8,
) -> Result<String, JsError> {
    let entropy = (entropy_hi as u64) << 32 | entropy_lo as u64;
    sharecode::world_share_code(entropy, sliders, archetype).map_err(|e| JsError::new(&e))
}

/// Gravity well sites of a share code: 8 floats per well (`x, y, z, core
/// radius, hollow radius, strength, range, kind`; kind 0 core, 1 floating
/// stone, 2 shepherd moon). See `level::level_wells`.
#[wasm_bindgen(js_name = levelWells)]
pub fn level_wells(code: &str) -> Result<Vec<f32>, JsError> {
    level::level_wells(code).map_err(|e| JsError::new(&e))
}

/// The asteroid belt of a share code (empty for classic levels): centre,
/// radii, half thickness, cocoon, exit and gate radii, the 8 start slots,
/// then the clear corridor's centre line. See `level::level_belt`.
#[wasm_bindgen(js_name = levelBelt)]
pub fn level_belt(code: &str) -> Result<Vec<f32>, JsError> {
    level::level_belt(code).map_err(|e| JsError::new(&e))
}

/// The share code of a seed (decimal string, any u64), the seven sliders in
/// share-code order (each clamped to 0..=100) and a level archetype index
/// (0 classic, 1 asteroid belt), for the current generator version (the
/// world workshop builds codes with it).
#[wasm_bindgen(js_name = makeShareCode)]
pub fn make_share_code(seed: &str, sliders: &[u8], archetype: u8) -> Result<String, JsError> {
    sharecode::make_share_code(seed, sliders, archetype).map_err(|e| JsError::new(&e))
}

/// Golden hashes of a share code (`ls_worldgen::golden::golden_hashes`) as
/// lowercase hex in fixture order: sdf, surface_nets, dual_contouring,
/// edited. Compare with `crates/ls-worldgen/tests/fixtures/golden_hashes.txt`
/// to cross-check WASM against native.
#[wasm_bindgen(js_name = goldenHashes)]
pub fn golden_hashes(code: &str) -> Result<Vec<String>, JsError> {
    let code = ls_worldgen::ShareCode::decode(code).map_err(|e| JsError::new(&e.to_string()))?;
    let h = ls_worldgen::golden::golden_hashes(&code).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(h.to_hex().to_vec())
}

/// Per-mode replay hashes (`ls_sim::golden::mode_replay_hashes`): one line
/// `<rule set file> <0x hash>` per built-in rule set. Compare with
/// `crates/ls-sim/tests/fixtures/mode_replays.txt` to cross-check WASM
/// against native (`tools/determinism-check.mjs`).
#[wasm_bindgen(js_name = modeReplayHashes)]
pub fn mode_replay_hashes() -> Vec<String> {
    ls_sim::golden::mode_replay_hashes()
}

/// The first packet to send after the WebSocket opens (`Hello`).
/// `preferred_team` < 0 lets the server choose.
#[wasm_bindgen(js_name = helloPacket)]
pub fn hello_packet(name: &str, preferred_team: i32) -> Result<Vec<u8>, JsError> {
    let team = u8::try_from(preferred_team).ok();
    ls_net::Message::Hello(ls_game::client_hello(name, team, None))
        .encode()
        .map_err(|e| JsError::new(&e.to_string()))
}

/// [`hello_packet`] asking for a ship type (`ShipType::index`; the server
/// falls back to the Tinker or the first allowed type).
#[wasm_bindgen(js_name = helloPacketAs)]
pub fn hello_packet_as(name: &str, preferred_team: i32, ship_type: u8) -> Result<Vec<u8>, JsError> {
    let team = u8::try_from(preferred_team).ok();
    let ship = ls_sim::ShipType::from_index(ship_type);
    ls_net::Message::Hello(ls_game::client_hello(name, team, ship))
        .encode()
        .map_err(|e| JsError::new(&e.to_string()))
}

/// [`hello_packet_as`] with a private room code and a party code (empty
/// strings for none); `ship_type` 255 lets the server choose.
#[wasm_bindgen(js_name = helloPacketWith)]
pub fn hello_packet_with(
    name: &str,
    preferred_team: i32,
    ship_type: u8,
    room: &str,
    party: &str,
) -> Result<Vec<u8>, JsError> {
    let team = u8::try_from(preferred_team).ok();
    let ship = ls_sim::ShipType::from_index(ship_type);
    let code = |s: &str| (!s.is_empty()).then(|| s.to_string());
    ls_net::Message::Hello(
        ls_game::client_hello(name, team, ship)
            .with_room(code(room))
            .with_party(code(party)),
    )
    .encode()
    .map_err(|e| JsError::new(&e.to_string()))
}

/// Mode ids in `ModeKind` index order (`kind` of the mode state).
#[wasm_bindgen(js_name = modeIds)]
pub fn mode_ids() -> Vec<String> {
    ls_sim::ModeKind::ALL
        .iter()
        .map(|k| k.id().to_string())
        .collect()
}

/// Gadget string ids in id order (`gadget` of [`Game::player_info`]).
#[wasm_bindgen(js_name = gadgetIds)]
pub fn gadget_ids() -> Vec<String> {
    ls_sim::GadgetTable::builtin()
        .iter()
        .map(|(_, d)| d.id.clone())
        .collect()
}

/// The `Disconnect` reason a server sends when it replaces a finished match
/// (`ls_game::MATCH_RESTART_REASON`): the client should rejoin, not report
/// an error.
#[wasm_bindgen(js_name = matchRestartReason)]
pub fn match_restart_reason() -> String {
    ls_game::MATCH_RESTART_REASON.to_string()
}

/// Weapon names in id order (`kind` of projectiles, `weaponSelect - 1`).
#[wasm_bindgen(js_name = weaponNames)]
pub fn weapon_names() -> Vec<String> {
    ls_sim::WeaponTable::builtin()
        .iter()
        .map(|(_, d)| d.name.clone())
        .collect()
}

/// Weapon string ids in id order (`scrap_laser`, …).
#[wasm_bindgen(js_name = weaponIds)]
pub fn weapon_ids() -> Vec<String> {
    ls_sim::WeaponTable::builtin()
        .iter()
        .map(|(_, d)| d.id.clone())
        .collect()
}

/// Visual shape names in shape-index order (the first number of each
/// [`Game::visuals`] row indexes this list).
#[wasm_bindgen(js_name = visualShapeNames)]
pub fn visual_shape_names() -> Vec<String> {
    scene::SHAPES
        .iter()
        .map(|&s| scene::shape_name(s).to_string())
        .collect()
}

/// `ReplicatedKind` names in wire-byte order (`kind` of [`Game::entities`]).
#[wasm_bindgen(js_name = entityKindNames)]
pub fn entity_kind_names() -> Vec<String> {
    (0u8..=u8::MAX)
        .map_while(ls_sim::ReplicatedKind::from_u8)
        .map(entity_kind_name)
        .map(str::to_string)
        .collect()
}

/// Lower camel case name of an entity kind (the web client's keys).
pub fn entity_kind_name(k: ls_sim::ReplicatedKind) -> &'static str {
    use ls_sim::ReplicatedKind::*;
    match k {
        Debris => "debris",
        WeaponPickup => "weaponPickup",
        UpgradePickup => "upgradePickup",
        GadgetPickup => "gadgetPickup",
        ScrapPile => "scrapPile",
        Wreck => "wreck",
        Decoy => "decoy",
        Ghost => "ghost",
        DustCloud => "dustCloud",
        Marker => "marker",
    }
}

/// Carried weapons and gadget for the weapon HUD (see
/// `inventory::LoadoutView`). Per weapon, primaries first (key order), then
/// secondaries: `names[i]`, `ammo[i]` and 4 bytes `info[4i..4i+4]` =
/// `[slot (0 primary, 1 secondary), selected, key (1-10, 0 none), level]`,
/// `shots[i]` (paid shots, the pattern phase) and `states[i]` (the weapon
/// module's state byte, e.g. the Paint Thrower colour; 0 = unset).
#[wasm_bindgen(getter_with_clone)]
pub struct LoadoutHud {
    pub names: Vec<String>,
    pub ammo: Vec<String>,
    pub info: Vec<u8>,
    pub shots: Vec<u32>,
    pub states: Vec<u8>,
    #[wasm_bindgen(js_name = maxLevels)]
    pub max_levels: Vec<u8>,
    /// Gadget name, empty for none.
    pub gadget: String,
    #[wasm_bindgen(js_name = gadgetActive)]
    pub gadget_active: bool,
}

/// Bytes per weapon in [`LoadoutHud::info`].
pub const LOADOUT_INFO_STRIDE: usize = 4;

pub fn loadout_hud(v: &inventory::LoadoutView) -> LoadoutHud {
    let mut h = LoadoutHud {
        names: Vec::new(),
        ammo: Vec::new(),
        info: Vec::new(),
        shots: Vec::new(),
        states: Vec::new(),
        max_levels: Vec::new(),
        gadget: v.gadget.clone(),
        gadget_active: v.gadget_active,
    };
    for e in v.primaries.iter().chain(&v.secondaries) {
        h.names.push(e.name.clone());
        h.ammo.push(e.ammo.clone());
        h.info.extend_from_slice(&[
            (e.slot == ls_sim::WeaponSlot::Secondary) as u8,
            e.selected as u8,
            e.key,
            e.level,
        ]);
        h.max_levels.push(e.max_level);
        h.shots.push(e.shots);
        h.states.push(e.state);
    }
    h
}

/// HUD values (see `game::HudState`).
#[wasm_bindgen(getter_with_clone)]
pub struct HudState {
    pub hull: f32,
    #[wasm_bindgen(js_name = hullMax)]
    pub hull_max: f32,
    pub energy: f32,
    #[wasm_bindgen(js_name = energyMax)]
    pub energy_max: f32,
    pub boost: f32,
    #[wasm_bindgen(js_name = boostMax)]
    pub boost_max: f32,
    pub scrap: f32,
    #[wasm_bindgen(js_name = scrapMax)]
    pub scrap_max: f32,
    pub weapon: String,
    pub ammo: String,
    pub secondary: String,
    pub paint0: f32,
    pub paint1: f32,
    #[wasm_bindgen(js_name = timeLeft)]
    pub time_left: f32,
    pub team: u8,
    pub alive: bool,
    pub status: String,
}

/// One chunk mesh (see `docs/architecture/contracts.md`, "Chunk meshes").
/// Each accessor returns a fresh copy; call `free()` when done.
#[wasm_bindgen]
pub struct ChunkMesh {
    key: String,
    positions: Vec<f32>,
    normals: Vec<f32>,
    indices: Vec<u32>,
    teams: Vec<u8>,
    surface: Vec<u8>,
    materials: Vec<u8>,
    material_weights: Vec<u8>,
    colors: Vec<u8>,
    ao: Vec<u8>,
    aabb: Vec<f32>,
}

#[wasm_bindgen]
impl ChunkMesh {
    /// Chunk key `"x,y,z"`.
    #[wasm_bindgen(getter)]
    pub fn key(&self) -> String {
        self.key.clone()
    }
    /// xyz triples, world space.
    pub fn positions(&self) -> Vec<f32> {
        self.positions.clone()
    }
    pub fn normals(&self) -> Vec<f32> {
        self.normals.clone()
    }
    /// Triangles, counter-clockwise seen from open space.
    pub fn indices(&self) -> Vec<u32> {
        self.indices.clone()
    }
    /// One paint team per vertex (255 = unpainted).
    pub fn teams(&self) -> Vec<u8> {
        self.teams.clone()
    }
    /// Surface state bits per vertex (`game::SURFACE_HOT`, `SURFACE_CHARGED`,
    /// `SURFACE_GLOW`), or empty when the terrain has none.
    pub fn surface(&self) -> Vec<u8> {
        self.surface.clone()
    }
    /// Rock material per vertex (`ls_math::terrain::MATERIAL_*`: 0 weak,
    /// 1 rock, 2 hard), or empty when the mesh carries none.
    pub fn materials(&self) -> Vec<u8> {
        self.materials.clone()
    }
    /// Display blend weights of weak and hard rock, two bytes per vertex
    /// (0..255; `ls_worldgen::MaterialRegion::blend_weights`: smooth across
    /// material borders), or empty when the mesh carries none.
    #[wasm_bindgen(js_name = materialWeights)]
    pub fn material_weights(&self) -> Vec<u8> {
        self.material_weights.clone()
    }
    /// Paint effect colour per vertex (`ls_math::terrain::PAINT_*`: 0 plain,
    /// 1 red, 2 blue, 3 yellow), or empty when the mesh carries none.
    #[wasm_bindgen(js_name = paintColors)]
    pub fn paint_colors(&self) -> Vec<u8> {
        self.colors.clone()
    }
    /// Baked ambient occlusion per vertex (255 = open, 0 = fully occluded;
    /// `ls_worldgen::surface_nets`), or empty when the mesh carries none.
    pub fn ao(&self) -> Vec<u8> {
        self.ao.clone()
    }
    /// Chunk box `[minX, minY, minZ, maxX, maxY, maxZ]`.
    pub fn aabb(&self) -> Vec<f32> {
        self.aabb.clone()
    }
}

/// Floats per ship in [`Game::remote_ships`].
pub const SHIP_STRIDE: usize = 14;
/// Floats per projectile in [`Game::projectiles`].
pub const PROJECTILE_STRIDE: usize = 10;
/// Floats per entity in [`Game::entities`].
pub const ENTITY_STRIDE: usize = 20;
/// Numbers per player in [`Game::player_info`].
pub const PLAYER_STRIDE: usize = 4;

/// Numbers per cue in [`Game::take_cues`].
pub const CUE_STRIDE: usize = 8;
/// Numbers in [`Game::charge`].
pub const CHARGE_LEN: usize = 8;

/// Per cue `[kind, source, target, px, py, pz, value, tick]` (ship ids 255
/// = none; `kind` is the `ls_sim::CueKind` byte).
pub fn cues_array(cues: &[mechanics::CueView]) -> Vec<f64> {
    let mut v = Vec::with_capacity(cues.len() * CUE_STRIDE);
    for c in cues {
        v.extend_from_slice(&[
            c.kind as f64,
            c.source as f64,
            c.target as f64,
            c.pos.x as f64,
            c.pos.y as f64,
            c.pos.z as f64,
            c.value as f64,
            c.tick as f64,
        ]);
    }
    v
}

/// `[weapon, fraction, heldSecs, fullSecs, minSecs, autoRelease,
/// overcharge, 1]` (the last entry marks a charge in progress).
pub fn charge_array(c: &mechanics::ChargeView) -> Vec<f64> {
    vec![
        c.weapon as f64,
        c.fraction as f64,
        c.held_secs as f64,
        c.full_secs as f64,
        c.min_secs as f64,
        c.auto_release as u8 as f64,
        c.overcharge as u8 as f64,
        1.0,
    ]
}

/// The own gadget for the HUD (see `mechanics::GadgetView`).
#[wasm_bindgen(getter_with_clone)]
pub struct GadgetHud {
    /// Gadget id, 255 for none.
    pub id: u8,
    /// String id (`shield`, `cloak`, `decoy`), empty for none.
    pub key: String,
    pub name: String,
    pub active: bool,
    pub toggle: bool,
    #[wasm_bindgen(js_name = cooldownLeft)]
    pub cooldown_left: f32,
    #[wasm_bindgen(js_name = cooldownSecs)]
    pub cooldown_secs: f32,
    #[wasm_bindgen(js_name = energyCost)]
    pub energy_cost: f32,
    #[wasm_bindgen(js_name = drainPerSec)]
    pub drain_per_sec: f32,
}

pub fn gadget_hud(g: &mechanics::GadgetView) -> GadgetHud {
    GadgetHud {
        id: g.id,
        key: g.key.clone(),
        name: g.name.clone(),
        active: g.active,
        toggle: g.toggle,
        cooldown_left: g.cooldown_left,
        cooldown_secs: g.cooldown_secs,
        energy_cost: g.energy_cost,
        drain_per_sec: g.drain_per_sec,
    }
}

/// JSON string literal for `s`.
pub(crate) fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// JSON number (rounded to hundredths; non-finite values become 0).
fn json_num(v: f32) -> String {
    if v.is_finite() {
        format!("{}", (v * 100.0).round() / 100.0)
    } else {
        "0".into()
    }
}

/// Ship choices as a JSON array of `{shipType, name, allowed, current,
/// next, hull, topSpeedPct, damageMult, secondarySlots, radius, turnRate,
/// canPassScrap, primaries: [name], secondaries: [[name, count]], gadget}`.
pub fn ship_choices_json(choices: &[mechanics::ShipChoice]) -> String {
    let rows: Vec<String> = choices
        .iter()
        .map(|c| {
            let primaries: Vec<String> = c.primaries.iter().map(|n| json_str(n)).collect();
            let secondaries: Vec<String> = c
                .secondaries
                .iter()
                .map(|(n, k)| format!("[{},{}]", json_str(n), k))
                .collect();
            let fields = [
                ("shipType", c.ship_type.to_string()),
                ("name", json_str(&c.name)),
                ("allowed", c.allowed.to_string()),
                ("current", c.current.to_string()),
                ("next", c.next.to_string()),
                ("hull", json_num(c.hull)),
                ("topSpeedPct", json_num(c.top_speed_pct)),
                ("damageMult", json_num(c.damage_mult)),
                ("secondarySlots", c.secondary_slots.to_string()),
                ("radius", json_num(c.radius)),
                ("turnRate", json_num(c.turn_rate)),
                ("canPassScrap", c.can_pass_scrap.to_string()),
                ("primaries", format!("[{}]", primaries.join(","))),
                ("secondaries", format!("[{}]", secondaries.join(","))),
                ("gadget", json_str(&c.gadget)),
            ];
            let body: Vec<String> = fields.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect();
            format!("{{{}}}", body.join(","))
        })
        .collect();
    format!("[{}]", rows.join(","))
}

/// Numbers in [`Game::debug_stats`].
pub const DEBUG_STATS_LEN: usize = 10;
/// Numbers per row in [`Game::scoreboard`].
pub const SCORE_STRIDE: usize = 6;
/// Numbers in [`Game::match_info`].
pub const MATCH_INFO_LEN: usize = 6;

/// `[pingMs (-1 = unknown), accepted, corrected, snapped, lastError,
/// shotsPredicted, shotsConfirmed, shotsDropped, shotsUnpredicted, synced]`.
pub fn debug_stats_array(d: &game::DebugStats) -> Vec<f64> {
    vec![
        d.ping_ms.unwrap_or(-1.0),
        d.accepted as f64,
        d.corrected as f64,
        d.snapped as f64,
        d.last_error as f64,
        d.shots.predicted as f64,
        d.shots.confirmed as f64,
        d.shots.dropped as f64,
        d.shots.unpredicted as f64,
        d.synced as u8 as f64,
    ]
}

/// Per row `[id, team, alive, kills, deaths, isYou]` (names: see
/// [`Game::scoreboard_names`]).
pub fn scoreboard_array(rows: &[game::ScoreRow]) -> Vec<f64> {
    let mut v = Vec::with_capacity(rows.len() * SCORE_STRIDE);
    for r in rows {
        v.extend_from_slice(&[
            r.id as f64,
            r.team as f64,
            r.alive as u8 as f64,
            r.kills as f64,
            r.deaths as f64,
            r.is_you as u8 as f64,
        ]);
    }
    v
}

/// `[over, winner (-1 = draw / none), team0Cells, team1Cells, surfaceCells,
/// secsSinceEnd]`.
pub fn match_info_array(m: &game::MatchInfo) -> Vec<f64> {
    vec![
        m.over as u8 as f64,
        m.winner.map_or(-1.0, |w| w as f64),
        m.team_cells[0] as f64,
        m.team_cells[1] as f64,
        m.surface_cells as f64,
        m.secs_since_end as f64,
    ]
}

/// Per ship `[id, team, alive, boosting, px, py, pz, qx, qy, qz, qw,
/// shipType, radius, gadgetActive]`.
pub fn ships_array(ships: &[game::ShipView]) -> Vec<f32> {
    let mut v = Vec::with_capacity(ships.len() * SHIP_STRIDE);
    for s in ships {
        v.extend_from_slice(&[
            s.id as f32,
            s.team as f32,
            s.alive as u8 as f32,
            s.boosting as u8 as f32,
            s.pos.x,
            s.pos.y,
            s.pos.z,
            s.rot.x,
            s.rot.y,
            s.rot.z,
            s.rot.w,
            s.ship_type as f32,
            s.radius,
            s.gadget_active as u8 as f32,
        ]);
    }
    v
}

/// Per projectile `[kind, owner, px, py, pz, vx, vy, vz, visual, state]`.
pub fn projectiles_array(shots: &[game::ProjectileView]) -> Vec<f32> {
    let mut v = Vec::with_capacity(shots.len() * PROJECTILE_STRIDE);
    for p in shots {
        v.extend_from_slice(&[
            p.kind as f32,
            p.owner as f32,
            p.pos.x,
            p.pos.y,
            p.pos.z,
            p.vel.x,
            p.vel.y,
            p.vel.z,
            p.visual as f32,
            p.state as f32,
        ]);
    }
    v
}

/// Per entity `[id, kind, visual, owner, team, px, py, pz, vx, vy, vz, qx,
/// qy, qz, qw, state0, state1, state2, state3, radius (0 = unknown)]`.
pub fn entities_array(entities: &[scene::EntityDraw]) -> Vec<f32> {
    let mut v = Vec::with_capacity(entities.len() * ENTITY_STRIDE);
    for e in entities {
        v.extend_from_slice(&[
            e.id as f32,
            e.kind as f32,
            e.visual as f32,
            e.owner as f32,
            e.team as f32,
            e.pos.x,
            e.pos.y,
            e.pos.z,
            e.vel.x,
            e.vel.y,
            e.vel.z,
            e.rot.x,
            e.rot.y,
            e.rot.z,
            e.rot.w,
            e.state[0] as f32,
            e.state[1] as f32,
            e.state[2] as f32,
            e.state[3] as f32,
            e.radius,
        ]);
    }
    v
}

/// The running game (online or practice), see `game::GameCore`.
#[wasm_bindgen]
pub struct Game {
    core: GameCore,
}

fn js_err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

#[wasm_bindgen]
impl Game {
    /// Online game from the server's first packet (`Welcome`). Throws with
    /// the server's reason on `Reject`.
    #[wasm_bindgen(js_name = fromWelcome)]
    pub fn from_welcome(packet: &[u8]) -> Result<Game, JsError> {
        GameCore::online(packet)
            .map(|core| Game { core })
            .map_err(js_err)
    }

    /// Offline practice on the level of `code` with `dummies` idle opponents.
    /// Generates the whole level up front (a noticeable pause).
    pub fn practice(code: &str, dummies: u8) -> Result<Game, JsError> {
        GameCore::practice(code, dummies)
            .map(|core| Game { core })
            .map_err(js_err)
    }

    /// A practice level for looking at (the start screen's live backdrop):
    /// no dummies, and chunks are generated when first meshed or probed
    /// instead of all up front, so building it does not block the page.
    /// Stepping it is not intended (the first paint score update would
    /// generate the rest).
    #[wasm_bindgen(js_name = practicePreview)]
    pub fn practice_preview(code: &str) -> Result<Game, JsError> {
        GameCore::practice_preview(code)
            .map(|core| Game { core })
            .map_err(js_err)
    }

    /// Practice levels from [`Game::practice_preview`]: generates the chunk
    /// nearest the point that is still missing (one chunk, tens of ms) and
    /// returns how many are left; 0 when all are generated (and online).
    #[wasm_bindgen(js_name = generateNearest)]
    pub fn generate_nearest(&mut self, x: f32, y: f32, z: f32) -> u32 {
        self.core.generate_nearest(Vec3::new(x, y, z)) as u32
    }

    /// [`Game::practice`] flying a ship type (`ShipType::index`; not
    /// allowed types fall back to the Tinker or the first allowed one).
    #[wasm_bindgen(js_name = practiceAs)]
    pub fn practice_as(code: &str, dummies: u8, ship_type: u8) -> Result<Game, JsError> {
        let ship = ls_sim::ShipType::from_index(ship_type).unwrap_or(ls_sim::ShipType::Tinker);
        GameCore::practice_as(code, dummies, ship)
            .map(|core| Game { core })
            .map_err(js_err)
    }

    /// Game cues since the last call, [`CUE_STRIDE`] numbers each (see
    /// [`cues_array`]).
    #[wasm_bindgen(js_name = takeCues)]
    pub fn take_cues(&mut self) -> Vec<f64> {
        cues_array(&self.core.take_cues())
    }

    /// Current simulation tick (online: the estimated server time), the
    /// clock of the cues' `tick`.
    #[wasm_bindgen(js_name = nowTick)]
    pub fn now_tick(&self) -> f64 {
        self.core.now_tick()
    }

    /// The charge in progress, [`CHARGE_LEN`] numbers (see
    /// [`charge_array`]), or empty when not charging.
    pub fn charge(&self) -> Vec<f64> {
        self.core
            .charge()
            .map_or_else(Vec::new, |c| charge_array(&c))
    }

    /// The own Leech Arc beam, drawn locally online instead of a round trip
    /// late (`crate::beam`): `[from x, y, z, to x, y, z, target]` (target
    /// 255 = none), or empty while the beam is off (and in practice, where
    /// the beam cues arrive at once).
    #[wasm_bindgen(js_name = localBeam)]
    pub fn local_beam(&mut self) -> Vec<f32> {
        self.core.local_beam().map_or_else(Vec::new, |b| {
            vec![
                b.from.x,
                b.from.y,
                b.from.z,
                b.to.x,
                b.to.y,
                b.to.z,
                b.target.map_or(255.0, f32::from),
            ]
        })
    }

    /// The own gadget for the HUD.
    pub fn gadget(&self) -> GadgetHud {
        gadget_hud(&self.core.gadget())
    }

    /// Ship types with numbers, rule set permission and loadout preview
    /// (JSON, see [`ship_choices_json`]).
    #[wasm_bindgen(js_name = shipChoices)]
    pub fn ship_choices(&self) -> String {
        ship_choices_json(&self.core.ship_choices())
    }

    /// Picks the ship type (`ShipType::index`) for the next respawn. Online
    /// it returns the `SelectShip` packet to send reliably. Throws for a
    /// type the rule set does not allow.
    #[wasm_bindgen(js_name = selectShip)]
    pub fn select_ship(&mut self, ship_type: u8) -> Result<Option<Vec<u8>>, JsError> {
        let ship = ls_sim::ShipType::from_index(ship_type)
            .ok_or_else(|| JsError::new(&format!("unknown ship type {ship_type}")))?;
        self.core.select_ship(ship).map_err(js_err)
    }

    /// The own Paint Thrower colour (`ls_math::terrain::PAINT_*`: 1 red,
    /// 2 blue, 3 yellow) while it is the selected primary, else 0. V
    /// (`Buttons::WEAPON_MODE`, bit 7 of the controls' buttons) cycles it in
    /// the simulation.
    /// Seconds until the own weapons are back online after a Borrowed Time
    /// rewind (0 = online).
    #[wasm_bindgen(getter, js_name = weaponsOfflineSecs)]
    pub fn weapons_offline_secs(&self) -> f32 {
        self.core.weapons_offline_secs()
    }

    #[wasm_bindgen(getter, js_name = paintColor)]
    pub fn paint_color(&self) -> u8 {
        self.core.paint_color()
    }

    /// The mode state as JSON (`modes::mode_state_json`): phase, timer,
    /// team and player scores, lives, meters, objective markers.
    #[wasm_bindgen(js_name = modeState)]
    pub fn mode_state(&self) -> String {
        modes::mode_state_json(&self.core.mode_state())
    }

    /// The lobby state as JSON (`modes::lobby_json`), `undefined` before
    /// the server sent one (and offline).
    #[wasm_bindgen(js_name = lobbyState)]
    pub fn lobby_state(&self) -> Option<String> {
        self.core.lobby().map(modes::lobby_json)
    }

    /// The match results as JSON (`modes::results_json`), `undefined` until
    /// the match ended.
    pub fn results(&self) -> Option<String> {
        self.core.results().map(modes::results_json)
    }

    /// True once after each match transition (a new `Welcome` on the open
    /// connection): the game now plays the next match. The old level's
    /// chunks are reported by `takeUnloads`; reset HUD and scene state.
    #[wasm_bindgen(js_name = takeMatchTransition)]
    pub fn take_match_transition(&mut self) -> bool {
        self.core.take_match_transition()
    }

    /// Match transitions handled so far.
    #[wasm_bindgen(getter)]
    pub fn transitions(&self) -> u32 {
        self.core.transitions() as u32
    }

    /// Room code of the match (empty for the public room).
    #[wasm_bindgen(getter, js_name = roomCode)]
    pub fn room_code(&self) -> String {
        self.core.room_code()
    }

    /// `Ready` packet for the lobby (send reliably; `undefined` offline).
    #[wasm_bindgen(js_name = readyPacket)]
    pub fn ready_packet(&self, ready: bool) -> Option<Vec<u8>> {
        self.core.ready_packet(ready)
    }

    /// `PickTeam` packet for the next match (`team` < 0 = auto-balance).
    #[wasm_bindgen(js_name = pickTeamPacket)]
    pub fn pick_team_packet(&self, team: i32) -> Option<Vec<u8>> {
        self.core.pick_team_packet(u8::try_from(team).ok())
    }

    /// `Spectate` packet (spectate from the next match on, or play again).
    #[wasm_bindgen(js_name = spectatePacket)]
    pub fn spectate_packet(&self, spectate: bool) -> Option<Vec<u8>> {
        self.core.spectate_packet(spectate)
    }

    #[wasm_bindgen(getter, js_name = isOnline)]
    pub fn is_online(&self) -> bool {
        self.core.is_online()
    }

    #[wasm_bindgen(getter, js_name = shareCode)]
    pub fn share_code(&self) -> String {
        self.core.share_code()
    }

    #[wasm_bindgen(getter)]
    pub fn player(&self) -> u8 {
        self.core.player()
    }

    #[wasm_bindgen(getter)]
    pub fn team(&self) -> u8 {
        self.core.team()
    }

    /// Level box `[minX, minY, minZ, maxX, maxY, maxZ]`.
    pub fn bounds(&self) -> Vec<f32> {
        let b = self.core.bounds();
        vec![b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z]
    }

    /// Sets the clock (ms, e.g. `performance.now()`) that times pings; call
    /// once per frame before handling that frame's packets.
    #[wasm_bindgen(js_name = setClockMs)]
    pub fn set_clock_ms(&mut self, ms: f64) {
        self.core.set_clock_ms(ms);
    }

    /// Handles one packet from the server. Returns the reason if the server
    /// ended the session, `undefined` otherwise; throws on a broken packet.
    #[wasm_bindgen(js_name = handlePacket)]
    pub fn handle_packet(&mut self, packet: &[u8]) -> Result<Option<String>, JsError> {
        self.core.handle_packet(packet).map_err(js_err)
    }

    /// Advances by `dt` seconds. `controls` = `[forward, right, up,
    /// rollRight, yawRight (rad), pitchUp (rad), buttons, weaponSelect,
    /// weaponKey (1-10), cyclePrimary (±1), cycleSecondary (+1)]`
    /// (`controls::CONTROLS_LEN`). Returns the packets to send, in order.
    pub fn frame(&mut self, dt: f64, controls: &[f32]) -> js_sys::Array {
        let out = self.core.frame(dt, Controls::from_slice(controls));
        out.iter()
            .map(|b| js_sys::Uint8Array::from(b.as_slice()))
            .collect()
    }

    /// Local ship `[px, py, pz, qx, qy, qz, qw, vx, vy, vz, alive]`, or
    /// `undefined` before the first snapshot.
    #[wasm_bindgen(js_name = localPose)]
    pub fn local_pose(&self) -> Option<Vec<f32>> {
        self.core.local_pose().map(|p| {
            vec![
                p.pos.x,
                p.pos.y,
                p.pos.z,
                p.rot.x,
                p.rot.y,
                p.rot.z,
                p.rot.w,
                p.vel.x,
                p.vel.y,
                p.vel.z,
                if p.alive { 1.0 } else { 0.0 },
            ]
        })
    }

    /// Other ships, [`SHIP_STRIDE`] floats each (see [`ships_array`]).
    #[wasm_bindgen(js_name = remoteShips)]
    pub fn remote_ships(&self) -> Vec<f32> {
        ships_array(&self.core.remote_ships())
    }

    /// Projectiles, [`PROJECTILE_STRIDE`] floats each (see
    /// [`projectiles_array`]).
    pub fn projectiles(&self) -> Vec<f32> {
        projectiles_array(&self.core.projectiles())
    }

    /// Entities, [`ENTITY_STRIDE`] floats each (see [`entities_array`]).
    pub fn entities(&self) -> Vec<f32> {
        entities_array(&self.core.entities())
    }

    /// The match's visual table, `scene::VISUAL_STRIDE` floats per visual
    /// id: `[shapeIndex (into visualShapeNames()), r, g, b (0..1), scale]`.
    /// Constant for a match; read it once.
    pub fn visuals(&self) -> Vec<f32> {
        scene::visual_rows(self.core.visuals())
    }

    /// Visual id of a weapon's projectiles (index into [`Game::visuals`]).
    #[wasm_bindgen(js_name = projectileVisual)]
    pub fn projectile_visual(&self, weapon: u8) -> u32 {
        u32::from(self.core.visuals().projectile(ls_sim::WeaponId(weapon)).0)
    }

    /// Players, [`PLAYER_STRIDE`] numbers each: `[id, team, shipType,
    /// gadget (255 none)]`; names in the same order from
    /// [`Game::player_names`].
    #[wasm_bindgen(js_name = playerInfo)]
    pub fn player_info(&self) -> Vec<f64> {
        let players = self.core.players();
        let mut v = Vec::with_capacity(players.len() * PLAYER_STRIDE);
        for p in &players {
            v.extend_from_slice(&[
                p.id as f64,
                p.team as f64,
                p.ship_type as f64,
                p.gadget as f64,
            ]);
        }
        v
    }

    /// Player names in [`Game::player_info`] order.
    #[wasm_bindgen(js_name = playerNames)]
    pub fn player_names(&self) -> Vec<String> {
        self.core.players().into_iter().map(|p| p.name).collect()
    }

    /// Carried weapons and gadget for the weapon HUD.
    pub fn loadout(&self) -> LoadoutHud {
        loadout_hud(&self.core.loadout())
    }

    pub fn hud(&self) -> HudState {
        let h = self.core.hud();
        HudState {
            hull: h.hull,
            hull_max: h.hull_max,
            energy: h.energy,
            energy_max: h.energy_max,
            boost: h.boost,
            boost_max: h.boost_max,
            scrap: h.scrap,
            scrap_max: h.scrap_max,
            weapon: h.weapon,
            ammo: h.ammo,
            secondary: h.secondary,
            paint0: h.paint[0],
            paint1: h.paint[1],
            time_left: h.time_left_secs,
            team: h.team,
            alive: h.alive,
            status: h.status,
        }
    }

    /// Kill-feed lines since the last call.
    #[wasm_bindgen(js_name = takeFeed)]
    pub fn take_feed(&mut self) -> Vec<String> {
        self.core.take_feed()
    }

    /// Debug counters, [`DEBUG_STATS_LEN`] numbers (see [`debug_stats_array`]).
    #[wasm_bindgen(js_name = debugStats)]
    pub fn debug_stats(&self) -> Vec<f64> {
        debug_stats_array(&self.core.debug_stats())
    }

    /// Scoreboard, [`SCORE_STRIDE`] numbers per row (see [`scoreboard_array`]).
    pub fn scoreboard(&self) -> Vec<f64> {
        scoreboard_array(&self.core.scoreboard())
    }

    /// Player names of the scoreboard rows, in row order.
    #[wasm_bindgen(js_name = scoreboardNames)]
    pub fn scoreboard_names(&self) -> Vec<String> {
        self.core.scoreboard().into_iter().map(|r| r.name).collect()
    }

    /// Match result, [`MATCH_INFO_LEN`] numbers (see [`match_info_array`]).
    #[wasm_bindgen(js_name = matchInfo)]
    pub fn match_info(&self) -> Vec<f64> {
        match_info_array(&self.core.match_info())
    }

    /// Meshes the next chunk around the camera (dirty chunks first), or
    /// `undefined` when all chunks in range are current. Call repeatedly
    /// within the frame's meshing budget.
    #[wasm_bindgen(js_name = meshNext)]
    pub fn mesh_next(&mut self, x: f32, y: f32, z: f32) -> Option<ChunkMesh> {
        let out = self.core.mesh_next(Vec3::new(x, y, z))?;
        let b = out.aabb;
        let surface = game::surface_flags(&out.mesh);
        let materials = game::material_ids(&out.mesh);
        let material_weights = game::material_weights(&out.mesh);
        let colors = game::paint_colors(&out.mesh);
        Some(ChunkMesh {
            materials,
            material_weights,
            colors,
            key: stream::chunk_key(out.coord),
            positions: out.mesh.positions.iter().flatten().copied().collect(),
            normals: out.mesh.normals.iter().flatten().copied().collect(),
            indices: out.mesh.indices,
            surface,
            teams: out.mesh.teams,
            ao: out.mesh.ao,
            aabb: vec![b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z],
        })
    }

    /// Keys of chunks to drop (far from the camera; after a match
    /// transition first every chunk of the previous level).
    #[wasm_bindgen(js_name = takeUnloads)]
    pub fn take_unloads(&mut self, x: f32, y: f32, z: f32) -> Vec<String> {
        self.core
            .take_unloads(Vec3::new(x, y, z))
            .into_iter()
            .map(stream::chunk_key)
            .collect()
    }

    /// Chunks waiting to be meshed around the camera.
    #[wasm_bindgen(js_name = pendingChunks)]
    pub fn pending_chunks(&mut self, x: f32, y: f32, z: f32) -> u32 {
        self.core.pending_chunks(Vec3::new(x, y, z)) as u32
    }

    #[wasm_bindgen(js_name = setViewRadius)]
    pub fn set_view_radius(&mut self, r: f32) {
        self.core.set_view_radius(r);
    }

    /// Makes every chunk mesh again (after the host dropped its meshes).
    #[wasm_bindgen(js_name = resetMeshes)]
    pub fn reset_meshes(&mut self) {
        self.core.reset_meshes();
    }

    /// Terrain edits applied so far.
    #[wasm_bindgen(getter, js_name = editCount)]
    pub fn edit_count(&self) -> u32 {
        self.core.edit_count()
    }

    /// Terrain desync hash as hex (expensive; debug only).
    #[wasm_bindgen(js_name = terrainHash)]
    pub fn terrain_hash(&self) -> String {
        format!("{:016x}", self.core.terrain_hash())
    }

    // ----- VQ1 scale cues (probe.rs) ----------------------------------------------

    /// Hitbox radius of the own ship type (u).
    #[wasm_bindgen(getter, js_name = shipRadius)]
    pub fn ship_radius(&self) -> f32 {
        self.core.tuning().radius
    }

    /// Distance from `(ox, oy, oz)` along `(dx, dy, dz)` to the first rock
    /// (or the level bounds), or -1 when nothing is hit within `max`.
    #[allow(clippy::too_many_arguments)]
    pub fn raycast(&self, ox: f32, oy: f32, oz: f32, dx: f32, dy: f32, dz: f32, max: f32) -> f32 {
        self.core
            .raycast(Vec3::new(ox, oy, oz), Vec3::new(dx, dy, dz), max)
            .unwrap_or(-1.0)
    }

    /// Sweeps a sphere of `radius` from `(ox, oy, oz)` along `(dx, dy, dz)`:
    /// `[travel, contact x, y, z, normal x, y, z]` at the first contact with
    /// rock (measured like the simulation's collision), or empty when the
    /// path is clear within `max`.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = sweepSphere)]
    pub fn sweep_sphere(
        &self,
        ox: f32,
        oy: f32,
        oz: f32,
        dx: f32,
        dy: f32,
        dz: f32,
        radius: f32,
        max: f32,
    ) -> Vec<f32> {
        self.core
            .sweep_sphere(Vec3::new(ox, oy, oz), Vec3::new(dx, dy, dz), radius, max)
            .map_or_else(Vec::new, |h| {
                vec![
                    h.travel, h.point.x, h.point.y, h.point.z, h.normal.x, h.normal.y, h.normal.z,
                ]
            })
    }

    /// Practice only (the visual gallery's fixed test shapes): carves
    /// (`add` false) or adds a sphere of rock outside the simulation.
    /// Returns false online.
    #[wasm_bindgen(js_name = practiceEdit)]
    pub fn practice_edit(&mut self, add: bool, x: f32, y: f32, z: f32, radius: f32) -> bool {
        let center = Vec3::new(x, y, z);
        let edit = if add {
            ls_math::terrain::TerrainEdit::AddSphere { center, radius }
        } else {
            ls_math::terrain::TerrainEdit::SubtractSphere { center, radius }
        };
        self.core.practice_edit(edit)
    }

    /// Practice only (the visual gallery's Squeeze scenes, M3 C1): puts The
    /// Squeeze's shell on the terrain (solid outside the sphere; a radius
    /// below 0 removes it) outside the simulation, so the next meshes show
    /// the wall. A practice frame's step sets the mode's own shell again.
    /// Returns false online.
    #[wasm_bindgen(js_name = practiceShell)]
    pub fn practice_shell(&mut self, x: f32, y: f32, z: f32, radius: f32) -> bool {
        let shell = (radius >= 0.0).then(|| ls_sim::ShellSphere {
            center: Vec3::new(x, y, z),
            radius,
        });
        self.core.practice_shell(shell)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tick_rate_is_60() {
        assert_eq!(super::tick_hz(), 60);
    }

    #[test]
    fn weapon_lists_are_in_id_order() {
        let ids = super::weapon_ids();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "WeaponTable sorts by id");
        assert_eq!(super::weapon_names().len(), ids.len());
    }

    #[test]
    fn overlay_arrays_have_the_documented_layout() {
        use crate::game::{DebugStats, MatchInfo, ScoreRow};
        let d = super::debug_stats_array(&DebugStats {
            ping_ms: None,
            corrected: 2,
            synced: true,
            ..Default::default()
        });
        assert_eq!(d.len(), super::DEBUG_STATS_LEN);
        assert_eq!((d[0], d[2], d[9]), (-1.0, 2.0, 1.0));
        let rows = [
            ScoreRow {
                id: 3,
                name: "Ada".into(),
                team: 1,
                alive: true,
                kills: 4,
                deaths: 1,
                is_you: false,
            },
            ScoreRow {
                id: 0,
                name: "You".into(),
                team: 0,
                alive: false,
                kills: 0,
                deaths: 2,
                is_you: true,
            },
        ];
        let s = super::scoreboard_array(&rows);
        assert_eq!(s.len(), 2 * super::SCORE_STRIDE);
        assert_eq!(&s[..6], &[3.0, 1.0, 1.0, 4.0, 1.0, 0.0]);
        assert_eq!(&s[6..], &[0.0, 0.0, 0.0, 0.0, 2.0, 1.0]);
        let m = super::match_info_array(&MatchInfo {
            over: true,
            winner: None,
            team_cells: [10, 20],
            surface_cells: 100,
            secs_since_end: 1.5,
        });
        assert_eq!(m, vec![1.0, -1.0, 10.0, 20.0, 100.0, 1.5]);
        assert_eq!(m.len(), super::MATCH_INFO_LEN);
    }

    #[test]
    fn scene_arrays_have_the_documented_layout() {
        use crate::game::{ProjectileView, ShipView};
        use crate::scene::EntityDraw;
        use ls_math::{Quat, Vec3};
        let ship = ShipView {
            id: 2,
            team: 1,
            alive: true,
            boosting: false,
            pos: Vec3::new(1.0, 2.0, 3.0),
            rot: Quat::IDENTITY,
            ship_type: 3,
            radius: 0.55,
            gadget_active: true,
        };
        let s = super::ships_array(&[ship]);
        assert_eq!(s.len(), super::SHIP_STRIDE);
        assert_eq!(&s[..5], &[2.0, 1.0, 1.0, 0.0, 1.0]);
        assert_eq!(&s[10..], &[1.0, 3.0, 0.55, 1.0]);
        let p = super::projectiles_array(&[ProjectileView {
            kind: 4,
            owner: 1,
            pos: Vec3::ZERO,
            vel: Vec3::X,
            visual: 17,
            state: 2,
        }]);
        assert_eq!(p.len(), super::PROJECTILE_STRIDE);
        assert_eq!((p[0], p[5], p[8], p[9]), (4.0, 1.0, 17.0, 2.0));
        let e = super::entities_array(&[EntityDraw {
            id: 300,
            kind: 5,
            visual: 2,
            owner: 255,
            team: 255,
            pos: Vec3::new(4.0, 5.0, 6.0),
            vel: Vec3::ZERO,
            rot: Quat::IDENTITY,
            state: [9, 8, 7, 6],
            radius: 1.25,
        }]);
        assert_eq!(e.len(), super::ENTITY_STRIDE);
        assert_eq!(&e[..8], &[300.0, 5.0, 2.0, 255.0, 255.0, 4.0, 5.0, 6.0]);
        assert_eq!(e[14], 1.0, "quaternion w");
        assert_eq!(&e[15..], &[9.0, 8.0, 7.0, 6.0, 1.25]);
    }

    #[test]
    fn name_lists_cover_every_kind_and_shape() {
        let kinds = super::entity_kind_names();
        assert_eq!(kinds.len(), 10);
        assert_eq!(kinds[ls_sim::ReplicatedKind::Wreck as usize], "wreck");
        assert_eq!(
            kinds[ls_sim::ReplicatedKind::DustCloud as usize],
            "dustCloud"
        );
        let shapes = super::visual_shape_names();
        assert_eq!(shapes.len(), crate::scene::SHAPES.len());
        assert_eq!(shapes[0], "bolt");
    }

    #[test]
    fn loadout_hud_flattens_primaries_then_secondaries() {
        use crate::inventory::{LoadoutView, WeaponEntry};
        use ls_sim::{WeaponId, WeaponSlot};
        let e = |name: &str, slot, selected, key: u8| WeaponEntry {
            shots: key as u32 * 10,
            state: key,
            weapon: WeaponId(0),
            name: name.into(),
            slot,
            ammo: "∞".into(),
            level: 2,
            max_level: 6,
            selected,
            key,
        };
        let v = LoadoutView {
            primaries: vec![
                e("A", WeaponSlot::Primary, false, 1),
                e("B", WeaponSlot::Primary, true, 2),
            ],
            secondaries: vec![e("R", WeaponSlot::Secondary, true, 0)],
            gadget: "Shield".into(),
            gadget_active: true,
        };
        let h = super::loadout_hud(&v);
        assert_eq!(h.names, ["A", "B", "R"]);
        assert_eq!(h.info.len(), 3 * super::LOADOUT_INFO_STRIDE);
        assert_eq!(&h.info[4..8], &[0, 1, 2, 2]);
        assert_eq!(&h.info[8..12], &[1, 1, 0, 2]);
        assert_eq!(h.max_levels, [6, 6, 6]);
        assert_eq!(h.shots, [10, 20, 0]);
        assert_eq!(h.states, [1, 2, 0]);
        assert!(h.gadget_active && h.gadget == "Shield");
    }

    #[test]
    fn share_code_info_mirrors_the_check() {
        let ok = super::check_share_code("LS0G-0000-0000-0001-AC58-T2XM-57ND");
        assert!(ok.ok);
        assert_eq!(ok.seed, "42");
        assert_eq!(ok.sliders.len(), 7);
        assert_eq!(ok.archetype, 0);
        let bad = super::check_share_code("nope");
        assert!(!bad.ok && !bad.error.is_empty());
    }
}
