//! Reading the CSV a card exports.
//!
//! Cards disagree on what the columns are called and what order they come in,
//! so they're found by header name rather than per issuer. Which ones got picked
//! comes back in the [`Layout`] for the import screen to show — a wrong guess
//! has to be visible, or it exports a statement that quietly doesn't add up.
//!
//! Amounts are taken as written: positive is a purchase, negative a refund.

use rust_decimal::Decimal;

use crate::server::parse;

/// Some exports open with a line or two of account details before the header.
const HEADER_SEARCH_ROWS: usize = 10;

/// Header names in order of preference, matched as substrings. A posted date can
/// be days after the receipt was printed, so it comes last.
const DATE: &[&str] = &["transaction date", "trans date", "purchase date", "date"];
const DESCRIPTION: &[&str] = &[
    "description",
    "merchant",
    "payee",
    "vendor",
    "memo",
    "details",
];
const AMOUNT: &[&str] = &["transaction amount", "amount", "total", "value"];

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error(
        "could not find date, description and amount columns in the first \
         {HEADER_SEARCH_ROWS} rows — is this the CSV your card exports?"
    )]
    NoColumns,
    #[error("no charges could be read — {0}")]
    NoCharges(String),
    #[error("could not read the file as CSV: {0}")]
    Csv(#[from] csv::Error),
}

/// One charge as the file had it.
#[derive(Debug)]
pub struct ParsedCharge {
    pub charged_on: jiff::civil::Date,
    pub description: String,
    /// Positive for a purchase, negative for a refund.
    pub amount: Decimal,
}

/// A column the sniffer picked: where it is, and what the file called it.
#[derive(Debug)]
pub struct Column {
    index: usize,
    pub name: String,
}

impl Column {
    /// A header named exactly what we want beats one that merely contains it, so
    /// a "Date Processed" or "Extended Details" column sitting ahead of the real
    /// one can't take it.
    fn find(row: &csv::StringRecord, wanted: &[&str]) -> Option<Self> {
        let headers: Vec<String> = row.iter().map(normalize).collect();
        let exactly = |want: &&str| headers.iter().position(|header| header == want);
        let containing = |want: &&str| headers.iter().position(|header| header.contains(*want));
        let index = wanted
            .iter()
            .find_map(exactly)
            .or_else(|| wanted.iter().find_map(containing))?;
        Some(Self {
            index,
            name: row[index].trim().to_string(),
        })
    }

    fn read<'a>(&self, row: &'a csv::StringRecord) -> &'a str {
        row.get(self.index).unwrap_or_default().trim()
    }
}

/// The three columns a statement is read through.
#[derive(Debug)]
pub struct Layout {
    pub date: Column,
    pub description: Column,
    pub amount: Column,
}

impl Layout {
    /// `None` when this row isn't the header, which is how the account details
    /// at the top of a file get skipped.
    fn sniff(row: &csv::StringRecord) -> Option<Self> {
        Some(Self {
            date: Column::find(row, DATE)?,
            description: Column::find(row, DESCRIPTION)?,
            amount: Column::find(row, AMOUNT)?,
        })
    }

    fn charge(&self, row: &csv::StringRecord) -> Result<ParsedCharge, String> {
        let date = self.date.read(row);
        let amount = self.amount.read(row);
        Ok(ParsedCharge {
            charged_on: parse::date(date).ok_or_else(|| format!("{date:?} is not a date"))?,
            // Zero counts as no amount: it's a summary artefact, not something
            // to reconcile.
            amount: parse::money(amount)
                .filter(|a| !a.is_zero())
                .ok_or_else(|| format!("no amount in {amount:?}"))?,
            // Statements pad descriptions out with runs of spaces.
            description: self
                .description
                .read(row)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        })
    }
}

#[derive(Debug)]
pub struct Parsed {
    /// In file order.
    pub charges: Vec<ParsedCharge>,
    pub layout: Layout,
    /// Rows read but not understood, and why. Surfaced rather than dropped
    /// quietly — a missing row is a statement that won't reconcile.
    pub skipped: Vec<String>,
}

impl Parsed {
    /// The period the file covers, taken from the charges themselves.
    pub fn range(&self) -> (jiff::civil::Date, jiff::civil::Date) {
        let dates = || self.charges.iter().map(|c| c.charged_on);
        // `charges` never returns without one.
        (dates().min().unwrap(), dates().max().unwrap())
    }
}

