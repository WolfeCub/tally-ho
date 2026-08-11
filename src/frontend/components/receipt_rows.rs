//! The receipt list.

use leptos::prelude::*;

use crate::frontend::text::{merchant, plural, total_or_why};
use crate::shared::dto::ReceiptSummary;

/// Named only because it's long enough to bury the markup it sits on.
const ROW: &str = "flex min-h-14 items-center gap-3 rounded-lg border border-edge bg-surface p-3 \
                   no-underline active:bg-edge";

#[component]
pub fn ReceiptRows(rows: Vec<ReceiptSummary>) -> impl IntoView {
    view! {
        <ul class="flex flex-col gap-2">
            {rows
                .into_iter()
                .map(|r| {
                    let total = total_or_why(r.total, &r.currency, r.status);
                    let problems = r.problems.len();
                    view! {
                        <li>
                            // Openable while it reads, too: the review screen
                            // says so for itself and fills in when it lands.
                            <a href=format!("/receipt/{}", r.id) class=ROW>
                                <span class="min-w-0 flex-1">
                                    <span class="block truncate">
                                        {merchant(&r.merchant).to_string()}
                                    </span>
                                    <span class="block text-xs text-muted">
                                        {r.purchased_on.to_string()} " · "
                                        {plural(r.item_count, "item")}
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
