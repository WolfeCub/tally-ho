//! An on/off filter above a list.

use leptos::prelude::*;

use super::CHIP;

/// A filter whose state reads from across the screen: on looks like the app's
/// primary action, off like one of its plain buttons.
///
/// A `<button aria-pressed>` rather than a checkbox. There is no form to submit
/// it with, and the browser's own checkbox is the one control on these screens
/// that looks like the browser's instead of the app's.
///
/// Read and write are separate halves so the state can live wherever suits the
/// screen: an `RwSignal`, which satisfies both, or the URL.
#[component]
pub fn Toggle(
    label: &'static str,
    #[prop(into)] on: Signal<bool>,
    #[prop(into)] set: SignalSetter<bool>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class=move || {
                let state = if on.get() {
                    "border-paper bg-paper font-medium text-ink"
                } else {
                    "border-edge text-muted active:bg-edge"
                };
                format!("{CHIP} {state}")
            }
            aria-pressed=move || on.get().to_string()
            on:click=move |_| set.set(!on.get_untracked())
        >
            {label}
        </button>
    }
}
