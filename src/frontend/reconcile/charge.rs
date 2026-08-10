//! One line off the statement, and the ways to account for it.

use leptos::prelude::*;
use leptos::web_sys::FormData;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::frontend::components::INPUT;
use crate::frontend::money::money;
use crate::frontend::text::plural;
use crate::shared::api::upload_receipt;
use crate::shared::dto::{Candidate, Charge, Matched, Person, ReceiptSummary, Resolution, Resolve};

/// A row action. Narrower than a full [`crate::frontend::components::BUTTON`] so
/// several fit across a phone, and still 44px tall.
pub const ACTION: &str = "min-h-11 rounded-lg border border-edge px-3 text-sm \
                          active:bg-edge disabled:opacity-40";

/// What every row needs and none of them owns.
#[derive(Clone, Copy)]
pub struct Shared {
    pub currency: StoredValue<String>,
    pub people: StoredValue<Vec<Person>>,
    /// Receipts nothing accounts for yet, for picking one by hand.
    pub spare: StoredValue<Vec<ReceiptSummary>>,
    /// Where the review screen should send you back to.
    pub statement_id: Uuid,
}

impl Shared {
    fn money(&self, amount: Decimal) -> String {
        self.currency.with_value(|currency| money(amount, currency))
    }
}

#[component]
pub fn ChargeRow(
    charge: Charge,
    shared: Shared,
    resolve: impl Fn(Uuid, Resolve) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    // A settled row is done with; anything else is still work, and says so from
    // across the screen.
    let edge = if charge.resolution.is_settled() {
        "border-edge"
    } else {
        "border-good"
    };

    view! {
        <li class=format!("rounded-lg border {edge} bg-surface p-3")>
            <div class="flex items-baseline gap-2">
                <span class="shrink-0 text-xs text-muted tabular-nums">
                    {format!("{:02}/{:02}", charge.charged_on.month(), charge.charged_on.day())}
                </span>
                <span class="min-w-0 flex-1 truncate">{charge.description.clone()}</span>
                <span class="shrink-0 tabular-nums">{shared.money(charge.amount)}</span>
            </div>

            <ChargeBody charge shared resolve />
        </li>
    }
}

/// Split out from the row so the nested views stay shallow — a deep one only
/// fails to compile in the wasm release build.
#[component]
fn ChargeBody(
    charge: Charge,
    shared: Shared,
    resolve: impl Fn(Uuid, Resolve) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let id = charge.id;
    let split = split_line(&charge, shared);
    let undo = move || {
        view! {
            <button type="button" class=ACTION on:click=move |_| resolve(id, Resolve::Clear)>
                "Undo"
            </button>
        }
    };

    match charge.resolution.clone() {
        Resolution::Confirmed(matched) => view! {
            <p class="mt-1 text-sm text-muted">{matched.merchant.clone()} " · " {split}</p>
            <Caveats matched=matched.clone() amount=charge.amount shared />
            <div class="mt-2 flex flex-wrap gap-2">
                {undo()}
                <Review receipt_id=matched.receipt_id statement_id=shared.statement_id label="Open the receipt" />
            </div>
        }
        .into_any(),

        Resolution::Proposed(matched) => view! {
            <p class="mt-1 text-sm">
                "Looks like " <span class="font-medium">{matched.merchant.clone()}</span>
                {format!(" · {}", matched.purchased_on)}
            </p>
            <p class="mt-0.5 text-sm text-muted">{split}</p>
            <Caveats matched=matched.clone() amount=charge.amount shared />
            <div class="mt-2 flex flex-wrap gap-2">
                <button
                    type="button"
                    class=format!("{ACTION} border-paper bg-paper font-medium text-ink")
                    on:click=move |_| resolve(id, Resolve::Receipt(matched.receipt_id))
                >
                    "That's the one"
                </button>
                {undo()}
                <Review receipt_id=matched.receipt_id statement_id=shared.statement_id label="Check it" />
            </div>
        }
        .into_any(),

        Resolution::NoReceipt { .. } => view! {
            <p class="mt-1 text-sm text-muted">"No receipt · " {split}</p>
            <div class="mt-2">{undo()}</div>
        }
        .into_any(),

        Resolution::Unresolved => view! {
            <Suggestions charge shared resolve />
        }
        .into_any(),
    }
}

