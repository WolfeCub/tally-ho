//! Which receipt pays for which charge.
//!
//! The amount does most of the work and the date bounds the search. The merchant
//! only ever corroborates: statements mangle the name, so plenty of receipts
//! that do pay for a charge share no word with the line that charged them.
//! Recognising the merchant counts for a receipt; not recognising it counts for
//! nothing, and never against.

mod merchant;

use std::cmp::Reverse;

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

/// How much of a charge a receipt's total explains. Declared best first: the
/// order here is the order candidates come back in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Fit {
    /// The charge to the cent.
    Exact,
    /// The total plus the tip we usually leave.
    UsualTip,
    /// Over the total, but not by more than anyone tips.
    UnusualTip,
    /// Nothing. The receipt is only here on the date or the merchant.
    Unrelated,
}

impl Fit {
    fn of(charge: Decimal, total: Decimal, receipt: &dto::Receipt) -> Self {
        let over = charge - total;
        if over.is_zero() {
            Self::Exact
        } else if with_tip(receipt).any(|tipped| (charge - tipped).abs() <= slack()) {
            Self::UsualTip
        // Past a quarter of the bill nobody tips that much: it's a different
        // purchase, and only worth offering as one.
        } else if over.is_sign_positive() && over <= total * Decimal::new(25, 2) {
            Self::UnusualTip
        } else {
            Self::Unrelated
        }
    }

    /// How to put it on screen. The receipt's total sits beside this, so the
    /// phrasing leaves the figures to it.
    fn says(self) -> Option<&'static str> {
        match self {
            Self::Exact => Some("exact amount"),
            Self::UsualTip => Some("plus the usual tip"),
            Self::UnusualTip => Some("under the charge — a tip?"),
            Self::Unrelated => None,
        }
    }
}

/// Best first. `description` is the statement's own line for the charge, and
/// `available` the receipts not already spoken for — which is also as far as a
/// misread date can be rescued from, nothing outside it being seen at all.
pub fn candidates<'a>(
    charged_on: jiff::civil::Date,
    description: &str,
    amount: Decimal,
    currency: &str,
    available: impl IntoIterator<Item = &'a dto::Receipt>,
) -> Vec<dto::Candidate> {
    // A refund has no receipt to match: totals are positive.
    if amount.is_sign_negative() {
        return Vec::new();
    }

    let line = merchant::Line::new(description);
    let mut ranked: Vec<_> = available
        .into_iter()
        .filter(|receipt| receipt.currency == currency)
        .filter_map(|receipt| {
            let total = receipt.total?;
            let fit = Fit::of(amount, total, receipt);
            let days = (charged_on - receipt.purchased_on).get_days();
            let in_window = (EARLIEST..=LATEST).contains(&days);
            let same_merchant = line.names(&receipt.merchant);

            // A date outside the window is either a different purchase or one the
            // extractor misread, and only the merchant tells them apart — with
            // the amount to back it, or every visit to a regular haunt would be
            // offered for every charge there.
            if !in_window && !(same_merchant && fit != Fit::Unrelated) {
                return None;
            }

            // Two things have to agree before a receipt is taken unasked. The
            // date is one, the amount the other — unless it's only close enough
            // for a tip, when the merchant makes up the difference.
            let confident = match fit {
                Fit::Exact | Fit::UsualTip => in_window,
                Fit::UnusualTip => in_window && same_merchant,
                Fit::Unrelated => false,
            };

            // Best fit first, the merchant to break a tie, then whichever is
            // closest on the amount and the date.
            let rank = (
                fit,
                Reverse(same_merchant),
                (amount - total).abs(),
                days.abs(),
            );

            Some((
                rank,
                dto::Candidate {
                    receipt_id: receipt.id,
                    merchant: receipt.merchant.clone(),
                    purchased_on: receipt.purchased_on,
                    total,
                    currency: receipt.currency.clone(),
                    why: why(fit, same_merchant, days, in_window),
                    same_merchant,
                    confident,
                },
            ))
        })
        .collect();

    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.truncate(MOST);
    ranked.into_iter().map(|(_, candidate)| candidate).collect()
}

/// Why a receipt is being offered: whichever of the merchant, the amount and the
/// date have anything to say, in that order.
fn why(fit: Fit, same_merchant: bool, days: i32, in_window: bool) -> String {
    let mut parts = Vec::new();
    if same_merchant {
        parts.push("same merchant".to_string());
    }
    parts.extend(fit.says().map(str::to_string));
    // The date is worth a mention when it's all there is, or when it's the reason
    // to look twice.
    if fit == Fit::Unrelated || !in_window {
        parts.push(days_apart(days));
    }
    parts.join(", ")
}

fn days_apart(days: i32) -> String {
    match days {
        0 => "same day".to_string(),
        -1 => "the day after".to_string(),
        1 => "the day before".to_string(),
        // Only a receipt the window let through can be this far the wrong side
        // of the charge, and then it's the whole point of showing it.
        n if n.is_negative() => format!("{} days after", -n),
        n => format!("{n} days before"),
    }
}

