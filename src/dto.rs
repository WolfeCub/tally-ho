//! Wire types shared by server functions and the UI.
//!
//! Deliberately separate from [`crate::models`]: these compile for wasm32,
//! carry no toasty dependency, and are free to differ in shape from the tables
//! (e.g. `ReceiptSummary` folds line-item totals the DB can't sum).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtractionStatus {
    Pending,
    Extracting,
    Done,
    Failed,
}

impl ExtractionStatus {
    /// Whether the client should keep polling.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineItem {
    pub id: Uuid,
    pub description: String,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>,
    pub total: Decimal,
    pub position: i64,
    pub edited: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub id: Uuid,
    pub purchased_on: jiff::civil::Date,
    pub merchant: String,
    pub subtotal: Option<Decimal>,
    pub tax: Option<Decimal>,
    /// `None` means the total is not yet known, never that it is zero.
    pub total: Option<Decimal>,
    pub currency: String,
    pub status: ExtractionStatus,
    pub extraction_error: Option<String>,
    pub reviewed: bool,
    pub line_items: Vec<LineItem>,
}

/// Human edits to a receipt's own fields.
///
/// Money and dates travel as strings, exactly as typed, and the server parses them
/// with the same routines the extractor uses — so "$12.34" and "8/12/21" work, and
/// there's one parser rather than two. An empty string clears an optional field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptEdit {
    pub id: Uuid,
    pub merchant: String,
    pub purchased_on: String,
    pub currency: String,
    pub subtotal: String,
    pub tax: String,
    pub total: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineItemEdit {
    pub id: Uuid,
    pub description: String,
    pub total: String,
}

pub fn line_item_sum(items: &[LineItem]) -> Decimal {
    items.iter().map(|i| i.total).sum()
}

/// Everything wrong with a receipt, in the order a human should care. A list, not
/// a bool, so the review screen can say *what* is wrong.
///
/// Note what isn't checked: line items against the total. Items are pre-tax, so on
/// a taxed receipt that always looks off by the tax.
pub fn problems_of(
    subtotal: Option<Decimal>,
    tax: Option<Decimal>,
    total: Option<Decimal>,
    items: &[LineItem],
) -> Vec<String> {
    let mut out = Vec::new();

    if total.is_none() {
        out.push("No total was read — reconciliation needs one.".to_string());
    }

    // Items should reconstruct the subtotal (pre-tax). A gap here means a line
    // was dropped, duplicated, or misread.
    if let Some(subtotal) = subtotal
        && !items.is_empty()
    {
        let sum = line_item_sum(items);
        let diff = subtotal - sum;
        if !diff.is_zero() {
            out.push(format!(
                "Line items add up to {sum}, but the subtotal says {subtotal} (off by {diff})."
            ));
        }
    }

    // subtotal + tax should equal what was actually charged.
    if let (Some(sub), Some(tax), Some(total)) = (subtotal, tax, total) {
        let diff = sub + tax - total;
        if !diff.is_zero() {
            out.push(format!(
                "{sub} + {tax} tax = {}, but the total says {total} (off by {diff}).",
                sub + tax
            ));
        }
    }

    if items.iter().any(|i| i.total.is_zero()) {
        out.push("One or more line items have no amount — they were unreadable.".to_string());
    }

    out
}

impl Receipt {
    /// Sum of the line items, which is *not* [`Self::total`] on a taxed
    /// receipt — it should match [`Self::subtotal`].
    pub fn line_item_sum(&self) -> Decimal {
        line_item_sum(&self.line_items)
    }

    /// A receipt with no total cannot be reconciled and needs a human.
    pub fn needs_total(&self) -> bool {
        self.total.is_none()
    }

    pub fn problems(&self) -> Vec<String> {
        problems_of(self.subtotal, self.tax, self.total, &self.line_items)
    }
}

/// One row in the period view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptSummary {
    pub id: Uuid,
    pub purchased_on: jiff::civil::Date,
    pub merchant: String,
    /// `None` when extraction could not read a total and nobody has fixed it.
    pub total: Option<Decimal>,
    /// ISO code. Carried so a row can be labelled with the right symbol — the
    /// extractor infers this from the receipt, so it is not always USD.
    pub currency: String,
    pub status: ExtractionStatus,
    pub item_count: usize,
    pub reviewed: bool,
    /// Computed by [`problems_of`] on the server. Carried in full rather than as
    /// a count so the list can say what is wrong without a second round trip.
    pub problems: Vec<String>,
}

