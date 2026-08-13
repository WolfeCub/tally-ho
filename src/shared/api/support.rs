//! Plumbing shared by the server functions. Server-only, so nothing here needs
//! a `cfg`.

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

/// Parses a human-typed amount, distinguishing "cleared" from "unparseable".
pub fn optional_money(
    field: &str,
    raw: &str,
) -> Result<Option<rust_decimal::Decimal>, ServerFnError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    crate::server::parse::money(raw)
        .map(Some)
        .ok_or_else(|| ServerFnError::new(format!("could not read {field} {raw:?} as an amount")))
}
