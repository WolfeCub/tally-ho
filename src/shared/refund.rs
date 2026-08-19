//! Working out who gets a refund back.
//!
//! A refund is a negative charge pointing at the purchase it came off. What went
//! back is on that purchase's receipt, so the amount is matched against its line
//! items and the money follows whoever bought them.

use rust_decimal::Decimal;

use crate::shared::dto::{LineItem, Person, Share};
use crate::shared::reconcile::{charged, minor_units, shares, weigh};

/// Past this many items the search isn't worth running: too many combinations,
/// and a sum that lands on the amount stops being evidence.
const MOST_ITEMS: usize = 20;

/// Splits a refund between whoever bought what went back.
///
/// When nothing adds up — a partial refund of a receiptless charge, a receipt
/// still being read — it comes off everyone in the proportions the purchase was
/// split in, which is the best that's left.
///
/// `amount` is negative, like the charge it came from. `purchase` is what the
/// card was charged for it, `items` are off its receipt, `split` is how it went.
pub fn split_refund(
    amount: Decimal,
    currency: &str,
    purchase: Decimal,
    items: &[LineItem],
    split: &[Share],
    people: &[Person],
) -> Vec<Share> {
    let weights = match went_back(-amount, currency, purchase, items) {
        Some(back) => weigh(&charged(back), people),
        None => people
            .iter()
            .map(|person| {
                split
                    .iter()
                    .find(|share| share.person_id == person.id)
                    .map_or(Decimal::ZERO, |share| share.amount)
            })
            .collect(),
    };

    shares(amount, currency, &weights, people)
}

/// The line items a refund handed back, if they can be told.
fn went_back<'a>(
    amount: Decimal,
    currency: &str,
    purchase: Decimal,
    items: &'a [LineItem],
) -> Option<Vec<&'a LineItem>> {
    let printed: Decimal = items.iter().map(|item| item.total).sum();
    if printed.is_zero() || purchase.is_zero() {
        return None;
    }

    // Items are pre-tax and a refund isn't, so it's scaled back down by the same
    // ratio a charge scales them up by before anything is compared. A minor unit
    // of slack absorbs the rounding in that.
    let target = amount * printed / purchase;
    let slack = Decimal::ONE / Decimal::from(10u64.pow(minor_units(currency)));

    // The whole lot going back is the common case and the cheapest to spot.
    if (printed - target).abs() <= slack {
        return Some(items.iter().collect());
    }
    if items.len() > MOST_ITEMS {
        return None;
    }

    adding_up(target, slack, items)
}

/// The first of `items` that add up to `target`, each one either in or out.
///
/// Overshooting prunes everything below it, which is what keeps this from
/// walking every combination. A receipt with a coupon on it can overshoot and
/// come back down, so one of those may find nothing — and a refund nothing
/// explains falls back to proportions, which is the safe way to be wrong.
fn adding_up(target: Decimal, slack: Decimal, items: &[LineItem]) -> Option<Vec<&LineItem>> {
    if target.abs() <= slack {
        return Some(Vec::new());
    }
    if target.is_sign_negative() {
        return None;
    }

    let (item, rest) = items.split_first()?;
    match adding_up(target - item.total, slack, rest) {
        Some(mut found) => {
            found.insert(0, item);
            Some(found)
        }
        None => adding_up(target, slack, rest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::reconcile::split_charge;
    use crate::shared::testing::{charged_to, dec, pair};

    fn amounts(shares: &[Share]) -> Vec<Decimal> {
        shares.iter().map(|s| s.amount).collect()
    }

    /// Milk is Josh's, dog food is Ash's, and the card was charged 35.36 for the
    /// 32.82 the receipt printed.
    fn costco() -> Vec<LineItem> {
        vec![charged_to(Some(1), "20.00"), charged_to(Some(2), "12.82")]
    }

    /// Refunds a purchase whose receipt explains it.
    fn refund(amount: &str, purchase: &str, items: &[LineItem]) -> Vec<Decimal> {
        let purchase = dec(purchase);
        let split = split_charge(purchase, "USD", items, &pair());
        amounts(&split_refund(
            dec(amount),
            "USD",
            purchase,
            items,
            &split,
            &pair(),
        ))
    }

    /// The point of the whole thing. 13.81 back is the 12.82 dog food plus the
    /// tax it carried, and it comes off Ash rather than off both of them.
    #[test]
    fn a_returned_item_comes_off_whoever_bought_it() {
        assert_eq!(
            refund("-13.81", "35.36", &costco()),
            [dec("0.00"), dec("-13.81")]
        );
    }

    /// The whole purchase back is the purchase's own split, negated.
    #[test]
    fn everything_back_undoes_the_charge() {
        assert_eq!(
            refund("-35.36", "35.36", &costco()),
            [dec("-21.55"), dec("-13.81")]
        );
    }

    /// Two items at once, which is why the amount is matched against sets of
    /// them and not just one.
    #[test]
    fn several_items_back_at_once_are_found() {
        let items = [
            charged_to(Some(1), "20.00"),
            charged_to(Some(2), "12.82"),
            charged_to(Some(1), "5.00"),
        ];
        // 12.82 and 5.00, so Ash's back in full and Josh's 5.
        assert_eq!(
            refund("-17.82", "37.82", &items),
            [dec("-5.00"), dec("-12.82")]
        );
    }

    /// An amount no set of items explains — a partial refund, a restocking fee
    /// off the top. Proportions are all that's left, and they still add up.
    #[test]
    fn an_amount_nothing_explains_falls_back_to_proportions() {
        let shares = refund("-10.00", "35.36", &costco());
        assert_eq!(shares, [dec("-6.09"), dec("-3.91")]);
        assert_eq!(shares.iter().sum::<Decimal>(), dec("-10.00"));
    }

    /// A refunded subscription has no receipt to go on, so it goes back to
    /// whoever was charged for it.
    #[test]
    fn a_receiptless_purchase_refunds_the_way_it_was_charged() {
        let ash = pair()[1].id;
        let split = crate::shared::reconcile::charge_to(dec("17.99"), "USD", Some(ash), &pair());
        let shares = split_refund(dec("-17.99"), "USD", dec("17.99"), &[], &split, &pair());
        assert_eq!(amounts(&shares), [dec("0.00"), dec("-17.99")]);
    }

    /// Nothing accounts for the purchase yet, so there are no proportions to
    /// follow and it splits evenly rather than landing on nobody.
    #[test]
    fn a_purchase_with_no_split_yet_refunds_evenly() {
        let shares = split_refund(dec("-10.01"), "USD", dec("10.01"), &[], &[], &pair());
        assert_eq!(amounts(&shares), [dec("-5.00"), dec("-5.01")]);
    }

    /// A long receipt is left alone rather than combed, but the whole lot coming
    /// back is still spotted.
    #[test]
    fn a_long_receipt_still_refunds_in_full() {
        let items: Vec<LineItem> = (0..MOST_ITEMS + 5)
            .map(|i| charged_to(Some(if i % 2 == 0 { 1 } else { 2 }), "1.00"))
            .collect();
        let total = Decimal::from(items.len());

        let split = split_charge(total, "USD", &items, &pair());
        let shares = split_refund(-total, "USD", total, &items, &split, &pair());
        assert_eq!(amounts(&shares), [dec("-13.00"), dec("-12.00")]);
    }
}
