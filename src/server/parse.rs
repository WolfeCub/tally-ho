//! Amounts and dates as a model, a card export or a human wrote them.
//!
//! Extraction, the statement CSV reader and the review screen all parse their
//! input here.

use rust_decimal::Decimal;

/// Money as printed on receipts, which is not the same as money as `Decimal`
/// parses it: currency symbols, thousands separators, trailing minus signs and
/// parenthesised negatives all show up.
pub fn money(raw: &str) -> Option<Decimal> {
    let mut s = raw.trim().to_string();

    // "(4.99)" and "4.99-" both mean negative on receipts.
    let mut negative = false;
    if s.starts_with('(') && s.ends_with(')') {
        negative = true;
        s = s[1..s.len() - 1].to_string();
    }
    if let Some(stripped) = s.strip_suffix('-') {
        negative = true;
        s = stripped.to_string();
    }

    // Drop currency symbols, codes and spaces; keep digits, separators, sign.
    s.retain(|c| c.is_ascii_digit() || c == '.' || c == ',' || c == '-');

    // Comma disambiguation: with both separators the last one is the decimal
    // point ("1,234.56" vs "1.234,56"). With only commas, treat a single comma
    // followed by exactly two digits as a decimal point, otherwise as grouping.
    let s = match (s.rfind('.'), s.rfind(',')) {
        (Some(dot), Some(comma)) if comma > dot => s.replace('.', "").replace(',', "."),
        (Some(_), Some(_)) => s.replace(',', ""),
        (None, Some(comma)) if s.len() - comma == 3 && s.matches(',').count() == 1 => {
            s.replace(',', ".")
        }
        (None, Some(_)) => s.replace(',', ""),
        _ => s,
    };

    if s.is_empty() || s == "-" {
        return None;
    }

    let d: Decimal = s.parse().ok()?;
    Some(if negative { -d } else { d })
}

