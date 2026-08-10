//! The receipt list.

use leptos::prelude::*;

use crate::frontend::money::money;
use crate::frontend::text::plural;
use crate::shared::dto::{ExtractionStatus, ReceiptSummary};

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
                                class="flex min-h-14 items-center gap-3 rounded-lg border border-edge bg-surface p-3 no-underline active:bg-edge"
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
                                        {r.purchased_on.to_string()} " · " {plural(r.item_count, "item")}
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
                                                    {plural(problems, "issue")}
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
