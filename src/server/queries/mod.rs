//! Everything that reads or writes the database, one module per table.
//!
//! A query across tables belongs to the one it is mostly about:
//! [`receipts::spare`] filters receipts by what the charges point at.
//!
//! The server functions in [`crate::shared::api`] call these and query nothing
//! themselves, which keeps each transaction in one place. Rows become wire types
//! on the way out, in [`crate::server::mappers`].

pub mod charges;
pub mod line_items;
pub mod people;
pub mod receipts;
pub mod statements;
