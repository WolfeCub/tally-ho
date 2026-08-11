//! What the info screen reports about the running app.

use serde::{Deserialize, Serialize};

/// Room left where the receipts and the database live.
///
/// In the container that's the mounted volume rather than the image, so this is
/// the number that says whether the claim needs growing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskUsage {
    /// The directory asked about, as the server sees it.
    pub path: String,
    pub total_bytes: u64,
    /// What's actually still writable here, which on ext4 is a few percent short
    /// of unused — the rest is reserved for root. So `total - available` reads a
    /// little high against `df`, and errs the safe way for a gauge whose job is
    /// to warn.
    pub available_bytes: u64,
}
