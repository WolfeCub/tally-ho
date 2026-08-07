//! Three parts: [`shared`] is what both sides agree on, [`frontend`] is the UI,
//! and [`server`] is everything that never leaves the machine.

pub mod frontend;
pub mod shared;

// toasty and the SQLite driver would never build for wasm32 anyway.
#[cfg(feature = "ssr")]
pub mod server;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(crate::frontend::App);
}
