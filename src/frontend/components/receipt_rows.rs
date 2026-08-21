//! The receipt list.

use leptos::prelude::*;

use super::{ROW, Verdict};
use crate::frontend::text::{merchant, plural, total_or_why};
use crate::shared::dto::ReceiptSummary;

#[component]
pub fn ReceiptRows(
    rows: Vec<ReceiptSummary>,
    /// Owned by the page, not this view: the list refetches while a receipt is
    /// being read, and a filter that reset itself on every poll would be worse
    /// than not having one.
    hide_checked: RwSignal<bool>,
) -> impl IntoView {
    let rows = StoredValue::new(rows);
    view! {
        <ul class="flex flex-col gap-2">
            {move || rows
                .get_value()
                .into_iter()
                .filter(|r| !(hide_checked.get() && r.reviewed))
                .map(|r| {
                    let total = total_or_why(r.total, &r.currency, r.status);
                    let problems = r.problems.len();
                    view! {
                        <li>
                            // Openable while it reads, too: the review screen
                            // says so for itself and fills in when it lands.
                            <a href=format!("/receipt/{}", r.id) class=ROW>
                                <span class="min-w-0 flex-1">
                                    <span class="flex items-center gap-1.5">
                                        <span class="truncate">
                                            {merchant(&r.merchant).to_string()}
                                        </span>
                                        {r
                                            .reviewed
                                            .then(|| {
                                                view! {
                                                    <Verdict ok=true />
                                                    // The tick is aria-hidden, so say it out loud
                                                    // for anything not looking at it.
                                                    <span class="sr-only">"checked"</span>
                                                }
                                            })}
                                    </span>
                                    <span class="block text-xs text-muted">
                                        {r.purchased_on.to_string()} " · "
                                        {plural(r.item_count, "item")}
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