/// What's shaky about a match. Not errors — a tip makes the amounts differ on
/// purpose — but the split is only as good as what's behind it.
#[component]
fn Caveats(matched: Matched, amount: Decimal, shared: Shared) -> impl IntoView {
    // Red only for the one that quietly corrupts the export; the rest are things
    // to know, and four red lines a row would train you to ignore them.
    let mut notes = Vec::new();

    if !matched.status.is_terminal() {
        notes.push(("Still being read.".to_string(), false));
    }
    if let Some(total) = matched.total.filter(|total| *total != amount) {
        notes.push((format!("The receipt says {}.", shared.money(total)), false));
    }
    if !matched.reviewed {
        notes.push(("Not checked against the photo yet.".to_string(), false));
    }
    if !matched.problems.is_empty() {
        notes.push((
            format!(
                "{} on the receipt, so the split is a guess.",
                plural(matched.problems.len(), "issue")
            ),
            true,
        ));
    }

    notes
        .into_iter()
        .map(|(note, bad)| {
            let class = if bad {
                "mt-0.5 text-sm text-danger"
            } else {
                "mt-0.5 text-sm text-muted"
            };
            view! { <p class=class>{note}</p> }
        })
        .collect_view()
}

#[component]
fn Review(receipt_id: Uuid, statement_id: Uuid, label: &'static str) -> impl IntoView {
    view! {
        // `back` so the review screen can offer the way here again — without it
        // the only route back to a half-reconciled statement is the nav bar.
        <a
            href=format!("/receipt/{receipt_id}?back=/reconcile/{statement_id}")
            class=format!("{ACTION} flex items-center no-underline")
        >
            {label}
        </a>
    }
}

/// Everything on offer for a charge nothing accounts for yet.
#[component]
fn Suggestions(
    charge: Charge,
    shared: Shared,
    resolve: impl Fn(Uuid, Resolve) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let charge_id = charge.id;
    view! {
        <ul class="mt-2 flex flex-col gap-1">
            {charge
                .suggestions
                .into_iter()
                .map(|candidate| view! { <Offer candidate charge_id shared resolve /> })
                .collect_view()}
        </ul>

        <div class="mt-2 flex flex-wrap gap-2">
            <AttachSelect charge_id shared resolve />
            <NoReceiptSelect charge_id shared resolve />
            <Photograph charge_id resolve />
        </div>
    }
}

/// One receipt matching thinks it might be, and why.
#[component]
fn Offer(
    candidate: Candidate,
    charge_id: Uuid,
    shared: Shared,
    resolve: impl Fn(Uuid, Resolve) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let receipt_id = candidate.receipt_id;
    view! {
        <li>
            <button
                type="button"
                class=format!("{ACTION} w-full py-2 text-left")
                on:click=move |_| resolve(charge_id, Resolve::Receipt(receipt_id))
            >
                <span class="font-medium">{candidate.merchant}</span>
                {format!(
                    " · {} · {} · {}",
                    candidate.purchased_on,
                    shared.money(candidate.total),
                    candidate.why,
                )}
            </button>
        </li>
    }
}

