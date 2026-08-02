//! Shared server state, handed to server functions via Leptos context.

use std::sync::Arc;

use super::extract::{Config, OllamaExtractor, ReceiptExtractor};
use super::store::Store;

/// Cheap to clone: `toasty::Db` is an `Arc` over a shared pool, and the other
/// two are behind `Arc`. Every request clones this because toasty queries take
/// `&mut Db`.
#[derive(Clone)]
pub struct AppState {
    pub db: toasty::Db,
    /// A trait object, so the rig/Ollama implementation can be swapped for a
    /// fake in tests or a hosted model later without touching call sites.
    pub extractor: Arc<dyn ReceiptExtractor>,
    pub store: Arc<Store>,
}

impl AppState {
    pub async fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let config = Config::from_env();
        tracing::info!(
            model = %config.model,
            url = %config.url,
            max_image_edge = config.max_image_edge,
            "extraction configured"
        );

        Ok(Self {
            db: crate::db::connect().await?,
            extractor: Arc::new(OllamaExtractor::new(config)?),
            store: Arc::new(Store::from_env()),
        })
    }
}
