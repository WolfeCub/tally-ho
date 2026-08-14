//! Reconciling a card statement: import the file, account for every charge, take
//! the split away as a CSV.

mod charge;

use leptos::prelude::*;
use uuid::Uuid;

use crate::frontend::actions::{error_of, succeeded};
use crate::frontend::components::{
    AS_BUTTON, BUTTON, Bar, DANGER, INPUT, PRIMARY, ROW, confirm, error_notice, upload_action,
    uploads_to,
};
use crate::frontend::money::{money, money_total, shares_line};
use crate::frontend::poll::{self, poll_until_settled};
use crate::frontend::route::id_param;
use crate::frontend::screen;
use crate::frontend::text::plural;
use crate::shared::api::{
    delete_statement, get_statement, import_statement, list_statements, resolve_charge,
    spare_receipts, statement_reading,
};
use crate::shared::dto::{Imported, ReceiptSummary, Resolve, Statement, StatementSummary};
use charge::{ChargeRow, Shared};

#[component]
pub fn StatementsPage() -> impl IntoView {
    let statements = Resource::new(|| (), |_| list_statements());
    let import = upload_action(import_statement);
    let imported = move || import.value().get().and_then(|r| r.ok());
    let chosen = RwSignal::new(false);

    Effect::new(move |_| {
        if imported().is_some() {
            statements.refetch();
        }
    });

    view! {
        <h1 class="mb-1 text-xl font-semibold">"Reconcile"</h1>
        <p class="mb-4 text-sm text-muted">
            "Upload the CSV your card exports. Every charge on it then gets a receipt, or a reason it hasn't got one."
        </p>

        <form class="mb-4 flex flex-col gap-3 md:max-w-md" on:submit=uploads_to(import)>
            <label class="flex flex-col gap-1">
                <span class="text-sm text-muted">"Statement CSV"</span>
                <input
                    type="file"
                    name="statement"
                    accept=".csv,text/csv"
                    class=INPUT
                    on:change:target=move |ev| chosen.set(!ev.target().value().is_empty())
                />
            </label>
            <button
                type="submit"
                disabled=move || !chosen.get() || import.pending().get()
                class=PRIMARY
            >
                {move || if import.pending().get() { "Reading…" } else { "Import" }}
            </button>
        </form>

        {move || error_notice(error_of(import))}
        {move || imported().map(|imported| view! { <ImportedCard imported /> })}

        <h2 class="mt-8 mb-2 font-semibold">"Imported"</h2>
        {screen::listing(
            statements,
            "No statements yet.",
            |rows| view! { <StatementRows rows /> }.into_any(),
        )}
    }
}

/// What the sniffer made of the file. Shown rather than logged: it guessed which
/// columns to read, and a wrong guess has to be visible before anything is
/// reconciled against it.
#[component]
fn ImportedCard(imported: Imported) -> impl IntoView {
    view! {
        <div class="mb-4 rounded-lg border border-good bg-surface p-3">
            <p class="font-medium">{format!("Read {}.", plural(imported.charge_count, "charge"))}</p>
            <p class="mt-1 text-sm text-muted">
                {format!("From the {} columns.", imported.columns.join(", "))}
            </p>

            {(!imported.skipped.is_empty())
                .then(|| {
                    view! {
                        <details class="mt-2 text-sm text-danger">
                            <summary>{plural(imported.skipped.len(), "row")} " skipped"</summary>
                            <ul class="mt-1 list-disc pl-5">
                                {imported
                                    .skipped
                                    .iter()
                                    .map(|why| view! { <li>{why.clone()}</li> })
                                    .collect_view()}
                            </ul>
                        </details>
                    }
                })}

            <a
                href=format!("/reconcile/{}", imported.id)
                class=format!("{PRIMARY} {AS_BUTTON} mt-3")
            >
                "Start reconciling"
            </a>
        </div>
    }
}

#[component]
fn StatementRows(rows: Vec<StatementSummary>) -> impl IntoView {
    view! {
        <ul class="flex flex-col gap-2">
            {rows
                .into_iter()
                .map(|s| {
                    let done = s.settled_count == s.charge_count;
                    view! {
                        <li>
                            <a
                                href=format!("/reconcile/{}", s.id)
                                class=ROW
                            >
                                <span class="min-w-0 flex-1">
                                    <span class="block truncate">{s.label}</span>
                                    <span class="block text-xs text-muted">
                                        {s.begins_on.to_string()} " – " {s.ends_on.to_string()}
                                    </span>
                                </span>
                                <span class="shrink-0 text-right text-sm tabular-nums">
                                    {format!("{} / {}", s.settled_count, s.charge_count)}
                                    <span class=if done {
                                        "block text-xs text-good"
                                    } else {
                                        "block text-xs text-muted"
                                    }>{if done { "done" } else { "accounted for" }}</span>
                                </span>
                            </a>
                        </li>
                    }
                })
                .collect_view()}
        </ul>
    }
}

