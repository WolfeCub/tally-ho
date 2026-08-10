//! Reconciling a card statement: import the file, account for every charge, take
//! the split away as a CSV.

mod charge;

use leptos::prelude::*;
use leptos::web_sys::{FormData, SubmitEvent};
use uuid::Uuid;

use crate::frontend::actions::{error_of, succeeded};
use crate::frontend::components::{
    AS_BUTTON, BUTTON, DANGER, INPUT, Notice, PRIMARY, Tone, confirm, form_element,
};
use crate::frontend::money::{money, money_total};
use crate::frontend::text::plural;
use crate::shared::api::{
    delete_statement, get_statement, import_statement, list_statements, resolve_charge,
    spare_receipts,
};
use crate::shared::dto::{Imported, ReceiptSummary, Resolve, Statement, StatementSummary};
use charge::{ChargeRow, Shared, still_reading};

#[component]
pub fn StatementsPage() -> impl IntoView {
    let statements = Resource::new(|| (), |_| async move { list_statements().await });
    // `_local` because FormData is not Send; the upload only ever runs client-side.
    let import = Action::new_local(|data: &FormData| import_statement(data.clone().into()));
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

        <form
            class="mb-4 flex flex-col gap-3 md:max-w-md"
            on:submit=move |ev: SubmitEvent| {
                ev.prevent_default();
                let data = FormData::new_with_form(&form_element(&ev)).unwrap();
                import.dispatch_local(data);
            }
        >
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

        {move || {
            import
                .value()
                .get()
                .and_then(|r| r.err())
                .map(|e| view! { <Notice tone=Tone::Bad>{e.to_string()}</Notice> })
        }}
        {move || imported().map(|imported| view! { <ImportedCard imported /> })}

        <h2 class="mt-8 mb-2 font-semibold">"Imported"</h2>
        <Transition fallback=|| {
            view! { <p class="text-muted">"Loading…"</p> }
        }>
            {move || Suspend::new(async move {
                match statements.await {
                    Err(e) => view! { <p class="text-danger">{e.to_string()}</p> }.into_any(),
                    Ok(rows) if rows.is_empty() => {
                        view! { <p class="text-muted">"No statements yet."</p> }.into_any()
                    }
                    Ok(rows) => view! { <StatementRows rows /> }.into_any(),
                }
            })}
        </Transition>
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
                                class="flex min-h-14 items-center gap-3 rounded-lg border border-edge bg-surface p-3 no-underline active:bg-edge"
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
    use leptos_router::hooks::use_params_map;

    let params = use_params_map();
    let id = move || {
        params
            .read()
            .get("id")
            .and_then(|s| Uuid::parse_str(&s).ok())
    };

    // Both in one resource: the picker offers the receipts nothing accounts for,
    // and a stale list would offer one that was just used.
    let data = Resource::new(id, |id| async move {
        let Some(id) = id else { return Ok(None) };
        let statement = get_statement(id).await?;
        let spare = spare_receipts(100).await?;
        Ok::<_, ServerFnError>(Some((statement, spare)))
    });

    // A receipt photographed here is attached while the model is still reading it,
    // and nothing pushes the result down when it lands — so ask again. In an effect
    // because timers are wasm-only, and here rather than in the view so that a
    // refetch reconsiders the one pending timer instead of stacking another on it.
    Effect::new(move |previous: Option<Option<TimeoutHandle>>| {
        if let Some(Some(timer)) = previous {
            timer.clear();
        }
        let waiting = matches!(
            data.get(),
            Some(Ok(Some((ref statement, _)))) if statement.charges.iter().any(still_reading)
        );
        if !waiting {
            return None;
        }
        set_timeout_with_handle(move || data.refetch(), std::time::Duration::from_secs(2)).ok()
    });

    view! {
        // Transition rather than Suspense: every decision refetches, and a
        // fallback would blank the statement each time.
        <Transition fallback=|| {
            view! { <p class="text-muted">"Loading…"</p> }
        }>
            {move || Suspend::new(async move {
                match data.await {
                    Err(e) => view! { <p class="text-danger">{e.to_string()}</p> }.into_any(),
                    Ok(None) => {
                        view! { <p class="text-danger">"Not a valid statement id."</p> }.into_any()
                    }
                    Ok(Some((statement, spare))) => {
                        view! {
                            <StatementView statement spare reload=move || data.refetch() />
                        }
                            .into_any()
                    }
                }
            })}
        </Transition>
    }
}

#[component]
fn StatementView(
    statement: Statement,
    spare: Vec<ReceiptSummary>,
    reload: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let id = statement.id;
    let resolve = Action::new(move |(charge_id, how): &(Uuid, Resolve)| {
        let (charge_id, how) = (*charge_id, *how);
        async move { resolve_charge(charge_id, how).await }
    });
    let discard = Action::new(move |id: &Uuid| {
        let id = *id;
        async move { delete_statement(id).await }
    });

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
    let hide_done = RwSignal::new(false);

    view! {
        <Summary statement />

        {move || error_of(resolve).map(|e| view! { <Notice tone=Tone::Bad>{e.to_string()}</Notice> })}

        <label class="mb-2 flex items-center gap-2 text-sm text-muted">
            <input
                type="checkbox"
                class="size-4"
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

    let owed: Vec<_> = statement
        .totals()
        .iter()
        .filter_map(|share| {
            let person = statement.people.iter().find(|p| p.id == share.person_id)?;
            Some(format!(
                "{} {}",
                person.name,
                money(share.amount, &currency)
            ))
        })
        .collect();

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
                    <p class="mt-1 tabular-nums">{owed.join(" · ")}</p>
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

            <p class="mt-3 text-sm text-muted">
                {format!("{done} of {count} accounted for")}
            </p>
            <div class="mt-1 h-1 rounded bg-edge">
                <div class="h-1 rounded bg-good" style=format!("width:{percent}%")></div>
            </div>

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