/// ISO first, then the formats receipts actually print.
///
/// **Ambiguous numeric dates are read as MM/DD/YY** (US convention), because
/// that is what the receipts this is built for print. `08/12/21` is therefore
/// 2021-08-12, not 2021-12-08. This is deliberately a fixed rule rather than a
/// guess: being consistently wrong on a non-US receipt is recoverable in the
/// review screen, whereas being unpredictably wrong is not detectable at all.
pub fn date(raw: &str) -> Option<jiff::civil::Date> {
    let s = raw.trim();

    if let Ok(d) = s.parse::<jiff::civil::Date>() {
        return Some(d);
    }

    if let Some(d) = spelled_month(s) {
        return Some(d);
    }

    let parts: Vec<&str> = s
        .split(['/', '-', '.'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let a: i32 = parts[0].parse().ok()?;
    let b: i8 = parts[1].parse().ok()?;
    let c: i32 = parts[2].parse().ok()?;

    // YYYY/MM/DD
    if parts[0].len() == 4 {
        return jiff::civil::Date::new(a as i16, b, c as i8).ok();
    }

    // MM/DD/YY(YY). Two-digit years are assumed current-century; receipts are
    // not historical documents.
    let year = if parts[2].len() <= 2 { 2000 + c } else { c };
    jiff::civil::Date::new(year as i16, a as i8, b).ok()
}

/// A month spelled out: "28 Jul 2026", "July 28, 2026", "28-JUL-26". Amex writes
/// its statements this way, and there's nothing to disambiguate once the month
/// names itself.
fn spelled_month(s: &str) -> Option<jiff::civil::Date> {
    // The day of the week, if it's there, says nothing the rest of the date
    // doesn't. Dropping it beats doubling the format list below.
    let named_day = |field: &str| {
        ["%a", "%A"]
            .iter()
            .any(|format| jiff::fmt::strtime::parse(format, field).is_ok())
    };

    // Down to single spaces first, so four formats cover whatever punctuation
    // the file separates the fields with.
    let tidied = s
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '-' | '/' | '.'))
        .filter(|field| !field.is_empty() && !named_day(field))
        .collect::<Vec<_>>()
        .join(" ");

    // Month names match whatever case they're written in. `%b` is the
    // abbreviation and `%B` the full name; neither accepts the other.
    let date = ["%d %b %Y", "%d %B %Y", "%b %d %Y", "%B %d %Y"]
        .iter()
        .find_map(|format| jiff::civil::Date::strptime(format, &tidied).ok())?;

    // `%Y` takes a two-digit year at face value, so "28 Jul 26" comes back as
    // year 26. Same rule as above: current century.
    match date.year() {
        ..100 => jiff::civil::Date::new(date.year() + 2000, date.month(), date.day()).ok(),
        _ => Some(date),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::dec;

    #[test]
    fn parses_plain_and_decorated_amounts() {
        assert_eq!(money("4.99"), Some(dec("4.99")));
        assert_eq!(money("$4.99"), Some(dec("4.99")));
        assert_eq!(money(" USD 4.99 "), Some(dec("4.99")));
        assert_eq!(money("1,234.56"), Some(dec("1234.56")));
        assert_eq!(money("1.234,56"), Some(dec("1234.56")));
        assert_eq!(money("4,99"), Some(dec("4.99")));
        assert_eq!(money("1,234"), Some(dec("1234")));
    }

    #[test]
    fn parses_receipt_style_negatives() {
        assert_eq!(money("-2.00"), Some(dec("-2.00")));
        assert_eq!(money("(2.00)"), Some(dec("-2.00")));
        assert_eq!(money("2.00-"), Some(dec("-2.00")));
    }

    #[test]
    fn rejects_unparseable_amounts() {
        assert_eq!(money(""), None);
        assert_eq!(money("n/a"), None);
        assert_eq!(money("-"), None);
    }

    /// Exactness is the whole reason for `Decimal`; a float round-trip would
    /// lose this.
    #[test]
    fn amounts_are_exact() {
        let sum: Decimal = ["0.10", "0.20"].iter().map(|s| dec(s)).sum();
        assert_eq!(sum, dec("0.30"));
        assert_eq!(money("19.99").unwrap() * dec("3"), dec("59.97"));
    }

    #[test]
    fn parses_common_receipt_date_formats() {
        let expected = jiff::civil::date(2026, 7, 14);
        assert_eq!(date("2026-07-14"), Some(expected));
        assert_eq!(date("07/14/2026"), Some(expected));
        assert_eq!(date("7/14/26"), Some(expected));
        assert_eq!(date("2026/07/14"), Some(expected));
        assert_eq!(date("07.14.2026"), Some(expected));
    }

    /// Amex writes the month out, and every row of a statement is the same
    /// shape, so missing this makes the whole file unreadable rather than one
    /// row.
    #[test]
    fn parses_dates_with_the_month_spelled_out() {
        let expected = jiff::civil::date(2026, 7, 28);
        assert_eq!(date("28 Jul 2026"), Some(expected));
        assert_eq!(date("28 July 2026"), Some(expected));
        assert_eq!(date("28-JUL-2026"), Some(expected));
        assert_eq!(date("Jul 28, 2026"), Some(expected));
        assert_eq!(date("July 28, 2026"), Some(expected));
        assert_eq!(date("28 Jul 26"), Some(expected));
        // A receipt that prints the day of the week too.
        assert_eq!(date("TUE JULY 28,2026"), Some(expected));
        assert_eq!(date("Tuesday, July 28, 2026"), Some(expected));
        // Still a word where the month should be.
        assert_eq!(date("28 Jly 2026"), None);
    }

    /// Regression guard. The model returned `2021-12-08` for a receipt printed
    /// `08/12/21`, silently moving it four months and into a different
    /// statement period. Both digits are valid months, so nothing downstream
    /// could have caught it — the ordering rule has to be enforced here.
    #[test]
    fn ambiguous_numeric_dates_are_month_first() {
        assert_eq!(date("08/12/21"), Some(jiff::civil::date(2021, 8, 12)));
        assert_eq!(date("01/02/26"), Some(jiff::civil::date(2026, 1, 2)));
        // Unambiguous: 13 cannot be a month, so this is not silently accepted
        // as December 13th.
        assert_eq!(date("13/02/26"), None);
    }

    #[test]
    fn rejects_unparseable_dates() {
        assert_eq!(date("last Tuesday"), None);
        assert_eq!(date("13/45/2026"), None);
        assert_eq!(date(""), None);
    }
}
