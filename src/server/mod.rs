//! Server-only code: none of this compiles into the wasm bundle.
//!
//! [`queries`] holds everything that touches the database. The rest is what it
//! runs on: the tables and the connection, the model calls that read a receipt,
//! and the parsing and matching around them.

pub mod ask;
pub mod assign;
pub mod db;
pub mod disk;
pub mod env;
pub mod extract;
pub mod image;
pub mod job;
pub mod mappers;
pub mod matching;
pub mod models;
pub mod queries;
pub mod state;
pub mod statement_csv;
pub mod store;

#[cfg(test)]
pub mod testing;
