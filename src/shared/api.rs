//! Server functions.
//!
//! This file builds for wasm too, so every server-only import has to sit inside
//! a function body rather than at module scope.

use leptos::prelude::*;
use leptos::server_fn::codec::{MultipartData, MultipartFormData};
use uuid::Uuid;

use crate::shared::dto;

/// Stores an uploaded photo, creates the receipt row, and kicks off extraction.
///
/// Returns as soon as the row exists — extraction runs in the background, so the
/// caller should poll [`receipt_status`].
#[server(input = MultipartFormData)]
pub async fn upload_receipt(data: MultipartData) -> Result<Uuid, ServerFnError> {
    use crate::server::models::Receipt;
    use crate::server::{job, state::AppState};

    let state = expect_context::<AppState>();

    // `into_inner()` is always `Some` on the server.
    let mut data = data.into_inner().expect("multipart data on the server");

    let mut bytes: Vec<u8> = Vec::new();
    while let Ok(Some(mut field)) = data.next_field().await {
        // The browser sends one file field; anything else is ignored rather
        // than trusted.
        if field.name() != Some("receipt") {
            continue;
        }
        while let Ok(Some(chunk)) = field.chunk().await {
            bytes.extend_from_slice(&chunk);
        }
        break;
    }

    if bytes.is_empty() {
        return Err(ServerFnError::new("no image was uploaded"));
    }

    let today = jiff::Zoned::now().date();
    let image_path = state
        .store
        .write_upload(&bytes, today)
        .await
        .map_err(|e| ServerFnError::new(format!("could not store image: {e}")))?;

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
    .map_err(|e| ServerFnError::new(format!("could not create receipt: {e}")))?;

    job::spawn(state.clone(), receipt.id);

    Ok(receipt.id)
}

/// Poll target while extraction runs.
#[server]
pub async fn receipt_status(id: Uuid) -> Result<dto::ExtractionStatus, ServerFnError> {
    use crate::server::models::Receipt;
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    let receipt = Receipt::get_by_id(&mut db, &id)
        .await
        .map_err(|e| ServerFnError::new(format!("no such receipt: {e}")))?;

    Ok(crate::server::mappers::to_dto_status(&receipt.status))
}

/// Parses a human-typed amount, distinguishing "cleared" from "unparseable".
#[cfg(feature = "ssr")]
fn optional_money(field: &str, raw: &str) -> Result<Option<rust_decimal::Decimal>, ServerFnError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    crate::server::extract::parse_money(raw)
        .map(Some)
        .ok_or_else(|| ServerFnError::new(format!("could not read {field} {raw:?} as an amount")))
}

/// Applies human corrections to a receipt's own fields.
#[server]
pub async fn update_receipt_meta(edit: dto::ReceiptEdit) -> Result<dto::Receipt, ServerFnError> {
    use crate::server::models::Receipt;
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    // Parsed before anything is written, so a typo in one field cannot leave the
    // receipt half-updated.
    let purchased_on = crate::server::extract::parse_date(&edit.purchased_on).ok_or_else(|| {
        ServerFnError::new(format!(
            "could not read {:?} as a date — try YYYY-MM-DD or MM/DD/YY",
            edit.purchased_on
        ))
    })?;
    let subtotal = optional_money("the subtotal", &edit.subtotal)?;
    let tax = optional_money("the tax", &edit.tax)?;
    let total = optional_money("the total", &edit.total)?;

    let merchant = edit.merchant.trim().to_string();
    let currency = edit.currency.trim().to_uppercase();

    let mut receipt = Receipt::get_by_id(&mut db, &edit.id)
        .await
        .map_err(|e| ServerFnError::new(format!("no such receipt: {e}")))?;

    toasty::update!(receipt {
        merchant: merchant,
        purchased_on: purchased_on,
        currency: currency,
        subtotal: subtotal,
        tax: tax,
        total: total,
    })
    .exec(&mut db)
    .await
    .map_err(|e| ServerFnError::new(format!("could not save receipt: {e}")))?;

    load_receipt(&mut db, edit.id).await
}

#[server]
pub async fn update_line_item(edit: dto::LineItemEdit) -> Result<dto::Receipt, ServerFnError> {
    use crate::server::models::LineItem;
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    let total = optional_money("the amount", &edit.total)?.unwrap_or_default();
    let description = edit.description.trim().to_string();
    if description.is_empty() {
        return Err(ServerFnError::new("a line item needs a description"));
    }

    let mut item = LineItem::get_by_id(&mut db, &edit.id)
        .await
        .map_err(|e| ServerFnError::new(format!("no such line item: {e}")))?;
    let receipt_id = item.receipt_id;

    toasty::update!(item {
        description: description,
        total: total,
        // Marks this row as human-checked, so a later re-extraction can know
        // not to clobber it.
        edited: true,
    })
    .exec(&mut db)
    .await
    .map_err(|e| ServerFnError::new(format!("could not save line item: {e}")))?;

    load_receipt(&mut db, receipt_id).await
}

#[server]
pub async fn add_line_item(
    receipt_id: Uuid,
    description: String,
    total: String,
) -> Result<dto::Receipt, ServerFnError> {
    use crate::server::models::{LineItem, Receipt};
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    let amount = optional_money("the amount", &total)?.unwrap_or_default();
    let description = description.trim().to_string();
    if description.is_empty() {
        return Err(ServerFnError::new("a line item needs a description"));
    }

    let receipt = Receipt::get_by_id(&mut db, &receipt_id)
        .await
        .map_err(|e| ServerFnError::new(format!("no such receipt: {e}")))?;
    let existing = receipt
        .line_items()
        .exec(&mut db)
        .await
        .map_err(|e| ServerFnError::new(format!("could not load line items: {e}")))?;
    // Append after whatever is already there, including rows a human added.
    let next = existing.iter().map(|i| i.position).max().unwrap_or(-1) + 1;

    toasty::create!(LineItem {
        receipt_id: receipt_id,
        description: description,
        total: amount,
        position: next,
        edited: true,
    })
    .exec(&mut db)
    .await
    .map_err(|e| ServerFnError::new(format!("could not add line item: {e}")))?;

    load_receipt(&mut db, receipt_id).await
}

