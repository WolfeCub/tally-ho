//! The line-item editor: what the model read off the receipt, and who owes it.
//!
//! Items live in a signal rather than the DOM, so adding and removing rows costs
//! nothing until you save and the running sum can update as you type. The whole
//! list goes to the server in one request, so a row here is just a row on screen
//! until then.

use leptos::prelude::*;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::frontend::components::INPUT;
use crate::frontend::money::money;
use crate::frontend::rows::{Keyed, Rows};
use crate::frontend::text::plural;
use crate::shared::dto::{LineItem, Person};
use crate::shared::parse;

/// A button that sits in a row rather than under one. Not the shared BUTTON:
/// its padding would make these taller than the inputs they line up with.
const IN_ROW: &str = "shrink-0 rounded-lg border border-edge px-3 active:bg-edge \
                      disabled:opacity-40";

/// A line item as the screen has it. `id` is `None` until a row you added has
/// been saved.
#[derive(Clone)]
pub struct Row {
    key: usize,
    pub id: Option<Uuid>,
    pub description: String,
    /// As typed, so a half-written amount doesn't vanish. Parsed on save.
    pub total: String,
    /// Who it's charged to. `None` is unassigned, which splits evenly.
    pub person_id: Option<Uuid>,
    /// Why the model thinks it's theirs. Shown but never sent: the server keeps
    /// its own note, and picking somebody yourself clears it.
    pub guessed_why: Option<String>,
}

impl Keyed for Row {
    fn key(&self) -> usize {
        self.key
    }
}

impl Row {
    pub fn from_items(items: &[LineItem]) -> Vec<Row> {
        items
            .iter()
            .enumerate()
            .map(|(key, item)| Row {
                key,
                id: Some(item.id),
                description: item.description.clone(),
                total: item.total.to_string(),
                person_id: item.person_id,
                guessed_why: item.guessed_why.clone(),
            })
            .collect()
    }
}

/// What the items add up to as typed, or `None` while one of them isn't a
/// number yet.
///
/// Read with the parser the save uses, so the sum accepts whatever the server
/// will: type "$4.99" and this agrees rather than going blank at you.
fn typed_sum(rows: &[Row]) -> Option<Decimal> {
    rows.iter()
        .map(|row| match row.total.trim() {
            "" => Some(Decimal::ZERO),
            typed => parse::money(typed),
        })
        .sum()
}

#[component]
pub fn LineItems(
    rows: RwSignal<Vec<Row>>,
    people: Vec<Person>,
    /// To check the items against. The server checks again on save; this is so
    /// a corrected amount balances while you're still looking at the photo.
    subtotal: Option<Decimal>,
    currency: String,
) -> impl IntoView {
    let has_people = !people.is_empty();
    let people = StoredValue::new(people);
    let currency = StoredValue::new(currency);

    let add_row = move || {
        rows.add(|key| Row {
            key,
            id: None,
            description: String::new(),
            total: String::new(),
            person_id: None,
            guessed_why: None,
        })
    };

    // Blank while an amount is half-typed and doesn't parse yet.
    let total = move || {
        let sum = typed_sum(&rows.get())?;
        let off = subtotal.filter(|stated| *stated != sum);
        let currency = currency.get_value();
        Some(view! {
            // Full brightness: it's the check on the whole list, and it sat at
            // the same weight as the guesses underneath the rows.
            <p class="font-medium" class=("text-danger", off.is_some())>
                "Items total: " {money(sum, &currency)}
            </p>
            {off
                .map(|stated| {
                    view! {
                        <p class="text-xs text-danger">
                            "The subtotal says " {money(stated, &currency)}
                        </p>
                    }
                })}
        })
    };

    view! {
        // mb matches the row's own pt below the rule above it, so the sum and
        // the button sit evenly between the two rules.
        <div class="mb-3">
            <div class="mb-2 flex items-baseline justify-between gap-2">
                <h2 class="font-semibold">"Line items"</h2>
                <span class="text-sm text-muted">{move || plural(rows.get().len(), "item")}</span>
            </div>

            <ul class="flex flex-col gap-2">
                <For
                    each=move || rows.get()
                    key=|row| row.key
                    children=move |row| view! { <ItemRow row rows people /> }
                />
            </ul>

            // Adding a row is a small thing next to the sum, not another
            // full-width button competing with the ones that save the receipt.
            <div class="mt-3 flex items-center justify-between gap-3 border-t border-edge pt-3">
                <div class="min-w-0">{total}</div>
                <button
                    type="button"
                    class=format!("{IN_ROW} py-1.5 text-sm")
                    on:click=move |_| add_row()
                >
                    "+ Add item"
                </button>
            </div>

            {(!has_people)
                .then(|| {
                    view! {
                        <p class="mt-2 text-sm text-muted">
                            "Add people in Settings to charge items to them."
                        </p>
                    }
                })}
        </div>
    }
}

