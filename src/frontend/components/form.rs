//! Labelled inputs, the plumbing that reads them back, and asking before
//! something irreversible.
//!
//! Every form here is uncontrolled — read on submit rather than a signal per
//! input. The server returns the whole receipt after each save, so there's
//! nothing worth keeping in sync between keystrokes.

use leptos::prelude::*;
use leptos::web_sys::{FormData, HtmlFormElement, SubmitEvent};

use super::INPUT;

#[component]
pub fn LabeledInput(
    label: &'static str,
    name: &'static str,
    /// Empty for a form that starts blank.
    #[prop(optional, into)]
    value: String,
    /// `date` gets the platform picker; the rest are text.
    #[prop(default = "text")]
    kind: &'static str,
    #[prop(optional)] numeric: bool,
) -> impl IntoView {
    view! {
        <label class="flex flex-col gap-1">
            <span class="text-sm text-muted">{label}</span>
            <input
                type=kind
                name=name
                value=value
                // Brings up the numeric keypad on a phone instead of the
                // full keyboard.
                inputmode=numeric.then_some("decimal")
                class=INPUT
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

/// Asks before something that can't be undone. A browser that won't ask counts
/// as a no.
pub fn confirm(question: &str) -> bool {
    leptos::prelude::window()
        .confirm_with_message(question)
        .unwrap_or(false)
}
