//! Plumbing shared by the server functions. Server-only, so nothing here needs
//! a `cfg`.

use std::fmt::Display;

use leptos::prelude::*;
use leptos::server_fn::codec::MultipartData;

/// The database handle, out of the request context. Every server function starts
/// with one.
pub fn db() -> toasty::Db {
    use crate::server::state::AppState;
    expect_context::<AppState>().db.clone()
}

/// The one file field a form sent, with whatever the browser called it.
///
/// Any other field is ignored rather than trusted, and the bytes come back empty
/// if `name` never turned up — which the callers report in their own words.
pub async fn one_file(data: MultipartData, name: &str) -> (Option<String>, Vec<u8>) {
    // `into_inner()` is always `Some` on the server.
    let mut data = data.into_inner().expect("multipart data on the server");

    let mut filename = None;
    let mut bytes = Vec::new();
    while let Ok(Some(mut field)) = data.next_field().await {
        if field.name() != Some(name) {
            continue;
        }
        filename = field.file_name().map(str::to_string);
        while let Ok(Some(chunk)) = field.chunk().await {
            bytes.extend_from_slice(&chunk);
        }
        break;
    }
    (filename, bytes)
}

/// How every server function ends: an error the screen can show.
///
/// `ServerFnError` carries a string across the wire, so the cause has to be
/// flattened into the message or it's gone for good.
pub trait Reported<T> {
    /// The error in its own words, for one already written for a human.
    fn reported(self) -> Result<T, ServerFnError>;

    /// The error behind a sentence saying what we were trying to do.
    fn reported_as(self, what: &str) -> Result<T, ServerFnError>;
}

impl<T, E: Display> Reported<T> for Result<T, E> {
    fn reported(self) -> Result<T, ServerFnError> {
        self.map_err(|e| ServerFnError::new(e.to_string()))
    }

    fn reported_as(self, what: &str) -> Result<T, ServerFnError> {
        self.map_err(|e| ServerFnError::new(format!("{what}: {e}")))
    }
}
