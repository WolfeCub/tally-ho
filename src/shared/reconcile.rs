//! Splitting a charge between people, and what a statement adds up to.
//!
//! Shared so the reconcile screen and the export can't disagree: both call
//! [`split_charge`], and the figures on screen are the ones in the file.

use rust_decimal::Decimal;
use uuid::Uuid;

use crate::shared::dto::{LineItem, Person, Share, Statement};

impl Statement {
    /// What the card was charged, refunds included.
    pub fn total(&self) -> Decimal {
        self.charges.iter().map(|c| c.amount).sum()
    }

    pub fn settled(&self) -> usize {
        self.charges
            .iter()
            .filter(|c| c.resolution.is_settled())
            .count()
    }

    /// Charges nobody has signed off yet, proposals included, and what they add
    /// up to. That figure is what the per-person columns are short of the
    /// statement.
    pub fn outstanding(&self) -> (usize, Decimal) {
        let left = self
            .charges
            .iter()
            .filter(|c| !c.resolution.is_settled())
            .map(|c| c.amount);
        (left.clone().count(), left.sum())
    }

    /// What each person owes across the whole statement. Proposals don't count —
    /// this figure and the export have to agree.
    pub fn totals(&self) -> Vec<Share> {
        let owed = |person: &Person| {
            self.charges
                .iter()
                .filter(|c| c.resolution.is_settled())
                .flat_map(|c| &c.split)
                .filter(|share| share.person_id == person.id)
                .map(|share| share.amount)
                .sum()
        };
        self.people
            .iter()
            .map(|person| Share {
                person_id: person.id,
                amount: owed(person),
            })
            .collect()
    }
}

/// Splits one charge between people, by what each of them bought.
///
/// Line items are pre-tax, so the shares are scaled up to the charge: tax and any
/// tip land on whoever bought the taxed items instead of dropping out of the
/// statement. Tax is really per item and a receipt doesn't say which items were
/// taxed, so this is an approximation — but the columns have to add up to the
/// charge, which is the whole point of the export.
///
/// Unassigned items split evenly, and so does a charge with nothing to go on.
pub fn split_charge(
    amount: Decimal,
    currency: &str,
    items: &[LineItem],
    people: &[Person],
) -> Vec<Share> {
    shares(amount, currency, &weigh(&charged(items), people), people)
}

/// Line items as [`weigh`] wants them: who each one is charged to, and how much.
pub fn charged<'a>(items: impl IntoIterator<Item = &'a LineItem>) -> Vec<(Option<Uuid>, Decimal)> {
    items
        .into_iter()
        .map(|item| (item.person_id, item.total))
        .collect()
}

/// What each person's share of these items comes to, unassigned ones spread
/// evenly between everybody. In `people` order.
pub fn weigh(items: &[(Option<Uuid>, Decimal)], people: &[Person]) -> Vec<Decimal> {
    let sum = |whose: Option<Uuid>| -> Decimal {
        items
            .iter()
            .filter(|(person_id, _)| *person_id == whose)
            .map(|(_, total)| *total)
            .sum()
    };
    let each = sum(None) / Decimal::from(people.len().max(1));

    people.iter().map(|p| sum(Some(p.id)) + each).collect()
}

/// The whole charge on one person, or split evenly when nobody is named.
///
/// For a charge that will never have a receipt: a subscription, a fee, interest.
pub fn charge_to(
    amount: Decimal,
    currency: &str,
    person_id: Option<Uuid>,
    people: &[Person],
) -> Vec<Share> {
    let weights: Vec<Decimal> = people
        .iter()
        .map(|person| match person_id {
            Some(whose) if person.id != whose => Decimal::ZERO,
            _ => Decimal::ONE,
        })
        .collect();

    shares(amount, currency, &weights, people)
}

/// Hands `amount` out in proportion to `weights`, one share per person.
pub(super) fn shares(
    amount: Decimal,
    currency: &str,
    weights: &[Decimal],
    people: &[Person],
) -> Vec<Share> {
    if people.is_empty() {
        return Vec::new();
    }

    // Nothing to go on: no items, items that cancel out, or a name that belongs
    // to somebody since removed. Split evenly rather than divide by zero.
    let mut weights = weights.to_vec();
    if weights.iter().sum::<Decimal>().is_zero() {
        weights.fill(Decimal::ONE);
    }

    people
        .iter()
        .zip(allocate(amount, currency, &weights))
        .map(|(person, amount)| Share {
            person_id: person.id,
            amount,
        })
        .collect()
}

/// How many decimal places the currency's smallest unit has.
pub(super) fn minor_units(currency: &str) -> u32 {
    iso_currency::Currency::from_code(currency)
        .and_then(|c| c.exponent())
        .unwrap_or(2) as u32
}

