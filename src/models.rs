//! Persistence models.
//!
//! Server-only by design: `#[derive(toasty::Model)]` pulls in toasty and the
//! SQLite driver, neither of which belongs in a wasm bundle downloaded by a
//! phone. The types the UI actually sees are the plain serde structs in
//! [`crate::dto`].

use rust_decimal::Decimal;
use uuid::Uuid;

#[derive(Debug, toasty::Model)]
pub struct Receipt {
    #[key]
    #[auto]
    pub id: Uuid,

    /// Calendar date printed on the receipt — the field statement periods filter
    /// on. A civil date, not a timestamp: a receipt is dated, not instantaneous,
    /// and we never want a timezone shifting a purchase across a period
    /// boundary. Indexed because the range query is the app's whole purpose.
    #[index]
    pub purchased_on: jiff::civil::Date,

    pub merchant: String,

    pub subtotal: Option<Decimal>,
    pub tax: Option<Decimal>,
    /// Nullable, meaning "not yet established" — **not** "zero".
    ///
    /// Every real receipt has a total, but extraction can fail to read one, and
    /// the row still has to exist so the image is stored and a human can fix
    /// it. Defaulting to `0` here would let an unreadable receipt silently
    /// under-report a statement period, which is the exact failure this app
    /// exists to prevent. The real-world invariant is enforced at review time,
    /// not by the column type. See [`crate::dto::PeriodTotal`].
    pub total: Option<Decimal>,
    pub currency: String,

    /// Path relative to `DATA_DIR`. The image bytes stay on disk — SQLite would
    /// bloat badly holding full-resolution phone photos.
    pub image_path: String,

    #[default(ExtractionStatus::Pending)]
    pub status: ExtractionStatus,
    /// Populated only when `status` is `Failed`.
    pub extraction_error: Option<String>,
    /// Which Ollama model produced this, so a bad batch is traceable.
    pub model_used: Option<String>,
    /// Raw model output, kept for debugging bad extractions. Never queried, so
    /// opaque text storage is fine — `Json<T>` requires an explicit column type.
    #[column(type = text)]
    pub raw_response: Option<toasty::Json<serde_json::Value>>,

    /// Set when a human has checked this receipt against the photo. Gated on
    /// having a total: an unreviewed receipt is a known-unknown, whereas one
    /// marked reviewed with no total would be a lie the period view trusts.
    pub reviewed_at: Option<jiff::Timestamp>,

    #[default(jiff::Timestamp::now())]
    pub created_at: jiff::Timestamp,
    #[update(jiff::Timestamp::now())]
    pub updated_at: jiff::Timestamp,

    #[has_many]
    pub line_items: toasty::Deferred<Vec<LineItem>>,
}

/// Extraction runs in the background (a 7B vision model takes far too long to
/// block an HTTP response on), so a receipt's progress has to be persisted for
/// the client to poll.
#[derive(Debug, PartialEq, toasty::Embed)]
pub enum ExtractionStatus {
    Pending,
    Extracting,
    Done,
    Failed,
}

#[derive(Debug, toasty::Model)]
pub struct LineItem {
    #[key]
    #[auto]
    pub id: Uuid,

    #[index]
    pub receipt_id: Uuid,
    #[belongs_to]
    pub receipt: toasty::Deferred<Receipt>,

    pub description: String,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>,
    pub total: Decimal,

    /// Preserves the order printed on the receipt.
    pub position: i64,
    /// Set once a human corrects the model's guess.
    #[default(false)]
    pub edited: bool,
}
