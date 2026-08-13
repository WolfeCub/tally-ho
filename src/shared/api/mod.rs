//! The server functions, split by screen and re-exported here.
//!
//! Each one parses what came in, calls a query in [`crate::server::queries`], and
//! turns an error into a message the screen can show.
//!
//! This all builds for wasm too, so every server-only import has to sit inside a
//! function body, or behind a `cfg` if more than one body needs it.

pub mod people;
pub mod receipts;
pub mod statements;
pub mod system;

#[cfg(feature = "ssr")]
mod support;

pub use people::*;
pub use receipts::*;
pub use statements::*;
pub use system::*;
