//! Capture and review: uploading a photo, polling while it is read, and saving
//! the corrections a human makes.

use leptos::prelude::*;
use leptos::server_fn::codec::{MultipartData, MultipartFormData};
use uuid::Uuid;

use crate::shared::dto;

// Server-only, so these are behind a `cfg` rather than plain imports.
#[cfg(feature = "ssr")]
use {
    super::support::{Reported as _, db, one_file},
    crate::server::queries::receipts,
    crate::server::state::AppState,
    crate::shared::parse,
};

/// Stores an uploaded photo, creates the receipt row, and kicks off extraction.
///
/// Returns as soon as the row exists — extraction runs in the background, so the
/// caller should poll [`receipt_status`].
#[server(input = MultipartFormData)]
pub async fn upload_receipt(data: MultipartData) -> Result<Uuid, ServerFnError> {
    let state = expect_context::<AppState>();

    let (_, bytes) = one_file(data, "receipt").await;
    if bytes.is_empty() {
        return Err(ServerFnError::new("no image was uploaded"));
    }

    let today = jiff::Zoned::now().date();
    let image_path = state
        .store
        .write_upload(&bytes, today)
        .await
        .reported_as("could not store image")?;

    let mut db = state.db.clone();
    let id = receipts::create(&mut db, &image_path, today)
        .await
        .reported_as("could not create receipt")?;

    state.jobs.push(id);

    Ok(id)
}

/// Poll target while extraction runs.
#[server]
pub async fn receipt_status(id: Uuid) -> Result<dto::ExtractionStatus, ServerFnError> {
    receipts::status(&mut db(), id)
        .await
        .reported_as("no such receipt")
}

/// Re-runs extraction on a receipt the model failed to read.
#[server]
pub async fn retry_extraction(id: Uuid) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    // Reset before queueing, so the caller's reload sees a receipt that's
    // working again rather than the failure it just retried.
    receipts::reset_for_retry(&mut db, id).await.reported()?;

    state.jobs.push(id);
    Ok(())
}

/// Reverse-chronological receipts, newest first, for the list tab.
#[server]
pub async fn recent_receipts(limit: usize) -> Result<Vec<dto::ReceiptSummary>, ServerFnError> {
    receipts::recent(&mut db(), limit)
        .await
        .reported_as("could not load receipts")
}

/// Full receipt with line items, for the review screen.
#[server]
pub async fn get_receipt(id: Uuid) -> Result<dto::Receipt, ServerFnError> {
    load_receipt(&mut db(), id).await
}

/// Applies the review screen's corrections — the receipt's own fields and the
/// line items as the human left them.
#[server]
pub async fn save_receipt(save: dto::ReceiptSave) -> Result<dto::Receipt, ServerFnError> {
    let mut db = db();

    let id = save.id;
    let parsed = parse_save(save)?;

    receipts::save(&mut db, id, parsed)
        .await
        .reported_as("could not save receipt")?;

    load_receipt(&mut db, id).await
}

/// Parses a human-typed amount, distinguishing "cleared" from "unparseable".
#[cfg(feature = "ssr")]
fn optional_money(field: &str, raw: &str) -> Result<Option<rust_decimal::Decimal>, ServerFnError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    parse::money(raw)
        .map(Some)
        .ok_or_else(|| ServerFnError::new(format!("could not read {field} {raw:?} as an amount")))
}

/// Checks everything the human typed before a single row is written.
#[cfg(feature = "ssr")]
fn parse_save(save: dto::ReceiptSave) -> Result<receipts::Save, ServerFnError> {
    let purchased_on = parse::date(&save.purchased_on).ok_or_else(|| {
        ServerFnError::new(format!(
            "could not read {:?} as a date — try YYYY-MM-DD or MM/DD/YY",
            save.purchased_on
        ))
    })?;

    let mut items = Vec::with_capacity(save.items.len());
    for item in save.items {
        let description = item.description.trim().to_string();
        // A row added and then left alone is dropped rather than complained about.
        if description.is_empty() && item.total.trim().is_empty() {
            continue;
        }
        if description.is_empty() {
            return Err(ServerFnError::new("a line item needs a description"));
        }
        items.push(receipts::SaveItem {
            id: item.id,
            description,
            total: optional_money("the amount", &item.total)?.unwrap_or_default(),
            person_id: item.person_id,
        });
    }

    Ok(receipts::Save {
        merchant: save.merchant.trim().to_string(),
        purchased_on,
        currency: save.currency.trim().to_uppercase(),
        subtotal: optional_money("the subtotal", &save.subtotal)?,
        tax: optional_money("the tax", &save.tax)?,
        total: optional_money("the total", &save.total)?,
        items,
    })
}

/// Guesses who owes what from the descriptions in Settings, and hands back the
/// receipt with whatever it decided.
///
/// Separate from saving because descriptions get written long after a receipt was
/// read, and because correcting a line item is a reason to ask again.
#[server]
pub async fn suggest_assignments(id: Uuid) -> Result<dto::Receipt, ServerFnError> {
    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    crate::server::assign::suggest(&mut db, &*state.assigner, id)
        .await
        .reported_as("could not guess who owes what")?;

    load_receipt(&mut db, id).await
}

/// Records that a human has checked this receipt against the photo.
#[server]
pub async fn mark_reviewed(id: Uuid) -> Result<dto::Receipt, ServerFnError> {
    let mut db = db();

    receipts::mark_reviewed(&mut db, id).await.reported()?;

    load_receipt(&mut db, id).await
}

/// Throws away a receipt, its line items and its photo.
///
/// For the duplicate upload and the unreadable photo. Without it a bad receipt
/// sits in the list forever and can be matched to a charge it never paid for.
#[server]
pub async fn delete_receipt(id: Uuid) -> Result<(), ServerFnError> {
    use crate::server::queries::receipts;
    // The whole state, not just the db: the photo has to go too.
    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    let image_path = receipts::delete(&mut db, id)
        .await
        .reported_as("could not delete receipt")?;

    // After the rows, and only a warning: the filesystem isn't part of the
    // transaction and the receipt is already gone. An orphaned image costs disk;
    // failing here would report a delete that plainly did happen.
    if let Err(e) = state.store.delete(&image_path).await {
        tracing::warn!(%id, %image_path, error = %e, "receipt deleted but its image remains");
    }

    Ok(())
}

/// One receipt as it now stands, items and all.
///
/// Every mutation ends here rather than reporting a bare success, so the client
/// never has to guess what the server did. [`get_receipt`] is the same thing
/// asked for on its own.
#[cfg(feature = "ssr")]
async fn load_receipt(db: &mut toasty::Db, id: Uuid) -> Result<dto::Receipt, ServerFnError> {
    receipts::load(db, id)
        .await
        .reported_as("could not load receipt")?
        .ok_or_else(|| ServerFnError::new("no such receipt"))
}
