//! Whether a statement line and a receipt name the same merchant.
//!
//! Statements mangle the name — "SQ *BLUE BOTTLE, SEATTLE WA" for a receipt
//! printed "Blue Bottle Coffee" — so the two are never compared whole. One
//! distinguishing word in common is enough; the rest of the line is address,
//! store number and processor noise.

/// Words too common to name anybody. Without these every cafe matches every
/// other one.
const GENERIC: &[&str] = &[
    "and",
    "bar",
    "cafe",
    "coffee",
    "com",
    "company",
    "corp",
    "grill",
    "inc",
    "kitchen",
    "llc",
    "ltd",
    "market",
    "restaurant",
    "shop",
    "store",
    "the",
    "www",
];

/// Shortest word worth comparing, which drops the "SQ *" and "TST*" a card
/// processor puts in front.
const SHORTEST: usize = 3;

/// A word this long with another starting on it is one name squashed or cut
/// short: "WHOLEFDS" against the "Whole" of Whole Foods.
const STEM: usize = 5;

/// One statement line, ready to compare against a pool of receipts.
pub struct Line(Vec<String>);

impl Line {
    pub fn new(description: &str) -> Self {
        Self(words(description))
    }

    /// True if the line and the name the receipt printed have a word in common.
    pub fn names(&self, printed: &str) -> bool {
        let printed = words(printed);
        self.0
            .iter()
            .any(|charged| printed.iter().any(|word| alike(charged, word)))
    }
}

/// The words in a name worth comparing: lowercased, punctuation and store
/// numbers gone.
fn words(name: &str) -> Vec<String> {
    name.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.len() >= SHORTEST)
        .filter(|word| !word.chars().all(char::is_numeric))
        .filter(|word| !GENERIC.contains(word))
        .map(str::to_string)
        .collect()
}

/// Whether two words are the same word however it got printed: truncated,
/// pluralised, abbreviated, or a character off after a bad read.
fn alike(a: &str, b: &str) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    (short.len() >= STEM && long.starts_with(short))
        // One edit per five characters: "costco" may lose one, "restaurant" two.
        || distance(short, long) <= long.len() / 5
}

/// Levenshtein distance, a row at a time.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, from) in a.chars().enumerate() {
        let mut corner = row[0];
        row[0] = i + 1;
        for (j, to) in b.iter().enumerate() {
            let cost = if from == *to {
                corner
            } else {
                1 + corner.min(row[j]).min(row[j + 1])
            };
            corner = row[j + 1];
            row[j + 1] = cost;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(description: &str, printed: &str) -> bool {
        Line::new(description).names(printed)
    }

    /// The shapes statements actually print, against what a receipt says.
    #[test]
    fn a_mangled_statement_line_still_names_the_merchant() {
        for (description, merchant) in [
            ("COSTCO WHSE #1050", "Costco Wholesale"),
            ("SQ *BLUE BOTTLE, SEATTLE WA", "Blue Bottle Coffee"),
            ("TST* CAFE FLORA", "Cafe Flora"),
            ("UBER EATS", "Uber Eats"),
            ("MCDONALDS F1234", "McDonald's"),
            ("STARBUCKS #1149", "Starbucks Coffee Company"),
            // Abbreviated on one side and misread on the other.
            ("WHOLEFDS MKT 10259", "Whole Foods Market"),
            ("TRADER JOE'S #130", "Trader Joes"),
        ] {
            assert!(same(description, merchant), "{description} / {merchant}");
        }
    }

    #[test]
    fn different_merchants_are_not_confused() {
        for (description, merchant) in [
            ("SHELL OIL 4471", "Chevron"),
            ("COSTCO WHSE #1050", "Safeway"),
            ("SQ *BLUE BOTTLE, SEATTLE WA", "Somewhere"),
            // Nothing but generic words in common.
            ("THE COFFEE STORE", "Corner Coffee Bar"),
            // Two edits is past what a bad read explains.
            ("PANDA EXPRESS 3011", "Panera Bread"),
        ] {
            assert!(!same(description, merchant), "{description} / {merchant}");
        }
    }

    /// A name that boils down to nothing can't corroborate anything.
    #[test]
    fn a_name_with_no_words_matches_nobody() {
        assert!(!same("SQ *4471", "Costco"));
        assert!(!same("COSTCO WHSE #1050", ""));
    }
}
