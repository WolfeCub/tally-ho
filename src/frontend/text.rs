//! Wording that would otherwise be a ternary in the middle of the markup.

use rust_decimal::Decimal;

use crate::frontend::money::money;
use crate::shared::dto::ExtractionStatus;

/// "1 receipt", "2 receipts". Every noun this app counts is regular.
pub fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// The merchant, or a stand-in. The model leaves it blank when it couldn't read
/// one, and an empty space reads as a broken row rather than an unnamed shop.
pub fn merchant(name: &str) -> &str {
    if name.is_empty() {
        "(no merchant)"
    } else {
        name
    }
}

/// What goes where the total goes: the amount, or why there isn't one.
///
/// A receipt still being read hasn't got one *yet*, which is a different thing
/// from a receipt that was read and had none — the first is worth waiting for.
pub fn total_or_why(total: Option<Decimal>, currency: &str, status: ExtractionStatus) -> String {
    match total {
        Some(total) => money(total, currency),
        None if !status.is_terminal() => "reading…".to_string(),
        None => "no total".to_string(),
    }
}