/// Divides `amount` in proportion to `weights`, in whole minor units, so the
/// parts add up to exactly `amount`.
///
/// Rounding each share on its own is out by a cent often enough to be noticed,
/// and columns that don't sum to the charge are worse than useless — that sum is
/// the thing being checked. So the leftover goes to the largest fractions.
fn allocate(amount: Decimal, currency: &str, weights: &[Decimal]) -> Vec<Decimal> {
    let places = minor_units(currency);
    let unit = Decimal::from(10u64.pow(places));
    let cents = (amount * unit).round();
    let total: Decimal = weights.iter().sum();

    let ideal: Vec<Decimal> = weights.iter().map(|w| cents * w / total).collect();
    // Floor, so every share starts under its ideal and the remainder to hand out
    // is positive — which holds for a refund as much as a purchase.
    let mut parts: Vec<Decimal> = ideal.iter().map(Decimal::floor).collect();

    let mut largest_fraction: Vec<usize> = (0..parts.len()).collect();
    largest_fraction.sort_by_key(|&i| std::cmp::Reverse(ideal[i] - parts[i]));

    let mut left = cents - parts.iter().sum::<Decimal>();
    for index in largest_fraction {
        if left.is_zero() {
            break;
        }
        parts[index] += Decimal::ONE;
        left -= Decimal::ONE;
    }

    parts
        .into_iter()
        .map(|part| {
            let mut share = part / unit;
            // Division drops trailing zeros, and a 10 in a column of 13.81s
            // reads as a different kind of number.
            share.rescale(places);
            share
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::dto::Resolution::{Confirmed, NoReceipt, Proposed, Refund, Unresolved};
    use crate::shared::testing::{charge, charged_to, dec, matched, pair, refunded, statement};

    /// The amounts a split hands out, in people order.
    fn amounts(shares: &[Share]) -> Vec<Decimal> {
        shares.iter().map(|s| s.amount).collect()
    }

    fn split(amount: &str, items: &[LineItem]) -> Vec<Share> {
        split_charge(dec(amount), "USD", items, &pair())
    }

    /// The load-bearing case. Line items are pre-tax, so splitting only the items
    /// would leave the tax on nobody and the columns short of the charge.
    #[test]
    fn tax_is_carried_onto_the_people_who_incurred_it() {
        let shares = split(
            "35.36",
            &[charged_to(Some(1), "20.00"), charged_to(Some(2), "12.82")],
        );
        // 32.82 of items grossed up to the 35.36 charged.
        assert_eq!(amounts(&shares), [dec("21.55"), dec("13.81")]);
        assert_eq!(
            amounts(&shares).iter().sum::<Decimal>(),
            dec("35.36"),
            "the columns must add up to the charge"
        );
    }

    #[test]
    fn unassigned_items_split_evenly_alongside_assigned_ones() {
        let shares = split(
            "20.00",
            &[charged_to(Some(1), "10.00"), charged_to(None, "10.00")],
        );
        // Josh's own 10, plus half the unassigned 10 each.
        assert_eq!(amounts(&shares), [dec("15.00"), dec("5.00")]);
    }

    /// A charge with no items to go on — a receipt still being read, or a
    /// subscription nobody photographed — is still split, not dropped.
    #[test]
    fn nothing_to_go_on_splits_evenly_to_the_last_cent() {
        let shares = split("10.01", &[]);
        assert_eq!(amounts(&shares), [dec("5.01"), dec("5.00")]);
    }

    /// A refund comes off whoever bought the thing, rather than being split.
    #[test]
    fn a_refund_lands_on_the_person_who_bought_it() {
        let shares = split("-12.00", &[charged_to(Some(2), "12.00")]);
        assert_eq!(amounts(&shares), [dec("0.00"), dec("-12.00")]);
    }

    /// JPY has no minor unit, so cents here would be a currency error rather
    /// than a rounding one.
    #[test]
    fn rounding_follows_the_currency() {
        let shares = split_charge(dec("101"), "JPY", &[], &pair());
        assert_eq!(amounts(&shares), [dec("51"), dec("50")]);
    }

    #[test]
    fn nobody_to_split_between_leaves_the_columns_empty() {
        assert!(split_charge(dec("10.00"), "USD", &[], &[]).is_empty());
        assert!(charge_to(dec("10.00"), "USD", None, &[]).is_empty());
    }

    /// A receiptless charge is one person's or everyone's, with no items to say.
    #[test]
    fn a_charge_with_no_receipt_goes_where_it_is_told() {
        let ash = pair()[1].id;
        assert_eq!(
            amounts(&charge_to(dec("17.99"), "USD", Some(ash), &pair())),
            [dec("0.00"), dec("17.99")]
        );
        assert_eq!(
            amounts(&charge_to(dec("17.99"), "USD", None, &pair())),
            [dec("9.00"), dec("8.99")]
        );
    }

    /// Somebody removed from settings takes their charges' assignment with them,
    /// and the charge still has to add up.
    #[test]
    fn a_name_nobody_answers_to_splits_evenly() {
        let shares = charge_to(dec("10.00"), "USD", Some(Uuid::from_u128(99)), &pair());
        assert_eq!(amounts(&shares), [dec("5.00"), dec("5.00")]);
    }

    /// The invariant the export exists for: what the people columns don't cover
    /// is exactly what's still outstanding.
    #[test]
    fn the_columns_plus_whats_outstanding_account_for_the_statement() {
        let statement = statement(vec![
            charge("COSTCO", "20.00", Confirmed(matched("Costco"))),
            charge("NETFLIX", "17.99", NoReceipt { person_id: None }),
            charge("CAFE", "9.00", Proposed(matched("Blue Bottle"))),
            charge("MYSTERY", "5.01", Unresolved),
            charge(
                "COSTCO RETURN",
                "-4.00",
                Refund(refunded("COSTCO", "20.00")),
            ),
        ]);

        assert_eq!(statement.settled(), 3, "a proposal is not settled");
        assert_eq!(statement.outstanding(), (2, dec("14.01")));

        let owed: Decimal = statement.totals().iter().map(|t| t.amount).sum();
        assert_eq!(owed + statement.outstanding().1, statement.total());
        assert_eq!(statement.total(), dec("48.00"));
    }
}
