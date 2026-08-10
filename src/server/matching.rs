//! Which receipt pays for which charge.
//!
//! The amount does nearly all the work; the date only bounds the search. Nothing
//! here reads the merchant — statement descriptions are mangled ("SQ *BLUE
//! BOTTLE"), and two receipts with the same total on the same day is not a case
//! worth guessing at.

use rust_decimal::Decimal;

use crate::shared::dto;

/// How many days a receipt may predate the charge that came from it. Cards post
/// a purchase the same day or a couple of days later; the day *after* is for a
/// late-night purchase landing on the next date.
const EARLIEST: i32 = -1;
const LATEST: i32 = 4;

/// The most receipts to offer for one charge. Past a handful it's a list to read
/// rather than an answer.
const MOST: usize = 5;

/// How far off a tip guess can be and still be the same charge.
fn slack() -> Decimal {
    Decimal::new(5, 2)
}

/// What the charge would be with the tip we usually leave: 13% of the total, or
/// 15% of the pre-tax subtotal. Restaurant receipts print before the tip, so
/// without these every meal falls through to a manual match.
fn with_tip(receipt: &dto::Receipt) -> impl Iterator<Item = Decimal> {
    let on_total = receipt.total.map(|total| total * Decimal::new(113, 2));
    let on_subtotal = receipt
        .subtotal
        .map(|subtotal| subtotal * Decimal::new(115, 2) + receipt.tax.unwrap_or_default());
    [on_total, on_subtotal].into_iter().flatten()
}

/// Best first. `available` is the receipts not already spoken for.
pub fn candidates<'a>(
    charged_on: jiff::civil::Date,
    amount: Decimal,
    currency: &str,
    available: impl IntoIterator<Item = &'a dto::Receipt>,
) -> Vec<dto::Candidate> {
    // A refund has no receipt to match: totals are positive.
    if amount.is_sign_negative() {
        return Vec::new();
    }

    let mut ranked: Vec<_> = available
        .into_iter()
        .filter(|receipt| receipt.currency == currency)
        .filter_map(|receipt| {
            let days = (charged_on - receipt.purchased_on).get_days();
            if !(EARLIEST..=LATEST).contains(&days) {
                return None;
            }
            let total = receipt.total?;
            let over = amount - total;

            // Ranked by how much of the charge the receipt explains. The screen
            // shows the total next to this, so the phrasing leaves amounts to it.
            let (rank, why, confident) = if over.is_zero() {
                (0, "exact amount".to_string(), true)
            } else if with_tip(receipt).any(|tipped| (amount - tipped).abs() <= slack()) {
                (1, "plus the usual tip".to_string(), true)
            // Past a quarter of the bill, nobody tips that much: it's a different
            // purchase, and only worth offering as one.
            } else if over.is_sign_positive() && over <= total * Decimal::new(25, 2) {
                (2, "under the charge — a tip?".to_string(), false)
            } else {
                (3, days_apart(days), false)
            };

            Some((
                rank,
                over.abs(),
                days.abs(),
                dto::Candidate {
                    receipt_id: receipt.id,
                    merchant: receipt.merchant.clone(),
                    purchased_on: receipt.purchased_on,
                    total,
                    currency: receipt.currency.clone(),
                    why,
                    confident,
                },
            ))
        })
        .collect();

    ranked.sort_by_key(|(rank, gap, days, _)| (*rank, *gap, *days));
    ranked.truncate(MOST);
    ranked
        .into_iter()
        .map(|(_, _, _, candidate)| candidate)
        .collect()
}

fn days_apart(days: i32) -> String {
    match days {
        0 => "same day".to_string(),
        -1 => "the day after".to_string(),
        1 => "the day before".to_string(),
        n => format!("{n} days before"),
    }
}

