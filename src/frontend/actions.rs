//! Reading back what a server action did.

use leptos::prelude::*;

/// True once an action has come back OK.
///
/// Failure deliberately doesn't count. A screen reloads itself on success, and
/// doing that after a rejected save would rebuild the form from the server and
/// throw away what you typed — along with the message saying why.
pub fn succeeded<I, O>(action: Action<I, Result<O, ServerFnError>>) -> bool
where
    I: Send + Sync + 'static,
    O: Clone + Send + Sync + 'static,
{
    matches!(action.value().get(), Some(Ok(_)))
}

pub fn error_of<I, O>(action: Action<I, Result<O, ServerFnError>>) -> Option<ServerFnError>
where
    I: Send + Sync + 'static,
    O: Clone + Send + Sync + 'static,
{
    action.value().get().and_then(|r| r.err())
}

/// The first thing that went wrong, for the notice at the top of a screen. One
/// message, since they all mean the same thing: that didn't save.
pub fn first_error(errors: impl IntoIterator<Item = Option<ServerFnError>>) -> Option<String> {
    errors.into_iter().flatten().next().map(|e| e.to_string())
}
