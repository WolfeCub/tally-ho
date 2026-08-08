//! Check what the model read against the photo, and fix it.
//!
//! The screen the whole app hinges on — a local model will misread thermal
//! receipts, so this is only trustworthy if correcting one is quick.

use leptos::prelude::*;
use leptos::web_sys::SubmitEvent;
use uuid::Uuid;

use crate::frontend::photo::ReceiptPhoto;
use crate::frontend::ui::{LabeledInput, field, form_element, reset_form};
use crate::shared::api::{
    add_line_item, delete_line_item, delete_receipt, get_receipt, mark_reviewed, update_line_item,
    update_receipt_meta,
};
use crate::shared::dto::{LineItemEdit, Receipt, ReceiptEdit};

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
        <Suspense fallback=|| {
            view! { <p class="text-muted">"Loading…"</p> }
        }>
            {move || Suspend::new(async move {
                match receipt.await {
                    Err(e) => view! { <p class="text-danger">{format!("{e}")}</p> }.into_any(),
                    Ok(None) => view! { <p class="text-danger">"Not a valid receipt id."</p> }.into_any(),
                    Ok(Some(r)) => view! { <ReviewForm receipt=r reload=move || receipt.refetch() /> }.into_any(),
                }
            })}
        </Suspense>
    }
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

    // Every mutation returns the updated receipt, so the only job here is to
    // surface failures and trigger a refetch.
    let save_meta = Action::new(move |edit: &ReceiptEdit| {
        let edit = edit.clone();
        async move { update_receipt_meta(edit).await }
    });
    let save_item = Action::new(move |edit: &LineItemEdit| {
        let edit = edit.clone();
        async move { update_line_item(edit).await }
    });
    let add_item = Action::new(move |(rid, desc, total): &(Uuid, String, String)| {
        let (rid, desc, total) = (*rid, desc.clone(), total.clone());
        async move { add_line_item(rid, desc, total).await }
    });
    let remove_item = Action::new(move |item_id: &Uuid| {
        let item_id = *item_id;
        async move { delete_line_item(item_id).await }
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

    // Any successful mutation invalidates what is on screen.
    Effect::new(move |_| {
        if save_meta.version().get() > 0
            || save_item.version().get() > 0
            || add_item.version().get() > 0
            || remove_item.version().get() > 0
            || review.version().get() > 0
        {
            reload();
        }
    });

    let money = |d: Option<rust_decimal::Decimal>| d.map(|v| v.to_string()).unwrap_or_default();

    // Collapses the five actions' errors into one place, so a failed save is
    // never silent.
    let error_text = move || {
        [
            save_meta.value().get().and_then(|r| r.err()),
            save_item.value().get().and_then(|r| r.err()),
            add_item.value().get().and_then(|r| r.err()),
            remove_item.value().get().and_then(|r| r.err()),
            review.value().get().and_then(|r| r.err()),
            discard.value().get().and_then(|r| r.err()),
        ]
        .into_iter()
        .flatten()
        .next()
        .map(|e| e.to_string())
    };

    let items = receipt.line_items.clone();

    view! {
        <h1 class="mb-4 text-xl font-semibold">
            "Review"
            {move || if reviewed { " · checked" } else { "" }}
        </h1>

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
                    <div class="mb-4 rounded-lg border border-danger p-3">
                        <p class="mb-2 font-semibold text-danger">"Needs attention"</p>
                        <ul class="list-disc pl-5 text-sm">
                            {problems
                                .iter()
                                .map(|p| view! { <li>{p.clone()}</li> })
                                .collect_view()}
                        </ul>
                    </div>
                }
            })}

        {warnings
            .map(|w| {
                view! {
                    <p class="mb-4 rounded-lg border border-edge p-3 text-sm text-muted">
                        "Extraction notes: " {w}
                    </p>
                }
            })}

        {move || {
            error_text()
                .map(|e| {
                    view! { <p class="mb-4 rounded-lg border border-danger p-3 text-danger">{e}</p> }
                })
        }}

        <form
            class="mb-6 flex flex-col gap-3"
            on:submit=move |ev: SubmitEvent| {
                ev.prevent_default();
                let form = form_element(&ev);
                save_meta
                    .dispatch(ReceiptEdit {
                        id,
                        merchant: field(&form, "merchant"),
                        purchased_on: field(&form, "purchased_on"),
                        currency: field(&form, "currency"),
                        subtotal: field(&form, "subtotal"),
                        tax: field(&form, "tax"),
                        total: field(&form, "total"),
                    });
            }
        >
            <LabeledInput label="Merchant" name="merchant" value=receipt.merchant.clone() />
            <LabeledInput
                label="Date"
                name="purchased_on"
                value=receipt.purchased_on.to_string()
            />
            <LabeledInput label="Currency" name="currency" value=receipt.currency.clone() />
            <LabeledInput label="Subtotal" name="subtotal" value=money(receipt.subtotal) numeric=true />
            <LabeledInput label="Tax" name="tax" value=money(receipt.tax) numeric=true />
            <LabeledInput label="Total" name="total" value=money(receipt.total) numeric=true />
            <button type="submit" class="rounded-lg border border-edge bg-surface px-4 py-3">
                "Save receipt"
            </button>
        </form>

        <h2 class="mb-2 font-semibold">
            "Line items " <span class="text-muted">"(" {items.len()} ")"</span>
        </h2>

        <ul class="mb-4 flex flex-col gap-3">
            {items
                .into_iter()
                .map(|item| {
                    let item_id = item.id;
                    view! {
                        <li class="rounded-lg border border-edge p-3">
                            <form
                                class="flex flex-col gap-2"
                                on:submit=move |ev: SubmitEvent| {
                                    ev.prevent_default();
                                    let form = form_element(&ev);
                                    save_item
                                        .dispatch(LineItemEdit {
                                            id: item_id,
                                            description: field(&form, "description"),
                                            total: field(&form, "total"),
                                        });
                                }
                            >
                                <input
                                    name="description"
                                    value=item.description.clone()
                                    class="rounded-lg border border-edge bg-ink p-2"
                                />
                                <div class="flex gap-2">
                                    <input
                                        name="total"
                                        value=item.total.to_string()
                                        inputmode="decimal"
                                        class="min-w-0 flex-1 rounded-lg border border-edge bg-ink p-2"
                                    />
                                    <button
                                        type="submit"
                                        class="rounded-lg border border-edge bg-surface px-3"
                                    >
                                        "Save"
                                    </button>
                                    <button
                                        type="button"
                                        class="rounded-lg border border-danger px-3 text-danger"
                                        on:click=move |_| {
                                            remove_item.dispatch(item_id);
                                        }
                                    >
                                        "Delete"
                                    </button>
                                </div>
                                {item.edited.then(|| view! { <p class="text-xs text-muted">"edited"</p> })}
                            </form>
                        </li>
                    }
                })
                .collect_view()}
        </ul>

        <form
            class="mb-6 flex flex-col gap-2 rounded-lg border border-edge p-3"
            on:submit=move |ev: SubmitEvent| {
                ev.prevent_default();
                let form = form_element(&ev);
                let desc = field(&form, "description");
                let total = field(&form, "total");
                add_item.dispatch((id, desc, total));
                reset_form(&form);
            }
        >
            <p class="font-semibold">"Add a missing item"</p>
            <input
                name="description"
                placeholder="Description"
                class="rounded-lg border border-edge bg-ink p-2"
            />
            <div class="flex gap-2">
                <input
                    name="total"
                    placeholder="0.00"
                    inputmode="decimal"
                    class="min-w-0 flex-1 rounded-lg border border-edge bg-ink p-2"
                />
                <button type="submit" class="rounded-lg border border-edge bg-surface px-3">
                    "Add"
                </button>
            </div>
        </form>

        <button
            class="w-full rounded-lg border border-edge bg-surface px-4 py-3 disabled:opacity-40"
            disabled=!has_total
            title=(!has_total).then_some("Enter a total first")
            on:click=move |_| {
                review.dispatch(id);
            }
        >
            {if reviewed { "Checked — mark again" } else { "Mark as checked" }}
        </button>
        {(!has_total)
            .then(|| {
                view! {
                    <p class="mt-2 text-sm text-muted">
                        "A receipt cannot be marked checked until it has a total."
                    </p>
                }
            })}

        <button
            class="mt-8 w-full rounded-lg border border-danger px-4 py-3 text-danger"
            on:click=move |_| {
                if window()
                    .confirm_with_message("Delete this receipt and its photo? This cannot be undone.")
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
    }
}
