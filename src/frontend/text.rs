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

/// A size in the units a volume is asked for: a claim of `20Gi` should read back
/// as 20 GiB, not the 21.5 GB the same bytes are in base ten.
pub fn bytes(count: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    let mut size = count as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    // A decimal place is noise on bytes and kibibytes, and the whole story on
    // the ones a disk is measured in.
    let precision = if unit < 2 { 0 } else { 1 };
    format!("{size:.precision$} {}", UNITS[unit])
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

#[cfg(test)]
mod tests {
    use super::bytes;

    #[test]
    fn sizes_read_the_way_a_volume_is_asked_for() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1024), "1 KiB");

        // The point of base two: a claim of 20Gi should read back as 20 GiB, not
        // as the 21.5 the same bytes come to in base ten.
        assert_eq!(bytes(20 * 1024 * 1024 * 1024), "20.0 GiB");

        // Past the last unit it keeps counting rather than running off the end of
        // the list.
        assert_eq!(bytes(5 * 1024_u64.pow(5)), "5120.0 TiB");
    }
}
