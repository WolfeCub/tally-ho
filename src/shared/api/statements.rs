//! Reconcile: importing a card's statement and resolving the charges on it.

use leptos::prelude::*;
use leptos::server_fn::codec::{Json, MultipartData, MultipartFormData};
use uuid::Uuid;

use crate::shared::dto;

// Server-only, so these are behind a `cfg` rather than plain imports.
#[cfg(feature = "ssr")]
use {
    super::support::{db, one_file},
    crate::server::queries::statements,
    anyhow::Context as _,
};

/// Reads an uploaded statement CSV, writes its charges, and proposes a receipt
/// for the ones that are obvious.
///
/// Returns the statement to reconcile, plus what the sniffer made of the file —
/// which columns it used and any row it couldn't read. Those are shown rather
/// than logged: picking the wrong column would otherwise export a total that
/// quietly doesn't match the card.
#[server(input = MultipartFormData)]
pub async fn import_statement(data: MultipartData) -> Result<dto::Imported, ServerFnError> {
    use crate::server::state::AppState;
    use crate::server::statement_csv;

    let state = expect_context::<AppState>();

    let (label, bytes) = one_file(data, "statement").await;
    if bytes.is_empty() {
        return Err(ServerFnError::new("no file was uploaded"));
    }
    // Only ever shown, so an unnamed upload gets a name rather than an error.
    let label = label.unwrap_or_else(|| "statement.csv".to_string());

    let parsed = statement_csv::charges(&bytes).map_err(|e| ServerFnError::new(e.to_string()))?;

    let mut db = state.db.clone();
    let id = statements::import(&mut db, &label, &state.currency, &parsed)
        .await
        .context("could not import the statement")
        .map_err(ServerFnError::new)?;

    Ok(dto::Imported {
        id,
        columns: [
            parsed.layout.date.name,
            parsed.layout.description.name,
            parsed.layout.amount.name,
        ],
        charge_count: parsed.charges.len(),
        skipped: parsed.skipped,
    })
}

/// Every statement imported, newest first.
#[server]
pub async fn list_statements() -> Result<Vec<dto::StatementSummary>, ServerFnError> {
    statements::list(&mut db())
        .await
        .context("could not load statements")
        .map_err(ServerFnError::new)
}

/// One statement to reconcile: every charge, what accounts for it, and what it
/// splits to.
#[server]
pub async fn get_statement(id: Uuid) -> Result<dto::Statement, ServerFnError> {
    statements::load(&mut db(), id)
        .await
        .context("could not load the statement")
        .map_err(ServerFnError::new)
}

/// Poll target while a receipt photographed onto this statement is being read.
#[server]
pub async fn statement_reading(id: Uuid) -> Result<bool, ServerFnError> {
    statements::reading(&mut db(), id)
        .await
        .context("could not check the statement")
        .map_err(ServerFnError::new)
}

/// Records what a human decided about one charge.
///
/// JSON, not the default form encoding: "split evenly" is `NoReceipt` with a
/// `person_id` of `None`, and urlencoding drops `None`, leaving the variant with
/// no fields and the whole argument missing from the body.
#[server(input = Json)]
pub async fn resolve_charge(charge_id: Uuid, how: dto::Resolve) -> Result<(), ServerFnError> {
    use crate::server::queries::charges;

    charges::resolve(&mut db(), charge_id, how)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Receipts nothing accounts for yet, for picking one by hand.
#[server]
pub async fn spare_receipts(limit: usize) -> Result<Vec<dto::ReceiptSummary>, ServerFnError> {
    use crate::server::queries::receipts;

    receipts::spare(&mut db(), limit)
        .await
        .context("could not load receipts")
        .map_err(ServerFnError::new)
}

/// Throws away a statement and its charges. The receipts stay.
#[server]
pub async fn delete_statement(id: Uuid) -> Result<(), ServerFnError> {
    statements::delete(&mut db(), id)
        .await
        .context("could not delete the statement")
        .map_err(ServerFnError::new)
}
