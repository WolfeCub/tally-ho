//! Labelled inputs, the plumbing that reads them back, and asking before
//! something irreversible.
//!
//! Every form here is uncontrolled — read on submit rather than a signal per
//! input. The server returns the whole receipt after each save, so there's
//! nothing worth keeping in sync between keystrokes.

use std::future::Future;

use leptos::prelude::*;
use leptos::server_fn::codec::MultipartData;
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

/// An action that posts a form to a server function taking a multipart body.
///
/// `_local` because `FormData` is not `Send`, and nothing is lost by it: an
/// upload only ever runs in the browser.
pub fn upload_action<O, Fut>(
    send: impl Fn(MultipartData) -> Fut + 'static,
) -> Action<FormData, Result<O, ServerFnError>>
where
    O: Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<O, ServerFnError>> + 'static,
{
    Action::new_local(move |data: &FormData| send(data.clone().into()))
}

/// An `on:submit` that sends the form to `action` instead of navigating.
pub fn uploads_to<O>(
    action: Action<FormData, Result<O, ServerFnError>>,
) -> impl Fn(SubmitEvent) + Copy + 'static
where
    O: Clone + Send + Sync + 'static,
{
    move |ev: SubmitEvent| {
        ev.prevent_default();
        if let Ok(data) = FormData::new_with_form(&form_element(&ev)) {
            action.dispatch_local(data);
        }
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