#[server]
pub async fn delete_line_item(id: Uuid) -> Result<dto::Receipt, ServerFnError> {
    use crate::server::models::LineItem;
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    let item = LineItem::get_by_id(&mut db, &id)
        .await
        .map_err(|e| ServerFnError::new(format!("no such line item: {e}")))?;
    let receipt_id = item.receipt_id;

    item.delete()
        .exec(&mut db)
        .await
        .map_err(|e| ServerFnError::new(format!("could not delete line item: {e}")))?;

    load_receipt(&mut db, receipt_id).await
}

/// Throws away a receipt, its line items and its photo.
///
/// For the duplicate upload and the unreadable photo. Without it a bad receipt
/// sits in the list forever and quietly pads the period total.
#[server]
pub async fn delete_receipt(id: Uuid) -> Result<(), ServerFnError> {
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    let image_path = crate::server::query::delete_receipt(&mut db, id)
        .await
        .map_err(|e| ServerFnError::new(format!("could not delete receipt: {e}")))?;

    // After the rows, and only a warning: the filesystem isn't part of the
    // transaction and the receipt is already gone. An orphaned image costs disk;
    // failing here would report a delete that plainly did happen.
    if let Err(e) = state.store.delete(&image_path).await {
        tracing::warn!(%id, %image_path, error = %e, "receipt deleted but its image remains");
    }

    Ok(())
}

/// Records that a human has checked this receipt against the photo.
///
/// Refuses while the receipt has no total: marking it reviewed is a claim the
/// period view relies on, and a reviewed receipt with no total would silently
/// under-report.
#[server]
pub async fn mark_reviewed(id: Uuid) -> Result<dto::Receipt, ServerFnError> {
    use crate::server::models::Receipt;
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    let mut receipt = Receipt::get_by_id(&mut db, &id)
        .await
        .map_err(|e| ServerFnError::new(format!("no such receipt: {e}")))?;

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
    .map_err(|e| ServerFnError::new(format!("could not mark reviewed: {e}")))?;

    load_receipt(&mut db, id).await
}

/// Shared tail for the mutations: they all return the receipt as it now stands,
/// so the client never has to guess what the server did.
#[cfg(feature = "ssr")]
async fn load_receipt(db: &mut toasty::Db, id: Uuid) -> Result<dto::Receipt, ServerFnError> {
    use crate::server::models::Receipt;

    let receipt = Receipt::get_by_id(db, &id)
        .await
        .map_err(|e| ServerFnError::new(format!("no such receipt: {e}")))?;
    let items = receipt
        .line_items()
        .exec(db)
        .await
        .map_err(|e| ServerFnError::new(format!("could not load line items: {e}")))?;
    Ok(crate::server::mappers::to_dto_receipt(&receipt, &items))
}

/// The reconciliation view: every receipt in a statement period, with a total.
///
/// Both ends are optional. Omitting them asks for the default period (the
/// previous whole month) — the client has no clock, and the returned
/// [`dto::PeriodSummary`] carries the dates that were actually used, so the date
/// pickers can be populated from the first response.
#[server]
pub async fn receipts_in_range(
    from: Option<jiff::civil::Date>,
    to: Option<jiff::civil::Date>,
) -> Result<dto::PeriodSummary, ServerFnError> {
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    let (from, to) = crate::server::query::resolve_range(from, to);
    let rows = crate::server::query::load_range(&mut db, from, to)
        .await
        .map_err(|e| ServerFnError::new(format!("could not load the period: {e}")))?;

    let summaries = rows
        .iter()
        .map(|(r, items)| crate::server::mappers::to_dto_summary(r, items))
        .collect();

    Ok(dto::PeriodSummary::new(from, to, summaries))
}

/// Reverse-chronological receipts, newest first, for the list tab.
#[server]
pub async fn recent_receipts(limit: usize) -> Result<Vec<dto::ReceiptSummary>, ServerFnError> {
    use crate::server::models::Receipt;
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    let receipts = Receipt::all()
        .order_by(Receipt::fields().purchased_on().desc())
        .limit(limit)
        .exec(&mut db)
        .await
        .map_err(|e| ServerFnError::new(format!("could not load receipts: {e}")))?;

    let mut out = Vec::with_capacity(receipts.len());
    for receipt in receipts {
        let items = receipt
            .line_items()
            .exec(&mut db)
            .await
            .map_err(|e| ServerFnError::new(format!("could not load line items: {e}")))?;
        out.push(crate::server::mappers::to_dto_summary(&receipt, &items));
    }
    Ok(out)
}

/// Full receipt with line items, for the review screen.
#[server]
pub async fn get_receipt(id: Uuid) -> Result<dto::Receipt, ServerFnError> {
    use crate::server::models::Receipt;
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    let receipt = Receipt::get_by_id(&mut db, &id)
        .await
        .map_err(|e| ServerFnError::new(format!("no such receipt: {e}")))?;
    let items = receipt
        .line_items()
        .exec(&mut db)
        .await
        .map_err(|e| ServerFnError::new(format!("could not load line items: {e}")))?;

    Ok(crate::server::mappers::to_dto_receipt(&receipt, &items))
}
