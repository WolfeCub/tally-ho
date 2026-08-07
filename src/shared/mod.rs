//! `dto` is the vocabulary the two halves agree on. `api` is the contract —
//! `#[server]` becomes a network call on the client and the real body on the
//! server, which is why it lives here and not under `server`.

pub mod api;
pub mod dto;
