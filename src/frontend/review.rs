//! Check what the model read against the photo, and fix it.
//!
//! The screen the whole app hinges on — a local model will misread thermal
//! receipts, so this is only trustworthy if correcting one is quick.

use leptos::prelude::*;
use leptos::web_sys::SubmitEvent;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::frontend::components::{
    BUTTON, INPUT, LabeledInput, Notice, PRIMARY, ReceiptPhoto, TAP, Tone, field, form_element,
};
use crate::frontend::money::money;
use crate::frontend::text::plural;
use crate::shared::api::{delete_receipt, get_receipt, mark_reviewed, save_receipt};
use crate::shared::dto::{LineItemSave, Receipt, ReceiptSave};

#[component]
pub fn ReviewPage() -> impl IntoView {
    use leptos_router::hooks::use_params_map;

    let params = use_params_map();
    let id = move || {
        params
            .read()
            .get("id")
            .and_then(|s| Uuid::parse_str(&s).ok())
    };

    let receipt = Resource::new(id, |id| async move {
        match id {
            Some(id) => get_receipt(id).await.map(Some),
            None => Ok(None),
        }
    });

    view! {
        // Transition rather than Suspense: saving refetches, and a fallback
        // would blank the whole screen every time.
        <Transition fallback=|| {
            view! { <p class="text-muted">"Loading…"</p> }
        }>
            {move || Suspend::new(async move {
                match receipt.await {
                    Err(e) => view! { <p class="text-danger">{format!("{e}")}</p> }.into_any(),
                    Ok(None) => view! { <p class="text-danger">"Not a valid receipt id."</p> }.into_any(),
                    Ok(Some(r)) => view! { <ReviewForm receipt=r reload=move || receipt.refetch() /> }.into_any(),
                }
            })}
        </Transition>
    }
}

/// A line item as the screen has it.
///
/// `key` is stable for the life of the row so `<For>` never rebuilds an input
/// you're typing in. `id` is `None` until a row you added has been saved.
#[derive(Clone)]
struct Row {
    key: usize,
    id: Option<Uuid>,
    description: String,
    total: String,
}

/// What the items add up to as typed, or `None` while one of them isn't a
/// number yet.
fn typed_sum(rows: &[Row]) -> Option<Decimal> {
    rows.iter()
        .map(|row| match row.total.trim() {
            "" => Some(Decimal::ZERO),
            typed => typed.parse().ok(),
        })
        .sum()
}

