//! The charges on a statement: what a human decided about one, and which
//! receipts are already spoken for.

use std::collections::HashSet;

use uuid::Uuid;

use crate::server::models;
use crate::shared::dto;

/// Records what a human decided about one charge.
pub async fn resolve(
    db: &mut toasty::Db,
    charge_id: Uuid,
    how: dto::Resolve,
) -> anyhow::Result<()> {
    let mut charge = models::Charge::get_by_id(db, &charge_id).await?;

    if let dto::Resolve::Receipt(receipt_id) = how {
        // Taking a receipt off another charge would leave that one looking
        // settled against nothing.
        let held = models::Charge::filter(models::Charge::fields().receipt_id().eq(receipt_id))
            .exec(db)
            .await?;
        if held.iter().any(|other| other.id != charge_id) {
            anyhow::bail!("that receipt already accounts for another charge");
        }
    }

    let (receipt_id, confirmed, no_receipt, person_id) = match how {
        dto::Resolve::Receipt(receipt_id) => (Some(receipt_id), true, false, None),
        dto::Resolve::NoReceipt { person_id } => (None, false, true, person_id),
        dto::Resolve::Clear => (None, false, false, None),
    };

    toasty::update!(charge {
        receipt_id: receipt_id,
        confirmed: confirmed,
        no_receipt: no_receipt,
        person_id: person_id,
    })
    .exec(db)
    .await?;
    Ok(())
}

/// Receipts already accounted for by a charge, on this statement or any other.
///
/// Every charge rather than a filtered query: toasty has no "is not null", and a
/// card's worth of statements is a few hundred rows.
pub async fn spoken_for(db: &mut toasty::Db) -> toasty::Result<HashSet<Uuid>> {
    Ok(models::Charge::all()
        .exec(db)
        .await?
        .into_iter()
        .filter_map(|charge| charge.receipt_id)
        .collect())
}

/// Whether anything accounts for this charge yet.
pub fn settled(charge: &models::Charge) -> bool {
    charge.no_receipt || (charge.receipt_id.is_some() && charge.confirmed)
}
