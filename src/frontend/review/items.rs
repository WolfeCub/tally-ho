//! The line-item editor: what the model read off the receipt, and who owes it.
//!
//! Items live in a signal rather than the DOM, so adding and removing rows costs
//! nothing until you save and the running sum can update as you type. The whole
//! list goes to the server in one request, so a row here is just a row on screen
//! until then.

use leptos::prelude::*;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::frontend::components::{BUTTON, INPUT};
use crate::frontend::money::money;
use crate::frontend::text::plural;
use crate::shared::dto::{LineItem, Person};

/// A line item as the screen has it.
///
/// `key` is stable for the life of the row so [`For`] never rebuilds an input
/// you're typing in. `id` is `None` until a row you added has been saved.
#[derive(Clone)]
pub struct Row {
    key: usize,
    pub id: Option<Uuid>,
    pub description: String,
    /// As typed, so a half-written amount doesn't vanish. Parsed on save.
    pub total: String,
    /// Who it's charged to. `None` is unassigned, which splits evenly.
    pub person_id: Option<Uuid>,
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
            })
            .collect()
    }
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

    // Keys only have to be unique within this form, so a counter does.
    let next_key = RwSignal::new(rows.get_untracked().len());
    let add_row = move || {
        let key = next_key.get_untracked();
        next_key.set(key + 1);
        rows.update(|rows| {
            rows.push(Row {
                key,
                id: None,
                description: String::new(),
                total: String::new(),
                person_id: None,
            })
        });
    };

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
                {off
                    .map(|stated| {
                        format!(" — the subtotal says {}", money(stated, &currency.get_value()))
                    })}
            </p>
        })
    };

    view! {
        <div class="mb-6">
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

            {sum_line}

            {(!has_people)
                .then(|| {
                    view! {
                        <p class="mt-2 text-sm text-muted">
                            "Add people in Settings to charge items to them."
                        </p>
                    }
                })}

            <button
                type="button"
                class=format!("{BUTTON} mt-3 w-full text-sm")
                on:click=move |_| add_row()
            >
                "Add an item"
            </button>
        </div>
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

    let edit = move |f: fn(&mut Row, String), value: String| {
        rows.update(|rows| {
            if let Some(row) = rows.iter_mut().find(|row| row.key == key) {
                f(row, value);
            }
        });
    };

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
                class="shrink-0 rounded-lg border border-edge px-3 text-muted active:bg-edge"
                aria-label="Remove this item"
                on:click=move |_| rows.update(|rows| rows.retain(|row| row.key != key))
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
                                )
                            }
                        >
                            <option value="" selected=person_id.is_none()>
                                "Unassigned — split evenly"
                            </option>
                            {people.with_value(|people| options(people, person_id))}
                        </select>
                    }
                })}
        </li>
    }
}