/// Lowercased and stripped of punctuation, so "Transaction Date",
/// "TRANSACTION_DATE" and "transaction  date" all sniff alike.
fn normalize(header: &str) -> String {
    let mut out = String::with_capacity(header.len());
    for ch in header.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim().to_string()
}

/// Reads the charges out of an uploaded CSV.
pub fn charges(bytes: &[u8]) -> Result<Parsed, ParseError> {
    // Bank exports aren't reliably UTF-8 — accented merchant names arrive as
    // Windows-1252. Lossy, because a mangled character in a description is
    // nothing next to refusing the file.
    let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes));

    let mut reader = csv::ReaderBuilder::new()
        // A trailing summary line is shorter than the header, and worth skipping
        // rather than failing on.
        .flexible(true)
        .has_headers(false)
        .from_reader(text.as_bytes());

    let all: Vec<csv::StringRecord> = reader.records().collect::<Result<_, _>>()?;
    let rows: Vec<&csv::StringRecord> = all
        .iter()
        .filter(|row| row.iter().any(|cell| !cell.trim().is_empty()))
        .collect();

    let (header_row, layout) = rows
        .iter()
        .take(HEADER_SEARCH_ROWS)
        .enumerate()
        .find_map(|(index, row)| Layout::sniff(row).map(|layout| (index, layout)))
        .ok_or(ParseError::NoColumns)?;

    let mut charges = Vec::new();
    let mut skipped = Vec::new();
    for (offset, row) in rows[header_row + 1..].iter().enumerate() {
        match layout.charge(row) {
            Ok(charge) => charges.push(charge),
            // Numbered as a human counts lines, header included.
            Err(why) => skipped.push(format!("line {}: {why}", header_row + offset + 2)),
        }
    }

    if charges.is_empty() {
        return Err(ParseError::NoCharges(match skipped.first() {
            Some(first) => format!("{} unreadable rows, starting {first}", skipped.len()),
            None => "there's nothing after the header".to_string(),
        }));
    }

    Ok(Parsed {
        charges,
        layout,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::dec;

    fn parsed(csv: &str) -> Parsed {
        charges(csv.as_bytes()).expect("should parse")
    }

    /// The columns the sniffer names are the ones the import screen shows, so
    /// they have to be the file's own spelling, not ours.
    #[test]
    fn finds_the_columns_by_name_whatever_the_order() {
        let p = parsed(
            "Transaction Date,Post Date,Description,Category,Type,Amount\n\
             07/04/2026,07/06/2026,COSTCO WHSE #1050,Groceries,Sale,35.36\n",
        );
        assert_eq!(p.layout.date.name, "Transaction Date");
        assert_eq!(p.layout.description.name, "Description");
        assert_eq!(p.layout.amount.name, "Amount");
        assert_eq!(p.charges[0].charged_on, jiff::civil::date(2026, 7, 4));
        assert_eq!(p.charges[0].description, "COSTCO WHSE #1050");
        assert_eq!(p.charges[0].amount, dec("35.36"));
    }

    /// A posted date can be days after the receipt was printed, so it's only
    /// used when the file has nothing better. Matching leans on this date.
    #[test]
    fn prefers_a_transaction_date_but_settles_for_a_posted_one() {
        let p = parsed("Posted Date,Description,Amount\n07/06/2026,CAFE,4.50\n");
        assert_eq!(p.layout.date.name, "Posted Date");
        assert_eq!(p.charges[0].charged_on, jiff::civil::date(2026, 7, 6));
    }

    #[test]
    fn reads_the_shapes_cards_actually_export() {
        // Amex: three columns, nothing else.
        let p = parsed("Date,Description,Amount\n07/09/2026,NETFLIX.COM,17.99\n");
        assert_eq!(p.charges.len(), 1);
        assert_eq!(p.charges[0].amount, dec("17.99"));

        // Underscored and cased differently, with the amount signed and quoted.
        let p = parsed(
            "TRANS_DATE,MERCHANT_NAME,TRANSACTION_AMOUNT\n\
             2026-07-11,\"SQ *BLUE BOTTLE, SEATTLE WA\",\"$1,024.00\"\n",
        );
        assert_eq!(p.layout.description.name, "MERCHANT_NAME");
        assert_eq!(p.charges[0].description, "SQ *BLUE BOTTLE, SEATTLE WA");
        assert_eq!(p.charges[0].amount, dec("1024.00"));

        // Some cards call the description "details" and quote every header.
        let p = parsed(
            "\"transaction_date\",\"post_date\",\"type\",\"details\",\"amount\",\"currency\"\n\
             \"2026-07-14\",\"2026-07-15\",\"Purchase\",\"UBER EATS\",\"28.41\",\"USD\"\n",
        );
        assert_eq!(p.layout.date.name, "transaction_date");
        assert_eq!(p.layout.description.name, "details");
        assert_eq!(p.charges[0].description, "UBER EATS");
        assert_eq!(p.charges[0].amount, dec("28.41"));

        // A second date column ahead of the plain one doesn't get picked.
        let p = parsed(
            "Date,Date Processed,Description,Card Member,Account #,Amount\n\
             07/16/2026,07/18/2026,SAFEWAY,A CARDHOLDER,-91007,63.28\n",
        );
        assert_eq!(p.layout.date.name, "Date");
        assert_eq!(p.charges[0].charged_on, jiff::civil::date(2026, 7, 16));
        assert_eq!(p.charges[0].amount, dec("63.28"));

        // The same export with the month written out, which is how Amex dates
        // it. Every row is the same shape, so this took the whole file down.
        let p = parsed(
            "Date,Date Processed,Description,Card Member,Account #,Amount\n\
             28 Jul 2026,28 Jul 2026,MEMBERSHIP FEE INSTALLMENT,A CARDHOLDER,-01006,15.99\n\
             28 Jul 2026,28 Jul 2026,NOODLE HOUSE        OAKVILLE,A CARDHOLDER,-01006,7.91\n",
        );
        assert_eq!(p.skipped, Vec::<String>::new());
        assert_eq!(p.charges[0].charged_on, jiff::civil::date(2026, 7, 28));
        assert_eq!(p.charges[0].description, "MEMBERSHIP FEE INSTALLMENT");
        assert_eq!(p.charges[1].description, "NOODLE HOUSE OAKVILLE");
        assert_eq!(p.charges[1].amount, dec("7.91"));
    }

    /// A refund stays negative: it belongs on the statement, and it comes off
    /// somebody's share rather than being dropped.
    #[test]
    fn a_refund_keeps_its_sign() {
        let p = parsed(
            "Date,Description,Amount\n\
             07/09/2026,TARGET,40.00\n\
             07/12/2026,TARGET REFUND,-12.00\n",
        );
        let amounts: Vec<_> = p.charges.iter().map(|c| c.amount).collect();
        assert_eq!(amounts, [dec("40.00"), dec("-12.00")]);
    }

    #[test]
    fn skips_account_details_above_the_header() {
        let p = parsed(
            "Account Number,****1234\n\
             Statement Period,July 2026\n\
             \n\
             Transaction Date,Description,Amount\n\
             07/04/2026,SHELL OIL,52.10\n",
        );
        assert_eq!(p.charges.len(), 1, "{:?}", p.charges);
        assert_eq!(p.charges[0].description, "SHELL OIL");
    }

    /// An unreadable row is called out. Dropping one silently would export a
    /// statement whose total is short and give nothing to notice it by.
    #[test]
    fn unreadable_rows_are_reported_not_dropped() {
        let p = parsed(
            "Date,Description,Amount\n\
             07/04/2026,SHELL OIL,52.10\n\
             ,MISSING EVERYTHING,\n\
             07/05/2026,PENDING,0.00\n\
             Total,,52.10\n",
        );
        assert_eq!(p.charges.len(), 1);
        assert_eq!(p.skipped.len(), 3, "{:?}", p.skipped);
        assert!(p.skipped[0].starts_with("line 3:"), "{:?}", p.skipped);
        assert!(p.skipped[1].contains("no amount"), "{:?}", p.skipped);
    }

    #[test]
    fn the_period_comes_from_the_charges() {
        let p = parsed(
            "Date,Description,Amount\n\
             07/20/2026,LATER,1.00\n\
             07/03/2026,EARLIER,2.00\n",
        );
        assert_eq!(
            p.range(),
            (
                jiff::civil::date(2026, 7, 3),
                jiff::civil::date(2026, 7, 20)
            )
        );
    }

    #[test]
    fn a_file_that_isnt_a_statement_is_refused() {
        let err = charges(b"one,two,three\n1,2,3\n").unwrap_err();
        assert!(matches!(err, ParseError::NoColumns), "{err}");

        let err = charges(b"Date,Description,Amount\n").unwrap_err();
        assert!(matches!(err, ParseError::NoCharges(_)), "{err}");
    }
}
