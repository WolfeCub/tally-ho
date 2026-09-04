//! Several receipts landing at once.
//!
//! What the capture screen shows in place of its stage bar. The queue reads them
//! one at a time, so the rows filling in one by one is the progress.

use leptos::prelude::*;
use uuid::Uuid;

use crate::frontend::components::ReceiptRows;
use crate::frontend::poll::poll_while;
use crate::frontend::screen;
use crate::shared::api::uploaded_receipts;

#[component]
pub fn Batch(ids: Memo<Vec<Uuid>>) -> impl IntoView {
    let tick = RwSignal::new(0u32);
    let rows = Resource::new(
        move || (ids.get(), tick.get()),
        |(ids, _)| async move { uploaded_receipts(ids).await },
    );

    // Starts settled, unlike the receipt list: nothing is being read until this
    // screen's own upload puts something there.
    let waiting = Memo::new(move |prev: Option<&bool>| match rows.get() {
        Some(Ok(rows)) => rows.iter().any(|r| !r.status.is_terminal()),
        _ => prev.copied().unwrap_or(false),
    });
    poll_while(tick, move || waiting.get());

    // Empty only if every one of them was deleted from another tab while this
    // sat here.
    screen::listing(rows, "Those receipts are gone.", move |rows| {
        view! { <ReceiptRows rows hide_checked=Signal::derive(|| false) /> }.into_any()
    })
}
