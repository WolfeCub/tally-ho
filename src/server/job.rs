//! Background extraction.
//!
//! Extraction takes ~11s against a local 12B model and can queue behind other
//! work, so it never runs inline with the upload request — a phone would sit
//! there spinning. The upload returns as soon as the row exists, and the client
//! polls `receipt_status`.

use uuid::Uuid;

use super::assign;
use super::models::{ExtractionStatus, Receipt};
use super::state::AppState;

/// Spawns extraction for an already-persisted receipt and returns immediately.
pub fn spawn(state: AppState, receipt_id: Uuid) {
    tokio::spawn(async move {
        if let Err(e) = run(&state, receipt_id).await {
            // The job owns its own error reporting: nothing is awaiting this
            // task, so a swallowed error would leave the receipt stuck in
            // `Extracting` forever with no trace.
            tracing::error!(%receipt_id, error = %e, "extraction job failed");
            let _ = mark_failed(&state, receipt_id, &e.to_string()).await;
        }
    });
}

/// Boxed error must be `Send + Sync`: this runs inside `tokio::spawn`, whose
/// future has to cross threads.
type JobError = Box<dyn std::error::Error + Send + Sync>;

async fn run(state: &AppState, receipt_id: Uuid) -> Result<(), JobError> {
    let mut db = state.db.clone();

    let mut receipt = Receipt::get_by_id(&mut db, &receipt_id).await?;
    let image_path = receipt.image_path.clone();

    toasty::update!(receipt {
        status: ExtractionStatus::Extracting
    })
    .exec(&mut db)
    .await?;

    let bytes = state.store.read(&image_path).await?;
    let extraction = state.extractor.extract(&bytes).await?;
    let n = extraction.receipt.normalize();

    tracing::info!(
        %receipt_id,
        elapsed_s = extraction.elapsed.as_secs_f32(),
        items = n.line_items.len(),
        warnings = n.warnings.len(),
        "extraction succeeded"
    );

    // Warnings are per-field parse failures (e.g. an unreadable amount). They
    // are recorded rather than discarded so the review screen can point at what
    // needs a human, but they are not themselves a failure.
    let note = (!n.warnings.is_empty()).then(|| n.warnings.join("; "));

    let mut receipt = Receipt::get_by_id(&mut db, &receipt_id).await?;
    toasty::update!(receipt {
        merchant: n.merchant.unwrap_or_default(),
        // `purchased_on` falls back to today only so the row stays queryable in
        // a period; a wrong-but-plausible date is exactly why the review screen
        // flags anything with warnings.
        purchased_on: n.purchased_on.unwrap_or_else(|| jiff::Zoned::now().date()),
        // Receipts often don't print a currency, and one in the wrong currency
        // matches no charge at all. So fall back to what the statements are in
        // rather than to a guess.
        currency: n.currency.unwrap_or_else(|| state.currency.clone()),
        subtotal: n.subtotal,
        tax: n.tax,
        total: n.total,
        extraction_error: note,
        model_used: Some(extraction.model.clone()),
        raw_response: Some(toasty::Json(serde_json::from_str(
            &extraction.structuring_raw
        )?)),
    })
    .exec(&mut db)
    .await?;

    for (position, item) in n.line_items.iter().enumerate() {
        toasty::create!(in receipt.line_items() {
            description: item.description.clone(),
            quantity: item.quantity,
            unit_price: item.unit_price,
            // An item whose amount could not be read contributes nothing rather
            // than a guess; the mismatch against the subtotal is what surfaces
            // it in review.
            total: item.total.unwrap_or_default(),
            position: position as i64,
        })
        .exec(&mut db)
        .await?;
    }

    // The second model call, and a stage of its own on the client: the photo has
    // been read by this point, so "reading the receipt" would be a lie for the
    // ten seconds this takes.
    toasty::update!(receipt {
        status: ExtractionStatus::Assigning
    })
    .exec(&mut db)
    .await?;

    // A bonus rather than part of reading the receipt: if it fails the receipt is
    // still fine, just with nobody's name on it.
    if let Err(e) = assign::suggest(&mut db, &*state.assigner, receipt_id).await {
        tracing::warn!(%receipt_id, error = %e, "could not guess who owes what");
    }

    // Done last, so a client that stops polling on it finds the items and the
    // guesses already there.
    toasty::update!(receipt {
        status: ExtractionStatus::Done
    })
    .exec(&mut db)
    .await?;

    Ok(())
}

async fn mark_failed(state: &AppState, receipt_id: Uuid, error: &str) -> Result<(), JobError> {
    let mut db = state.db.clone();
    let mut receipt = Receipt::get_by_id(&mut db, &receipt_id).await?;
    toasty::update!(receipt {
        status: ExtractionStatus::Failed,
        extraction_error: Some(error.to_string()),
    })
    .exec(&mut db)
    .await?;
    Ok(())
}
