//! The receipt list tab.

use leptos::prelude::*;

use crate::frontend::components::{ReceiptRows, Toggle};
use crate::frontend::poll::poll_while;
use crate::frontend::screen;
use crate::shared::api::recent_receipts;

#[component]
pub fn ReceiptListPage() -> impl IntoView {
    // A row saying "reading…" keeps saying it unless the list is re-asked.
    let tick = RwSignal::new(0u32);
    let receipts = Resource::new(move || tick.get(), |_| recent_receipts(100));

    // Poll only while something is actually being read, which on a settled list
    // is never.
    //
    // The resource reads as `None` both before its first load and during every
    // refetch, and neither means "nothing left to wait for" — taking it that way
    // stops the timer on the first tick, which is to say immediately. So hold the
    // last real answer, and start out assuming there is something to wait for:
    // one load where everything is terminal turns the polling off, and nothing
    // else does.
    let waiting = Memo::new(move |prev: Option<&bool>| match receipts.get() {
        Some(Ok(rows)) => rows.iter().any(|r| !r.status.is_terminal()),
        _ => prev.copied().unwrap_or(true),
    });

    poll_while(tick, move || waiting.get());

    // Held here rather than in the list, which is rebuilt on every poll.
    let hide_checked = RwSignal::new(false);

    view! {
        <h1 class="mb-4 text-xl font-semibold">"Receipts"</h1>

        <div class="mb-3 flex">
            <Toggle label="Hide checked" on=hide_checked />
        </div>

        {screen::listing(
            receipts,
            "No receipts yet.",
            move |rows| view! { <ReceiptRows rows hide_checked /> }.into_any(),
        )}
    }
}
