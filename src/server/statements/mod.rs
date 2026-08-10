//! The database side of reconciliation: importing a statement, reading one back
//! with its matches, and recording what a human decided about a charge.

pub mod parse;

use std::collections::HashSet;

use uuid::Uuid;

use crate::server::{mappers, matching, models, query};
use crate::shared::dto;
use crate::shared::reconcile::{charge_to, split_charge};

/// How far either side of a statement's own dates to look for receipts. Wider
/// than the match window, so a receipt right at the edge still gets offered.
const SLACK: i32 = 7;

/// Every statement imported, newest first.
pub async fn list(db: &mut toasty::Db) -> toasty::Result<Vec<dto::StatementSummary>> {
    let statements = models::Statement::all()
        .order_by(models::Statement::fields().imported_at().desc())
        .exec(db)
        .await?;

    let mut out = Vec::with_capacity(statements.len());
    for statement in statements {
        let charges = statement.charges().exec(db).await?;
        out.push(dto::StatementSummary {
            id: statement.id,
            label: statement.label.clone(),
            begins_on: statement.begins_on,
            ends_on: statement.ends_on,
            currency: statement.currency.clone(),
            charge_count: charges.len(),
            settled_count: charges.iter().filter(|c| is_settled(c)).count(),
        });
    }
    Ok(out)
}

/// Writes a parsed file and its charges, proposing a receipt where one is
/// obvious.
///
/// One transaction: half a statement reads exactly like a whole one and would
/// reconcile to a total that's quietly short.
pub async fn import(
    db: &mut toasty::Db,
    label: &str,
    currency: &str,
    parsed: &parse::Parsed,
) -> anyhow::Result<Uuid> {
    let (begins_on, ends_on) = parsed.range();
    let free = pool(db, begins_on, ends_on).await?;
    // Two identical charges must not both propose the same receipt.
    let mut taken = HashSet::new();

    let mut tx = db.transaction().await?;
    let statement = toasty::create!(models::Statement {
        label: label,
        currency: currency,
        begins_on: begins_on,
        ends_on: ends_on,
    })
    .exec(&mut tx)
    .await?;

    for (position, charge) in parsed.charges.iter().enumerate() {
        let untaken = free.iter().filter(|receipt| !taken.contains(&receipt.id));
        let proposed = matching::automatic(&matching::candidates(
            charge.charged_on,
            charge.amount,
            currency,
            untaken,
        ));
        if let Some(receipt) = proposed {
            taken.insert(receipt);
        }

        toasty::create!(models::Charge {
            statement_id: statement.id,
            charged_on: charge.charged_on,
            description: &charge.description,
            amount: charge.amount,
            position: position as i64,
            receipt_id: proposed,
        })
        .exec(&mut tx)
        .await?;
    }

    tx.commit().await?;
    Ok(statement.id)
}

/// One statement, with each charge's resolution, split and suggestions.
pub async fn load(db: &mut toasty::Db, id: Uuid) -> anyhow::Result<dto::Statement> {
    let statement = models::Statement::get_by_id(db, &id).await?;
    let mut rows = statement.charges().exec(db).await?;
    rows.sort_by_key(|charge| charge.position);

    let currency = statement.currency.as_str();
    let people = query::list_people(db).await?;
    let free = pool(db, statement.begins_on, statement.ends_on).await?;

    let mut charges = Vec::with_capacity(rows.len());
    for row in &rows {
        // A matched receipt is deliberately absent from `free`, and may be
        // outside the window anyway if it was attached by hand.
        let matched = match row.receipt_id {
            Some(receipt_id) => receipt(db, receipt_id).await?,
            None => None,
        };

        let (resolution, split) = match matched {
            Some(receipt) => {
                let split = split_charge(row.amount, currency, &receipt.line_items, &people);
                let matched = to_matched(&receipt);
                let resolution = if row.confirmed {
                    dto::Resolution::Confirmed(matched)
                } else {
                    dto::Resolution::Proposed(matched)
                };
                (resolution, split)
            }
            None if row.no_receipt => (
                dto::Resolution::NoReceipt {
                    person_id: row.person_id,
                },
                charge_to(row.amount, currency, row.person_id, &people),
            ),
            None => (dto::Resolution::Unresolved, Vec::new()),
        };

        charges.push(dto::Charge {
            id: row.id,
            charged_on: row.charged_on,
            description: row.description.clone(),
            amount: row.amount,
            suggestions: match resolution {
                dto::Resolution::Unresolved => {
                    matching::candidates(row.charged_on, row.amount, currency, &free)
                }
                // Nothing to suggest once something accounts for it; the screen
                // offers a way back to unresolved instead.
                _ => Vec::new(),
            },
            resolution,
            split,
        });
    }

    Ok(dto::Statement {
        id: statement.id,
        label: statement.label.clone(),
        currency: statement.currency.clone(),
        begins_on: statement.begins_on,
        ends_on: statement.ends_on,
        charges,
        people,
    })
}