#[component]
pub fn ReconcilePage() -> impl IntoView {
    let id = id_param();

    // Both in one resource: the picker offers the receipts nothing accounts for,
    // and a stale list would offer one that was just used.
    let data = screen::for_id(id, |id| async move {
        let statement = get_statement(id).await?;
        let spare = spare_receipts(100).await?;
        Ok((statement, spare))
    });

    // A receipt photographed here is attached while the model is still reading
    // it, and nothing pushes the result down when it lands. Only that one
    // question is polled: re-asking for the statement rebuilds every row on a
    // timer, which resets the "hide what's done" box while you're using it.
    let tick = RwSignal::new(0u32);
    let reading = poll::keyed(id, tick, statement_reading);
    poll_until_settled(
        tick,
        move || reading.get().flatten(),
        move || data.refetch(),
    );

    // Every decision reloads the statement. A photograph also starts a job the
    // poll above has to be told to go looking for.
    let reload = move || {
        data.refetch();
        tick.update(|t| *t += 1);
    };

    // Out here so it survives those reloads.
    let hide_done = RwSignal::new(false);

    screen::detail(
        data,
        "Not a valid statement id.",
        move |(statement, spare)| {
            view! { <StatementView statement spare reload hide_done /> }.into_any()
        },
    )
}

#[component]
fn StatementView(
    statement: Statement,
    spare: Vec<ReceiptSummary>,
    reload: impl Fn() + Copy + Send + Sync + 'static,
    /// Owned by the page, not this view — every decision reloads the statement,
    /// and a filter that quietly turned itself off each time would be worse than
    /// not having one.
    hide_done: RwSignal<bool>,
) -> impl IntoView {
    let id = statement.id;
    let resolve = Action::new(|&(charge_id, how): &(Uuid, Resolve)| resolve_charge(charge_id, how));
    let discard = Action::new(|id: &Uuid| delete_statement(*id));

    Effect::new(move |_| {
        if succeeded(resolve) {
            reload();
        }
    });

    // Nothing left to refetch once it's gone, so go back to the list.
    let navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| {
        if succeeded(discard) {
            navigate("/reconcile", Default::default());
        }
    });

    let shared = Shared {
        currency: StoredValue::new(statement.currency.clone()),
        people: StoredValue::new(statement.people.clone()),
        spare: StoredValue::new(spare),
        statement_id: id,
    };
    let charges = StoredValue::new(statement.charges.clone());

    view! {
        <Summary statement />

        {move || error_notice(error_of(resolve))}

        <label class="mb-2 flex items-center gap-2 text-sm text-muted">
            // `prop:` and not the attribute: the attribute is only the starting
            // state, so the box would drift from the filter it stands for.
            <input
                type="checkbox"
                class="size-4"
                prop:checked=move || hide_done.get()
                on:change:target=move |ev| hide_done.set(ev.target().checked())
            />
            "Hide what's done"
        </label>

        <ul class="flex flex-col gap-2">
            {move || {
                charges
                    .get_value()
                    .into_iter()
                    .filter(|charge| !(hide_done.get() && charge.resolution.is_settled()))
                    .map(|charge| {
                        view! {
                            <ChargeRow
                                charge
                                shared
                                resolve=move |charge_id, how| {
                                    resolve.dispatch((charge_id, how));
                                }
                            />
                        }
                    })
                    .collect_view()
            }}
        </ul>

        <button
            type="button"
            class=format!("{DANGER} mt-8 w-full")
            on:click=move |_| {
                if confirm("Delete this statement and its charges? The receipts stay.") {
                    discard.dispatch(id);
                }
            }
        >
            "Delete statement"
        </button>
    }
}

/// The figure the whole screen is for, and how far off it is.
#[component]
fn Summary(statement: Statement) -> impl IntoView {
    let currency = statement.currency.clone();
    let count = statement.charges.len();
    let done = statement.settled();
    let (left, short) = statement.outstanding();
    let percent = done * 100 / count.max(1);

    let owed = shares_line(&statement.totals(), &statement.people, &currency);

    view! {
        <div class="mb-4 rounded-lg border border-edge bg-surface p-4">
            <div class="sm:flex sm:items-start sm:justify-between sm:gap-4">
                <div class="min-w-0">
                    <p class="truncate text-sm text-muted">
                        {statement.label.clone()} " · " {statement.begins_on.to_string()} " – "
                        {statement.ends_on.to_string()}
                    </p>
                    <p class="text-3xl font-semibold tabular-nums">
                        {money_total(statement.total(), &currency)}
                    </p>
                    <p class="mt-1 tabular-nums">{owed}</p>
                </div>
                // `download` is required: leptos_router intercepts same-origin
                // anchors without it and navigates the SPA to the URL, which
                // renders the not-found page.
                <a
                    href=format!("/statement/{}/export.csv", statement.id)
                    download
                    class=format!("{BUTTON} {AS_BUTTON} mt-3 shrink-0 sm:mt-0")
                >
                    "Export CSV"
                </a>
            </div>

            <p class="mt-3 text-sm text-muted">{format!("{done} of {count} accounted for")}</p>
            <Bar percent />

            {(left > 0)
                .then(|| {
                    view! {
                        <p class="mt-2 text-sm text-danger">
                            {format!(
                                "{} still to go, so the figures above are {} short of the statement.",
                                plural(left, "charge"),
                                money(short, &currency),
                            )}
                        </p>
                    }
                })}
        </div>
    }
}
