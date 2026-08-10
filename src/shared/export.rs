//! The statement as a spreadsheet.

use crate::shared::dto::{Resolution, Statement};

/// One row per charge, one column per person.
///
/// The person columns add up to the charge on a settled row and are blank on
/// every other one, so `SUM(charge) - SUM(people)` is exactly what's still
/// unaccounted for. A proposal is deliberately blank: an unconfirmed guess in a
/// column that gets summed is worse than a gap.
pub fn statement_to_csv(statement: &Statement) -> String {
    let mut out = String::from("date,description,charge,receipt,status");
    for person in &statement.people {
        out.push(',');
        out.push_str(&text(&person.name));
    }
    out.push_str("\r\n");

    for charge in &statement.charges {
        let (receipt, status) = match &charge.resolution {
            Resolution::Unresolved => ("", "unresolved"),
            Resolution::Proposed(m) => (m.merchant.as_str(), "needs confirming"),
            Resolution::Confirmed(m) => (m.merchant.as_str(), "matched"),
            Resolution::NoReceipt { .. } => ("", "no receipt"),
        };
        out.push_str(&format!(
            "{},{},{},{},{status}",
            charge.charged_on,
            text(&charge.description),
            charge.amount,
            text(receipt),
        ));

        for person in &statement.people {
            out.push(',');
            let share = charge
                .split
                .iter()
                .find(|share| share.person_id == person.id);
            if let Some(share) = share.filter(|_| charge.resolution.is_settled()) {
                out.push_str(&share.amount.to_string());
            }
        }
        out.push_str("\r\n");
    }

    out
}

/// Escapes one field per RFC 4180.
fn field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Escapes a field that came off a bank file or out of a model reading a photo.
///
/// A leading `=`, `+`, `@` or control character makes spreadsheets treat the cell
/// as a formula. Text columns only — we render the amounts ourselves, and a
/// leading `-` there is a negative number.
fn text(value: &str) -> String {
    let dangerous = matches!(value.chars().next(), Some('=' | '+' | '@' | '\t' | '\r'));
    if dangerous {
        field(&format!("'{value}"))
    } else {
        field(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::dto::Resolution::{Confirmed, NoReceipt, Proposed, Unresolved};
    use crate::shared::testing::{charge, matched, statement};

    #[test]
    fn settled_rows_are_filled_in_and_the_rest_left_blank() {
        let csv = statement_to_csv(&statement(vec![
            charge("COSTCO", "20.00", Confirmed(matched("Costco"))),
            charge("NETFLIX", "17.99", NoReceipt { person_id: None }),
            charge("CAFE", "9.00", Proposed(matched("Blue Bottle"))),
            charge("MYSTERY", "5.00", Unresolved),
        ]));
        let lines: Vec<_> = csv.lines().collect();

        assert_eq!(lines[0], "date,description,charge,receipt,status,Josh,Ash");
        assert_eq!(
            lines[1],
            "2026-07-04,COSTCO,20.00,Costco,matched,10.00,10.00"
        );
        assert_eq!(
            lines[2], "2026-07-04,NETFLIX,17.99,,no receipt,9.00,8.99",
            "a receiptless charge still gets split"
        );
        assert_eq!(
            lines[3], "2026-07-04,CAFE,9.00,Blue Bottle,needs confirming,,",
            "a proposal is named but not counted"
        );
        assert_eq!(lines[4], "2026-07-04,MYSTERY,5.00,,unresolved,,");
    }

    /// Descriptions come off a bank file and merchants out of a model reading an
    /// arbitrary photo, so both are untrusted input heading into a spreadsheet.
    #[test]
    fn separators_are_quoted_and_formulas_defused() {
        let csv = statement_to_csv(&statement(vec![
            charge("SQ *BOB'S, INC", "1.00", Unresolved),
            charge(
                "=cmd|'/c calc'!A1",
                "-5.00",
                Confirmed(matched("+Refunds \"R\" Us")),
            ),
        ]));

        assert!(csv.contains("\"SQ *BOB'S, INC\""), "{csv}");
        assert!(csv.contains("'=cmd|"), "description neutralized: {csv}");
        assert!(csv.contains("\"'+Refunds \"\"R\"\" Us\""), "{csv}");
        // The amounts are ours, not theirs, and have to stay numbers.
        assert!(csv.contains(",-5.00,"), "amount untouched: {csv}");
        assert!(!csv.contains("'-5.00"), "amount must not be quoted: {csv}");
    }
}
