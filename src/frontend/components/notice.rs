//! The bordered message blocks: what needs attention, what the model wasn't
//! sure of, and what just failed to save.

use leptos::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum Tone {
    /// Something is wrong and it's yours to fix.
    Bad,
    /// Worth knowing, not worth alarming anyone.
    Quiet,
}

#[component]
pub fn Notice(tone: Tone, children: Children) -> impl IntoView {
    let class = match tone {
        Tone::Bad => "mb-4 rounded-lg border border-danger p-3 text-danger",
        Tone::Quiet => "mb-4 rounded-lg border border-edge p-3 text-sm text-muted",
    };
    view! { <div class=class>{children()}</div> }
}
