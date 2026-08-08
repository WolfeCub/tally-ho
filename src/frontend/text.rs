//! Wording that would otherwise be a ternary in the middle of the markup.

/// "1 receipt", "2 receipts". Every noun this app counts is regular.
pub fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
