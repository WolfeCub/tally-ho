use leptos::prelude::*;
use leptos_meta::{MetaTags, Stylesheet, Title, provide_meta_context};
use leptos_router::components::{A, Route, Router, Routes};
use leptos_router::path;

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                // viewport-fit=cover pairs with the px-safe/pb-safe utilities:
                // required so content doesn't sit under the iOS notch and home
                // indicator once installed to the home screen.
                <meta
                    name="viewport"
                    content="width=device-width, initial-scale=1, viewport-fit=cover"
                />
                <meta name="theme-color" content="#16181a" />
                <AutoReload options=options.clone() />
                <HydrationScripts options />
                <MetaTags />
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/tally_ho.css" />
        <Title text="tally-ho" />

        <Router>
            // Nav first so it can stick to the top on desktop. On a phone it is
            // `fixed`, so DOM order doesn't affect where it lands.
            <NavBar />
            <main class="px-safe mx-auto w-full max-w-4xl pt-4 pb-28 md:pb-12">
                <Routes fallback=|| view! { <p>"Not found."</p> }>
                    <Route path=path!("/") view=CapturePage />
                    <Route path=path!("/receipts") view=ReceiptListPage />
                    <Route path=path!("/receipt/:id") view=ReviewPage />
                    <Route path=path!("/period") view=PeriodPage />
                </Routes>
            </main>
        </Router>
    }
}

/// Thumb-reachable bottom bar on a phone, ordinary top bar on a desktop.
#[component]
fn NavBar() -> impl IntoView {
    // min-h-11 is 44px — the smallest comfortable thumb target. Tabs split the
    // width evenly on a phone; on desktop they shrink to their labels so they
    // don't stretch across the page.
    let link = "flex min-h-11 flex-1 items-center justify-center text-sm text-muted \
                no-underline active:bg-edge aria-[current=page]:text-paper \
                md:flex-none md:px-4 md:hover:text-paper";
    let bar = "pb-safe fixed inset-x-0 bottom-0 z-10 flex border-t border-edge bg-surface \
               md:sticky md:top-0 md:bottom-auto md:border-t-0 md:border-b md:pb-0";
    view! {
        <nav class=bar>
            // Same max-width as <main>, so the tabs line up with the content.
            <div class="px-safe mx-auto flex w-full max-w-4xl">
                <span class="mr-auto hidden items-center pr-4 font-semibold md:flex">"tally-ho"</span>
                <A href="/" attr:class=link>
                    "Capture"
                </A>
                <A href="/receipts" attr:class=link>
                    "Receipts"
                </A>
                <A href="/period" attr:class=link>
                    "Period"
                </A>
            </div>
        </nav>
    }
}

#[component]
fn CapturePage() -> impl IntoView {
    use crate::api::{receipt_status, upload_receipt};
    use crate::dto::ExtractionStatus;
    use leptos::wasm_bindgen::JsCast;
    use leptos::web_sys::{FormData, HtmlFormElement, SubmitEvent};

    // `_local` because FormData is not Send; the upload only ever runs client-side.
    let upload = Action::new_local(|data: &FormData| upload_receipt(data.clone().into()));

    // Id of the receipt currently being extracted, if any.
    let receipt_id = Memo::new(move |_| match upload.value().get() {
        Some(Ok(id)) => Some(id),
        _ => None,
    });

    // Extraction runs in the background, so the only way to know it finished is
    // to ask. Ticking a signal re-runs the resource.
    let tick = RwSignal::new(0u32);
    let status = Resource::new(
        move || (receipt_id.get(), tick.get()),
        |(id, _)| async move {
            match id {
                Some(id) => receipt_status(id).await.ok(),
                None => None,
            }
        },
    );

    Effect::new(move |prev_handle: Option<Option<IntervalHandle>>| {
        // Clear any previous timer before starting another.
        if let Some(Some(handle)) = prev_handle {
            handle.clear();
        }
        receipt_id.get()?;
        // Stop polling once the outcome is settled, rather than hammering the
        // server for the life of the page.
        let settled = matches!(
            status.get().flatten(),
            Some(ExtractionStatus::Done) | Some(ExtractionStatus::Failed)
        );
        if settled {
            return None;
        }
        set_interval_with_handle(
            move || tick.update(|t| *t += 1),
            std::time::Duration::from_millis(1500),
        )
        .ok()
    });

    view! {
        <h1 class="mb-4 text-xl font-semibold">"Capture"</h1>

        // Capped on desktop: a file picker and one button have no business
        // spanning the full content width.
        <form
            class="flex flex-col gap-4 md:max-w-md"
            on:submit=move |ev: SubmitEvent| {
                ev.prevent_default();
                let form = ev.target().unwrap().unchecked_into::<HtmlFormElement>();
                let data = FormData::new_with_form(&form).unwrap();
                upload.dispatch_local(data);
            }
        >
            // `capture="environment"` opens the rear camera directly on a phone,
            // and needs no JavaScript at all.
            <input
                type="file"
                name="receipt"
                accept="image/*"
                capture="environment"
                class="rounded-lg border border-edge bg-surface p-3"
            />
            <button type="submit" class="rounded-lg border border-edge bg-surface px-4 py-3">
                "Upload"
            </button>
        </form>

        <div class="mt-6">
            {move || {
                if upload.pending().get() {
                    return view! { <p class="text-muted">"Uploading photo…"</p> }.into_any();
                }
                match upload.value().get() {
                    None => view! { <p class="text-muted">"Photograph a receipt."</p> }.into_any(),
                    Some(Err(e)) => {
                        view! { <p class="text-danger">{format!("Upload failed: {e}")}</p> }
                            .into_any()
                    }
                    Some(Ok(id)) => {
                        let settled = status.get().flatten();
                        match settled {
                            Some(ExtractionStatus::Done) => {
                                view! {
                                    <p class="mb-3">"Extraction finished."</p>
                                    <a
                                        href=format!("/receipt/{id}")
                                        class="inline-block rounded-lg border border-edge bg-surface px-4 py-3"
                                    >
                                        "Review it"
                                    </a>
                                }
                                    .into_any()
                            }
                            Some(ExtractionStatus::Failed) => {
                                view! {
                                    <p class="text-danger mb-3">"Extraction failed."</p>
                                    <a
                                        href=format!("/receipt/{id}")
                                        class="inline-block rounded-lg border border-edge bg-surface px-4 py-3"
                                    >
                                        "Enter it by hand"
                                    </a>
                                }
                                    .into_any()
                            }
                            // Pending, Extracting, or not yet polled.
                            _ => {
                                view! {
                                    <p class="text-muted">
                                        "Reading the receipt… this takes about 10 seconds."
                                    </p>
                                }
                                    .into_any()
                            }
                        }
                    }
                }
            }}
        </div>
    }
}

