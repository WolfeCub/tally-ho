//! Coarse progress. The server reports stages, not percentages, so a smooth bar
//! would be inventing detail it doesn't have.

use leptos::prelude::*;

#[component]
pub fn StepBar(reached: usize, of: usize, #[prop(optional)] bad: bool) -> impl IntoView {
    let lit = if bad { "bg-danger" } else { "bg-paper" };
    view! {
        <div class="flex gap-1.5" aria-hidden="true">
            {(1..=of)
                .map(|step| {
                    let fill = if step <= reached { lit } else { "bg-edge" };
                    view! { <span class=format!("h-1 flex-1 rounded-full {fill}")></span> }
                })
                .collect_view()}
        </div>
    }
}
