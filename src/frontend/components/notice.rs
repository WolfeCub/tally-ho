//! Telling you what's going on: the bordered blocks for what needs attention or
//! failed to save, and the bare lines a screen shows in place of content it
//! hasn't got.

use leptos::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum Tone {
    /// Something is wrong and it's yours to fix.
    Bad,
    /// Worth knowing, not worth alarming anyone.
    Quiet,
}

/// What a screen shows while its resource is in flight: `fallback=loading`.
pub fn loading() -> impl IntoView {
    view! { <p class="text-muted">"Loading…"</p> }
}

/// Shown instead of the content, when there isn't going to be any — a failed
/// load, or a URL naming something that isn't there.
pub fn failed(why: impl ToString) -> AnyView {
    view! { <p class="text-danger">{why.to_string()}</p> }.into_any()
}

/// What a screen says at the top when something it tried didn't work.
///
/// Takes the `Option` rather than the action so it reads the same whether the
/// message came from one [`error_of`](crate::frontend::actions::error_of) or
/// from [`first_error`](crate::frontend::actions::first_error) over several.
pub fn error_notice(why: Option<impl ToString>) -> Option<AnyView> {
    // To the string before the view, or the children closure has to carry the
    // error itself and nothing says it's `Send`.
    let why = why?.to_string();
    Some(view! { <Notice tone=Tone::Bad>{why}</Notice> }.into_any())
}

#[component]
pub fn Notice(tone: Tone, children: Children) -> impl IntoView {
    let class = match tone {
        Tone::Bad => "mb-4 rounded-lg border border-danger p-3 text-danger",
        Tone::Quiet => "mb-4 rounded-lg border border-edge p-3 text-sm text-muted",
    };
    view! { <div class=class>{children()}</div> }
}
