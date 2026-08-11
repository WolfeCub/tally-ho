//! Re-asking the server about work it is still doing.
//!
//! Extraction runs in the background, after the request that started it has
//! already returned, and nothing is pushed to the client — so the only way to
//! find out it finished is to ask again.

use std::time::Duration;

use leptos::prelude::*;
use uuid::Uuid;

use crate::shared::api::receipt_status;
use crate::shared::dto::ExtractionStatus;

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

/// Polls until `busy` says the work has stopped, then runs `settled` once.
///
/// For the screens that show something the work will *replace*, rather than
/// something it updates as it goes: re-asking for that on every tick rebuilds it
/// under you, which throws away whatever local state it was holding and, where
/// there's a photo, flickers it.
///
/// `None` from `busy` is no answer — nothing to watch yet, or a check still in
/// flight. Reading it as "finished" would both stop the polling on its own first
/// tick and fire `settled` for work that was never running.
pub fn poll_until_settled(
    tick: RwSignal<u32>,
    busy: impl Fn() -> Option<bool> + Send + Sync + 'static,
    settled: impl Fn() + Send + Sync + 'static,
) {
    let working = RwSignal::new(false);
    Effect::new(move |_| {
        let Some(busy) = busy() else { return };
        if working.get_untracked() && !busy {
            settled();
        }
        working.set(busy);
    });
    poll_while(tick, move || working.get());
}

/// One receipt's extraction status, re-asked whenever `tick` moves.
///
/// `None` covers both "no receipt to watch yet" and "no answer has landed",
/// which come to the same thing for a caller: not finished.
pub fn extraction_status(
    id: impl Fn() -> Option<Uuid> + Send + Sync + 'static,
    tick: RwSignal<u32>,
) -> Resource<Option<ExtractionStatus>> {
    Resource::new(
        move || (id(), tick.get()),
        |(id, _)| async move {
            match id {
                Some(id) => receipt_status(id).await.ok(),
                None => None,
            }
        },
    )
}
