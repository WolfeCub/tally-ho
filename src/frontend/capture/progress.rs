//! How far one uploaded receipt has got, stage by stage.

use leptos::prelude::*;
use uuid::Uuid;

use crate::frontend::components::{AS_BUTTON, BUTTON, Spinner, StepBar, Verdict};
use crate::shared::dto::ExtractionStatus;

/// How far the upload has got, or `None` before there is one. Derived in one
/// place so the card, the progress bar and the button can't disagree.
#[derive(Clone, Copy, PartialEq)]
pub enum Stage {
    Sending,
    Queued,
    Reading,
    Assigning,
    Done,
    Failed,
}

impl Stage {
    /// Where a receipt the server already holds has got to. `None` is a poll
    /// that hasn't landed, which looks the same as one that hasn't started.
    pub fn of(status: Option<ExtractionStatus>) -> Self {
        match status {
            Some(ExtractionStatus::Done) => Self::Done,
            Some(ExtractionStatus::Failed) => Self::Failed,
            Some(ExtractionStatus::Extracting) => Self::Reading,
            Some(ExtractionStatus::Assigning) => Self::Assigning,
            Some(ExtractionStatus::Pending) | None => Self::Queued,
        }
    }

    pub fn working(self) -> bool {
        matches!(
            self,
            Self::Sending | Self::Queued | Self::Reading | Self::Assigning
        )
    }

    /// Segments lit on the progress bar, out of four.
    fn reached(self) -> usize {
        match self {
            Self::Sending => 1,
            Self::Queued | Self::Reading => 2,
            Self::Assigning => 3,
            Self::Done | Self::Failed => 4,
        }
    }

    fn heading(self) -> &'static str {
        match self {
            Self::Sending => "Sending it over",
            Self::Queued => "Waiting for the model",
            Self::Reading => "Reading the receipt",
            Self::Assigning => "Working out who owes what",
            Self::Done => "Read it",
            Self::Failed => "Could not read it",
        }
    }

    /// Says "receipt", never "photo": by this point it may be a PDF invoice.
    fn detail(self) -> &'static str {
        match self {
            Self::Sending => "Uploading.",
            Self::Queued => "Queued behind the others.",
            Self::Reading => "Extracting the text content",
            Self::Assigning => "Matching the items to people.",
            Self::Done => "Check it against the receipt before you rely on it.",
            Self::Failed => "It saved, so you can still type it in.",
        }
    }
}

/// Where the upload has got to, and the way out once it stops moving.
#[component]
pub fn Progress(
    stage: Stage,
    reason: Option<String>,
    /// `None` while there's still something to wait for.
    receipt_id: Option<Uuid>,
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
                <StepBar reached=stage.reached() of=4 bad=failed />
            </div>

            {receipt_id
                .map(|id| {
                    view! {
                        <a href=format!("/receipt/{id}") class=format!("{BUTTON} {AS_BUTTON} mt-4")>
                            {if failed { "Enter it by hand" } else { "Review it" }}
                        </a>
                    }
                })}
        </div>
    }
}
