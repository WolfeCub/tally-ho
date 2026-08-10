//! Server functions.
//!
//! This file builds for wasm too, so every server-only import has to sit inside
//! a function body rather than at module scope.

use leptos::prelude::*;
use leptos::server_fn::codec::{Json, MultipartData, MultipartFormData};
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

/// Applies the review screen's corrections — the receipt's own fields and the
/// line items as the human left them.
#[server]
pub async fn save_receipt(save: dto::ReceiptSave) -> Result<dto::Receipt, ServerFnError> {
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    let id = save.id;
    let parsed = parse_save(save)?;

    crate::server::query::save_receipt(&mut db, id, parsed)
        .await
        .map_err(|e| ServerFnError::new(format!("could not save receipt: {e}")))?;

    load_receipt(&mut db, id).await
}

/// Checks everything the human typed before a single row is written.
#[cfg(feature = "ssr")]
fn parse_save(save: dto::ReceiptSave) -> Result<crate::server::query::ReceiptSave, ServerFnError> {
    use crate::server::query::{ItemSave, ReceiptSave};

    let purchased_on = crate::server::extract::parse_date(&save.purchased_on).ok_or_else(|| {
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
        items.push(ItemSave {
            id: item.id,
            description,
            total: optional_money("the amount", &item.total)?.unwrap_or_default(),
            person_id: item.person_id,
        });
    }

    Ok(ReceiptSave {
        merchant: save.merchant.trim().to_string(),
        purchased_on,
        currency: save.currency.trim().to_uppercase(),
        subtotal: optional_money("the subtotal", &save.subtotal)?,
        tax: optional_money("the tax", &save.tax)?,
        total: optional_money("the total", &save.total)?,
        items,
    })
}

/// Everyone a line item can be charged to, by name.
#[server]
pub async fn list_people() -> Result<Vec<dto::Person>, ServerFnError> {
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    crate::server::query::list_people(&mut db)
        .await
        .map_err(|e| ServerFnError::new(format!("could not load people: {e}")))
}

/// Applies the settings screen: everyone it ended up with, in one write.
///
/// Anybody missing from the list is removed, and whatever was charged to them
/// goes back to unassigned.
#[server]
pub async fn save_people(people: Vec<dto::PersonSave>) -> Result<(), ServerFnError> {
    use crate::server::query::PersonSave;
    use crate::server::state::AppState;

    let mut parsed = Vec::with_capacity(people.len());
    for person in people {
        let name = person.name.trim();
        // A row added and then left alone is dropped rather than complained about.
        if name.is_empty() && person.description.trim().is_empty() {
            continue;
        }
        if name.is_empty() {
            return Err(ServerFnError::new("a person needs a name"));
        }
        parsed.push(PersonSave {
            id: person.id,
            name: name.to_string(),
            // A blank box means no description, not an empty one.
            description: {
                let described = person.description.trim();
                (!described.is_empty()).then(|| described.to_string())
            },
        });
    }

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    crate::server::query::save_people(&mut db, parsed)
        .await
        .map_err(|e| ServerFnError::new(format!("could not save people: {e}")))
}

/// Throws away a receipt, its line items and its photo.
///
/// For the duplicate upload and the unreadable photo. Without it a bad receipt
/// sits in the list forever and can be matched to a charge it never paid for.
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
/// Refuses while the receipt has no total: reconciliation matches on the total, so
/// a reviewed receipt without one is a claim that nothing can act on.
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
    use crate::server::statements;

    let state = expect_context::<AppState>();
    let mut data = data.into_inner().expect("multipart data on the server");

    let mut label = String::new();
    let mut currency = String::new();
    let mut bytes: Vec<u8> = Vec::new();
    while let Ok(Some(mut field)) = data.next_field().await {
        match field.name() {
            Some("statement") => {
                label = field.file_name().unwrap_or("statement.csv").to_string();
                while let Ok(Some(chunk)) = field.chunk().await {
                    bytes.extend_from_slice(&chunk);
                }
            }
            Some("currency") => currency = field.text().await.unwrap_or_default(),
            // Anything else is ignored rather than trusted.
            _ => continue,
        }
    }

    if bytes.is_empty() {
        return Err(ServerFnError::new("no file was uploaded"));
    }

    let parsed =
        statements::parse::charges(&bytes).map_err(|e| ServerFnError::new(e.to_string()))?;
    let currency = match currency.trim() {
        "" => "USD".to_string(),
        typed => typed.to_uppercase(),
    };

    let mut db = state.db.clone();
    let id = statements::import(&mut db, &label, &currency, &parsed)
        .await
        .map_err(|e| ServerFnError::new(format!("could not import the statement: {e}")))?;

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
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    crate::server::statements::list(&mut db)
        .await
        .map_err(|e| ServerFnError::new(format!("could not load statements: {e}")))
}

/// One statement to reconcile: every charge, what accounts for it, and what it
/// splits to.
#[server]
pub async fn get_statement(id: Uuid) -> Result<dto::Statement, ServerFnError> {
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    crate::server::statements::load(&mut db, id)
        .await
        .map_err(|e| ServerFnError::new(format!("could not load the statement: {e}")))
}

/// Records what a human decided about one charge.
///
/// JSON, not the default form encoding: "split evenly" is `NoReceipt` with a
/// `person_id` of `None`, and urlencoding drops `None`, leaving the variant with
/// no fields and the whole argument missing from the body.
#[server(input = Json)]
pub async fn resolve_charge(charge_id: Uuid, how: dto::Resolve) -> Result<(), ServerFnError> {
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    crate::server::statements::resolve(&mut db, charge_id, how)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Receipts nothing accounts for yet, for picking one by hand.
#[server]
pub async fn spare_receipts(limit: usize) -> Result<Vec<dto::ReceiptSummary>, ServerFnError> {
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    crate::server::statements::spare(&mut db, limit)
        .await
        .map_err(|e| ServerFnError::new(format!("could not load receipts: {e}")))
}

/// Throws away a statement and its charges. The receipts stay.
#[server]
pub async fn delete_statement(id: Uuid) -> Result<(), ServerFnError> {
    use crate::server::state::AppState;

    let state = expect_context::<AppState>();
    let mut db = state.db.clone();

    crate::server::statements::delete(&mut db, id)
        .await
        .map_err(|e| ServerFnError::new(format!("could not delete the statement: {e}")))
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
