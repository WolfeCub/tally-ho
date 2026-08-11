//! Photograph a receipt, upload it, and wait for the model to read it.

use leptos::prelude::*;
use leptos::web_sys::{FormData, SubmitEvent};

use crate::frontend::components::{
    AS_BUTTON, BUTTON, CameraIcon, Spinner, StepBar, Verdict, form_element,
};
use crate::frontend::poll::{extraction_status, poll_while};
use crate::shared::api::upload_receipt;
use crate::shared::dto::ExtractionStatus;

/// How far the upload has got, or `None` before there is one. Derived in one
/// place so the card, the progress bar and the button can't disagree.
#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Sending,
    Queued,
    Reading,
    Done,
    Failed,
}

impl Stage {
    fn working(self) -> bool {
        matches!(self, Self::Sending | Self::Queued | Self::Reading)
    }

    /// Segments lit on the progress bar, out of three.
    fn reached(self) -> usize {
        match self {
            Self::Sending => 1,
            Self::Queued | Self::Reading => 2,
            Self::Done | Self::Failed => 3,
        }
    }

    fn heading(self) -> &'static str {
        match self {
            Self::Sending => "Sending the photo",
            Self::Queued => "Waiting for the model",
            Self::Reading => "Reading the receipt",
            Self::Done => "Read it",
            Self::Failed => "Could not read it",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::Sending => "Uploading from your phone.",
            Self::Queued => "Queued behind the other photos.",
            Self::Reading => "Usually about ten seconds.",
            Self::Done => "Check it against the photo before you rely on it.",
            Self::Failed => "The photo saved, so you can still type it in.",
        }
    }
}

#[component]
pub fn CapturePage() -> impl IntoView {
    // `_local` because FormData is not Send; the upload only ever runs client-side.
    let upload = Action::new_local(|data: &FormData| upload_receipt(data.clone().into()));

    // Whether the picker holds a photo. Reading `value` rather than `files`
    // keeps us to the web-sys features leptos already turns on.
    let chosen = RwSignal::new(false);

    // Id of the receipt currently being extracted, if any.
    let receipt_id = Memo::new(move |_| upload.value().get().and_then(|r| r.ok()));

    // Extraction runs in the background, so the only way to know it finished is
    // to ask. Ticking a signal re-runs the resource.
    let tick = RwSignal::new(0u32);
    let status = extraction_status(move || receipt_id.get(), tick);

    let stage = Memo::new(move |_| {
        if upload.pending().get() {
            return Some(Stage::Sending);
        }
        match upload.value().get()? {
            Err(_) => Some(Stage::Failed),
            Ok(_) => Some(match status.get().flatten() {
                Some(ExtractionStatus::Done) => Stage::Done,
                Some(ExtractionStatus::Failed) => Stage::Failed,
                Some(ExtractionStatus::Extracting) => Stage::Reading,
                // No status yet means the first poll hasn't landed.
                Some(ExtractionStatus::Pending) | None => Stage::Queued,
            }),
        }
    });

    let working = move || stage.get().is_some_and(Stage::working);

    // Nothing to poll about before there is a receipt, and nothing to learn once
    // the outcome is settled.
    poll_while(tick, move || working() && receipt_id.get().is_some());

    view! {
        <h1 class="mb-4 text-xl font-semibold">"Capture"</h1>

        // Capped on desktop: a picker and one button have no business spanning
        // the full content width.
        <form
            class="flex flex-col gap-3 md:max-w-md"
            on:submit=move |ev: SubmitEvent| {
                ev.prevent_default();
                let data = FormData::new_with_form(&form_element(&ev)).unwrap();
                upload.dispatch_local(data);
            }
        >
            // The whole panel is the tap target rather than the browser's own
            // file button, which is small and looks nothing like the app.
            <label class="flex cursor-pointer flex-col items-center gap-2 rounded-xl border-2 border-dashed border-edge bg-surface px-4 py-10 text-center active:bg-edge">
                <CameraIcon />
                <span class="font-medium">
                    {move || if chosen.get() { "Photo ready" } else { "Photograph a receipt" }}
                </span>
                <span class="text-sm text-muted">
                    {move || if chosen.get() { "Tap to retake" } else { "Opens the camera" }}
                </span>
                // `capture="environment"` opens the rear camera directly on a
                // phone, and needs no JavaScript at all. Hidden rather than
                // removed, so the label still drives it and the form still
                // carries the file.
                <input
                    type="file"
                    name="receipt"
                    accept="image/*"
                    capture="environment"
                    class="sr-only"
                    on:change:target=move |ev| chosen.set(!ev.target().value().is_empty())
                />
            </label>

            // Nothing to send until a photo is picked, and nothing to send twice
            // while one is in flight.
            <button
                type="submit"
                disabled=move || !chosen.get() || working()
                class=format!("{BUTTON} font-medium")
            >
                "Upload"
            </button>
        </form>

        // aria-live so the stages are announced as they change, not just drawn.
        <div class="mt-4 md:max-w-md" aria-live="polite">
            {move || {
                stage
                    .get()
                    .map(|stage| {
                        view! {
                            <Progress
                                stage
                                // A failed upload has a reason worth reading. A
                                // failed extraction doesn't — the receipt is
                                // there to fill in.
                                reason=upload.value().get().and_then(|r| r.err()).map(|e| e.to_string())
                                receipt_id=receipt_id.get().filter(|_| !stage.working())
                            />
                        }
                    })
            }}
        </div>
    }
}

/// Where the upload has got to, and the way out once it stops moving.
#[component]
fn Progress(
    stage: Stage,
    reason: Option<String>,
    /// `None` while there's still something to wait for.
    receipt_id: Option<uuid::Uuid>,
) -> impl IntoView {
    let failed = stage == Stage::Failed;
    let edge = if failed {
        "border-danger"
    } else {
        "border-edge"
    };

    view! {
        <div class=format!("rounded-xl border {edge} bg-surface p-4")>
            <div class="flex items-start gap-3">
                {if stage.working() {
                    view! { <Spinner class="mt-0.5" /> }.into_any()
                } else {
                    view! { <Verdict ok=!failed class="mt-0.5" /> }.into_any()
                }}
                <div class="min-w-0 flex-1">
                    <p class=if failed { "font-medium text-danger" } else { "font-medium" }>
                        {stage.heading()}
                    </p>
                    <p class="mt-0.5 text-sm text-muted">
                        {reason.unwrap_or_else(|| stage.detail().to_string())}
                    </p>
                </div>
            </div>

            <div class="mt-4">
                <StepBar reached=stage.reached() of=3 bad=failed />
            </div>

            {receipt_id
                .map(|id| {
                    view! {
                        <a
                            href=format!("/receipt/{id}")
                            class=format!("{BUTTON} {AS_BUTTON} mt-4")
                        >
                            {if failed { "Enter it by hand" } else { "Review it" }}
                        </a>
                    }
                })}
        </div>
    }
}
