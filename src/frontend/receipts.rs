//! The receipt list tab.

use leptos::prelude::*;

use crate::frontend::actions::error_of;
use crate::frontend::components::{CHIP, ReceiptRows, Toggle, error_notice};
use crate::frontend::poll::poll_while;
use crate::frontend::screen;
use crate::shared::api::{create_manual_receipt, recent_receipts};

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

    // In the URL, because opening a receipt tears this page down and a plain
    // signal would go with it. `replace` so the back button doesn't have to undo
    // a filter.
    let (hidden, set_hidden) = leptos_router::hooks::query_signal_with_options::<bool>(
        "hide_checked",
        leptos_router::NavigateOptions {
            replace: true,
            ..Default::default()
        },
    );
    let hide_checked = Signal::derive(move || hidden.get().unwrap_or(false));

    let blank = Action::new(|(): &()| create_manual_receipt());
    let navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| {
        if let Some(Ok(id)) = blank.value().get() {
            navigate(&format!("/receipt/{id}"), Default::default());
        }
    });

    view! {
        <h1 class="mb-4 text-xl font-semibold">"Receipts"</h1>

        {move || error_notice(error_of(blank))}

        <div class="mb-3 flex items-center justify-between gap-3">
            <Toggle
                label="Hide checked"
                on=hide_checked
                // `None` when it's off, so the parameter isn't in the URL at all.
                set=SignalSetter::map(move |on: bool| set_hidden.set(on.then_some(true)))
            />
            <button
                type="button"
                class=format!("{CHIP} border-edge bg-surface active:bg-edge disabled:opacity-40")
                disabled=move || blank.pending().get()
                on:click=move |_| {
                    blank.dispatch(());
                }
            >
                {move || if blank.pending().get() { "Adding…" } else { "New receipt" }}
            </button>
        </div>

        {screen::listing(
            receipts,
            "No receipts yet.",
            move |rows| view! { <ReceiptRows rows hide_checked /> }.into_any(),
        )}
    }
}