#[component]
fn ReceiptListPage() -> impl IntoView {
    use crate::api::recent_receipts;

    let receipts = Resource::new(|| (), |_| async move { recent_receipts(100).await });

    view! {
        <h1 class="mb-4 text-xl font-semibold">"Receipts"</h1>
        <Suspense fallback=|| {
            view! { <p class="text-muted">"Loading…"</p> }
        }>
            {move || Suspend::new(async move {
                match receipts.await {
                    Err(e) => view! { <p class="text-danger">{format!("{e}")}</p> }.into_any(),
                    Ok(rows) if rows.is_empty() => {
                        view! { <p class="text-muted">"No receipts yet."</p> }.into_any()
                    }
                    Ok(rows) => view! { <ReceiptRows rows /> }.into_any(),
                }
            })}
        </Suspense>
    }
}

/// Money as it is shown, always to the cent.
///
/// Display-only. The CSV export deliberately does *not* go through this: an
/// export is meant to be a faithful copy of what is stored, not a rounded view
/// of it.
fn money(amount: rust_decimal::Decimal) -> String {
    format!("{amount:.2}")
}

/// The receipt list, shared by the list tab and the period view.
#[component]
fn ReceiptRows(rows: Vec<crate::dto::ReceiptSummary>) -> impl IntoView {
    use crate::dto::ExtractionStatus;

    view! {
        <ul class="flex flex-col gap-2">
            {rows
                .into_iter()
                .map(|r| {
                    // A receipt still being read has no meaningful figures yet,
                    // so say that rather than showing a blank total.
                    let pending = !matches!(
                        r.status,
                        ExtractionStatus::Done | ExtractionStatus::Failed,
                    );
                    let total = match r.total {
                        Some(t) => money(t),
                        None if pending => "reading…".to_string(),
                        None => "no total".to_string(),
                    };
                    let problems = r.problems.len();
                    view! {
                        <li>
                            <a
                                href=format!("/receipt/{}", r.id)
                                class="flex min-h-14 items-center gap-3 rounded-lg border border-edge bg-surface p-3 no-underline"
                            >
                                <span class="min-w-0 flex-1">
                                    <span class="block truncate">
                                        {if r.merchant.is_empty() {
                                            "(no merchant)".to_string()
                                        } else {
                                            r.merchant.clone()
                                        }}
                                    </span>
                                    <span class="block text-xs text-muted">
                                        {r.purchased_on.to_string()} " · "
                                        {format!("{} item{}", r.item_count, if r.item_count == 1 { "" } else { "s" })}
                                        {r.reviewed.then_some(" · checked")}
                                    </span>
                                </span>
                                <span class="text-right">
                                    <span class=if r.total.is_some() {
                                        "block tabular-nums"
                                    } else {
                                        "block text-sm text-muted"
                                    }>{total}</span>
                                    {(problems > 0)
                                        .then(|| {
                                            view! {
                                                <span class="block text-xs text-danger">
                                                    {format!(
                                                        "{problems} issue{}",
                                                        if problems == 1 { "" } else { "s" },
                                                    )}
                                                </span>
                                            }
                                        })}
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
fn ReviewPage() -> impl IntoView {
    use crate::api::get_receipt;
    use leptos_router::hooks::use_params_map;
    use uuid::Uuid;

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
fn ReviewForm(
    receipt: crate::dto::Receipt,
    reload: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    use crate::api::{
        add_line_item, delete_line_item, mark_reviewed, update_line_item, update_receipt_meta,
    };
    use crate::dto::{LineItemEdit, ReceiptEdit};

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
    let add_item = Action::new(move |(rid, desc, total): &(uuid::Uuid, String, String)| {
        let (rid, desc, total) = (*rid, desc.clone(), total.clone());
        async move { add_line_item(rid, desc, total).await }
    });
    let remove_item = Action::new(move |item_id: &uuid::Uuid| {
        let item_id = *item_id;
        async move { delete_line_item(item_id).await }
    });
    let review = Action::new(move |rid: &uuid::Uuid| {
        let rid = *rid;
        async move { mark_reviewed(rid).await }
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

    let money = |d: Option<rust_decimal::Decimal>| {
        d.map(|v| v.to_string()).unwrap_or_default()
    };

    // Collapses the five actions' errors into one place, so a failed save is
    // never silent.
    let error_text = move || {
        [
            save_meta.value().get().and_then(|r| r.err()),
            save_item.value().get().and_then(|r| r.err()),
            add_item.value().get().and_then(|r| r.err()),
            remove_item.value().get().and_then(|r| r.err()),
            review.value().get().and_then(|r| r.err()),
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
            <img
                src=format!("/receipt-image/{id}")
                alt="receipt photo"
                // Underscores, not spaces, in the arbitrary value — and calc needs
                // the spaces or Tailwind drops the class without a word.
                class="mb-4 max-h-96 w-full rounded-lg border border-edge object-contain
                       md:sticky md:top-16 md:mb-0 md:max-h-[calc(100vh_-_5rem)]"
            />

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
            on:submit=move |ev: leptos::web_sys::SubmitEvent| {
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
                                on:submit=move |ev: leptos::web_sys::SubmitEvent| {
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
            on:submit=move |ev: leptos::web_sys::SubmitEvent| {
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

            </div>
        </div>
    }
}

#[component]
fn LabeledInput(
    label: &'static str,
    name: &'static str,
    value: String,
    #[prop(optional)] numeric: bool,
) -> impl IntoView {
    view! {
        <label class="flex flex-col gap-1">
            <span class="text-sm text-muted">{label}</span>
            <input
                name=name
                value=value
                // Brings up the numeric keypad on a phone instead of the
                // full keyboard.
                inputmode=numeric.then_some("decimal")
                class="rounded-lg border border-edge bg-ink p-2"
            />
        </label>
    }
}

/// The `<form>` that raised a submit event.
fn form_element(ev: &leptos::web_sys::SubmitEvent) -> leptos::web_sys::HtmlFormElement {
    use leptos::wasm_bindgen::JsCast;
    ev.target()
        .expect("submit event has a target")
        .unchecked_into::<leptos::web_sys::HtmlFormElement>()
}

/// Reads one named field out of a form as a string.
fn field(form: &leptos::web_sys::HtmlFormElement, name: &str) -> String {
    leptos::web_sys::FormData::new_with_form(form)
        .ok()
        .and_then(|d| d.get(name).as_string())
        .unwrap_or_default()
}

fn reset_form(form: &leptos::web_sys::HtmlFormElement) {
    form.reset();
}

#[component]
fn PeriodPage() -> impl IntoView {
    use crate::api::receipts_in_range;
    use jiff::civil::Date;

    // What the user has typed, empty until they touch a picker. The default
    // period is the server's to pick — `jiff` has no clock on wasm without its
    // `js` feature — so an untouched input shows whatever period came back.
    let from_str = RwSignal::new(String::new());
    let to_str = RwSignal::new(String::new());

    // The range actually queried, separate from the inputs so that typing a date
    // doesn't fire a request per keystroke and half-entered ranges never load.
    // `(None, None)` asks the server for its default.
    let range = RwSignal::new((None::<Date>, None::<Date>));
    // Blocking so the whole period arrives in the initial HTML rather than being
    // streamed in after it. One local SQLite read, so the cost to first byte is
    // negligible. (This does not fill the date inputs below: reading a resource
    // synchronously outside a Suspense yields nothing on the server, so those
    // populate on hydration.)
    let summary = Resource::new_blocking(
        move || range.get(),
        |(from, to)| async move { receipts_in_range(from, to).await },
    );

    let loaded = move || summary.get().and_then(|r| r.ok());
    // Derived rather than written into the signals by an effect: an untouched
    // input tracks the loaded period, and clearing one puts it back to tracking
    // rather than leaving a stale date behind.
    let shown_from = move || match from_str.get() {
        s if s.is_empty() => loaded().map(|s| s.from.to_string()).unwrap_or_default(),
        s => s,
    };
    let shown_to = move || match to_str.get() {
        s if s.is_empty() => loaded().map(|s| s.to.to_string()).unwrap_or_default(),
        s => s,
    };

    let apply = move |ev: leptos::web_sys::SubmitEvent| {
        ev.prevent_default();
        // An unparseable or blank end falls back to the server default, which is
        // what the input was already showing. `<input type="date">` only ever
        // yields YYYY-MM-DD or an empty string; this also covers manual entry.
        let parse = |s: String| s.trim().parse::<Date>().ok();
        range.set((parse(shown_from()), parse(shown_to())));
    };

    let date_input = "min-h-11 flex-1 rounded-lg border border-edge bg-ink p-2 sm:flex-none";

    view! {
        <h1 class="mb-4 text-xl font-semibold">"Period"</h1>

        // Outside the Suspense on purpose: if the query fails, the controls to
        // ask for a different period must still be there.
        // One row as soon as it fits; stacked below that so the dates stay
        // full-width and tappable.
        <form class="mb-6 flex flex-col gap-2 sm:flex-row sm:items-center" on:submit=apply>
            <div class="flex items-center gap-2">
                // `value` is what the server renders; `prop:value` is what keeps
                // the live DOM in step once hydrated.
                <input
                    type="date"
                    class=date_input
                    value=shown_from
                    prop:value=shown_from
                    on:input:target=move |ev| from_str.set(ev.target().value())
                />
                <span class="text-muted">"→"</span>
                <input
                    type="date"
                    class=date_input
                    value=shown_to
                    prop:value=shown_to
                    on:input:target=move |ev| to_str.set(ev.target().value())
                />
            </div>
            <button type="submit" class="min-h-11 rounded-lg border border-edge bg-surface px-4">
                "Show period"
            </button>
        </form>

        <Suspense fallback=|| {
            view! { <p class="text-muted">"Loading…"</p> }
        }>
            {move || Suspend::new(async move {
                match summary.await {
                    Err(e) => view! { <p class="text-danger">{format!("{e}")}</p> }.into_any(),
                    Ok(s) => view! { <PeriodBody summary=s /> }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
fn PeriodBody(summary: crate::dto::PeriodSummary) -> impl IntoView {
    use crate::dto::PeriodTotal;

    let count = summary.receipts.len();
    let attention = summary.needing_attention();
    // Built from the loaded period rather than the input signals, so the export
    // can never disagree with the figures on screen.
    let export = format!("/export.csv?from={}&to={}", summary.from, summary.to);

    // An end date before the start matches nothing, which would otherwise render
    // as a confident "0.00 · 0 receipts" — indistinguishable from a month in
    // which nothing was bought.
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
            // Export sits beside the total rather than below the list: on desktop
            // this screen exists to read the figure and take the CSV.
            <div class="sm:flex sm:items-end sm:justify-between sm:gap-4">
                <div>
                    <p class="text-sm text-muted">
                        {summary.from.to_string()} " – " {summary.to.to_string()}
                    </p>
                    <p class="text-3xl font-semibold tabular-nums">
                        {money(summary.total.known())}
                    </p>
                    <p class="text-sm text-muted">
                        {format!("{count} receipt{}", if count == 1 { "" } else { "s" })}
                    </p>
                </div>
                <a
                    href=export
                    class="mt-3 flex min-h-11 items-center justify-center rounded-lg border border-edge px-4 no-underline sm:mt-0"
                >
                    "Export CSV"
                </a>
            </div>

            // The whole point of PeriodTotal being an enum: an incomplete figure
            // cannot be rendered as if it were the real one.
            {match summary.total {
                PeriodTotal::Complete(_) => ().into_any(),
                PeriodTotal::Partial { missing, .. } => {
                    view! {
                        <p class="mt-2 rounded-lg border border-danger p-2 text-sm text-danger">
                            {format!(
                                "This is a floor, not the total: {missing} receipt{} ha{} no amount yet.",
                                if missing == 1 { "" } else { "s" },
                                if missing == 1 { "s" } else { "ve" },
                            )}
                        </p>
                    }
                        .into_any()
                }
            }}
        </div>

        {(attention > 0)
            .then(|| {
                view! {
                    <p class="mb-4 text-sm text-muted">
                        {format!(
                            "{attention} receipt{} need{} checking — marked below.",
                            if attention == 1 { "" } else { "s" },
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
