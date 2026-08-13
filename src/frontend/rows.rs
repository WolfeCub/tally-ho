//! The rows a form is editing.
//!
//! Held in a signal rather than the DOM, so adding and removing one costs nothing
//! until you save and the whole list goes to the server in one request. The line
//! items and the people are both edited this way.

use leptos::prelude::*;

/// A row a form can find again after an edit.
///
/// The key is stable for the life of the row, so [`leptos::prelude::For`] never
/// rebuilds a box you're typing in.
pub trait Keyed {
    fn key(&self) -> usize;
}

/// Editing one row of a list held in a signal.
pub trait Rows<T> {
    /// Adds a row, handing `make` the key it will answer to.
    fn add(&self, make: impl FnOnce(usize) -> T);

    fn remove(&self, key: usize);

    /// Applies `edit` to one row, if it is still there.
    fn edit(&self, key: usize, edit: impl FnOnce(&mut T));
}

impl<T: Keyed + Send + Sync + 'static> Rows<T> for RwSignal<Vec<T>> {
    fn add(&self, make: impl FnOnce(usize) -> T) {
        self.update(|rows| {
            // Only has to be unique among the rows on screen, so the row that
            // was removed to make room is welcome to its key back.
            let key = rows.iter().map(Keyed::key).max().map_or(0, |last| last + 1);
            rows.push(make(key));
        });
    }

    fn remove(&self, key: usize) {
        self.update(|rows| rows.retain(|row| row.key() != key));
    }

    fn edit(&self, key: usize, edit: impl FnOnce(&mut T)) {
        self.update(|rows| {
            if let Some(row) = rows.iter_mut().find(|row| row.key() == key) {
                edit(row);
            }
        });
    }
}
