//! The receipt list tab.

use leptos::prelude::*;

use crate::frontend::components::ReceiptRows;
use crate::shared::api::recent_receipts;

#[component]
pub fn ReceiptListPage() -> impl IntoView {
    let receipts = Resource::new(|| (), |_| async move { recent_receipts(100).await });

    view! {
        <h1 class="mb-4 text-xl font-semibold">"Receipts"</h1>
        <Suspense fallback=|| {
            view! { <p class="text-muted">"Loading…"</p> }
        }>
            {move || Suspend::new(async move {
                match receipts.await {
                    Err(e) => view! { <p class="text-danger">{format!("{e}")}</p> }.into_any(),
                    Ok(rows) if rows.is_empty() => {
                        view! { <p class="text-muted">"No receipts yet."</p> }.into_any()
                    }
                    Ok(rows) => view! { <ReceiptRows rows /> }.into_any(),
                }
            })}
        </Suspense>
    }
}