/// The receipt to attach without asking: one confident candidate and no other.
///
/// Two receipts that both fit are left alone deliberately — picking either would
/// be a coin toss, and the wrong one puts the money on the wrong person while
/// every total still balances.
pub fn automatic(candidates: &[dto::Candidate]) -> Option<uuid::Uuid> {
    let mut confident = candidates.iter().filter(|c| c.confident);
    let only = confident.next()?;
    confident.next().is_none().then_some(only.receipt_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn receipt(day: i8, total: &str) -> dto::Receipt {
        dto::Receipt {
            id: uuid::Uuid::new_v4(),
            purchased_on: jiff::civil::date(2026, 7, day),
            merchant: "Somewhere".into(),
            subtotal: None,
            tax: None,
            total: Some(dec(total)),
            currency: "USD".into(),
            status: dto::ExtractionStatus::Done,
            extraction_error: None,
            reviewed: false,
            line_items: Vec::new(),
        }
    }

    fn offers(amount: &str, available: &[dto::Receipt]) -> Vec<dto::Candidate> {
        candidates(
            jiff::civil::date(2026, 7, 10),
            dec(amount),
            "USD",
            available,
        )
    }

    #[test]
    fn an_exact_amount_in_the_window_matches_by_itself() {
        let receipts = [receipt(8, "35.36")];
        let offered = offers("35.36", &receipts);
        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0].why, "exact amount");
        assert_eq!(automatic(&offered), Some(receipts[0].id));
    }

    /// Amount does the work, but a receipt from three weeks ago with the same
    /// total is a different purchase.
    #[test]
    fn the_date_window_is_bounded_at_both_ends() {
        assert!(
            offers("10.00", &[receipt(5, "10.00")]).is_empty(),
            "too old"
        );
        assert!(
            offers("10.00", &[receipt(12, "10.00")]).is_empty(),
            "too new"
        );
        // Both edges are inclusive: four days before, and the day after.
        assert_eq!(offers("10.00", &[receipt(6, "10.00")]).len(), 1);
        assert_eq!(offers("10.00", &[receipt(11, "10.00")]).len(), 1);
    }

    /// The case exact matching misses: the card is charged the total plus a tip.
    #[test]
    fn a_restaurant_charge_matches_the_receipt_plus_a_tip() {
        // 13% of the total.
        let receipts = [receipt(10, "48.00")];
        let offered = offers("54.24", &receipts);
        assert!(offered[0].confident, "{:?}", offered[0].why);
        assert_eq!(automatic(&offered), Some(receipts[0].id));

        // 15% of the pre-tax subtotal, plus the tax.
        let mut with_tax = receipt(10, "48.00");
        with_tax.subtotal = Some(dec("44.00"));
        with_tax.tax = Some(dec("4.00"));
        let offered = offers("54.60", std::slice::from_ref(&with_tax));
        assert!(offered[0].confident, "{:?}", offered[0].why);

        // A few cents off a rounded-up tip still counts.
        let offered = offers("54.25", &receipts);
        assert!(offered[0].confident, "{:?}", offered[0].why);
    }

    /// An unusual tip is offered but never taken automatically — the difference
    /// could as easily be a different purchase.
    #[test]
    fn an_amount_over_the_total_is_only_a_suggestion() {
        let offered = offers("55.00", &[receipt(10, "48.00")]);
        assert!(!offered[0].confident);
        assert!(offered[0].why.contains("tip?"), "{}", offered[0].why);
        assert_eq!(automatic(&offered), None);
    }

    /// Two receipts that both fit is exactly when guessing does damage: the
    /// totals balance either way and the split silently goes to the wrong person.
    #[test]
    fn two_equally_good_receipts_are_left_for_a_human() {
        let receipts = [receipt(9, "35.36"), receipt(10, "35.36")];
        let offered = offers("35.36", &receipts);
        assert_eq!(offered.len(), 2);
        assert!(offered.iter().all(|c| c.confident));
        assert_eq!(automatic(&offered), None);
    }

    #[test]
    fn nothing_crosses_currencies_or_matches_a_refund() {
        let mut canadian = receipt(10, "35.36");
        canadian.currency = "CAD".into();
        assert!(offers("35.36", &[canadian]).is_empty());

        assert!(offers("-12.00", &[receipt(10, "12.00")]).is_empty());
    }

    /// Receipts in the window that explain none of the charge are still worth
    /// offering — they're what "pick a receipt" starts from — but ranked last.
    #[test]
    fn closer_amounts_outrank_bare_coincidence() {
        let receipts = [receipt(10, "9.00"), receipt(9, "35.36")];
        let offered = offers("35.36", &receipts);
        assert_eq!(offered[0].receipt_id, receipts[1].id, "exact first");
        assert_eq!(offered[1].why, "same day");
        assert!(!offered[1].confident);
    }
}
