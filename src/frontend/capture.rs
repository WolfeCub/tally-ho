//! Photograph a receipt, upload it, and wait for the model to read it.

use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use leptos::web_sys::{FormData, HtmlFormElement, SubmitEvent};

use crate::frontend::ui::Working;
use crate::shared::api::{receipt_status, upload_receipt};
use crate::shared::dto::ExtractionStatus;

#[component]
pub fn CapturePage() -> impl IntoView {
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

        // aria-live so the stages are announced as they change, not just drawn.
        <div class="mt-6" aria-live="polite">
            {move || {
                if upload.pending().get() {
                    return view! { <Working label="Sending the photo…" /> }.into_any();
                }
                match upload.value().get() {
                    None => view! { <p class="text-muted">"Photograph a receipt."</p> }.into_any(),
                    Some(Err(e)) => {
                        view! { <p class="text-danger">{format!("Upload failed: {e}")}</p> }
                            .into_any()
                    }
                    Some(Ok(id)) => {
                        match status.get().flatten() {
                            Some(ExtractionStatus::Done) => {
                                view! {
                                    <p class="mb-3">"Done reading the receipt."</p>
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
                                    <p class="text-danger mb-3">"Could not read the receipt."</p>
                                    <a
                                        href=format!("/receipt/{id}")
                                        class="inline-block rounded-lg border border-edge bg-surface px-4 py-3"
                                    >
                                        "Enter it by hand"
                                    </a>
                                }
                                    .into_any()
                            }
                            // Saved, but the model hasn't picked it up — usually a
                            // queue behind other uploads.
                            Some(ExtractionStatus::Pending) => {
                                view! {
                                    <Working label="Photo saved. Waiting for the model…" />
                                }
                                    .into_any()
                            }
                            Some(ExtractionStatus::Extracting) => {
                                view! {
                                    <Working label="Reading the receipt… about 10 seconds." />
                                }
                                    .into_any()
                            }
                            // Uploaded, but the first poll hasn't landed yet.
                            None => view! { <Working label="Photo saved." /> }.into_any(),
                        }
                    }
                }
            }}
        </div>
    }
}
