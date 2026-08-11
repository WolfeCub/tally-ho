//! An imported card statement and the charges on it.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ExtractionStatus, Person};

/// What came of reading an uploaded file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Imported {
    pub id: Uuid,
    /// The date, description and amount columns the sniffer used, named as the
    /// file printed them. Shown, so a wrong guess is visible.
    pub columns: [String; 3],
    pub charge_count: usize,
    /// Rows that couldn't be read, and why.
    pub skipped: Vec<String>,
}

/// One row in the list of imported statements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatementSummary {
    pub id: Uuid,
    pub label: String,
    pub begins_on: jiff::civil::Date,
    pub ends_on: jiff::civil::Date,
    pub currency: String,
    pub charge_count: usize,
    /// How many charges a human has signed off, so a list row can show how far
    /// through a statement you are.
    pub settled_count: usize,
}

/// A receipt offered for a charge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub receipt_id: Uuid,
    pub merchant: String,
    pub purchased_on: jiff::civil::Date,
    pub total: Decimal,
    pub currency: String,
    /// Why it's being offered — "exact amount", "same day".
    pub why: String,
    /// The statement's line for the charge names this merchant. Only ever
    /// evidence for the receipt: most lines name nothing recognisable, so a
    /// `false` says nothing either way.
    pub same_merchant: bool,
    /// Good enough to have been matched on its own. Two of these means neither
    /// was, unless the merchant tells them apart.
    pub confident: bool,
}

/// The receipt a charge is matched to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Matched {
    pub receipt_id: Uuid,
    pub merchant: String,
    pub purchased_on: jiff::civil::Date,
    /// The receipt's own total. It won't equal the charge when a tip was added,
    /// which is fine — the charge is what gets split.
    pub total: Option<Decimal>,
    pub status: ExtractionStatus,
    pub reviewed: bool,
    /// From [`crate::shared::problems::problems_of`]. A receipt whose items
    /// don't add up splits badly, and this is the only warning of it.
    pub problems: Vec<String>,
}

/// How a charge is accounted for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Resolution {
    /// Nothing accounts for this yet.
    Unresolved,
    /// Matched by amount and date, waiting for someone to agree. Filled in for
    /// convenience, never treated as an answer: an amount can match the wrong
    /// receipt and leave every total balancing.
    Proposed(Matched),
    /// A human said this receipt pays for this charge.
    Confirmed(Matched),
    /// Deliberately receiptless — a subscription, a fee, interest. `None` splits
    /// it evenly.
    NoReceipt { person_id: Option<Uuid> },
}

impl Resolution {
    /// Whether a human has signed this row off. Only these reach the export.
    pub fn is_settled(&self) -> bool {
        matches!(self, Self::Confirmed(_) | Self::NoReceipt { .. })
    }

    pub fn receipt(&self) -> Option<&Matched> {
        match self {
            Self::Proposed(matched) | Self::Confirmed(matched) => Some(matched),
            _ => None,
        }
    }
}

/// What a human decided about one charge.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Resolve {
    /// This receipt pays for it. Also how a proposal is agreed to — the client
    /// names the receipt it actually saw, so there's no guessing which.
    Receipt(Uuid),
    /// No receipt is coming. `None` splits it evenly.
    NoReceipt { person_id: Option<Uuid> },
    /// Back to unresolved.
    Clear,
}

/// What one person owes for one charge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Share {
    pub person_id: Uuid,
    pub amount: Decimal,
}

/// One line off a statement, with everything needed to resolve it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Charge {
    pub id: Uuid,
    pub charged_on: jiff::civil::Date,
    pub description: String,
    /// Positive for a purchase, negative for a refund.
    pub amount: Decimal,
    pub resolution: Resolution,
    /// Receipts worth offering, best first. Empty once one is attached.
    pub suggestions: Vec<Candidate>,
    /// What each person owes, in [`Statement::people`] order. Empty while
    /// nothing accounts for the charge.
    ///
    /// Filled in for a proposal too — seeing the figures is how you decide
    /// whether to agree — but left out of the totals and the export until it's
    /// confirmed.
    pub split: Vec<Share>,
}

/// A statement and everything needed to reconcile it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Statement {
    pub id: Uuid,
    pub label: String,
    pub currency: String,
    pub begins_on: jiff::civil::Date,
    pub ends_on: jiff::civil::Date,
    /// In file order, so the screen reads like the statement.
    pub charges: Vec<Charge>,
    /// Everyone charges get split between, in the order their columns come out.
    pub people: Vec<Person>,
}
