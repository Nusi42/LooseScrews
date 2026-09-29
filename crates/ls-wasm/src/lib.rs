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
//!   or offline practice (a local `ls_sim::World`).
//!
//! This file only converts to JavaScript types. Flat arrays keep the
//! boundary cheap; their layouts are documented on each method and mirrored
//! by `web/src/wasm/api.ts`.

use wasm_bindgen::prelude::*;

pub mod controls;
pub mod game;
pub mod sharecode;
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
    ls_net::Message::Hello(ls_net::Hello::new(name, team))
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
    /// Chunk box `[minX, minY, minZ, maxX, maxY, maxZ]`.
    pub fn aabb(&self) -> Vec<f32> {
        self.aabb.clone()
    }
}

/// Floats per ship in [`Game::remote_ships`].
pub const SHIP_STRIDE: usize = 11;
/// Floats per projectile in [`Game::projectiles`].
pub const PROJECTILE_STRIDE: usize = 8;

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

    /// Handles one packet from the server. Returns the reason if the server
    /// ended the session, `undefined` otherwise; throws on a broken packet.
    #[wasm_bindgen(js_name = handlePacket)]
    pub fn handle_packet(&mut self, packet: &[u8]) -> Result<Option<String>, JsError> {
        self.core.handle_packet(packet).map_err(js_err)
    }

    /// Advances by `dt` seconds. `controls` = `[forward, right, up,
    /// rollRight, yawRight (rad), pitchUp (rad), buttons, weaponSelect]`.
    /// Returns the packets to send, in order.
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

    /// Other ships, [`SHIP_STRIDE`] floats each: `[id, team, alive,
    /// boosting, px, py, pz, qx, qy, qz, qw]`.
    #[wasm_bindgen(js_name = remoteShips)]
    pub fn remote_ships(&self) -> Vec<f32> {
        let ships = self.core.remote_ships();
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
            ]);
        }
        v
    }

    /// Projectiles, [`PROJECTILE_STRIDE`] floats each: `[kind, owner, px,
    /// py, pz, vx, vy, vz]`.
    pub fn projectiles(&self) -> Vec<f32> {
        let shots = self.core.projectiles();
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
            ]);
        }
        v
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

    /// Meshes the next chunk around the camera (dirty chunks first), or
    /// `undefined` when all chunks in range are current. Call repeatedly
    /// within the frame's meshing budget.
    #[wasm_bindgen(js_name = meshNext)]
    pub fn mesh_next(&mut self, x: f32, y: f32, z: f32) -> Option<ChunkMesh> {
        let out = self.core.mesh_next(Vec3::new(x, y, z))?;
        let b = out.aabb;
        Some(ChunkMesh {
            key: stream::chunk_key(out.coord),
            positions: out.mesh.positions.iter().flatten().copied().collect(),
            normals: out.mesh.normals.iter().flatten().copied().collect(),
            indices: out.mesh.indices,
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
    fn share_code_info_mirrors_the_check() {
        let ok = super::check_share_code("LS04-0000-0000-000A-K4M6-8001-8F1P");
        assert!(ok.ok);
        assert_eq!(ok.seed, "42");
        assert_eq!(ok.sliders.len(), 7);
        let bad = super::check_share_code("nope");
        assert!(!bad.ok && !bad.error.is_empty());
    }
}
