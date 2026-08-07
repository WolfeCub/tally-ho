//! Formatting amounts for display only — the CSV export and the edit inputs
//! both show raw values, so nothing round-trips through here.

use rust_decimal::Decimal;

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

/// A total, with the ISO code spelled out — USD, CAD and AUD all use `$`, too
/// ambiguous for a summed figure.
pub fn money_total(amount: Decimal, currency: &str) -> String {
    match iso_currency::Currency::from_code(currency) {
        Some(_) => format!("{} {currency}", money(amount, currency)),
        // Already ends in the code.
        None => money(amount, currency),
    }
}
