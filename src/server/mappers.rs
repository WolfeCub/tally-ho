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
        models::ExtractionStatus::Assigning => dto::ExtractionStatus::Assigning,
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
        person_id: item.person_id,
        guessed_why: item.guessed_why.clone(),
    }
}

pub fn to_dto_person(person: &models::Person) -> dto::Person {
    dto::Person {
        id: person.id,
        name: person.name.clone(),
        description: person.description.clone(),
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
    // are not sent, since a list shows the conclusion, not the rows.
    let converted: Vec<_> = items.iter().map(to_dto_line_item).collect();
    let status = to_dto_status(&receipt.status);

    // A receipt that hasn't been read yet fails the checks for the uninteresting
    // reason that nothing is filled in — every one of them would report "no
    // total" on a receipt whose total simply hasn't been extracted. That's the
    // job not being finished, not something wrong with the receipt, so hold the
    // checks until it is. `Failed` still reports: there the fields really are
    // final, and empty.
    let read = status.is_terminal();

    dto::ReceiptSummary {
        id: receipt.id,
        purchased_on: receipt.purchased_on,
        merchant: receipt.merchant.clone(),
        total: receipt.total,
        currency: receipt.currency.clone(),
        status,
        item_count: items.len(),
        reviewed: receipt.reviewed_at.is_some(),
        problems: if read {
            crate::shared::problems::problems_of(
                receipt.subtotal,
                receipt.tax,
                receipt.total,
                &converted,
            )
        } else {
            Vec::new()
        },
    }
}
