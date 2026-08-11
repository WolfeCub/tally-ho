//! The receipt list tab.

use leptos::prelude::*;

use crate::frontend::components::{ReceiptRows, failed, loading};
use crate::frontend::poll::poll_while;
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

    view! {
        <h1 class="mb-4 text-xl font-semibold">"Receipts"</h1>
        // Transition rather than Suspense: polling refetches, and a fallback
        // would blank the whole list every tick while a receipt is being read.
        <Transition fallback=loading>
            {move || Suspend::new(async move {
                match receipts.await {
                    Err(e) => failed(e),
                    Ok(rows) if rows.is_empty() => {
                        view! { <p class="text-muted">"No receipts yet."</p> }.into_any()
                    }
                    Ok(rows) => view! { <ReceiptRows rows /> }.into_any(),
                }
            })}
        </Transition>
    }
}
