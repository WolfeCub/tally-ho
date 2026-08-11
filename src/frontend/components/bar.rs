//! The thin bar under a figure.

use leptos::prelude::*;

/// How far through something you are, or how full it is.
#[component]
pub fn Bar(
    /// Out of 100. Clamped, so a figure that shouldn't happen can't draw past
    /// the end of the track.
    percent: usize,
    /// Colour of the filled part, because "more" is progress in one place and
    /// trouble in another.
    #[prop(default = "bg-good")]
    fill: &'static str,
) -> impl IntoView {
    let width = percent.min(100);
    view! {
        <div class="mt-1 h-1 rounded bg-edge">
            <div class=format!("h-1 rounded {fill}") style=format!("width:{width}%")></div>
        </div>
    }
}
