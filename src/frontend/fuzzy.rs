//! Narrowing a long list by typing at it.
//!
//! A statement's worth of receipts is more than anyone reads through a dropdown,
//! and the useful thing to type is rarely a prefix: "cf 35" for a Cafe Flora
//! receipt at $35.36, where the date sits between the two.

/// How well `query` matches `text`, **lower being better**, or `None` when it
/// doesn't match at all.
///
/// The query's characters have to appear in `text` in order, but not together,
/// so any run of the words on screen will do, in any combination. Whitespace in
/// the query is ignored, which is what lets "cf 35" work at all.
///
/// A character landing at the start of a word is free, however far it was to
/// get there. Anywhere else costs, and costs more the further it skipped. So
/// "bb" puts Blue Bottle above Rabbit hutch, which has the same two letters
/// buried in one word.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let query: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    // One pass left to right would spend the "b" of "bottle" on the B of "Blue
    // Bottle" and then pay for the gap, so every place the first character
    // appears gets a turn and the cheapest of them wins.
    match query.first() {
        None => Some(0),
        Some(first) => (0..text.len())
            .filter(|&start| text[start] == *first)
            .filter_map(|start| cost_from(&query, &text, start))
            .min(),
    }
}

/// One left-to-right pass beginning at `start`, taking each character as early
/// as it can.
fn cost_from(query: &[char], text: &[char], start: usize) -> Option<i32> {
    let mut at = start;
    let mut cost = 0;

    for wanted in query {
        let found = at + text[at..].iter().position(|c| c == wanted)?;
        if found > 0 && text[found - 1].is_alphanumeric() {
            cost += 1 + (found - at) as i32;
        }
        at = found + 1;
    }

    Some(cost)
}

/// The options a query matches, best first. Ties keep the order they came in,
/// which is the order whatever built the list thought best.
pub fn matching<T: Clone>(query: &str, options: &[(T, String)]) -> Vec<(T, String)> {
    let mut found: Vec<_> = options
        .iter()
        .filter_map(|(value, text)| Some((score(query, text)?, value.clone(), text.clone())))
        .collect();
    // Stable, so equal scores stay in the caller's order.
    found.sort_by_key(|(cost, ..)| *cost);
    found
        .into_iter()
        .map(|(_, value, text)| (value, text))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Vec<(u8, String)> {
        vec![
            (1, "2026-08-01 · Costco · $35.36".into()),
            (2, "2026-08-05 · Cafe Flora · $48.00".into()),
            (3, "2026-08-15 · Blue Bottle · $6.25".into()),
        ]
    }

    fn ids(query: &str) -> Vec<u8> {
        matching(query, &options())
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    #[test]
    fn nothing_typed_leaves_the_list_alone() {
        assert_eq!(ids(""), [1, 2, 3]);
    }

    #[test]
    fn a_word_from_anywhere_in_the_option_finds_it() {
        assert_eq!(ids("costco"), [1]);
        assert_eq!(ids("flora"), [2]);
        // The amount, which is the end of the line rather than the start.
        assert_eq!(ids("6.25"), [3]);
    }

    /// The point of fuzzy rather than substring: the initials of the words, and
    /// the parts of a line that aren't next to each other.
    #[test]
    fn scattered_characters_match_in_order() {
        assert_eq!(ids("bb"), [3]);
        assert_eq!(ids("cf 48"), [2]);
        assert_eq!(ids("cos 35"), [1]);
    }

    #[test]
    fn out_of_order_and_absent_characters_match_nothing() {
        assert!(ids("ocstco").is_empty());
        assert!(ids("waitrose").is_empty());
    }

    /// Word starts are free, so the option where the query lands on them ranks
    /// above one where the same characters are buried mid-word.
    #[test]
    fn word_starts_outrank_the_middle_of_a_word() {
        let options = vec![
            (1, "Rabbit hutch".to_string()),
            (2, "Blue Bottle".to_string()),
        ];
        // Both contain a b and then another b, so it's the ranking that
        // separates them and not the filter.
        assert_eq!(matching("bb", &options).len(), 2);
        assert_eq!(matching("bb", &options)[0].0, 2);
    }

    /// A whole word typed out should cost the same wherever in the text it sits.
    /// One pass left to right scores this worse, having spent the "b" on "Blue"
    /// and then paid for the gap to "Bottle".
    #[test]
    fn the_cheapest_place_to_start_is_the_one_used() {
        assert_eq!(score("bottle", "Blue Bottle"), score("bottle", "Bottle"));
    }

    #[test]
    fn matching_ignores_case_either_way_round() {
        assert_eq!(ids("COSTCO"), [1]);
        assert_eq!(ids("cOsTcO"), [1]);
    }
}
