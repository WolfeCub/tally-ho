//! Settings. So far that means the people line items get charged to.

use leptos::prelude::*;
use leptos::web_sys::SubmitEvent;
use uuid::Uuid;

use crate::frontend::actions::{error_of, succeeded};
use crate::frontend::components::{BUTTON, INPUT, Notice, PRIMARY, Tone, failed, loading};
use crate::shared::api::{list_people, save_people};
use crate::shared::dto::{Person, PersonSave};

#[component]
pub fn SettingsPage() -> impl IntoView {
    let people = Resource::new(|| (), |_| list_people());

    view! {
        <h1 class="mb-6 text-xl font-semibold">"Settings"</h1>

        <section>
            <h2 class="font-semibold">"People"</h2>
            <p class="mt-1 mb-4 text-sm text-muted">
                "Who a line item gets charged to — anything left unassigned is split evenly. "
                "Removing someone unassigns whatever was charged to them."
            </p>

            // Transition, not Suspense: saving refetches, and a fallback would
            // blank the list you were just editing.
            <Transition fallback=loading>
                {move || Suspend::new(async move {
                    match people.await {
                        Err(e) => failed(e),
                        Ok(list) => {
                            view! { <PeopleForm people=list reload=move || people.refetch() /> }
                                .into_any()
                        }
                    }
                })}
            </Transition>
        </section>
    }
}

/// A person as the screen has them.
///
/// `key` is stable for the life of the row so [`For`] never rebuilds a box
/// you're typing in. `id` is `None` until a row you added has been saved.
#[derive(Clone)]
struct Row {
    key: usize,
    id: Option<Uuid>,
    name: String,
    description: String,
}

#[component]
fn PeopleForm(
    people: Vec<Person>,
    reload: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    // Everyone lives here until you save, so adding and removing costs nothing
    // and one button commits the lot.
    let rows = RwSignal::new(
        people
            .into_iter()
            .enumerate()
            .map(|(key, person)| Row {
                key,
                id: Some(person.id),
                name: person.name,
                description: person.description.unwrap_or_default(),
            })
            .collect::<Vec<_>>(),
    );
    // Keys only have to be unique within this form, so a counter does.
    let next_key = RwSignal::new(rows.get_untracked().len());
    let add_row = move || {
        let key = next_key.get_untracked();
        next_key.set(key + 1);
        rows.update(|rows| {
            rows.push(Row {
                key,
                id: None,
                name: String::new(),
                description: String::new(),
            })
        });
    };

    let save = Action::new(|people: &Vec<PersonSave>| save_people(people.clone()));

    Effect::new(move |_| {
        if succeeded(save) {
            reload();
        }
    });

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        save.dispatch(
            rows.get_untracked()
                .into_iter()
                .map(|row| PersonSave {
                    id: row.id,
                    name: row.name,
                    description: row.description,
                })
                .collect(),
        );
    };

    view! {
        {move || {
            error_of(save).map(|e| view! { <Notice tone=Tone::Bad>{e.to_string()}</Notice> })
        }}

        <form class="flex flex-col gap-3" on:submit=submit>
            <ul class="flex flex-col gap-2">
                <For
                    each=move || rows.get()
                    key=|row| row.key
                    children=move |row| view! { <PersonRow row rows /> }
                />
            </ul>

            {move || {
                rows.get()
                    .is_empty()
                    .then(|| {
                        view! {
                            <p class="text-muted">
                                "Nobody yet — add someone and you can start charging items to them."
                            </p>
                        }
                    })
            }}

            <button
                type="button"
                class=format!("{BUTTON} self-start text-sm")
                on:click=move |_| add_row()
            >
                "Add a person"
            </button>

            <button
                type="submit"
                disabled=move || save.pending().get()
                class=format!("{PRIMARY} mt-3 self-start")
            >
                {move || if save.pending().get() { "Saving…" } else { "Save people" }}
            </button>
        </form>
    }
}

#[component]
fn PersonRow(row: Row, rows: RwSignal<Vec<Row>>) -> impl IntoView {
    let key = row.key;

    let edit = move |f: fn(&mut Row, String), value: String| {
        rows.update(|rows| {
            if let Some(row) = rows.iter_mut().find(|row| row.key == key) {
                f(row, value);
            }
        });
    };

    view! {
        <li class="flex gap-2 rounded-lg border border-edge p-3">
            // Name beside the description once there's room, stacked on a phone.
            <div class="flex min-w-0 flex-1 flex-col gap-2 sm:flex-row">
                <input
                    class=format!("{INPUT} sm:w-40 sm:shrink-0")
                    placeholder="Name"
                    aria-label="Name"
                    value=row.name
                    on:input:target=move |ev| edit(|row, v| row.name = v, ev.target().value())
                />
                <textarea
                    class=format!("{INPUT} min-w-0 flex-1 text-sm")
                    rows="3"
                    placeholder="What they tend to buy"
                    aria-label="Description"
                    on:input:target=move |ev| edit(|row, v| row.description = v, ev.target().value())
                >
                    {row.description}
                </textarea>
            </div>

            <button
                type="button"
                class="shrink-0 self-start rounded-lg border border-edge px-3 py-2 text-muted active:bg-edge"
                aria-label="Remove this person"
                on:click=move |_| rows.update(|rows| rows.retain(|row| row.key != key))
            >
                "✕"
            </button>
        </li>
    }
}
