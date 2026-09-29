//! wasm-bindgen bindings for the Loose Screws browser client.
//!
//! Planned contents (see `docs/implementation-plan.md` section 4.6): share-code
//! decoding, chunk generation and meshing returned as typed arrays, terrain edits
//! and client-side prediction for the local ship.

use wasm_bindgen::prelude::*;

/// Installs a panic hook that forwards Rust panics to `console.error`.
/// Call once from JavaScript right after the module is initialised.
#[wasm_bindgen]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Returns the simulation tick rate in Hz, so the client and server agree.
#[wasm_bindgen]
pub fn tick_hz() -> u32 {
    ls_math::TICK_HZ
}

#[cfg(test)]
mod tests {
    #[test]
    fn tick_rate_is_60() {
        assert_eq!(super::tick_hz(), 60);
    }
}
