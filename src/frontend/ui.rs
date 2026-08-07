//! Small pieces more than one screen uses, plus the form plumbing.
//!
//! Every form here is an uncontrolled `<form>` read on submit rather than a
//! signal per input — the server returns the whole receipt after each save, so
//! there's nothing worth keeping in sync between keystrokes.

use leptos::prelude::*;
use leptos::web_sys::{FormData, HtmlFormElement, SubmitEvent};

/// A spinner and a label, so a slow stage doesn't read as a frozen page.
#[component]
pub fn Working(#[prop(into)] label: String) -> impl IntoView {
    view! {
        <p class="flex items-center gap-3 text-muted">
            <span
                class="inline-block size-4 shrink-0 animate-spin rounded-full border-2 border-edge border-t-paper"
                aria-hidden="true"
            ></span>
            {label}
        </p>
    }
}

#[component]
pub fn LabeledInput(
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
pub fn form_element(ev: &SubmitEvent) -> HtmlFormElement {
    use leptos::wasm_bindgen::JsCast;
    ev.target()
        .expect("submit event has a target")
        .unchecked_into::<HtmlFormElement>()
}

/// Reads one named field out of a form as a string.
pub fn field(form: &HtmlFormElement, name: &str) -> String {
    FormData::new_with_form(form)
        .ok()
        .and_then(|d| d.get(name).as_string())
        .unwrap_or_default()
}

pub fn reset_form(form: &HtmlFormElement) {
    form.reset();
}
