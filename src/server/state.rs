//! Shared server state, handed to server functions via Leptos context.

use std::sync::Arc;

use super::extract::{Config, OllamaExtractor, ReceiptExtractor};
use super::store::Store;

const DEFAULT_CURRENCY: &str = "USD";

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
    /// The ISO code imported statements are in, and what a receipt is taken to
    /// be in when it doesn't print one. One card, one currency — so it's
    /// deployment config rather than a question on every upload.
    pub currency: String,
}

impl AppState {
    pub async fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let config = Config::from_env();
        let currency = currency_from_env()?;
        tracing::info!(
            model = %config.label(),
            url = %config.url,
            max_image_edge = config.max_image_edge,
            ocr_context = config.ocr_context,
            %currency,
            "extraction configured"
        );

        Ok(Self {
            db: crate::server::db::connect().await?,
            extractor: Arc::new(OllamaExtractor::new(config)?),
            store: Arc::new(Store::from_env()),
            currency,
        })
    }
}

/// Refuses to start on a code the ISO list doesn't have: matching never crosses
/// currencies, so a typo would silently match nothing at all.
fn currency_from_env() -> Result<String, Box<dyn std::error::Error>> {
    let code = crate::server::env::string("CURRENCY", DEFAULT_CURRENCY)
        .trim()
        .to_uppercase();

    if iso_currency::Currency::from_code(&code).is_none() {
        return Err(
            format!("CURRENCY={code:?} isn't an ISO currency code — try USD or CAD").into(),
        );
    }
    Ok(code)
}
