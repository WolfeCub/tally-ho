//! Line items written on their own. Only the guesses come through here;
//! everything else about an item arrives with the review screen's save, in
//! [`super::receipts::save`].

use crate::server::assign::{self, Guess};
use crate::server::models;

/// Writes what the model guessed about a receipt's items, and hands back how many
/// it named. The two lists line up, so the items must be in the order they were
/// guessed about.
///
/// A guess replaces the last guess and never an answer. That an item was
/// deliberately left unassigned isn't recorded anywhere, so it can be guessed at
/// again.
pub async fn apply_guesses(
    db: &mut toasty::Db,
    items: Vec<models::LineItem>,
    guesses: Vec<Option<Guess>>,
) -> toasty::Result<usize> {
    let mut named = 0;
    let mut tx = db.transaction().await?;

    for (mut item, guess) in items.into_iter().zip(guesses) {
        if assign::decided(&item) {
            continue;
        }
        named += usize::from(guess.is_some());

        let (person_id, why) = match guess {
            Some(guess) => (Some(guess.person_id), Some(guess.why)),
            None => (None, None),
        };
        // Saying the same thing again is not worth a write.
        if item.person_id == person_id && item.guessed_why == why {
            continue;
        }

        toasty::update!(item {
            person_id: person_id,
            guessed_why: why,
        })
        .exec(&mut tx)
        .await?;
    }

    tx.commit().await?;
    Ok(named)
}