/// The total for a statement period.
///
/// An enum, not a bare `Decimal`: an unreadable total must not count as 0, but
/// silently dropping it understates the period just as badly and is harder to
/// spot. This way the UI can't render a figure without handling that case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PeriodTotal {
    /// Every receipt in the period has a total; this figure is complete.
    Complete(Decimal),
    /// Some receipts have no total yet. `known` excludes them entirely — it is
    /// a floor, not the real figure.
    Partial { known: Decimal, missing: usize },
}

impl PeriodTotal {
    fn new(known: Decimal, missing: usize) -> Self {
        if missing == 0 {
            Self::Complete(known)
        } else {
            Self::Partial { known, missing }
        }
    }

    /// The amount actually accounted for. Callers that show this to a human are
    /// responsible for also showing [`Self::missing`].
    pub fn known(&self) -> Decimal {
        match *self {
            Self::Complete(t) => t,
            Self::Partial { known, .. } => known,
        }
    }

    /// How many receipts are unaccounted for in [`Self::known`].
    pub fn missing(&self) -> usize {
        match *self {
            Self::Complete(_) => 0,
            Self::Partial { missing, .. } => missing,
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete(_))
    }
}

/// Result of a statement-period query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeriodSummary {
    pub from: jiff::civil::Date,
    pub to: jiff::civil::Date,
    pub receipts: Vec<ReceiptSummary>,
    /// One total per currency, sorted by code — normally a single entry. Split up
    /// because adding currencies together is arithmetic on different units, and
    /// the extractor reads the currency off the receipt.
    ///
    /// Summed in Rust: toasty has no SUM, and SQLite holds `Decimal` as TEXT.
    pub totals: Vec<CurrencyTotal>,
}

/// A period's total for one currency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrencyTotal {
    /// ISO code, e.g. `USD`.
    pub currency: String,
    pub total: PeriodTotal,
}

impl PeriodSummary {
    pub fn new(
        from: jiff::civil::Date,
        to: jiff::civil::Date,
        mut receipts: Vec<ReceiptSummary>,
    ) -> Self {
        // Chronological, with a stable tie-break, so the view and the CSV agree
        // regardless of what order the database handed rows back in.
        receipts.sort_by(|a, b| {
            a.purchased_on
                .cmp(&b.purchased_on)
                .then_with(|| a.merchant.cmp(&b.merchant))
        });
        // BTreeMap so the order is by currency code and therefore stable.
        let mut grouped: std::collections::BTreeMap<&str, (Decimal, usize)> =
            std::collections::BTreeMap::new();
        for r in &receipts {
            let entry = grouped
                .entry(r.currency.as_str())
                .or_insert((Decimal::ZERO, 0));
            match r.total {
                Some(t) => entry.0 += t,
                // Counted, never treated as zero.
                None => entry.1 += 1,
            }
        }
        let totals = grouped
            .into_iter()
            .map(|(currency, (known, missing))| CurrencyTotal {
                currency: currency.to_string(),
                total: PeriodTotal::new(known, missing),
            })
            .collect();

        Self {
            from,
            to,
            totals,
            receipts,
        }
    }

    /// Receipts a human should look at before trusting the period figure.
    pub fn needing_attention(&self) -> usize {
        self.receipts
            .iter()
            .filter(|r| !r.problems.is_empty())
            .count()
    }
}

/// The default statement period: the previous calendar month, whole.
///
/// Takes `today` rather than reading the clock so it is testable, and because
/// the client has no clock — `jiff` needs its `js` feature on wasm, so the
/// server supplies the default.
pub fn last_full_month(today: jiff::civil::Date) -> (jiff::civil::Date, jiff::civil::Date) {
    // The day before the 1st of this month is the last day of the previous one,
    // which handles both month lengths and the December/January rollover.
    let to = today
        .first_of_month()
        .yesterday()
        .expect("a date before today's month exists");
    (to.first_of_month(), to)
}

