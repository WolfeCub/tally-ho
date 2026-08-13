//! Capture and review: uploading a photo, polling while it is read, and saving
//! the corrections a human makes.

use leptos::prelude::*;
use leptos::server_fn::codec::{MultipartData, MultipartFormData};
use uuid::Uuid;

use crate::shared::dto;

#[cfg(feature = "ssr")]
use anyhow::Context as _;

/// Stores an uploaded photo, creates the receipt row, and kicks off extraction.
///
/// Returns as soon as the row exists — extraction runs in the background, so the
/// caller should poll [`receipt_status`].
#[server(input = MultipartFormData)]
pub async fn upload_receipt(data: MultipartData) -> Result<Uuid, ServerFnError> {
    use crate::server::models::Receipt;
    use crate::server::{job, state::AppState};

    let state = expect_context::<AppState>();

    let (_, bytes) = super::support::one_file(data, "receipt").await;
    if bytes.is_empty() {
        return Err(ServerFnError::new("no image was uploaded"));
    }

    let today = jiff::Zoned::now().date();
    let image_path = state
        .store
        .write_upload(&bytes, today)
        .await
        .context("could not store image")
        .map_err(ServerFnError::new)?;

    let mut db = state.db.clone();
    // Deliberately minimal: everything else is unknown until extraction runs.
    // `purchased_on` is provisionally today and `total` stays null — null means
    // "not yet known", never zero.
    let receipt = toasty::create!(Receipt {
        purchased_on: today,
        merchant: "",
        currency: "USD",
        image_path: image_path.clone(),
    })
    .exec(&mut db)
    .await
    .context("could not create receipt")
    .map_err(ServerFnError::new)?;

    job::spawn(state.clone(), receipt.id);

    Ok(receipt.id)
}

/// Poll target while extraction runs.
#[server]
pub async fn receipt_status(id: Uuid) -> Result<dto::ExtractionStatus, ServerFnError> {
    use crate::server::models::Receipt;

    let mut db = super::support::db();

    let receipt = Receipt::get_by_id(&mut db, &id)
        .await
        .context("no such receipt")
        .map_err(ServerFnError::new)?;

    Ok(crate::server::mappers::to_dto_status(&receipt.status))
}

/// Re-runs extraction on a receipt the model failed to read.
///
/// Only from `Failed`: one still extracting already has a job running, and a
/// finished one has line items a second run would duplicate.
#[server]
pub async fn retry_extraction(id: Uuid) -> Result<(), ServerFnError> {
    use crate::server::models::{ExtractionStatus, Receipt};
    use crate::server::{job, state::AppState};

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    let mut receipt = Receipt::get_by_id(&mut db, &id)
        .await
        .context("no such receipt")
        .map_err(ServerFnError::new)?;

    if receipt.status != ExtractionStatus::Failed {
        return Err(ServerFnError::new(
            "this receipt didn't fail — nothing to retry",
        ));
    }

    // Cleared before spawning, so the caller's reload sees a receipt that's
    // working again rather than the failure it just retried.
    toasty::update!(receipt {
        status: ExtractionStatus::Pending,
        extraction_error: None,
    })
    .exec(&mut db)
    .await
    .context("could not reset receipt")
    .map_err(ServerFnError::new)?;

    job::spawn(state, id);
    Ok(())
}

/// Reverse-chronological receipts, newest first, for the list tab.
#[server]
pub async fn recent_receipts(limit: usize) -> Result<Vec<dto::ReceiptSummary>, ServerFnError> {
    use crate::server::queries::receipts;

    Ok(receipts::recent(&mut super::support::db(), limit)
        .await
        .context("could not load receipts")
        .map_err(ServerFnError::new)?
        .iter()
        .map(|(receipt, items)| crate::server::mappers::to_dto_summary(receipt, items))
        .collect())
}

/// Full receipt with line items, for the review screen.
#[server]
pub async fn get_receipt(id: Uuid) -> Result<dto::Receipt, ServerFnError> {
    load_receipt(&mut super::support::db(), id).await
}

/// Applies the review screen's corrections — the receipt's own fields and the
/// line items as the human left them.
#[server]
pub async fn save_receipt(save: dto::ReceiptSave) -> Result<dto::Receipt, ServerFnError> {
    use crate::server::queries::receipts;

    let mut db = super::support::db();

    let id = save.id;
    let parsed = parse_save(save)?;

    receipts::save(&mut db, id, parsed)
        .await
        .context("could not save receipt")
        .map_err(ServerFnError::new)?;

    load_receipt(&mut db, id).await
}

/// Checks everything the human typed before a single row is written.
#[cfg(feature = "ssr")]
fn parse_save(
    save: dto::ReceiptSave,
) -> Result<crate::server::queries::receipts::Save, ServerFnError> {
    use super::support::optional_money;
    use crate::server::queries::receipts::{Save, SaveItem};

    let purchased_on = crate::server::parse::date(&save.purchased_on).ok_or_else(|| {
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
        items.push(SaveItem {
            id: item.id,
            description,
            total: optional_money("the amount", &item.total)?.unwrap_or_default(),
            person_id: item.person_id,
        });
    }

    Ok(Save {
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
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    crate::server::assign::suggest(&mut db, &*state.assigner, id)
        .await
        .context("could not guess who owes what")
        .map_err(ServerFnError::new)?;

    load_receipt(&mut db, id).await
}

/// Records that a human has checked this receipt against the photo.
///
/// Refuses while the receipt has no total: reconciliation matches on the total, so
/// a reviewed receipt without one is a claim that nothing can act on.
#[server]
pub async fn mark_reviewed(id: Uuid) -> Result<dto::Receipt, ServerFnError> {
    use crate::server::models::Receipt;

    let mut db = super::support::db();

    let mut receipt = Receipt::get_by_id(&mut db, &id)
        .await
        .context("no such receipt")
        .map_err(ServerFnError::new)?;

    if receipt.total.is_none() {
        return Err(ServerFnError::new(
            "this receipt has no total yet — enter one before marking it reviewed",
        ));
    }

    toasty::update!(receipt {
        reviewed_at: Some(jiff::Timestamp::now()),
    })
    .exec(&mut db)
    .await
    .context("could not mark reviewed")
    .map_err(ServerFnError::new)?;

    load_receipt(&mut db, id).await
}

/// Throws away a receipt, its line items and its photo.
///
/// For the duplicate upload and the unreadable photo. Without it a bad receipt
/// sits in the list forever and can be matched to a charge it never paid for.
#[server]
pub async fn delete_receipt(id: Uuid) -> Result<(), ServerFnError> {
    use crate::server::queries::receipts;
    use crate::server::state::AppState;

    // The whole state, not just the db: the photo has to go too.
    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    let image_path = receipts::delete(&mut db, id)
        .await
        .context("could not delete receipt")
        .map_err(ServerFnError::new)?;

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
    use crate::server::models::Receipt;

    let receipt = Receipt::get_by_id(db, &id)
        .await
        .context("no such receipt")
        .map_err(ServerFnError::new)?;
    let items = receipt
        .line_items()
        .exec(db)
        .await
        .context("could not load line items")
        .map_err(ServerFnError::new)?;
    Ok(crate::server::mappers::to_dto_receipt(&receipt, &items))
}
