//! `dto` is the vocabulary the two halves agree on. `api` is the contract —
//! `#[server]` becomes a network call on the client and the real body on the
//! server, which is why it lives here and not under `server`.
//!
//! The rest is what the vocabulary means, shared for the same reason: the screen
//! and the export have to reach the same figures.

pub mod api;
pub mod dto;
pub mod export;
pub mod problems;
pub mod reconcile;

#[cfg(test)]
pub mod testing;
