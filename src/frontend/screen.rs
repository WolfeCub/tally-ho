//! The shape a screen has around its content: ask the server, then show the
//! answer, the fact there isn't one, or why it couldn't.
//!
//! A `Transition` in every case, never a `Suspense`. Each of these screens
//! refetches while you're looking at it — polling while a receipt is read,
//! reloading after a decision — and a `Suspense` fallback would blank the
//! content every time.

use std::future::Future;

use leptos::prelude::*;
use uuid::Uuid;

use crate::frontend::components::{failed, loading};

/// A resource keyed on the `:id` in the path, for a screen with nothing to show
/// without one.
///
/// A URL naming nothing comes back as `Ok(None)` rather than an error, since
/// that's what a stale link is. [`detail`] is what says so.
pub fn for_id<T, Fut>(
    id: impl Fn() -> Option<Uuid> + Send + Sync + 'static,
    fetch: impl Fn(Uuid) -> Fut + Send + Sync + 'static,
) -> Resource<Result<Option<T>, ServerFnError>>
where
    // Serde because the server streams the first load down to the client rather
    // than making it ask again.
    T: Clone + Send + Sync + serde::Serialize + serde::de::DeserializeOwned + 'static,
    Fut: Future<Output = Result<T, ServerFnError>> + Send + 'static,
{
    Resource::new(id, move |id| {
        // Called out here: the async block moves what it captures, and `fetch`
        // has to stay put for the next load.
        let pending = id.map(&fetch);
        async move {
            match pending {
                Some(fetch) => fetch.await.map(Some),
                None => Ok(None),
            }
        }
    })
}

/// The body of an id-keyed screen: the thing, or why there isn't one.
pub fn detail<T>(
    res: Resource<Result<Option<T>, ServerFnError>>,
    missing: &'static str,
    show: impl Fn(T) -> AnyView + Copy + Send + Sync + 'static,
) -> impl IntoView
where
    T: Clone + Send + Sync + 'static,
{
    view! {
        <Transition fallback=loading>
            {move || Suspend::new(async move {
                match res.await {
                    Err(e) => failed(e),
                    Ok(None) => failed(missing),
                    Ok(Some(found)) => show(found),
                }
            })}
        </Transition>
    }
}

/// The body of a list screen: the rows, or a line saying there are none.
pub fn listing<T>(
    res: Resource<Result<Vec<T>, ServerFnError>>,
    empty: &'static str,
    show: impl Fn(Vec<T>) -> AnyView + Copy + Send + Sync + 'static,
) -> impl IntoView
where
    T: Clone + Send + Sync + 'static,
{
    view! {
        <Transition fallback=loading>
            {move || Suspend::new(async move {
                match res.await {
                    Err(e) => failed(e),
                    Ok(rows) if rows.is_empty() => {
                        view! { <p class="text-muted">{empty}</p> }.into_any()
                    }
                    Ok(rows) => show(rows),
                }
            })}
        </Transition>
    }
}
