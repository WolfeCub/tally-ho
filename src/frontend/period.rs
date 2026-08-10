//! Reconciliation: everything bought in a statement period, and the CSV of it.

use jiff::civil::Date;
use leptos::prelude::*;
use leptos::web_sys::SubmitEvent;

use crate::frontend::components::{AS_BUTTON, BUTTON, INPUT, ReceiptRows};
use crate::frontend::money::money_total;
use crate::frontend::text::plural;
use crate::shared::api::receipts_in_range;
use crate::shared::dto::PeriodSummary;

#[component]
pub fn PeriodPage() -> impl IntoView {
    // What's in the pickers — blank until the first period lands and fills them
    // in. The server picks that default, since `jiff` has no clock on wasm
    // without its `js` feature.
    let from_str = RwSignal::new(String::new());
    let to_str = RwSignal::new(String::new());

    // The range actually queried, separate from the inputs so that typing a date
    // doesn't fire a request per keystroke and half-entered ranges never load.
    // `(None, None)` asks the server for its default.
    let range = RwSignal::new((None::<Date>, None::<Date>));
    // Blocking so the period arrives in the initial HTML instead of streaming in.
    let summary = Resource::new_blocking(
        move || range.get(),
        |(from, to)| async move { receipts_in_range(from, to).await },
    );

    // In an effect rather than derived from the resource: the inputs render
    // before the period lands, and reading it during render would put different
    // dates in the server's HTML than the client's.
    Effect::new(move |_| {
        if let Some(Ok(summary)) = summary.get() {
            from_str.set(summary.from.to_string());
            to_str.set(summary.to.to_string());
        }
    });

    let apply = move |ev: SubmitEvent| {
        ev.prevent_default();
        // A blank or unparseable end falls back to the server default, which is
        // what the input was showing anyway.
        let parse = |s: String| s.trim().parse::<Date>().ok();
        range.set((parse(from_str.get()), parse(to_str.get())));
    };

    let date_input = format!("{INPUT} min-h-11 flex-1 sm:flex-none");

    view! {
        <h1 class="mb-4 text-xl font-semibold">"Period"</h1>

        // Outside the Suspense on purpose: if the query fails, you still need the
        // controls to ask for a different period. One row as soon as it fits.
        <form class="mb-6 flex flex-col gap-2 sm:flex-row sm:items-center" on:submit=apply>
            <div class="flex items-center gap-2">
                // `prop:value` rather than `value`: the effect above fills these
                // in after the first render, and only the property moves the
                // live DOM.
                <input
                    type="date"
                    class=date_input.clone()
                    prop:value=move || from_str.get()
                    on:input:target=move |ev| from_str.set(ev.target().value())
                />
                <span class="text-muted">"→"</span>
                <input
                    type="date"
                    class=date_input
                    prop:value=move || to_str.get()
                    on:input:target=move |ev| to_str.set(ev.target().value())
                />
            </div>
            <button type="submit" class=BUTTON>
                "Show period"
            </button>
        </form>

        <Suspense fallback=|| {
            view! { <p class="text-muted">"Loading…"</p> }
        }>
            {move || Suspend::new(async move {
                match summary.await {
                    Err(e) => view! { <p class="text-danger">{e.to_string()}</p> }.into_any(),
                    Ok(s) => view! { <PeriodBody summary=s /> }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
fn PeriodBody(summary: PeriodSummary) -> impl IntoView {
    let count = summary.receipts.len();
    let attention = summary.needing_attention();
    // Built from the loaded period rather than the input signals, so the export
    // can never disagree with the figures on screen.
    let export = format!("/export.csv?from={}&to={}", summary.from, summary.to);

    // A backwards range matches nothing, which would otherwise look exactly like a
    // month where nothing was bought.
    if summary.from > summary.to {
        return view! {
            <p class="rounded-lg border border-danger p-3 text-danger">
                "The end date is before the start date, so this period is empty. Swap them."
            </p>
        }
        .into_any();
    }

    view! {
        <div class="mb-4 rounded-lg border border-edge bg-surface p-4">
            // Export sits beside the total, not below the list — on desktop this
            // screen exists to read the figure and grab the CSV.
            <div class="sm:flex sm:items-end sm:justify-between sm:gap-4">
                <div>
                    <p class="text-sm text-muted">
                        {summary.from.to_string()} " – " {summary.to.to_string()}
                    </p>

                    // One figure per currency, so nothing adds different units
                    // together. Almost always a single line.
                    {if summary.totals.is_empty() {
                        view! { <p class="text-3xl font-semibold text-muted">"—"</p> }.into_any()
                    } else {
                        summary
                            .totals
                            .iter()
                            .map(|t| {
                                view! {
                                    <p class="text-3xl font-semibold tabular-nums">
                                        {money_total(t.total.known(), &t.currency)}
                                    </p>
                                }
                            })
                            .collect_view()
                            .into_any()
                    }}

                    <p class="text-sm text-muted">
                        {plural(count, "receipt")}
                    </p>
                </div>
                // `download` is required: leptos_router intercepts same-origin
                // anchors without it and navigates the SPA to /export.csv, which
                // renders the not-found page. Content-Disposition names the file.
                <a
                    href=export
                    download
                    class=format!("{BUTTON} {AS_BUTTON} mt-3 sm:mt-0")
                >
                    "Export CSV"
                </a>
            </div>

            // The figures above exclude receipts with no total, so say so.
            {
                let missing: usize = summary.totals.iter().map(|t| t.total.missing()).sum();
                (missing > 0)
                    .then(|| {
                        view! {
                            <p class="mt-2 rounded-lg border border-danger p-2 text-sm text-danger">
                                {format!(
                                    "This is a floor, not the total: {} ha{} no amount yet.",
                                    plural(missing, "receipt"),
                                    if missing == 1 { "s" } else { "ve" },
                                )}
                            </p>
                        }
                    })
            }
        </div>

        {(attention > 0)
            .then(|| {
                view! {
                    <p class="mb-4 text-sm text-muted">
                        {format!(
                            "{} need{} checking — marked below.",
                            plural(attention, "receipt"),
                            if attention == 1 { "s" } else { "" },
                        )}
                    </p>
                }
            })}

        {if summary.receipts.is_empty() {
            view! { <p class="text-muted">"No receipts in this period."</p> }.into_any()
        } else {
            view! { <ReceiptRows rows=summary.receipts /> }.into_any()
        }}
    }
    .into_any()
}
