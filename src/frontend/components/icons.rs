//! Inline SVG, so there's no icon font or sprite sheet to fetch.
//!
//! Each takes an optional `class` for whatever the caller needs on top —
//! alignment, mostly.

use leptos::prelude::*;

/// For any stage that hasn't settled, so a slow one doesn't read as a frozen page.
#[component]
pub fn Spinner(#[prop(optional, into)] class: String) -> impl IntoView {
    view! {
        <span
            class=format!(
                "size-5 shrink-0 animate-spin rounded-full border-2 border-edge border-t-paper {class}",
            )
            aria-hidden="true"
        ></span>
    }
}

/// A tick or an alert, for a stage that has.
#[component]
pub fn Verdict(ok: bool, #[prop(optional, into)] class: String) -> impl IntoView {
    let tone = if ok { "text-good" } else { "text-danger" };
    view! {
        <svg
            class=format!("size-5 shrink-0 {tone} {class}")
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            aria-hidden="true"
        >
            <circle cx="12" cy="12" r="9" />
            <path
                stroke-linecap="round"
                stroke-linejoin="round"
                d=if ok { "M8 12.5l2.5 2.5L16 9.5" } else { "M12 7.5v5m0 3.5h.01" }
            />
        </svg>
    }
}

#[component]
pub fn CameraIcon(#[prop(optional, into)] class: String) -> impl IntoView {
    view! {
        <svg
            class=format!("size-9 text-muted {class}")
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            aria-hidden="true"
        >
            <path
                stroke-linecap="round"
                stroke-linejoin="round"
                d="M6.827 6.175A2.31 2.31 0 0 1 5.186 7.23c-.38.054-.757.112-1.134.175C2.999 7.58 2.25 8.507 2.25 9.574V18a2.25 2.25 0 0 0 2.25 2.25h15A2.25 2.25 0 0 0 21.75 18V9.574c0-1.067-.75-1.994-1.802-2.169a47.865 47.865 0 0 0-1.134-.175 2.31 2.31 0 0 1-1.64-1.055l-.822-1.316a2.192 2.192 0 0 0-1.736-1.039 48.774 48.774 0 0 0-5.232 0 2.192 2.192 0 0 0-1.736 1.039l-.821 1.316Z"
            />
            <path
                stroke-linecap="round"
                stroke-linejoin="round"
                d="M16.5 12.75a4.5 4.5 0 1 1-9 0 4.5 4.5 0 0 1 9 0ZM18.75 10.5h.008v.008h-.008V10.5Z"
            />
        </svg>
    }
}
