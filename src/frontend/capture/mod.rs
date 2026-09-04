//! Getting receipts in: photograph one, or hand over a pile of PDF invoices.
//! Either way the upload returns as soon as the rows exist, and the screen waits
//! on the model.
//!
//! Two pickers in one form, because one input cannot do both jobs:
//! `capture="environment"` is what sends a phone straight to its camera, and it
//! overrides `multiple` and hides the file browser on iOS. So the camera keeps
//! its own input and the file browser gets a second one, both named `receipts`,
//! and whichever holds something is what gets sent.

mod batch;
mod progress;

use leptos::prelude::*;
use leptos::web_sys::HtmlInputElement;

use crate::frontend::actions::error_of;
use crate::frontend::components::{
    BUTTON, CameraIcon, Notice, Problems, Spinner, Tone, upload_action, uploads_to,
};
use crate::frontend::poll::{extraction_status, poll_while};
use crate::shared::api::upload_receipts;
use batch::Batch;
use progress::{Progress, Stage};

/// How many files a picker is holding.
fn count(input: &HtmlInputElement) -> u32 {
    input.files().map_or(0, |files| files.length())
}

#[component]
pub fn CapturePage() -> impl IntoView {
    let upload = upload_action(upload_receipts);

    // How many each picker holds. One signal per input, so clearing the file
    // browser doesn't forget that a photo is waiting, and a count rather than a
    // flag because it decides which progress view to show before the upload
    // starts.
    let photo = RwSignal::new(0u32);
    let files = RwSignal::new(0u32);
    let picked = move || photo.get() + files.get();

    // The last upload's result, and nothing while another is in flight: the
    // action keeps its previous value through the next dispatch, which would
    // otherwise show a new photo's progress under the last batch's rows.
    let uploaded = Memo::new(move |_| {
        (!upload.pending().get())
            .then(|| upload.value().get()?.ok())
            .flatten()
    });
    let queued = Memo::new(move |_| uploaded.get().map(|u| u.receipt_ids).unwrap_or_default());
    let rejected = Memo::new(move |_| uploaded.get().map(|u| u.rejected).unwrap_or_default());
    // The receipt, when the upload made exactly one. Only then does a stage
    // describe anything: the queue reads a batch one at a time, so those get
    // [`Batch`]'s rows instead.
    let alone = Memo::new(move |_| match queued.get()[..] {
        [id] => Some(id),
        _ => None,
    });

    // Extraction runs in the background, so the only way to know it finished is
    // to ask. Ticking a signal re-runs the resource.
    let tick = RwSignal::new(0u32);
    let status = extraction_status(move || alone.get(), tick);

    let stage = Memo::new(move |_| {
        // Nothing has come back, so what was picked has to stand in for it.
        if upload.pending().get() {
            return (picked() == 1).then_some(Stage::Sending);
        }
        if upload.value().get()?.is_err() {
            return Some(Stage::Failed);
        }
        alone.get().map(|_| Stage::of(status.get().flatten()))
    });

    let working = move || stage.get().is_some_and(Stage::working);

    // Nothing to poll about before there is a receipt, and nothing to learn once
    // the outcome is settled.
    poll_while(tick, move || working() && alone.get().is_some());

    view! {
        <h1 class="mb-4 text-xl font-semibold">"Capture"</h1>

        // Capped on desktop: two pickers and one button have no business
        // spanning the full content width.
        <form class="flex flex-col gap-3 md:max-w-md" on:submit=uploads_to(upload)>
            // The whole panel is the tap target rather than the browser's own
            // file button, which is small and looks nothing like the app.
            <label class="flex cursor-pointer flex-col items-center gap-2 rounded-xl border-2 border-dashed border-edge bg-surface px-4 py-10 text-center active:bg-edge">
                <CameraIcon />
                <span class="font-medium">
                    {move || if photo.get() > 0 { "Photo ready" } else { "Photograph a receipt" }}
                </span>
                <span class="text-sm text-muted">
                    {move || if photo.get() > 0 { "Tap to retake" } else { "Opens the camera" }}
                </span>
                // `capture="environment"` opens the rear camera directly on a
                // phone, and needs no JavaScript at all. Hidden rather than
                // removed, so the label still drives it and the form still
                // carries the file.
                <input
                    type="file"
                    name="receipts"
                    accept="image/*"
                    capture="environment"
                    class="sr-only"
                    on:change:target=move |ev| photo.set(count(&ev.target()))
                />
            </label>

            // Quieter than the camera and below it, so a phone still reads as
            // camera-first. No `capture` here, which is what lets it take
            // several files and a PDF among them.
            <label class="flex min-h-11 cursor-pointer items-center justify-center text-sm text-muted underline underline-offset-4 active:text-paper">
                {move || match files.get() {
                    0 => "or choose files instead".to_string(),
                    1 => "1 file ready: tap to change".to_string(),
                    n => format!("{n} files ready: tap to change"),
                }}
                <input
                    type="file"
                    name="receipts"
                    accept="image/*,application/pdf"
                    multiple
                    class="sr-only"
                    on:change:target=move |ev| files.set(count(&ev.target()))
                />
            </label>

            // Nothing to send until something is picked, and nothing to send
            // twice while one is in flight.
            <button
                type="submit"
                disabled=move || picked() == 0 || working() || upload.pending().get()
                class=format!("{BUTTON} font-medium")
            >
                "Upload"
            </button>
        </form>

        // aria-live so the stages are announced as they change, not just drawn.
        <div class="mt-4 md:max-w-md" aria-live="polite">
            {move || view! { <Problems heading="Not uploaded" items=rejected.get() /> }}

            {move || {
                stage
                    .get()
                    .map(|stage| {
                        view! {
                            <Progress
                                stage
                                // A failed upload has a reason worth reading. A
                                // failed extraction doesn't: the receipt is
                                // there to fill in.
                                reason=error_of(upload).map(|e| e.to_string())
                                receipt_id=alone.get().filter(|_| !stage.working())
                            />
                        }
                    })
            }}

            // What a batch gets instead of the stage bar while it uploads. The
            // `when` is braced, or the macro reads the `>` as the end of the tag.
            <Show when=move || { upload.pending().get() && picked() > 1 }>
                <Notice tone=Tone::Quiet>
                    <div class="flex items-center gap-2">
                        <Spinner />
                        {move || format!("Sending {} files…", picked())}
                    </div>
                </Notice>
            </Show>

            <Show when=move || { queued.get().len() > 1 }>
                <Batch ids=queued />
            </Show>
        </div>
    }
}
