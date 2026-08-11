//! Check what the model read against the photo, and fix it.
//!
//! The screen the whole app hinges on — a local model will misread thermal
//! receipts, so this is only trustworthy if correcting one is quick.

mod items;

use leptos::prelude::*;
use leptos::web_sys::SubmitEvent;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::frontend::actions::{error_of, first_error, succeeded};
use crate::frontend::components::{
    BUTTON, DANGER, LabeledInput, Notice, PRIMARY, ReceiptPhoto, Spinner, Tone, confirm, failed,
    field, form_element, loading,
};
use crate::frontend::poll::{extraction_status, poll_until_settled};
use crate::frontend::route::id_param;
use crate::frontend::text::total_or_why;
use crate::shared::api::{
    delete_receipt, get_receipt, list_people, mark_reviewed, retry_extraction, save_receipt,
};
use crate::shared::dto::{ExtractionStatus, LineItemSave, Person, Receipt, ReceiptSave};
use items::{LineItems, Row};

/// Ties the Save button to the fields, so it can sit with the other buttons at
/// the bottom instead of stranded at the end of the form.
const FORM_ID: &str = "receipt";

#[component]
pub fn ReviewPage() -> impl IntoView {
    let id = id_param();

    // Set when you got here from a statement, so there's a way back to one
    // half-reconciled. A path only — a query parameter shouldn't be able to point
    // the link anywhere else.
    let query = leptos_router::hooks::use_query_map();
    let back = move || {
        query
            .read()
            .get("back")
            .filter(|href| href.starts_with('/'))
    };

    // Both in one resource: without the people list the assignment dropdowns
    // would quietly come up empty, which reads as "nobody to charge this to".
    let receipt = Resource::new(id, |id| async move {
        let Some(id) = id else { return Ok(None) };
        let receipt = get_receipt(id).await?;
        let people = list_people().await?;
        Ok::<_, ServerFnError>(Some((receipt, people)))
    });

    // A receipt can still be extracting when this screen opens — a fresh upload
    // followed here, or a retry from here — and what it lands on replaces the
    // whole form, so it's loaded once, when there's finally something to load.
    let tick = RwSignal::new(0u32);
    let status = extraction_status(id, tick);
    poll_until_settled(
        tick,
        move || status.get().flatten().map(|s| !s.is_terminal()),
        move || receipt.refetch(),
    );

    // Saving and reviewing just want the receipt back. A retry also needs the
    // status asked again, or nothing would notice the job it just started.
    let reload = move || {
        receipt.refetch();
        tick.update(|t| *t += 1);
    };

    view! {
        {move || {
            back()
                .map(|href| {
                    view! {
                        <a href=href class="mb-3 inline-block text-sm">
                            "← Back to the statement"
                        </a>
                    }
                })
        }}
        // Transition rather than Suspense: saving refetches, and a fallback
        // would blank the whole screen every time.
        <Transition fallback=loading>
            {move || Suspend::new(async move {
                match receipt.await {
                    Err(e) => failed(e),
                    Ok(None) => failed("Not a valid receipt id."),
                    Ok(Some((r, people))) => {
                        view! { <ReviewForm receipt=r people reload /> }.into_any()
                    }
                }
            })}
        </Transition>
    }
}

