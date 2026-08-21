//! An on/off filter above a list.

use leptos::prelude::*;

/// Sized and shaped like the row actions rather than like a form field: these
/// sit above a list, not in a form.
const CHIP: &str = "min-h-11 rounded-lg border px-3 text-sm";

/// A filter whose state reads from across the screen: on looks like the app's
/// primary action, off like one of its plain buttons.
///
/// A `<button aria-pressed>` rather than a checkbox. There is no form to submit
/// it with, and the browser's own checkbox is the one control on these screens
/// that looks like the browser's instead of the app's.
#[component]
pub fn Toggle(label: &'static str, on: RwSignal<bool>) -> impl IntoView {
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
            on:click=move |_| on.update(|on| *on = !*on)
        >
            {label}
        </button>
    }
}
