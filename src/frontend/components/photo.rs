//! What a receipt was read from, beside the fields it produced: a photo with a
//! full-screen lightbox, or a PDF in the browser's own viewer.

use leptos::prelude::*;

use crate::shared::dto::Document;

/// The frame either kind sits in: pinned beside the fields once there's room to
/// put it there, so it stays put while they scroll.
const FRAME: &str = "mb-4 rounded-lg border border-edge md:sticky md:top-16 md:mb-0";

/// The file behind a receipt, whichever kind it is.
#[component]
pub fn ReceiptDocument(kind: Document, src: String) -> impl IntoView {
    match kind {
        Document::Photo => view! { <ReceiptPhoto src /> }.into_any(),
        Document::Pdf => view! { <ReceiptPdf src /> }.into_any(),
    }
}

/// A vendor's invoice. No lightbox, every PDF viewer already zooms and scrolls,
/// and one nested in an overlay fights whatever the browser does with Escape.
#[component]
fn ReceiptPdf(src: String) -> impl IntoView {
    let fallback = src.clone();
    view! {
        <div class=FRAME>
            // `object` rather than `iframe`: it shows its children instead when
            // the browser won't render a PDF inline, which on iOS it won't.
            <object
                data=src
                type="application/pdf"
                class="block h-96 w-full md:h-[calc(100vh_-_5rem)]"
            >
                <a href=fallback class="block p-4">
                    "Open the PDF"
                </a>
            </object>
        </div>
    }
}

/// Tap to fill the screen, tap again to zoom.
#[component]
fn ReceiptPhoto(src: String) -> impl IntoView {
    let open = RwSignal::new(false);
    let zoomed = RwSignal::new(false);
    let dismiss = move || {
        open.set(false);
        zoomed.set(false);
    };

    // Focus the overlay when it opens, so Escape lands on it.
    let overlay = NodeRef::<leptos::html::Div>::new();
    Effect::new(move |_| {
        if open.get()
            && let Some(el) = overlay.get()
        {
            let _ = el.focus();
        }
    });

    // Copy, so both the thumbnail and the overlay closures can read it. `Show`
    // children have to be `Fn`, so a String can't just be moved in.
    let src = StoredValue::new(src);

    // Scrolls its own overflow, which the PDF's viewer does for itself: a long
    // receipt scaled to fit the viewport is unreadable. calc needs the spaces
    // around the `-`, written as underscores, or Tailwind drops the class
    // without a word.
    let framed = format!("{FRAME} md:max-h-[calc(100vh_-_5rem)] md:overflow-y-auto");

    view! {
        <div class=framed>
            // A button, so it's reachable by keyboard and announced as activatable.
            <button
                type="button"
                class="block w-full cursor-zoom-in"
                aria-label="View photo full screen"
                on:click=move |_| open.set(true)
            >
                <img
                    src=move || src.get_value()
                    alt="receipt photo"
                    class="block max-h-96 w-full object-contain md:max-h-none"
                />
            </button>
        </div>

        // Sibling of the pinned box, not a child: a fixed overlay inside a sticky
        // overflow-y-auto container doesn't behave.
        <Show when=move || open.get()>
            // Block, not flex: a flex item shrinks back under the container width,
            // so the zoom below wouldn't hold. tabindex to take focus for Escape.
            <div
                node_ref=overlay
                tabindex="-1"
                class="fixed inset-0 z-50 overflow-auto bg-ink/95"
                on:keydown=move |ev: leptos::web_sys::KeyboardEvent| {
                    if ev.key() == "Escape" {
                        dismiss();
                    }
                }
            >
                // Fit fills the width and scrolls down; zoom widens past the
                // viewport and scrolls both ways.
                <img
                    src=move || src.get_value()
                    alt="receipt photo"
                    class=move || {
                        if zoomed.get() {
                            "block w-[250%] max-w-none cursor-zoom-out"
                        } else {
                            "block w-full cursor-zoom-in"
                        }
                    }
                    on:click=move |_| zoomed.update(|z| *z = !*z)
                />
                <button
                    type="button"
                    class="fixed top-2 right-2 flex min-h-11 items-center rounded-lg border border-edge bg-surface px-4"
                    on:click=move |_| dismiss()
                >
                    "Close"
                </button>
            </div>
        </Show>
    }
}
