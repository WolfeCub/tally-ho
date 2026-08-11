//! Formatting amounts for display only — the CSV export and the edit inputs
//! both show raw values, so nothing round-trips through here.

use rust_decimal::Decimal;

use crate::shared::dto::{Person, Share};

/// Decimal places come from the currency's minor unit, so JPY gets none.
pub fn money(amount: Decimal, currency: &str) -> String {
    let sign = if amount.is_sign_negative() { "-" } else { "" };
    let value = amount.abs();

    match iso_currency::Currency::from_code(currency) {
        Some(iso) => {
            let precision = iso.exponent().unwrap_or(2) as usize;
            format!("{sign}{}{value:.precision$}", iso.symbol())
        }
        // Not a currency code we recognise, so show it verbatim.
        None => format!("{sign}{value:.2} {currency}"),
    }
}

/// "Josh $21.55 · Ash $13.81" — who owes what, whether that's for one charge or
/// a whole statement.
///
/// A share nobody on the list answers to is dropped rather than shown nameless:
/// removing someone hands their items back, so this can only be a stale view.
pub fn shares_line(shares: &[Share], people: &[Person], currency: &str) -> String {
    shares
        .iter()
        .filter_map(|share| {
            let person = people.iter().find(|p| p.id == share.person_id)?;
            Some(format!("{} {}", person.name, money(share.amount, currency)))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A total, with the ISO code spelled out — USD, CAD and AUD all use `$`, too
/// ambiguous for a summed figure.
pub fn money_total(amount: Decimal, currency: &str) -> String {
    match iso_currency::Currency::from_code(currency) {
        Some(_) => format!("{} {currency}", money(amount, currency)),
        // Already ends in the code.
        None => money(amount, currency),
    }
}
