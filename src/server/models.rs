//! Persistence models.
//!
//! Server-only by design: `#[derive(toasty::Model)]` pulls in toasty and the
//! SQLite driver, neither of which belongs in a wasm bundle downloaded by a
//! phone. The types the UI actually sees are the plain serde structs in
//! [`crate::shared::dto`].

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
    /// under-report a statement, which is the exact failure this app exists to
    /// prevent. The real-world invariant is enforced at review time, not by the
    /// column type: a receipt with no total can't be matched to a charge.
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

/// Someone a line item can be charged to.
///
/// Reconciling a card statement means splitting it between the people who share
/// it, so who bought what has to be recorded item by item.
#[derive(Debug, toasty::Model)]
pub struct Person {
    #[key]
    #[auto]
    pub id: Uuid,

    pub name: String,
    pub description: Option<String>,

    #[default(jiff::Timestamp::now())]
    pub created_at: jiff::Timestamp,
}

/// One uploaded credit-card statement, the thing receipts get reconciled
/// against.
///
/// Stored rather than held in the browser because resolving a charge can mean
/// photographing a receipt, waiting for the model, and correcting it on the
/// review screen — work that outlives any page.
#[derive(Debug, toasty::Model)]
pub struct Statement {
    #[key]
    #[auto]
    pub id: Uuid,

    /// The uploaded filename, so a list of these is recognizable.
    pub label: String,
    /// ISO code every charge on it is in. Matching never crosses currencies: a
    /// CAD 45.00 receipt is not a USD 45.00 charge.
    pub currency: String,

    /// Earliest and latest charge on the file. Read off the rows rather than
    /// asked for — the statement already knows its own period.
    pub begins_on: jiff::civil::Date,
    pub ends_on: jiff::civil::Date,

    #[default(jiff::Timestamp::now())]
    pub imported_at: jiff::Timestamp,

    #[has_many]
    pub charges: toasty::Deferred<Vec<Charge>>,
}

/// One line off a statement. A payment or refund is a negative charge.
#[derive(Debug, toasty::Model)]
pub struct Charge {
    #[key]
    #[auto]
    pub id: Uuid,

    #[index]
    pub statement_id: Uuid,
    #[belongs_to]
    pub statement: toasty::Deferred<Statement>,

    /// Whichever date the file carried — the transaction date where there is
    /// one, since that is the day the receipt was printed.
    pub charged_on: jiff::civil::Date,
    pub description: String,
    /// Positive for a purchase, negative for a credit. Normalized on import:
    /// issuers disagree about which way round they write it.
    pub amount: Decimal,
    /// Row order in the file, so the screen reads like the statement.
    pub position: i64,

    /// The receipt this charge is accounted for by. Indexed because matching
    /// asks whether a receipt is already spoken for.
    #[index]
    pub receipt_id: Option<Uuid>,
    /// Whether a human has agreed to the match. Matching fills [`Self::receipt_id`]
    /// in by itself, which is a proposal and not an answer — an amount can match
    /// the wrong receipt and every total still balances.
    #[default(false)]
    pub confirmed: bool,
    /// Set when a charge is deliberately receiptless — a subscription, a fee.
    /// Without it those never resolve and a statement can't be finished.
    #[default(false)]
    pub no_receipt: bool,
    /// Whose a receiptless charge is; `None` splits it evenly. Read only when
    /// [`Self::no_receipt`] is set — a matched receipt splits by its own items.
    #[index]
    pub person_id: Option<Uuid>,
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

    /// Who this one is charged to, `None` until somebody says. Indexed because
    /// reconciliation asks "what did this person buy in this period".
    #[index]
    pub person_id: Option<Uuid>,

    /// Preserves the order printed on the receipt.
    pub position: i64,
    /// Set once a human corrects the model's guess.
    #[default(false)]
    pub edited: bool,
}