/// Any receipt nothing accounts for — the way out when a misread date put the
/// receipt nowhere near the charge that paid for it.
#[component]
fn AttachSelect(
    charge_id: Uuid,
    shared: Shared,
    resolve: impl Fn(Uuid, Resolve) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let describe = |receipt: &ReceiptSummary| {
        format!(
            "{} · {} · {}",
            receipt.purchased_on,
            if receipt.merchant.is_empty() {
                "(no merchant)"
            } else {
                &receipt.merchant
            },
            match receipt.total {
                Some(total) => money(total, &receipt.currency),
                None => "no total".to_string(),
            },
        )
    };

    view! {
        <select
            class=format!("{INPUT} min-h-11 flex-1 text-sm")
            aria-label="Attach a receipt"
            on:change:target=move |ev| {
                if let Ok(receipt_id) = Uuid::parse_str(&ev.target().value()) {
                    resolve(charge_id, Resolve::Receipt(receipt_id));
                }
            }
        >
            <option value="">"Attach a receipt…"</option>
            {shared
                .spare
                .with_value(|spare| {
                    spare
                        .iter()
                        .map(|receipt| {
                            view! {
                                <option value=receipt.id.to_string()>{describe(receipt)}</option>
                            }
                        })
                        .collect_view()
                })}
        </select>
    }
}

/// For a charge that will never have one: a subscription, a fee, interest.
#[component]
fn NoReceiptSelect(
    charge_id: Uuid,
    shared: Shared,
    resolve: impl Fn(Uuid, Resolve) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <select
            class=format!("{INPUT} min-h-11 text-sm")
            aria-label="Mark as having no receipt"
            on:change:target=move |ev| {
                let value = ev.target().value();
                if !value.is_empty() {
                    // Anything unparseable means nobody in particular, so the
                    // even split needs no id of its own.
                    let person_id = Uuid::parse_str(&value).ok();
                    resolve(charge_id, Resolve::NoReceipt { person_id });
                }
            }
        >
            <option value="">"No receipt…"</option>
            <option value="evenly">"No receipt — split evenly"</option>
            {shared
                .people
                .with_value(|people| {
                    people
                        .iter()
                        .map(|person| {
                            view! {
                                <option value=person.id
                                    .to_string()>{format!("No receipt — {}", person.name)}</option>
                            }
                        })
                        .collect_view()
                })}
        </select>
    }
}

/// Photograph the receipt for this charge here and now.
///
/// A photo taken for a charge belongs to it by intent rather than by amount, so it
/// is attached as soon as the upload lands — while the model is still reading it.
#[component]
fn Photograph(
    charge_id: Uuid,
    resolve: impl Fn(Uuid, Resolve) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let upload = Action::new_local(|data: &FormData| upload_receipt(data.clone().into()));

    Effect::new(move |_| {
        if let Some(Ok(receipt_id)) = upload.value().get() {
            resolve(charge_id, Resolve::Receipt(receipt_id));
        }
    });

    view! {
        // Its own form, so `FormData` can pick the file up the same way the
        // capture screen does.
        <form>
            <label class=format!("{ACTION} flex cursor-pointer items-center")>
                {move || if upload.pending().get() { "Sending…" } else { "Photograph it" }}
                // `capture` opens the rear camera straight away on a phone.
                <input
                    type="file"
                    name="receipt"
                    accept="image/*"
                    capture="environment"
                    class="sr-only"
                    on:change:target=move |ev| {
                        if let Some(form) = ev.target().form()
                            && let Ok(data) = FormData::new_with_form(&form)
                        {
                            upload.dispatch_local(data);
                        }
                    }
                />
            </label>
        </form>

        {move || {
            upload
                .value()
                .get()
                .and_then(|r| r.err())
                .map(|e| view! { <p class="mt-1 text-sm text-danger">{e.to_string()}</p> })
        }}
    }
}

/// Who owes what for one charge, as the export will have it.
fn split_line(charge: &Charge, shared: Shared) -> String {
    shared.people.with_value(|people| {
        charge
            .split
            .iter()
            .filter_map(|share| {
                let person = people.iter().find(|p| p.id == share.person_id)?;
                Some(format!("{} {}", person.name, shared.money(share.amount)))
            })
            .collect::<Vec<_>>()
            .join(" · ")
    })
}
