//! Everything that reads or writes the database, one module per table.
//!
//! A query across tables belongs to the one it is mostly about:
//! [`receipts::spare`] filters receipts by what the charges point at.
//!
//! The server functions in [`crate::shared::api`] call these and query nothing
//! themselves, which keeps each transaction in one place. Rows become wire types
//! on the way out, in [`crate::server::mappers`].

use std::collections::HashMap;
use std::hash::Hash;

pub mod charges;
pub mod line_items;
pub mod people;
pub mod receipts;
pub mod statements;

/// Child rows bucketed by the parent they belong to.
///
/// toasty has no join and no `IN` loading, so the obvious way to fetch children
/// is a query per parent — which the list screens would then run a hundred of,
/// every time they poll while something is being read. Reading the child table
/// whole is a few thousand rows against a local SQLite file, and it costs the
/// same whether the batch is one row or a hundred.
fn group_by<T, K: Eq + Hash>(rows: Vec<T>, parent: impl Fn(&T) -> K) -> HashMap<K, Vec<T>> {
    let mut grouped: HashMap<K, Vec<T>> = HashMap::new();
    for row in rows {
        grouped.entry(parent(&row)).or_default().push(row);
    }
    grouped
}
