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
/// Note what isn't checked: the subtotal itself. It means the pre-tax total on
/// one receipt and only the goods on the next, so there's no one figure to hold
/// it to, and nothing computes with it.
pub fn problems_of(
    subtotal: Option<Decimal>,
    tax: Option<Decimal>,
    total: Option<Decimal>,
    items: &[LineItem],
) -> Vec<String> {
    let mut out = Vec::new();

    if total.is_none() {
        out.push("No total was read: reconciliation needs one.".to_string());
    }

    // Items should reconstruct the pre-tax figure. A gap here means a line was
    // dropped, duplicated, or misread.
    if let Some(subtotal) = subtotal
        && !items.is_empty()
    {
        // `total - tax` on any receipt. The printed subtotal is only that figure
        // when nothing was added below it, which an invoice often does, so it's
        // the fallback. A receipt that printed no subtotal is left unchecked.
        let pre_tax = match (tax, total) {
            (Some(tax), Some(total)) => total - tax,
            _ => subtotal,
        };

        let sum = line_item_sum(items);
        let diff = pre_tax - sum;
        if !diff.is_zero() {
            out.push(format!(
                "Line items add up to {sum}, but the receipt comes to {pre_tax} before tax \
                 (off by {diff})."
            ));
        }
    }

    if items.iter().any(|i| i.total.is_zero()) {
        out.push("One or more line items have no amount: they were unreadable.".to_string());
    }

    out
}

impl Receipt {
    /// Sum of the line items, which is *not* [`Receipt::total`] on a taxed
    /// receipt: it should match `total - tax`.
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
    use crate::shared::testing::{self, dec, item};

    /// The checks are all about these three fields against the items.
    fn receipt(
        subtotal: Option<&str>,
        tax: Option<&str>,
        total: Option<&str>,
        items: Vec<LineItem>,
    ) -> Receipt {
        Receipt {
            subtotal: subtotal.map(dec),
            tax: tax.map(dec),
            ..testing::receipt(total, items)
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
    fn items_not_matching_the_total_after_tax_is_a_problem() {
        let r = receipt(
            Some("32.82"),
            Some("2.54"),
            Some("35.36"),
            vec![item("11.72"), item("2.96")], // three items missing
        );
        let p = r.problems();
        assert!(
            p.iter().any(|s| s.contains("comes to 32.82 before tax")),
            "got {p:?}"
        );
        assert!(p.iter().any(|s| s.contains("off by 18.14")), "got {p:?}");
    }

    /// Without a tax there's nothing to derive the pre-tax figure from, so the
    /// printed subtotal is what the items are held to. The gap says which was
    /// used: against the 35.36 total it would be 23.64.
    #[test]
    fn items_are_checked_against_the_subtotal_when_no_tax_was_read() {
        let r = receipt(Some("32.82"), None, Some("35.36"), vec![item("11.72")]);
        let p = r.problems();
        assert!(p.iter().any(|s| s.contains("off by 21.10")), "got {p:?}");
    }

    /// Voilà prints a subtotal covering only the groceries, then adds a delivery
    /// fee, a bag charge and a voucher below it. Those are line items, so they
    /// sum past the printed subtotal, and the receipt is still right.
    #[test]
    fn a_subtotal_that_excludes_fees_is_not_a_problem() {
        let r = receipt(
            Some("80.78"), // groceries only
            Some("0.07"),
            Some("81.35"),
            vec![
                item("80.78"),
                item("11.99"),  // delivery
                item("0.50"),   // bag charge
                item("-11.99"), // free-delivery voucher
            ],
        );
        assert_eq!(r.line_item_sum(), dec("81.28"));
        assert!(r.problems().is_empty(), "got {:?}", r.problems());
    }

    /// Nothing is held to the subtotal, so a nonsensical one is not a problem
    /// on its own: the items and the total agree, and the split follows those.
    #[test]
    fn a_subtotal_nothing_agrees_with_is_left_alone() {
        let r = receipt(
            Some("0.01"),
            Some("1.00"),
            Some("11.00"),
            vec![item("10.00")],
        );
        assert!(r.problems().is_empty(), "got {:?}", r.problems());
    }

    /// An unreadable amount is stored as zero rather than guessed, so it has to
    /// be called out explicitly: it would otherwise look like a free item.
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

    /// A receipt whose items, subtotal, tax and total all agree is flagged for
    /// nothing. The refund slip is here because those figures go negative
    /// together, and a tax of zero because plenty of receipts print one.
    #[test]
    fn a_receipt_where_every_figure_agrees_is_never_flagged() {
        for (subtotal, tax) in [
            ("32.82", "2.54"),
            ("19.99", "0.00"),
            ("100.00", "13.00"),
            ("-5.00", "-0.65"), // a refund slip
        ] {
            let total = dec(subtotal) + dec(tax);
            let r = receipt(
                Some(subtotal),
                Some(tax),
                Some(&total.to_string()),
                vec![item(subtotal)],
            );
            assert!(
                r.problems().is_empty(),
                "{subtotal}+{tax}: {:?}",
                r.problems()
            );
        }
    }

    #[test]
    fn problems_accumulate_rather_than_short_circuiting() {
        // No total AND items that don't match the subtotal.
        let r = receipt(Some("50.00"), None, None, vec![item("10.00")]);
        assert_eq!(r.problems().len(), 2, "got {:?}", r.problems());
    }
}
