//! A photographed receipt and the people its items get charged to.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtractionStatus {
    Pending,
    Extracting,
    Done,
    Failed,
}

impl ExtractionStatus {
    /// Whether the client should keep polling.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineItem {
    pub id: Uuid,
    pub description: String,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>,
    pub total: Decimal,
    pub position: i64,
    pub edited: bool,
    /// Who this is charged to. `None` is unassigned, which is also what a person
    /// being removed leaves behind.
    pub person_id: Option<Uuid>,
}

/// Someone line items can be charged to, as the settings screen manages them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
}

/// One person as the settings screen left them. The screen sends everyone it
/// ended up with, so anyone missing from the list was removed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonSave {
    /// `None` for someone added on screen and not yet written.
    pub id: Option<Uuid>,
    pub name: String,
    /// Empty means no description. Trimmed on the way in.
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub id: Uuid,
    pub purchased_on: jiff::civil::Date,
    pub merchant: String,
    pub subtotal: Option<Decimal>,
    pub tax: Option<Decimal>,
    /// `None` means the total is not yet known, never that it is zero.
    pub total: Option<Decimal>,
    pub currency: String,
    pub status: ExtractionStatus,
    pub extraction_error: Option<String>,
    pub reviewed: bool,
    pub line_items: Vec<LineItem>,
}

/// A whole receipt as the review screen left it — the header fields plus the
/// complete line-item list, saved in one go.
///
/// Money and dates travel as strings, exactly as typed, and the server parses them
/// with the same routines the extractor uses — so "$12.34" and "8/12/21" work, and
/// there's one parser rather than two. An empty string clears an optional field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptSave {
    pub id: Uuid,
    pub merchant: String,
    pub purchased_on: String,
    pub currency: String,
    pub subtotal: String,
    pub tax: String,
    pub total: String,
    /// Every item the receipt should end up with, in order. Anything missing
    /// from here was deleted on screen.
    pub items: Vec<LineItemSave>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineItemSave {
    /// `None` for a row added on the review screen and not yet written.
    pub id: Option<Uuid>,
    pub description: String,
    pub total: String,
    pub person_id: Option<Uuid>,
}

/// One row in a list of receipts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptSummary {
    pub id: Uuid,
    pub purchased_on: jiff::civil::Date,
    pub merchant: String,
    /// `None` when extraction could not read a total and nobody has fixed it.
    pub total: Option<Decimal>,
    /// ISO code. Carried so a row can be labelled with the right symbol — the
    /// extractor infers this from the receipt, so it is not always USD.
    pub currency: String,
    pub status: ExtractionStatus,
    pub item_count: usize,
    pub reviewed: bool,
    /// Computed by [`crate::shared::problems::problems_of`] on the server.
    /// Carried in full rather than as a count so the list can say what is wrong
    /// without a second round trip.
    pub problems: Vec<String>,
}
