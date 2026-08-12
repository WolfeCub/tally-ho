//! What's wrong with a receipt.
//!
//! Run on the server and carried down as sentences, so the review screen and the
//! lists say the same thing without either of them working it out.

use rust_decimal::Decimal;

use crate::shared::dto::{LineItem, Receipt};

pub fn line_item_sum(items: &[LineItem]) -> Decimal {
    items.iter().map(|i| i.total).sum()
}

/// Everything wrong with a receipt, in the order a human should care. A list,
/// not a bool, so the review screen can say *what* is wrong.
///
/// Note what isn't checked: line items against the total. Items are pre-tax, so
/// on a taxed receipt that always looks off by the tax.
pub fn problems_of(
    subtotal: Option<Decimal>,
    tax: Option<Decimal>,
    total: Option<Decimal>,
    items: &[LineItem],
) -> Vec<String> {
    let mut out = Vec::new();

    if total.is_none() {
        out.push("No total was read — reconciliation needs one.".to_string());
    }

    // Items should reconstruct the subtotal (pre-tax). A gap here means a line
    // was dropped, duplicated, or misread.
    if let Some(subtotal) = subtotal
        && !items.is_empty()
    {
        let sum = line_item_sum(items);
        let diff = subtotal - sum;
        if !diff.is_zero() {
            out.push(format!(
                "Line items add up to {sum}, but the subtotal says {subtotal} (off by {diff})."
            ));
        }
    }

    // subtotal + tax should equal what was actually charged.
    if let (Some(sub), Some(tax), Some(total)) = (subtotal, tax, total) {
        let diff = sub + tax - total;
        if !diff.is_zero() {
            out.push(format!(
                "{sub} + {tax} tax = {}, but the total says {total} (off by {diff}).",
                sub + tax
            ));
        }
    }

    if items.iter().any(|i| i.total.is_zero()) {
        out.push("One or more line items have no amount — they were unreadable.".to_string());
    }

    out
}

impl Receipt {
    /// Sum of the line items, which is *not* [`Receipt::total`] on a taxed
    /// receipt — it should match [`Receipt::subtotal`].
    pub fn line_item_sum(&self) -> Decimal {
        line_item_sum(&self.line_items)
    }

    pub fn problems(&self) -> Vec<String> {
        problems_of(self.subtotal, self.tax, self.total, &self.line_items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::dto::ExtractionStatus;
    use std::str::FromStr;
    use uuid::Uuid;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn item(total: &str) -> LineItem {
        LineItem {
            id: Uuid::nil(),
            description: "x".into(),
            quantity: None,
            unit_price: None,
            total: dec(total),
            position: 0,
            edited: false,
            person_id: None,
            guessed_why: None,
        }
    }

    fn receipt(
        subtotal: Option<&str>,
        tax: Option<&str>,
        total: Option<&str>,
        items: Vec<LineItem>,
    ) -> Receipt {
        Receipt {
            id: Uuid::nil(),
            purchased_on: jiff::civil::date(2026, 7, 1),
            merchant: "M".into(),
            subtotal: subtotal.map(dec),
            tax: tax.map(dec),
            total: total.map(dec),
            currency: "USD".into(),
            status: ExtractionStatus::Done,
            extraction_error: None,
            reviewed: false,
            line_items: items,
        }
    }

    /// A taxed receipt that balances perfectly must not be flagged. This is the
    /// case a naive items-vs-total comparison gets wrong, reporting a difference
    /// exactly equal to the tax on every correct receipt.
    #[test]
    fn tax_is_not_mistaken_for_a_mismatch() {
        // 32.82 of items, + 2.54 tax = 35.36 charged.
        let r = receipt(
            Some("32.82"),
            Some("2.54"),
            Some("35.36"),
            vec![
                item("11.72"),
                item("2.96"),
                item("3.98"),
                item("2.44"),
                item("11.72"),
            ],
        );
        assert_eq!(r.line_item_sum(), dec("32.82"));
        assert!(r.problems().is_empty(), "got {:?}", r.problems());
    }

    #[test]
    fn a_missing_total_is_a_problem() {
        let r = receipt(Some("10.00"), None, None, vec![item("10.00")]);
        assert_eq!(r.problems().len(), 1);
        assert!(r.problems()[0].contains("No total"));
    }

    /// A dropped or duplicated line item shows up here and nowhere else.
    #[test]
    fn items_not_matching_the_subtotal_is_a_problem() {
        let r = receipt(
            Some("32.82"),
            Some("2.54"),
            Some("35.36"),
            vec![item("11.72"), item("2.96")], // three items missing
        );
        let p = r.problems();
        assert!(
            p.iter().any(|s| s.contains("Line items add up to 14.68")),
            "got {p:?}"
        );
    }

    #[test]
    fn subtotal_plus_tax_not_matching_the_total_is_a_problem() {
        let r = receipt(
            Some("10.00"),
            Some("1.00"),
            Some("99.00"),
            vec![item("10.00")],
        );
        let p = r.problems();
        assert!(p.iter().any(|s| s.contains("off by -88")), "got {p:?}");
    }

    /// An unreadable amount is stored as zero rather than guessed, so it has to
    /// be called out explicitly — it would otherwise look like a free item.
    #[test]
    fn a_zero_amount_line_item_is_flagged() {
        let r = receipt(
            Some("10.00"),
            None,
            Some("10.00"),
            vec![item("10.00"), item("0")],
        );
        let p = r.problems();
        assert!(p.iter().any(|s| s.contains("no amount")), "got {p:?}");
    }

    #[test]
    fn problems_accumulate_rather_than_short_circuiting() {
        // No total AND items that don't match the subtotal.
        let r = receipt(Some("50.00"), None, None, vec![item("10.00")]);
        assert_eq!(r.problems().len(), 2, "got {:?}", r.problems());
    }
}
