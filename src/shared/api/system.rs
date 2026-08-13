//! Server functions about the machine itself.

use leptos::prelude::*;

use crate::shared::dto;

#[cfg(feature = "ssr")]
use anyhow::Context as _;

/// Room left on the volume the photos and the database sit on.
#[server]
pub async fn disk_usage() -> Result<dto::DiskUsage, ServerFnError> {
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let root = state.store.root();

    crate::server::disk::usage(root)
        .with_context(|| format!("could not read the free space at {}", root.display()))
        .map_err(ServerFnError::new)
}