/// The editable receipt. Split out from [`ReviewPage`] so the whole form is
/// rebuilt from server state after each save, rather than trying to keep local
/// signals in sync with the database.
#[component]
fn ReviewForm(receipt: Receipt, reload: impl Fn() + Copy + Send + Sync + 'static) -> impl IntoView {
    let id = receipt.id;
    let problems = receipt.problems();
    let warnings = receipt.extraction_error.clone();
    let reviewed = receipt.reviewed;
    let has_total = receipt.total.is_some();
    let subtotal = receipt.subtotal;
    let currency = StoredValue::new(receipt.currency.clone());

    // Line items live here rather than in the DOM, so adding and removing rows
    // costs nothing until you save, and the running sum can be live.
    let rows = RwSignal::new(
        receipt
            .line_items
            .iter()
            .enumerate()
            .map(|(key, item)| Row {
                key,
                id: Some(item.id),
                description: item.description.clone(),
                total: item.total.to_string(),
            })
            .collect::<Vec<_>>(),
    );
    // Keys only have to be unique within this form, so a counter does.
    let next_key = RwSignal::new(receipt.line_items.len());

    let edit_row = move |key: usize, f: fn(&mut Row, String), value: String| {
        rows.update(|rows| {
            if let Some(row) = rows.iter_mut().find(|row| row.key == key) {
                f(row, value);
            }
        });
    };
    let add_row = move || {
        let key = next_key.get_untracked();
        next_key.set(key + 1);
        rows.update(|rows| {
            rows.push(Row {
                key,
                id: None,
                description: String::new(),
                total: String::new(),
            })
        });
    };

    let save = Action::new(move |save: &ReceiptSave| {
        let save = save.clone();
        async move { save_receipt(save).await }
    });
    let review = Action::new(move |rid: &Uuid| {
        let rid = *rid;
        async move { mark_reviewed(rid).await }
    });
    let discard = Action::new(move |rid: &Uuid| {
        let rid = *rid;
        async move { delete_receipt(rid).await }
    });

    // Deliberately not part of the reload effect below — there is nothing left
    // to refetch, so go back to the list instead.
    let navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| {
        if matches!(discard.value().get(), Some(Ok(()))) {
            navigate("/receipts", Default::default());
        }
    });

    // Only on success: a rejected save has to leave what you typed on screen,
    // next to the reason it was rejected.
    Effect::new(move |_| {
        if matches!(save.value().get(), Some(Ok(_))) || matches!(review.value().get(), Some(Ok(_)))
        {
            reload();
        }
    });

    let error_text = move || {
        [
            save.value().get().and_then(|r| r.err()),
            review.value().get().and_then(|r| r.err()),
            discard.value().get().and_then(|r| r.err()),
        ]
        .into_iter()
        .flatten()
        .next()
        .map(|e| e.to_string())
    };

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let form = form_element(&ev);
        save.dispatch(ReceiptSave {
            id,
            merchant: field(&form, "merchant"),
            purchased_on: field(&form, "purchased_on"),
            currency: field(&form, "currency"),
            subtotal: field(&form, "subtotal"),
            tax: field(&form, "tax"),
            total: field(&form, "total"),
            items: rows
                .get_untracked()
                .into_iter()
                .map(|row| LineItemSave {
                    id: row.id,
                    description: row.description,
                    total: row.total,
                })
                .collect(),
        });
    };

    // Live, so a corrected amount balances against the subtotal while you're
    // still looking at the photo. The server's own check only runs on save.
    let sum_line = move || {
        let sum = typed_sum(&rows.get())?;
        let off = subtotal.filter(|stated| *stated != sum);
        let class = if off.is_some() {
            "mt-2 text-sm text-danger"
        } else {
            "mt-2 text-sm text-muted"
        };
        Some(view! {
            <p class=class>
                "Items add up to " {money(sum, &currency.get_value())}
                {off.map(|stated| format!(" — the subtotal says {}", money(stated, &currency.get_value())))}
            </p>
        })
    };

    // Editing shows the raw value, not a formatted one — it has to parse back.
    let editable = |d: Option<Decimal>| d.map(|v| v.to_string()).unwrap_or_default();

    view! {
        <div class="mb-4 flex flex-wrap items-baseline gap-x-3">
            <h1 class="text-xl font-semibold">
                {if receipt.merchant.is_empty() {
                    "Review".to_string()
                } else {
                    receipt.merchant.clone()
                }}
            </h1>
            <p class="text-sm text-muted">
                {receipt.purchased_on.to_string()} " · "
                {match receipt.total {
                    Some(t) => money(t, &receipt.currency),
                    None => "no total".to_string(),
                }} {reviewed.then_some(" · checked")}
            </p>
        </div>

        // Two columns once there's room: the photo stays put while you scroll the
        // fields, which is the whole job of this screen. Stacked on a phone.
        <div class="md:grid md:grid-cols-2 md:items-start md:gap-6">
            // The photo is the source of truth; everything else is a claim about it.
            <ReceiptPhoto src=format!("/receipt-image/{id}") />

            // min-w-0 so long merchant names can't push the column wider than half.
            <div class="min-w-0">
                {(!problems.is_empty())
                    .then(|| {
                        view! {
                            <Notice tone=Tone::Bad>
                                <p class="mb-2 font-semibold">"Needs attention"</p>
                                <ul class="list-disc pl-5 text-sm">
                                    {problems
                                        .iter()
                                        .map(|p| view! { <li>{p.clone()}</li> })
                                        .collect_view()}
                                </ul>
                            </Notice>
                        }
                    })}

                {warnings
                    .map(|w| {
                        view! { <Notice tone=Tone::Quiet>"Extraction notes: " {w}</Notice> }
                    })}

                {move || {
                    error_text().map(|e| view! { <Notice tone=Tone::Bad>{e}</Notice> })
                }}

                // The id is what lets the Save button live down with the others
                // instead of stranded at the bottom of the fields.
                <form id="receipt" class="mb-6 flex flex-col gap-3" on:submit=submit>
                    <LabeledInput label="Merchant" name="merchant" value=receipt.merchant.clone() />
                    <LabeledInput
                        label="Date"
                        name="purchased_on"
                        kind="date"
                        value=receipt.purchased_on.to_string()
                    />
                    <LabeledInput label="Currency" name="currency" value=receipt.currency.clone() />
                    <div class="grid grid-cols-2 gap-3">
                        <LabeledInput
                            label="Subtotal"
                            name="subtotal"
                            value=editable(receipt.subtotal)
                            numeric=true
                        />
                        <LabeledInput
                            label="Tax"
                            name="tax"
                            value=editable(receipt.tax)
                            numeric=true
                        />
                    </div>
                    <LabeledInput
                        label="Total"
                        name="total"
                        value=editable(receipt.total)
                        numeric=true
                    />
                </form>

                <div class="mb-6">
                    <div class="mb-2 flex items-baseline justify-between gap-2">
                        <h2 class="font-semibold">"Line items"</h2>
                        <span class="text-sm text-muted">
                            {move || plural(rows.get().len(), "item")}
                        </span>
                    </div>

                    <ul class="flex flex-col gap-2">
                        <For
                            each=move || rows.get()
                            key=|row| row.key
                            children=move |row| {
                                let key = row.key;
                                view! {
                                    <li class="flex gap-2">
                                        <input
                                            class=format!("{INPUT} min-w-0 flex-1")
                                            placeholder="Description"
                                            value=row.description
                                            on:input:target=move |ev| {
                                                edit_row(
                                                    key,
                                                    |row, v| row.description = v,
                                                    ev.target().value(),
                                                )
                                            }
                                        />
                                        <input
                                            class=format!("{INPUT} w-24 shrink-0 tabular-nums")
                                            placeholder="0.00"
                                            inputmode="decimal"
                                            value=row.total
                                            on:input:target=move |ev| {
                                                edit_row(key, |row, v| row.total = v, ev.target().value())
                                            }
                                        />
                                        <button
                                            type="button"
                                            class="shrink-0 rounded-lg border border-edge px-3 text-muted active:bg-edge"
                                            aria-label="Remove this item"
                                            on:click=move |_| {
                                                rows.update(|rows| rows.retain(|row| row.key != key))
                                            }
                                        >
                                            "✕"
                                        </button>
                                    </li>
                                }
                            }
                        />
                    </ul>

                    {sum_line}

                    <button
                        type="button"
                        class=format!("{BUTTON} mt-3 min-h-11 w-full px-4 text-sm")
                        on:click=move |_| add_row()
                    >
                        "Add an item"
                    </button>
                </div>

                <div class="flex flex-col gap-2 border-t border-edge pt-6">
                    <button
                        type="submit"
                        form="receipt"
                        disabled=move || save.pending().get()
                        class=format!("{PRIMARY} {TAP} w-full disabled:opacity-40")
                    >
                        {move || if save.pending().get() { "Saving…" } else { "Save receipt" }}
                    </button>

                    <button
                        type="button"
                        class=format!("{BUTTON} {TAP} w-full disabled:opacity-40")
                        disabled=!has_total
                        on:click=move |_| {
                            review.dispatch(id);
                        }
                    >
                        {if reviewed { "Checked — mark again" } else { "Mark as checked" }}
                    </button>
                    {(!has_total)
                        .then(|| {
                            view! {
                                <p class="text-sm text-muted">
                                    "Enter a total and save it before marking this checked."
                                </p>
                            }
                        })}

                    <button
                        type="button"
                        class=format!("{TAP} mt-8 w-full rounded-lg border border-danger text-danger")
                        on:click=move |_| {
                            if window()
                                .confirm_with_message(
                                    "Delete this receipt and its photo? This cannot be undone.",
                                )
                                .unwrap_or(false)
                            {
                                discard.dispatch(id);
                            }
                        }
                    >
                        "Delete receipt"
                    </button>
                </div>
            </div>
        </div>
    }
}
