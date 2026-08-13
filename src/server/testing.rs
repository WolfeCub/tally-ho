//! Fixtures for the tests that touch the database. The wire-type ones are in
//! [`crate::shared::testing`].

use rust_decimal::Decimal;
use std::str::FromStr;

pub fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

/// A database of its own per test, schema and all. In-memory, so nothing is
/// shared between tests and nothing is left behind.
pub async fn memory_db() -> toasty::Db {
    crate::server::db::connect_url("sqlite::memory:")
        .await
        .unwrap()
}
