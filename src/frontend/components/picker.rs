//! Picking one of a long list of things by typing at it.

use leptos::prelude::*;
use uuid::Uuid;

use super::INPUT;
use crate::frontend::fuzzy;

/// The most options to put on screen at once. Past this it's a list to scroll
/// rather than an answer, and the line below the list says what's still hidden.
const SHOWN: usize = 8;

/// A list narrowed by what you type, best match first.
///
/// Not a `<select>`: a card's worth of receipts is far more than anyone reads
/// through a native dropdown, and what you want to type is usually a word from
/// the middle of the line rather than the letter it starts with.
#[component]
pub fn FuzzyPick(
    /// Placeholder, and the accessible name: there's no room for a label.
    prompt: &'static str,
    /// What can be picked, best first: the id to hand back, and the line to show
    /// for it. Ties in the search keep this order.
    options: Vec<(Uuid, String)>,
    /// Called with whichever one was picked.
    on_pick: impl Fn(Uuid) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let options = StoredValue::new(options);
    let query = RwSignal::new(String::new());
    // Shut until asked for, so a row offering three of these isn't three open
    // lists.
    let open = RwSignal::new(false);

    let found = move || options.with_value(|all| fuzzy::matching(&query.get(), all));

    view! {
        // The list is positioned against this, rather than making the row taller
        // every time it opens.
        <div class="relative min-w-48 flex-1">
            <input
                type="search"
                class=format!("{INPUT} min-h-11 w-full text-sm")
                placeholder=prompt
                aria-label=prompt
                prop:value=move || query.get()
                on:focus=move |_| open.set(true)
                on:blur=move |_| open.set(false)
                on:input:target=move |ev| {
                    query.set(ev.target().value());
                    open.set(true);
                }
            />

            {move || {
                open.get()
                    .then(|| {
                        let found = found();
                        let hidden = found.len().saturating_sub(SHOWN);
                        view! {
                            <ul class="absolute inset-x-0 top-full z-20 mt-1 max-h-72 overflow-y-auto rounded-lg border border-edge bg-surface py-1 shadow-lg">
                                {found
                                    .into_iter()
                                    .take(SHOWN)
                                    .map(|(id, text)| {
                                        view! {
                                            <li>
                                                <button
                                                    type="button"
                                                    class="min-h-11 w-full px-3 text-left text-sm active:bg-edge"
                                                    // Not `on:click`: closing on blur would race it,
                                                    // and pointerdown lands first.
                                                    on:pointerdown=move |_| {
                                                        query.set(String::new());
                                                        open.set(false);
                                                        on_pick(id);
                                                    }
                                                >
                                                    {text}
                                                </button>
                                            </li>
                                        }
                                    })
                                    .collect_view()}

                                // Never a silently short list: say what typing more would reach.
                                {(hidden > 0)
                                    .then(|| {
                                        view! {
                                            <li class="px-3 py-2 text-xs text-muted">
                                                {format!("{hidden} more, keep typing")}
                                            </li>
                                        }
                                    })}
                            </ul>
                        }
                    })
            }}
        </div>
    }
}