/// The receipt to attach without asking: one confident candidate and no other,
/// or one the statement also names.
///
/// Two receipts that both fit on the amount alone are left alone deliberately —
/// picking either would be a coin toss, and the wrong one puts the money on the
/// wrong person while every total still balances. The merchant is what turns
/// that coin toss into an answer.
pub fn automatic(candidates: &[dto::Candidate]) -> Option<uuid::Uuid> {
    let confident = || candidates.iter().filter(|c| c.confident);
    let only = sole(confident())
        // Two that fit equally well stay a coin toss until the statement names
        // one of the two merchants and not the other.
        .or_else(|| sole(confident().filter(|c| c.same_merchant)))?;
    Some(only.receipt_id)
}

/// The one and only, or nothing.
fn sole<'a>(
    mut candidates: impl Iterator<Item = &'a dto::Candidate>,
) -> Option<&'a dto::Candidate> {
    let first = candidates.next()?;
    candidates.next().is_none().then_some(first)
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

    fn at(merchant: &str, day: i8, total: &str) -> dto::Receipt {
        dto::Receipt {
            merchant: merchant.into(),
            ..receipt(day, total)
        }
    }

    /// Offers for a charge whose line names nothing in `available`.
    fn offers(amount: &str, available: &[dto::Receipt]) -> Vec<dto::Candidate> {
        described("SQ *BLUE BOTTLE, SEATTLE WA", amount, available)
    }

    /// Offers for a charge as the statement printed it.
    fn described(
        description: &str,
        amount: &str,
        available: &[dto::Receipt],
    ) -> Vec<dto::Candidate> {
        candidates(
            jiff::civil::date(2026, 7, 10),
            description,
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

    /// The same two receipts, once the statement says where the money went.
    #[test]
    fn the_merchant_settles_what_the_amount_cannot() {
        let receipts = [at("Safeway", 9, "35.36"), at("Costco", 10, "35.36")];
        let offered = described("COSTCO WHSE #1050", "35.36", &receipts);

        assert_eq!(offered[0].receipt_id, receipts[1].id);
        assert_eq!(offered[0].why, "same merchant, exact amount");
        assert_eq!(automatic(&offered), Some(receipts[1].id));

        // Two receipts from the same place is a coin toss again.
        let receipts = [at("Costco", 9, "35.36"), at("Costco", 10, "35.36")];
        let offered = described("COSTCO WHSE #1050", "35.36", &receipts);
        assert!(offered.iter().all(|c| c.same_merchant));
        assert_eq!(automatic(&offered), None);
    }

    /// A tip we don't usually leave, at a restaurant the statement names. Without
    /// the name this is the suggestion the test above leaves alone.
    #[test]
    fn an_unusual_tip_is_taken_where_the_statement_names_the_restaurant() {
        // 20% of 48.00.
        let receipts = [at("Cafe Flora", 10, "48.00")];
        let offered = described("TST* CAFE FLORA", "57.60", &receipts);
        assert!(offered[0].confident, "{}", offered[0].why);
        assert_eq!(automatic(&offered), Some(receipts[0].id));

        // Still not a match for an amount no tip explains.
        let offered = described("TST* CAFE FLORA", "148.00", &receipts);
        assert!(!offered[0].confident);
        assert_eq!(offered[0].why, "same merchant, same day");
    }

    /// The date the extractor read is the one thing that can be flatly wrong, and
    /// a receipt nowhere near the charge used to be invisible — findable only
    /// through the list of every receipt going spare.
    #[test]
    fn a_merchant_the_statement_names_rescues_a_misread_date() {
        let receipts = [at("Cafe Flora", 25, "48.00")];
        let offered = described("TST* CAFE FLORA", "48.00", &receipts);
        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0].why, "same merchant, exact amount, 15 days after");

        // Offered, never taken: the date says nothing, so a human decides.
        assert!(!offered[0].confident);
        assert_eq!(automatic(&offered), None);

        // And it stays invisible to a charge that names somebody else.
        assert!(offers("48.00", &receipts).is_empty());
    }

    /// The merchant alone would offer every visit to a regular haunt for every
    /// charge there, which is a list to read rather than an answer.
    #[test]
    fn a_misread_date_needs_the_amount_as_well() {
        let receipts = [at("Cafe Flora", 25, "9.00")];
        assert!(described("TST* CAFE FLORA", "48.00", &receipts).is_empty());
    }

    /// A date we believe beats one we don't, and is still what gets taken.
    #[test]
    fn a_believable_date_outranks_a_rescued_one() {
        let receipts = [at("Cafe Flora", 25, "48.00"), at("Cafe Flora", 9, "48.00")];
        let offered = described("TST* CAFE FLORA", "48.00", &receipts);
        assert_eq!(offered[0].receipt_id, receipts[1].id);
        assert_eq!(automatic(&offered), Some(receipts[1].id));
    }

    /// A name nobody recognises is not held against a receipt: most statement
    /// lines look nothing like the name the receipt printed.
    #[test]
    fn an_unrecognised_merchant_costs_a_receipt_nothing() {
        let receipts = [at("Costco", 8, "35.36")];
        let offered = described("WHSE CLUB 1050", "35.36", &receipts);
        assert!(!offered[0].same_merchant);
        assert!(offered[0].confident);
        assert_eq!(automatic(&offered), Some(receipts[0].id));
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
