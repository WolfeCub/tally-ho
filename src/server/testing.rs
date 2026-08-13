//! The database fixture. The wire types have their own, in
//! [`crate::shared::testing`].

/// A database of its own per test, schema and all. In-memory, so nothing is
/// shared between tests and nothing is left behind.
pub async fn memory_db() -> toasty::Db {
    crate::server::db::connect_url("sqlite::memory:")
        .await
        .unwrap()
}