/// The editable receipt. Split out from [`ReviewPage`] so the whole form is
/// rebuilt from server state after each save, rather than trying to keep local
/// signals in sync with the database.
#[component]
fn ReviewForm(
    receipt: Receipt,
    people: Vec<Person>,
    reload: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let id = receipt.id;
    // Off the loaded receipt rather than the polled status: the page reloads it
    // as extraction settles, so this can't be stale for longer than that.
    let extracting = !receipt.status.is_terminal();
    let reviewed = receipt.reviewed;
    let has_total = receipt.total.is_some();
    let rows = RwSignal::new(Row::from_items(&receipt.line_items));

    let save = Action::new(|save: &ReceiptSave| save_receipt(save.clone()));
    let review = Action::new(|id: &Uuid| mark_reviewed(*id));
    let discard = Action::new(|id: &Uuid| delete_receipt(*id));
    let retry = Action::new(|id: &Uuid| retry_extraction(*id));

    // All three end the same way: with whatever the server now has.
    Effect::new(move |_| {
        if succeeded(save) || succeeded(review) || succeeded(retry) {
            reload();
        }
    });

    // Not part of the reload above — there is nothing left to refetch, so go
    // back to the list instead.
    let navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| {
        if succeeded(discard) {
            navigate("/receipts", Default::default());
        }
    });

    let error_text = move || {
        first_error([
            error_of(save),
            error_of(review),
            error_of(discard),
            error_of(retry),
        ])
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
                    person_id: row.person_id,
                })
                .collect(),
        });
    };

    view! {
        <ReceiptHeading receipt=receipt.clone() />

        // Two columns once there's room: the photo stays put while you scroll the
        // fields, which is the whole job of this screen. Stacked on a phone.
        <div class="md:grid md:grid-cols-2 md:items-start md:gap-6">
            // The photo is the source of truth; everything else is a claim about it.
            <ReceiptPhoto src=format!("/receipt-image/{id}") />

            // min-w-0 so long merchant names can't push the column wider than half.
            <div class="min-w-0">
                <ExtractionNotice receipt=receipt.clone() retry />

                {move || error_text().map(|e| view! { <Notice tone=Tone::Bad>{e}</Notice> })}

                <ReceiptFields receipt=receipt.clone() submit />

                <LineItems
                    rows
                    people
                    subtotal=receipt.subtotal
                    currency=receipt.currency.clone()
                />

                <div class="flex flex-col gap-2 border-t border-edge pt-6">
                    <button
                        type="submit"
                        form=FORM_ID
                        disabled=move || save.pending().get() || extracting
                        class=format!("{PRIMARY} w-full")
                    >
                        {move || if save.pending().get() { "Saving…" } else { "Save receipt" }}
                    </button>

                    <button
                        type="button"
                        class=format!("{BUTTON} w-full")
                        disabled=!has_total || extracting
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
                        class=format!("{DANGER} mt-8 w-full")
                        on:click=move |_| {
                            if confirm("Delete this receipt and its photo? This cannot be undone.") {
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

/// What extraction has to say for itself: still reading, gave up, or landed on
/// something worth a closer look.
#[component]
fn ExtractionNotice(
    receipt: Receipt,
    retry: Action<Uuid, Result<(), ServerFnError>>,
) -> impl IntoView {
    let id = receipt.id;
    let pending = move || retry.pending().get();
    // Doubles as the reason it failed and, on success, the extractor's per-field
    // parse notes.
    let notes = receipt.extraction_error.clone();

    match receipt.status {
        ExtractionStatus::Pending | ExtractionStatus::Extracting => view! {
            <Notice tone=Tone::Quiet>
                <div class="flex items-center gap-2">
                    <Spinner />
                    "Reading the receipt…"
                </div>
            </Notice>
        }
        .into_any(),

        ExtractionStatus::Failed => view! {
            <Notice tone=Tone::Bad>
                <p class="mb-2 font-semibold">"Could not read this receipt"</p>
                <p class="mb-3 text-sm">
                    {notes.unwrap_or_else(|| "The model gave up on it.".to_string())}
                </p>
                <button
                    type="button"
                    class=BUTTON
                    disabled=pending
                    on:click=move |_| {
                        retry.dispatch(id);
                    }
                >
                    {move || if pending() { "Retrying…" } else { "Retry extraction" }}
                </button>
            </Notice>
        }
        .into_any(),

        ExtractionStatus::Done => {
            let problems = receipt.problems();
            view! {
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
                {notes.map(|n| view! { <Notice tone=Tone::Quiet>"Extraction notes: " {n}</Notice> })}
            }
                .into_any()
        }
    }
}

/// Merchant, date and total as they currently stand — what you'd check first.
#[component]
fn ReceiptHeading(receipt: Receipt) -> impl IntoView {
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
                {total_or_why(receipt.total, &receipt.currency, receipt.status)}
                {receipt.reviewed.then_some(" · checked")}
            </p>
        </div>
    }
}

#[component]
fn ReceiptFields(receipt: Receipt, submit: impl Fn(SubmitEvent) + 'static) -> impl IntoView {
    // Editing shows the raw value, not a formatted one — it has to parse back.
    let editable = |d: Option<Decimal>| d.map(|v| v.to_string()).unwrap_or_default();

    view! {
        <form id=FORM_ID class="mb-6 flex flex-col gap-3" on:submit=submit>
            <LabeledInput label="Merchant" name="merchant" value=receipt.merchant />
            <LabeledInput
                label="Date"
                name="purchased_on"
                kind="date"
                value=receipt.purchased_on.to_string()
            />
            <LabeledInput label="Currency" name="currency" value=receipt.currency />
            <div class="grid grid-cols-2 gap-3">
                <LabeledInput
                    label="Subtotal"
                    name="subtotal"
                    value=editable(receipt.subtotal)
                    numeric=true
                />
                <LabeledInput label="Tax" name="tax" value=editable(receipt.tax) numeric=true />
            </div>
            <LabeledInput label="Total" name="total" value=editable(receipt.total) numeric=true />
        </form>
    }
}
