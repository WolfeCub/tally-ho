//! Re-asking the server about work it is still doing.
//!
//! Extraction runs in the background, after the request that started it has
//! already returned, and nothing is pushed to the client — so the only way to
//! find out it finished is to ask again.

use std::time::Duration;

use leptos::prelude::*;

/// How often to re-ask. Quick enough that a receipt landing feels immediate,
/// slow enough that an open tab isn't a load generator: extraction takes seconds
/// against a local model, so there is nothing to be gained by asking harder.
const EVERY: Duration = Duration::from_millis(1500);

/// Ticks `tick` every [`EVERY`] for as long as `working` reads true.
///
/// Use `tick` as a resource's source and the resource re-runs on each tick.
/// `working` is read reactively — usually off that same resource — so polling
/// stops on the first answer that says the work is done, instead of running for
/// as long as the page is open.
///
/// Takes the signal rather than making one because the caller needs it first:
/// the resource is built from it, and `working` then reads the resource.
pub fn poll_while(tick: RwSignal<u32>, working: impl Fn() -> bool + Send + Sync + 'static) {
    Effect::new(move |prev: Option<Option<IntervalHandle>>| {
        // Clear any previous timer before starting another, or a condition that
        // flickers would leave a timer running per flicker.
        if let Some(Some(handle)) = prev {
            handle.clear();
        }
        if !working() {
            return None;
        }
        set_interval_with_handle(move || tick.update(|t| *t += 1), EVERY).ok()
    });
}