/// Receipts no charge accounts for, newest first. What the picker offers when
/// nothing was suggested — a receipt whose date was misread is nowhere near the
/// charge that paid for it.
pub async fn spare(db: &mut toasty::Db, limit: usize) -> toasty::Result<Vec<dto::ReceiptSummary>> {
    let taken = spoken_for(db).await?;
    let receipts = models::Receipt::all()
        .order_by(models::Receipt::fields().purchased_on().desc())
        .exec(db)
        .await?;

    let mut out = Vec::new();
    for receipt in receipts {
        if out.len() == limit {
            break;
        }
        if taken.contains(&receipt.id) {
            continue;
        }
        let items = receipt.line_items().exec(db).await?;
        out.push(mappers::to_dto_summary(&receipt, &items));
    }
    Ok(out)
}

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

/// Throws away a statement and its charges.
///
/// Receipts are left alone: they're evidence of a purchase, not part of the file
/// that happened to bill for it. Nothing cascades, so the charges go first.
pub async fn delete(db: &mut toasty::Db, id: Uuid) -> toasty::Result<()> {
    let mut tx = db.transaction().await?;

    let statement = models::Statement::get_by_id(&mut tx, &id).await?;
    for charge in statement.charges().exec(&mut tx).await? {
        charge.delete().exec(&mut tx).await?;
    }
    statement.delete().exec(&mut tx).await?;

    tx.commit().await
}

fn is_settled(charge: &models::Charge) -> bool {
    charge.no_receipt || (charge.receipt_id.is_some() && charge.confirmed)
}

fn to_matched(receipt: &dto::Receipt) -> dto::Matched {
    dto::Matched {
        receipt_id: receipt.id,
        merchant: receipt.merchant.clone(),
        purchased_on: receipt.purchased_on,
        total: receipt.total,
        status: receipt.status,
        reviewed: receipt.reviewed,
        problems: receipt.problems(),
    }
}

/// The receipts still going spare near a statement's dates.
async fn pool(
    db: &mut toasty::Db,
    begins_on: jiff::civil::Date,
    ends_on: jiff::civil::Date,
) -> toasty::Result<Vec<dto::Receipt>> {
    let slack = jiff::Span::new().days(SLACK);
    let begins_on = begins_on.checked_sub(slack).unwrap_or(begins_on);
    let ends_on = ends_on.checked_add(slack).unwrap_or(ends_on);

    let taken = spoken_for(db).await?;
    Ok(query::load_range(db, begins_on, ends_on)
        .await?
        .iter()
        .map(|(receipt, items)| mappers::to_dto_receipt(receipt, items))
        .filter(|receipt| !taken.contains(&receipt.id))
        .collect())
}

/// Receipts already accounted for by a charge, on this statement or any other.
///
/// Every charge rather than a filtered query: toasty has no "is not null", and a
/// card's worth of statements is a few hundred rows.
async fn spoken_for(db: &mut toasty::Db) -> toasty::Result<HashSet<Uuid>> {
    Ok(models::Charge::all()
        .exec(db)
        .await?
        .into_iter()
        .filter_map(|charge| charge.receipt_id)
        .collect())
}

