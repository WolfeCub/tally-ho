//! Server functions about the machine itself.

use leptos::prelude::*;

use crate::shared::dto;

#[cfg(feature = "ssr")]
use super::support::Reported as _;

/// Room left on the volume the photos and the database sit on.
#[server]
pub async fn disk_usage() -> Result<dto::DiskUsage, ServerFnError> {
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let root = state.store.root();

    crate::server::disk::usage(root).reported_as(&format!(
        "could not read the free space at {}",
        root.display()
    ))
}
