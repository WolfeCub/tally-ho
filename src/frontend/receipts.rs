//! The receipt list tab, and the row markup the period view borrows.

use leptos::prelude::*;

use crate::frontend::money::money;
use crate::shared::api::recent_receipts;
use crate::shared::dto::{ExtractionStatus, ReceiptSummary};

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

/// The receipt list, shared by the list tab and the period view.
#[component]
pub fn ReceiptRows(rows: Vec<ReceiptSummary>) -> impl IntoView {
    view! {
        <ul class="flex flex-col gap-2">
            {rows
                .into_iter()
                .map(|r| {
                    // A receipt still being read has no meaningful figures yet,
                    // so say that rather than showing a blank total.
                    let pending = !matches!(
                        r.status,
                        ExtractionStatus::Done | ExtractionStatus::Failed,
                    );
                    let total = match r.total {
                        Some(t) => money(t, &r.currency),
                        None if pending => "reading…".to_string(),
                        None => "no total".to_string(),
                    };
                    let problems = r.problems.len();
                    view! {
                        <li>
                            <a
                                href=format!("/receipt/{}", r.id)
                                class="flex min-h-14 items-center gap-3 rounded-lg border border-edge bg-surface p-3 no-underline"
                            >
                                <span class="min-w-0 flex-1">
                                    <span class="block truncate">
                                        {if r.merchant.is_empty() {
                                            "(no merchant)".to_string()
                                        } else {
                                            r.merchant.clone()
                                        }}
                                    </span>
                                    <span class="block text-xs text-muted">
                                        {r.purchased_on.to_string()} " · "
                                        {format!("{} item{}", r.item_count, if r.item_count == 1 { "" } else { "s" })}
                                        {r.reviewed.then_some(" · checked")}
                                    </span>
                                </span>
                                <span class="text-right">
                                    <span class=if r.total.is_some() {
                                        "block tabular-nums"
                                    } else {
                                        "block text-sm text-muted"
                                    }>{total}</span>
                                    {(problems > 0)
                                        .then(|| {
                                            view! {
                                                <span class="block text-xs text-danger">
                                                    {format!(
                                                        "{problems} issue{}",
                                                        if problems == 1 { "" } else { "s" },
                                                    )}
                                                </span>
                                            }
                                        })}
                                </span>
                            </a>
                        </li>
                    }
                })
                .collect_view()}
        </ul>
    }
}