/// Escapes one CSV field per RFC 4180.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Escapes a field the extractor read off an arbitrary image.
///
/// A leading `=`, `+`, `@` or control character makes spreadsheets treat the cell
/// as a formula. Text columns only — we render the amounts ourselves, and a leading
/// `-` there is a negative number.
fn csv_text(value: &str) -> String {
    let dangerous = matches!(value.chars().next(), Some('=' | '+' | '@' | '\t' | '\r'));
    if dangerous {
        csv_field(&format!("'{value}"))
    } else {
        csv_field(value)
    }
}

/// The period as a spreadsheet, one row per line item.
///
/// `receipt_total` appears only on a receipt's first row. Repeated on every row,
/// summing the column would multiply each receipt by its item count. Blank cells
/// mean a plain `SUM` gives the period total.
///
/// `item_amount` won't sum to `receipt_total` — line items are pre-tax.
pub fn receipts_to_csv(receipts: &[Receipt]) -> String {
    let mut out = String::from(
        "date,merchant,receipt_total,currency,item,item_amount,reviewed,receipt_id\r\n",
    );

    for r in receipts {
        let date = r.purchased_on.to_string();
        let merchant = csv_text(&r.merchant);
        let total = r.total.map(|t| t.to_string()).unwrap_or_default();
        let currency = csv_field(&r.currency);
        let reviewed = if r.reviewed { "yes" } else { "no" };

        // A receipt with no line items still gets a row — it is a real charge on
        // the statement, and omitting it would make the CSV disagree with the
        // period total.
        if r.line_items.is_empty() {
            out.push_str(&format!(
                "{date},{merchant},{total},{currency},,,{reviewed},{}\r\n",
                r.id
            ));
            continue;
        }

        for (n, item) in r.line_items.iter().enumerate() {
            let total = if n == 0 { total.as_str() } else { "" };
            out.push_str(&format!(
                "{date},{merchant},{total},{currency},{},{},{reviewed},{}\r\n",
                csv_text(&item.description),
                item.total,
                r.id
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn summary(total: Option<&str>) -> ReceiptSummary {
        summary_in("USD", total)
    }

    fn summary_in(currency: &str, total: Option<&str>) -> ReceiptSummary {
        ReceiptSummary {
            id: Uuid::nil(),
            purchased_on: jiff::civil::date(2026, 7, 1),
            merchant: "M".into(),
            total: total.map(dec),
            currency: currency.into(),
            status: ExtractionStatus::Done,
            item_count: 1,
            reviewed: false,
            problems: Vec::new(),
        }
    }

    /// The period's only total, for the single-currency cases below.
    fn only(s: &PeriodSummary) -> &PeriodTotal {
        assert_eq!(s.totals.len(), 1, "expected one currency: {:?}", s.totals);
        &s.totals[0].total
    }

    #[test]
    fn all_totals_present_is_complete() {
        let s = PeriodSummary::new(
            jiff::civil::date(2026, 7, 1),
            jiff::civil::date(2026, 7, 31),
            vec![summary(Some("10.00")), summary(Some("5.50"))],
        );
        assert_eq!(s.totals[0].currency, "USD");
        assert!(only(&s).is_complete());
        assert_eq!(only(&s).known(), dec("15.50"));
        assert_eq!(only(&s).missing(), 0);
    }

    /// The load-bearing case: a receipt with no total must neither contribute
    /// zero nor vanish silently.
    #[test]
    fn a_missing_total_makes_the_period_partial() {
        let s = PeriodSummary::new(
            jiff::civil::date(2026, 7, 1),
            jiff::civil::date(2026, 7, 31),
            vec![summary(Some("10.00")), summary(None), summary(Some("5.50"))],
        );
        assert!(!only(&s).is_complete(), "must not claim to be complete");
        assert_eq!(only(&s).known(), dec("15.50"), "known excludes the unknown");
        assert_eq!(only(&s).missing(), 1);
        assert!(matches!(only(&s), PeriodTotal::Partial { .. }));
    }

    /// Currencies are never added together. The extractor reads the currency off
    /// the receipt, so one stray CAD receipt in a USD month is reachable, and
    /// summing the two would silently invent money.
    #[test]
    fn each_currency_gets_its_own_total() {
        let s = PeriodSummary::new(
            jiff::civil::date(2026, 7, 1),
            jiff::civil::date(2026, 7, 31),
            vec![
                summary_in("USD", Some("10.00")),
                summary_in("CAD", Some("7.00")),
                summary_in("USD", Some("5.50")),
                // Missing totals belong to their own currency's tally.
                summary_in("CAD", None),
            ],
        );

        // Sorted by code, so CAD comes first.
        let codes: Vec<_> = s.totals.iter().map(|t| t.currency.as_str()).collect();
        assert_eq!(codes, ["CAD", "USD"]);

        assert_eq!(s.totals[0].total.known(), dec("7.00"));
        assert_eq!(s.totals[0].total.missing(), 1);
        assert!(!s.totals[0].total.is_complete());

        assert_eq!(s.totals[1].total.known(), dec("15.50"));
        assert!(s.totals[1].total.is_complete());
    }

    fn item(total: &str) -> LineItem {
        LineItem {
            id: Uuid::nil(),
            description: "x".into(),
            quantity: None,
            unit_price: None,
            total: dec(total),
            position: 0,
            edited: false,
        }
    }

    fn receipt(
        subtotal: Option<&str>,
        tax: Option<&str>,
        total: Option<&str>,
        items: Vec<LineItem>,
    ) -> Receipt {
        Receipt {
            id: Uuid::nil(),
            purchased_on: jiff::civil::date(2026, 7, 1),
            merchant: "M".into(),
            subtotal: subtotal.map(dec),
            tax: tax.map(dec),
            total: total.map(dec),
            currency: "USD".into(),
            status: ExtractionStatus::Done,
            extraction_error: None,
            reviewed: false,
            line_items: items,
        }
    }

    #[test]
    fn a_balanced_receipt_has_no_problems() {
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
        assert!(r.problems().is_empty(), "unexpected: {:?}", r.problems());
        assert!(!r.needs_total());
    }

    #[test]
    fn a_missing_total_is_a_problem() {
        let r = receipt(Some("10.00"), None, None, vec![item("10.00")]);
        assert!(r.needs_total());
        assert_eq!(r.problems().len(), 1);
        assert!(r.problems()[0].contains("No total"));
    }

    /// A dropped or duplicated line item shows up here and nowhere else.
    #[test]
    fn items_not_matching_the_subtotal_is_a_problem() {
        let r = receipt(
            Some("32.82"),
            Some("2.54"),
            Some("35.36"),
            vec![item("11.72"), item("2.96")], // three items missing
        );
        let p = r.problems();
        assert!(
            p.iter().any(|s| s.contains("Line items add up to 14.68")),
            "got {p:?}"
        );
    }

    #[test]
    fn subtotal_plus_tax_not_matching_the_total_is_a_problem() {
        let r = receipt(
            Some("10.00"),
            Some("1.00"),
            Some("99.00"),
            vec![item("10.00")],
        );
        let p = r.problems();
        assert!(p.iter().any(|s| s.contains("off by -88")), "got {p:?}");
    }

    /// An unreadable amount is stored as zero rather than guessed, so it has to
    /// be called out explicitly — it would otherwise look like a free item.
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

    #[test]
    fn problems_accumulate_rather_than_short_circuiting() {
        // No total AND items that don't match the subtotal.
        let r = receipt(Some("50.00"), None, None, vec![item("10.00")]);
        assert_eq!(r.problems().len(), 2, "got {:?}", r.problems());
    }

    /// No receipts means no currency to report a total in, rather than a zero in
    /// some assumed one.
    #[test]
    fn an_empty_period_has_no_totals() {
        let s = PeriodSummary::new(
            jiff::civil::date(2026, 7, 1),
            jiff::civil::date(2026, 7, 31),
            vec![],
        );
        assert!(s.totals.is_empty());
        assert_eq!(s.needing_attention(), 0);
    }

    /// A taxed receipt that balances perfectly must not be flagged. This is the
    /// case a naive items-vs-total comparison gets wrong, reporting a difference
    /// exactly equal to the tax on every correct receipt.
    #[test]
    fn tax_is_not_mistaken_for_a_mismatch() {
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
    fn the_default_period_is_the_previous_whole_month() {
        // Mid-month.
        assert_eq!(
            last_full_month(jiff::civil::date(2026, 8, 6)),
            (
                jiff::civil::date(2026, 7, 1),
                jiff::civil::date(2026, 7, 31)
            )
        );
        // On the 1st, "last month" is still the month before, not this one.
        assert_eq!(
            last_full_month(jiff::civil::date(2026, 8, 1)),
            (
                jiff::civil::date(2026, 7, 1),
                jiff::civil::date(2026, 7, 31)
            )
        );
        // Year rollover.
        assert_eq!(
            last_full_month(jiff::civil::date(2026, 1, 15)),
            (
                jiff::civil::date(2025, 12, 1),
                jiff::civil::date(2025, 12, 31)
            )
        );
        // February, leap and common, taking the end-of-month from the calendar
        // rather than a hardcoded length.
        assert_eq!(
            last_full_month(jiff::civil::date(2024, 3, 10)).1,
            jiff::civil::date(2024, 2, 29)
        );
        assert_eq!(
            last_full_month(jiff::civil::date(2026, 3, 10)).1,
            jiff::civil::date(2026, 2, 28)
        );
    }

    fn full(merchant: &str, total: Option<&str>, items: Vec<LineItem>) -> Receipt {
        let mut r = receipt(None, None, total, items);
        r.merchant = merchant.into();
        r
    }

    fn named(description: &str, total: &str) -> LineItem {
        let mut i = item(total);
        i.description = description.into();
        i
    }

    #[test]
    fn csv_repeats_the_receipt_total_on_no_row_but_the_first() {
        let csv = receipts_to_csv(&[full(
            "Walmart",
            Some("35.36"),
            vec![named("Milk", "4.99"), named("Bread", "3.50")],
        )]);
        let lines: Vec<_> = csv.lines().collect();
        assert_eq!(lines.len(), 3, "header + 2 items: {csv}");
        assert!(
            lines[1].contains(",35.36,"),
            "first row carries it: {}",
            lines[1]
        );
        assert!(
            lines[2].starts_with("2026-07-01,Walmart,,USD,Bread,3.50,"),
            "second row's total column is blank: {}",
            lines[2]
        );
    }

    /// Without this row, a receipt whose items failed to extract would be in the
    /// period total but absent from the export.
    #[test]
    fn a_receipt_with_no_line_items_still_gets_a_row() {
        let csv = receipts_to_csv(&[full("Shell", Some("42.00"), vec![])]);
        let lines: Vec<_> = csv.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[1],
            "2026-07-01,Shell,42.00,USD,,,no,00000000-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn csv_quotes_commas_and_quotes() {
        let csv = receipts_to_csv(&[full(
            "Bob's, Inc",
            Some("1.00"),
            vec![named("6\" sub", "1.00")],
        )]);
        assert!(csv.contains("\"Bob's, Inc\""), "{csv}");
        assert!(csv.contains("\"6\"\" sub\""), "{csv}");
    }

    /// Descriptions come from a model reading an arbitrary image, so a leading
    /// `=` is untrusted input heading into a spreadsheet.
    #[test]
    fn csv_defuses_spreadsheet_formulas_in_text_but_not_amounts() {
        let csv = receipts_to_csv(&[full(
            "=cmd|'/c calc'!A1",
            Some("-5.00"),
            vec![named("+refund", "-5.00")],
        )]);
        assert!(csv.contains("'=cmd|"), "merchant neutralized: {csv}");
        assert!(csv.contains("'+refund"), "description neutralized: {csv}");
        // The negative amount is ours, not the model's, and must stay a number.
        assert!(csv.contains(",-5.00,"), "amount untouched: {csv}");
        assert!(!csv.contains("'-5.00"), "amount must not be quoted: {csv}");
    }

    #[test]
    fn period_receipts_come_back_chronologically() {
        let mut a = summary(Some("1.00"));
        a.purchased_on = jiff::civil::date(2026, 7, 20);
        let mut b = summary(Some("2.00"));
        b.purchased_on = jiff::civil::date(2026, 7, 2);
        let s = PeriodSummary::new(
            jiff::civil::date(2026, 7, 1),
            jiff::civil::date(2026, 7, 31),
            vec![a, b],
        );
        let dates: Vec<_> = s.receipts.iter().map(|r| r.purchased_on.day()).collect();
        assert_eq!(dates, [2, 20]);
    }
}