/// The model's reason, a line at a time: it reasons its way to a name, so the
/// whole thing can run to a couple of sentences. Tap for the rest.
#[component]
fn Guessed(why: String) -> impl IntoView {
    let open = RwSignal::new(false);

    view! {
        <button
            type="button"
            class="w-full text-left text-xs text-muted"
            class=("line-clamp-1", move || !open.get())
            aria-expanded=move || open.get().to_string()
            on:click=move |_| open.update(|open| *open = !*open)
        >
            "Guessed: "
            {why}
        </button>
    }
}

// `use<>` because the view owns its strings — without it the return type
// captures the borrow and can't outlive the `with_value` it's read inside.
fn options(people: &[Person], selected: Option<Uuid>) -> impl IntoView + use<> {
    people
        .iter()
        .map(|person| {
            view! {
                <option value=person.id.to_string() selected=selected == Some(person.id)>
                    {person.name.clone()}
                </option>
            }
        })
        .collect_view()
}

#[component]
fn ItemRow(row: Row, rows: RwSignal<Vec<Row>>, people: StoredValue<Vec<Person>>) -> impl IntoView {
    let key = row.key;
    let person_id = row.person_id;
    let has_people = people.with_value(|people| !people.is_empty());

    let edit = rows.setter(key);

    // This row's own, so typing in another one doesn't disturb it. Cleared when
    // you pick somebody, which the save then agrees with server-side.
    let guessed_why = RwSignal::new(row.guessed_why.clone());

    // The assignment select wraps onto its own line, so rows need a rule
    // between them to still read as rows.
    let li = if has_people {
        "flex flex-wrap gap-2 border-b border-edge pb-2 last:border-0 last:pb-0"
    } else {
        "flex gap-2"
    };

    view! {
        <li class=li>
            <input
                class=format!("{INPUT} min-w-0 flex-1")
                placeholder="Description"
                value=row.description
                on:input:target=move |ev| edit(|row, v| row.description = v, ev.target().value())
            />
            <input
                class=format!("{INPUT} w-24 shrink-0 tabular-nums")
                placeholder="0.00"
                inputmode="decimal"
                value=row.total
                on:input:target=move |ev| edit(|row, v| row.total = v, ev.target().value())
            />
            <button
                type="button"
                class=format!("{IN_ROW} text-muted")
                aria-label="Remove this item"
                on:click=move |_| rows.remove(key)
            >
                "✕"
            </button>

            {has_people
                .then(|| {
                    view! {
                        <select
                            class=format!("{INPUT} w-full")
                            aria-label="Charge this item to"
                            on:change:target=move |ev| {
                                edit(
                                    |row, v| row.person_id = Uuid::parse_str(&v).ok(),
                                    ev.target().value(),
                                );
                                guessed_why.set(None);
                            }
                        >
                            <option value="" selected=person_id.is_none()>
                                "Unassigned — split evenly"
                            </option>
                            {people.with_value(|people| options(people, person_id))}
                        </select>

                        // Says whose word this is, so a wrong one gets changed
                        // rather than trusted.
                        {move || guessed_why.get().map(|why| view! { <Guessed why /> })}
                    }
                })}
        </li>
    }
}
