//! Database rows into the wire types the client sees.
//!
//! One direction only. Edits arrive as strings and get parsed in
//! [`crate::shared::api`], so there's no other half to keep in step.

use crate::server::models;
use crate::shared::dto;

pub fn to_dto_status(status: &models::ExtractionStatus) -> dto::ExtractionStatus {
    match status {
        models::ExtractionStatus::Pending => dto::ExtractionStatus::Pending,
        models::ExtractionStatus::Extracting => dto::ExtractionStatus::Extracting,
        models::ExtractionStatus::Done => dto::ExtractionStatus::Done,
        models::ExtractionStatus::Failed => dto::ExtractionStatus::Failed,
    }
}

pub fn to_dto_line_item(item: &models::LineItem) -> dto::LineItem {
    dto::LineItem {
        id: item.id,
        description: item.description.clone(),
        quantity: item.quantity,
        unit_price: item.unit_price,
        total: item.total,
        position: item.position,
        edited: item.edited,
    }
}

pub fn to_dto_receipt(receipt: &models::Receipt, items: &[models::LineItem]) -> dto::Receipt {
    let mut line_items: Vec<_> = items.iter().map(to_dto_line_item).collect();
    // Ordering is the receipt's own, not the database's.
    line_items.sort_by_key(|i| i.position);

    dto::Receipt {
        id: receipt.id,
        purchased_on: receipt.purchased_on,
        merchant: receipt.merchant.clone(),
        subtotal: receipt.subtotal,
        tax: receipt.tax,
        total: receipt.total,
        currency: receipt.currency.clone(),
        status: to_dto_status(&receipt.status),
        extraction_error: receipt.extraction_error.clone(),
        reviewed: receipt.reviewed_at.is_some(),
        line_items,
    }
}

pub fn to_dto_summary(
    receipt: &models::Receipt,
    items: &[models::LineItem],
) -> dto::ReceiptSummary {
    // Converts the items only to run the shared problem checks over them; they
    // are not sent, since the period list shows the conclusion, not the rows.
    let converted: Vec<_> = items.iter().map(to_dto_line_item).collect();

    dto::ReceiptSummary {
        id: receipt.id,
        purchased_on: receipt.purchased_on,
        merchant: receipt.merchant.clone(),
        total: receipt.total,
        currency: receipt.currency.clone(),
        status: to_dto_status(&receipt.status),
        item_count: items.len(),
        reviewed: receipt.reviewed_at.is_some(),
        problems: dto::problems_of(receipt.subtotal, receipt.tax, receipt.total, &converted),
    }
}