async fn receipt(db: &mut toasty::Db, id: Uuid) -> anyhow::Result<Option<dto::Receipt>> {
    // Gone rather than an error: deleting a receipt clears the charges pointing
    // at it, but a page loaded a moment earlier can still ask for one.
    let Ok(receipt) = models::Receipt::get_by_id(db, &id).await else {
        return Ok(None);
    };
    let items = receipt.line_items().exec(db).await?;
    Ok(Some(mappers::to_dto_receipt(&receipt, &items)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::models::{Person, Receipt};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    async fn memory_db() -> toasty::Db {
        crate::server::db::connect_url("sqlite::memory:")
            .await
            .unwrap()
    }

    /// A charge a receipt explains, and one nothing does.
    const CSV: &str = "Transaction Date,Description,Amount\n\
                       07/10/2026,COSTCO WHSE #1050,35.36\n\
                       07/11/2026,NETFLIX.COM,17.99\n";

    async fn people(db: &mut toasty::Db) -> (Uuid, Uuid) {
        let josh = toasty::create!(Person { name: "Josh" })
            .exec(db)
            .await
            .unwrap();
        let ash = toasty::create!(Person { name: "Ash" })
            .exec(db)
            .await
            .unwrap();
        (josh.id, ash.id)
    }

    /// A Costco receipt for exactly what the card was charged.
    async fn costco(db: &mut toasty::Db, josh: Uuid, ash: Uuid) -> Uuid {
        toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 7, 8),
            merchant: "Costco",
            subtotal: dec("32.82"),
            tax: dec("2.54"),
            total: dec("35.36"),
            currency: "USD",
            image_path: "a.jpg",
            line_items: [
                { description: "Milk", total: dec("20.00"), position: 0, person_id: josh },
                { description: "Dog food", total: dec("12.82"), position: 1, person_id: ash },
            ],
        })
        .exec(db)
        .await
        .unwrap()
        .id
    }

    /// The whole path: import, propose, agree, and account for the rest.
    #[tokio::test]
    async fn a_statement_is_reconciled_one_charge_at_a_time() {
        let mut db = memory_db().await;
        let (josh, ash) = people(&mut db).await;
        let receipt_id = costco(&mut db, josh, ash).await;

        let parsed = parse::charges(CSV.as_bytes()).unwrap();
        let id = import(&mut db, "july.csv", "USD", &parsed).await.unwrap();

        let statement = load(&mut db, id).await.unwrap();
        let (proposed, netflix) = (&statement.charges[0], &statement.charges[1]);

        // Matched but not settled: the amount is only a proposal until somebody
        // agrees to it.
        assert!(matches!(proposed.resolution, dto::Resolution::Proposed(_)));
        assert!(matches!(netflix.resolution, dto::Resolution::Unresolved));
        assert_eq!(statement.settled(), 0);
        assert_eq!(statement.outstanding(), (2, dec("53.35")));

        // The split shows anyway — it's what you're agreeing to — with the tax
        // carried onto the people who incurred it. People come in name order, so
        // Ash first.
        let owed: Vec<_> = proposed.split.iter().map(|s| s.amount).collect();
        assert_eq!(owed, [dec("13.81"), dec("21.55")]);

        resolve(&mut db, proposed.id, dto::Resolve::Receipt(receipt_id))
            .await
            .unwrap();
        resolve(
            &mut db,
            netflix.id,
            dto::Resolve::NoReceipt {
                person_id: Some(ash),
            },
        )
        .await
        .unwrap();

        let statement = load(&mut db, id).await.unwrap();
        assert!(matches!(
            statement.charges[0].resolution,
            dto::Resolution::Confirmed(_)
        ));
        assert_eq!(statement.settled(), 2);
        assert_eq!(statement.outstanding(), (0, Decimal::ZERO));

        // Nothing is left over: the columns now account for the whole statement.
        let totals = statement.totals();
        assert_eq!(
            totals.iter().map(|t| t.amount).sum::<Decimal>(),
            statement.total()
        );
        assert_eq!(
            totals[0].amount,
            dec("31.80"),
            "Ash: 13.81 + all of Netflix"
        );
    }

    /// Two charges for the same amount is where guessing does damage, so only the
    /// first gets the receipt and neither is settled without a human.
    #[tokio::test]
    async fn a_receipt_can_only_account_for_one_charge() {
        let mut db = memory_db().await;
        let (josh, ash) = people(&mut db).await;
        let receipt_id = costco(&mut db, josh, ash).await;

        let twice = "Date,Description,Amount\n\
                     07/10/2026,COSTCO WHSE #1050,35.36\n\
                     07/10/2026,COSTCO WHSE #1050,35.36\n";
        let parsed = parse::charges(twice.as_bytes()).unwrap();
        let id = import(&mut db, "july.csv", "USD", &parsed).await.unwrap();

        let statement = load(&mut db, id).await.unwrap();
        assert!(matches!(
            statement.charges[0].resolution,
            dto::Resolution::Proposed(_)
        ));
        assert!(
            matches!(statement.charges[1].resolution, dto::Resolution::Unresolved),
            "the receipt was already spoken for"
        );
        assert!(
            statement.charges[1].suggestions.is_empty(),
            "and it isn't offered again"
        );

        // Attaching it by hand is refused too, rather than quietly leaving the
        // other charge settled against nothing.
        let err = resolve(
            &mut db,
            statement.charges[1].id,
            dto::Resolve::Receipt(receipt_id),
        )
        .await
        .unwrap_err();
        assert!(format!("{err}").contains("already accounts"), "{err}");
    }

    /// Nothing cascades, so a deleted receipt would otherwise leave a charge
    /// pointing at a row that isn't there.
    #[tokio::test]
    async fn deleting_a_receipt_puts_its_charge_back() {
        let mut db = memory_db().await;
        let (josh, ash) = people(&mut db).await;
        let receipt_id = costco(&mut db, josh, ash).await;

        let parsed = parse::charges(CSV.as_bytes()).unwrap();
        let id = import(&mut db, "july.csv", "USD", &parsed).await.unwrap();
        let charge_id = load(&mut db, id).await.unwrap().charges[0].id;
        resolve(&mut db, charge_id, dto::Resolve::Receipt(receipt_id))
            .await
            .unwrap();

        query::delete_receipt(&mut db, receipt_id).await.unwrap();

        let statement = load(&mut db, id).await.unwrap();
        assert!(matches!(
            statement.charges[0].resolution,
            dto::Resolution::Unresolved
        ));
        assert_eq!(statement.settled(), 0);
    }
}
