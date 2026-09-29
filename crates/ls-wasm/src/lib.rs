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
//!   number key, wheel and X.
//!
//! This file only converts to JavaScript types. Flat arrays keep the
//! boundary cheap; their layouts are documented on each method and mirrored
//! by `web/src/wasm/api.ts`.

use wasm_bindgen::prelude::*;

pub mod controls;
pub mod game;
pub mod inventory;
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
}

/// Full share-code validation (checksum and generator version included).
#[wasm_bindgen(js_name = checkShareCode)]
pub fn check_share_code(text: &str) -> ShareCodeInfo {
    match sharecode::check_share_code(text) {
        CodeCheck::Valid {
            printed,
            seed,
            sliders,
            ..
        } => ShareCodeInfo {
            ok: true,
            code: printed,
            error: String::new(),
            seed: seed.to_string(),
            sliders: sliders.to_vec(),
        },
        CodeCheck::Invalid(error) => ShareCodeInfo {
            ok: false,
            code: String::new(),
            error,
            seed: String::new(),
            sliders: Vec::new(),
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

/// The first packet to send after the WebSocket opens (`Hello`).
/// `preferred_team` < 0 lets the server choose.
#[wasm_bindgen(js_name = helloPacket)]
pub fn hello_packet(name: &str, preferred_team: i32) -> Result<Vec<u8>, JsError> {
    let team = u8::try_from(preferred_team).ok();
    ls_net::Message::Hello(ls_game::client_hello(name, team, None))
        .encode()
        .map_err(|e| JsError::new(&e.to_string()))
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
/// `[slot (0 primary, 1 secondary), selected, key (1-10, 0 none), level]`.
#[wasm_bindgen(getter_with_clone)]
pub struct LoadoutHud {
    pub names: Vec<String>,
    pub ammo: Vec<String>,
    pub info: Vec<u8>,
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
        Some(ChunkMesh {
            key: stream::chunk_key(out.coord),
            positions: out.mesh.positions.iter().flatten().copied().collect(),
            normals: out.mesh.normals.iter().flatten().copied().collect(),
            indices: out.mesh.indices,
            surface,
            teams: out.mesh.teams,
            aabb: vec![b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z],
        })
    }

    /// Keys of chunks to drop (far from the camera).
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
        let e = |name: &str, slot, selected, key| WeaponEntry {
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
        assert!(h.gadget_active && h.gadget == "Shield");
    }

    #[test]
    fn share_code_info_mirrors_the_check() {
        let ok = super::check_share_code("LS08-0000-0000-000A-K4M6-8001-8F1T");
        assert!(ok.ok);
        assert_eq!(ok.seed, "42");
        assert_eq!(ok.sliders.len(), 7);
        let bad = super::check_share_code("nope");
        assert!(!bad.ok && !bad.error.is_empty());
    }
}
