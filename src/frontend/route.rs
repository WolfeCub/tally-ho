//! Reading the part of the URL a screen is keyed on.

use leptos::prelude::*;
use uuid::Uuid;

/// The `:id` out of the path, if it is a uuid at all — `None` covers both a
/// missing segment and a malformed one, which a screen reports the same way.
///
/// Hands back a closure rather than a value so it stays reactive: a resource
/// keyed on it reloads when the router moves from one receipt to the next,
/// instead of showing the first one forever.
pub fn id_param() -> impl Fn() -> Option<Uuid> + Copy + Send + Sync + 'static {
    let params = leptos_router::hooks::use_params_map();
    move || {
        params
            .read()
            .get("id")
            .and_then(|id| Uuid::parse_str(&id).ok())
    }
}
