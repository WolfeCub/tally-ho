//! Importing a statement, reading one back with its matches, and the summaries
//! the list screen shows. The charges are next door in [`super::charges`].

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use rust_decimal::Decimal;
use uuid::Uuid;

use crate::server::matching::merchant;
use crate::server::{mappers, matching, models, statement_csv};
use crate::shared::dto;
use crate::shared::reconcile::{charge_to, split_charge};
use crate::shared::refund::split_refund;

/// How far either side of a statement's own dates to look for receipts. Wider
/// than the match window, so a receipt right at the edge still gets offered.
const SLACK: i32 = 7;

/// How far back a refund may reach for the purchase it came off. Return windows
/// run to a couple of months and the refund posts after that.
const REACH: i32 = 180;

/// Every statement imported, newest first.
pub async fn list(db: &mut toasty::Db) -> toasty::Result<Vec<dto::StatementSummary>> {
    let statements = models::Statement::all()
        .order_by(models::Statement::fields().imported_at().desc())
        .exec(db)
        .await?;

    // Nothing to hang charges off, so don't read the charge table at all.
    if statements.is_empty() {
        return Ok(Vec::new());
    }

    let all_charges = models::Charge::all().exec(db).await?;
    let mut charges = super::group_by(all_charges, |charge| charge.statement_id);

    Ok(statements
        .into_iter()
        .map(|statement| {
            let charges = charges.remove(&statement.id).unwrap_or_default();
            dto::StatementSummary {
                id: statement.id,
                label: statement.label,
                begins_on: statement.begins_on,
                ends_on: statement.ends_on,
                currency: statement.currency,
                charge_count: charges.len(),
                settled_count: charges
                    .iter()
                    .filter(|c| super::charges::settled(c))
                    .count(),
            }
        })
        .collect())
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
    parsed: &statement_csv::Parsed,
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
            &charge.description,
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
    let people = super::people::list(db).await?;
    let free = pool(db, statement.begins_on, statement.ends_on).await?;

    // A refund points at the purchase it came off, which is usually on the
    // statement before this one — so the whole card is needed, both to look that
    // purchase up and to find the refunds pointing back at these charges.
    let card = super::charges::by_id(db).await?;

    // A matched receipt is deliberately absent from `free`, and may be outside
    // the window anyway if it was attached by hand — so they're fetched here,
    // all at once. One query each would be two per charge, on a screen that
    // reloads the whole statement after every decision. The purchases behind any
    // refunds come along too: their items are what the refund is matched to.
    let wanted: Vec<Uuid> = rows
        .iter()
        .flat_map(|row| {
            let purchase = row
                .refunds_charge_id
                .and_then(|id| card.get(&id)?.receipt_id);
            [row.receipt_id, purchase]
        })
        .flatten()
        .collect();
    let matched_receipts = super::receipts::by_id(db, wanted).await?;

    let items_of = |charge: &models::Charge| {
        charge
            .receipt_id
            .and_then(|id| matched_receipts.get(&id))
            .map(|receipt| receipt.line_items.as_slice())
            .unwrap_or_default()
    };
    let split_of = |charge: &models::Charge| match charge.receipt_id {
        Some(_) => split_charge(charge.amount, currency, items_of(charge), &people),
        None if charge.no_receipt => charge_to(charge.amount, currency, charge.person_id, &people),
        None => Vec::new(),
    };

    let mut charges = Vec::with_capacity(rows.len());
    for row in &rows {
        let refunded = row.refunds_charge_id.and_then(|id| card.get(&id));
        let matched = row.receipt_id.and_then(|id| matched_receipts.get(&id));

        let (resolution, split) = match (refunded, matched) {
            (Some(purchase), _) => (
                dto::Resolution::Refund(dto::Refunded {
                    charged_on: purchase.charged_on,
                    description: purchase.description.clone(),
                    amount: purchase.amount,
                }),
                split_refund(
                    row.amount,
                    currency,
                    purchase.amount,
                    items_of(purchase),
                    &split_of(purchase),
                    &people,
                ),
            ),
            (None, Some(receipt)) => {
                let matched = to_matched(receipt);
                let resolution = if row.confirmed {
                    dto::Resolution::Confirmed(matched)
                } else {
                    dto::Resolution::Proposed(matched)
                };
                (resolution, split_of(row))
            }
            (None, None) if row.no_receipt => (
                dto::Resolution::NoReceipt {
                    person_id: row.person_id,
                },
                split_of(row),
            ),
            (None, None) => (dto::Resolution::Unresolved, Vec::new()),
        };

        let unresolved = matches!(resolution, dto::Resolution::Unresolved);
        charges.push(dto::Charge {
            id: row.id,
            charged_on: row.charged_on,
            description: row.description.clone(),
            amount: row.amount,
            // Nothing to suggest once something accounts for it; the screen
            // offers a way back to unresolved instead.
            suggestions: if unresolved {
                matching::candidates(
                    row.charged_on,
                    &row.description,
                    row.amount,
                    currency,
                    &free,
                )
            } else {
                Vec::new()
            },
            // Money coming back is the only thing a purchase can account for.
            refundable: if unresolved && row.amount.is_sign_negative() {
                refundable(row, &card)
            } else {
                Vec::new()
            },
            came_back: came_back(row.id, &card),
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

/// Whether a receipt any of these charges points at is still being read.
///
/// The reconcile screen's poll target, and two queries rather than the whole
/// statement — which is a receipt and its line items per charge, and would
/// rebuild every row on screen each time it landed.
pub async fn reading(db: &mut toasty::Db, id: Uuid) -> anyhow::Result<bool> {
    let statement = models::Statement::get_by_id(db, &id).await?;
    let matched: HashSet<Uuid> = statement
        .charges()
        .exec(db)
        .await?
        .into_iter()
        .filter_map(|charge| charge.receipt_id)
        .collect();

    // Every receipt rather than one query each: toasty has no `IN`, and there
    // are a few hundred rows.
    Ok(models::Receipt::all()
        .exec(db)
        .await?
        .into_iter()
        .any(|r| matched.contains(&r.id) && !mappers::to_dto_status(&r.status).is_terminal()))
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

/// The purchases a refund might have come off, best first.
///
/// The merchant leads: a refund line names the shop that gave the money back,
/// and there are usually several charges of the same size on a card. The amount
/// then picks between that shop's charges. Both are on screen in the option
/// itself, so neither needs saying twice.
fn refundable(
    refund: &models::Charge,
    card: &HashMap<Uuid, models::Charge>,
) -> Vec<dto::Refundable> {
    let line = merchant::Line::new(&refund.description);
    let back = -refund.amount;

    let mut found: Vec<_> = card
        .values()
        // More back than went out is a different purchase, whatever else fits.
        .filter(|purchase| purchase.amount >= back)
        .filter_map(|purchase| {
            let days = (refund.charged_on - purchase.charged_on).get_days();
            let rank = (
                Reverse(line.names(&purchase.description)),
                Reverse(purchase.amount == back),
                days,
            );
            (0..=REACH).contains(&days).then_some((rank, purchase))
        })
        .collect();

    found.sort_by_key(|(rank, _)| *rank);
    found
        .into_iter()
        .map(|(_, purchase)| dto::Refundable {
            charge_id: purchase.id,
            charged_on: purchase.charged_on,
            description: purchase.description.clone(),
            amount: purchase.amount,
        })
        .collect()
}

/// What the refunds pointing at a charge add up to, so money that came back off
/// it is visible from the row it came off.
fn came_back(charge_id: Uuid, card: &HashMap<Uuid, models::Charge>) -> Decimal {
    card.values()
        .filter(|refund| refund.refunds_charge_id == Some(charge_id))
        .map(|refund| refund.amount)
        .sum()
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
///
/// Public for `match_probe`, which dry-runs a statement against the real
/// database and has to see the same pool an import would.
pub async fn pool(
    db: &mut toasty::Db,
    begins_on: jiff::civil::Date,
    ends_on: jiff::civil::Date,
) -> toasty::Result<Vec<dto::Receipt>> {
    let slack = jiff::Span::new().days(SLACK);
    let begins_on = begins_on.checked_sub(slack).unwrap_or(begins_on);
    let ends_on = ends_on.checked_add(slack).unwrap_or(ends_on);

    let taken = super::charges::spoken_for(db).await?;
    Ok(super::receipts::load_range(db, begins_on, ends_on)
        .await?
        .iter()
        .map(|(receipt, items)| mappers::to_dto_receipt(receipt, items))
        .filter(|receipt| !taken.contains(&receipt.id))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::models::{Person, Receipt};
    use crate::server::queries::charges::resolve;
    use crate::server::queries::receipts;
    use crate::server::testing::memory_db;
    use crate::shared::testing::dec;
    use rust_decimal::Decimal;

    /// A charge a receipt explains, and one nothing does.
    const CSV: &str = "Transaction Date,Description,Amount\n\
                       07/10/2026,COSTCO WHSE #1050,35.36\n\
                       07/11/2026,NETFLIX.COM,17.99\n";

    /// Reads a file and imports it, which is what the API does with an upload.
    async fn imported(db: &mut toasty::Db, label: &str, csv: &str) -> Uuid {
        let parsed = statement_csv::charges(csv.as_bytes()).unwrap();
        import(db, label, "USD", &parsed).await.unwrap()
    }

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

    /// The counts come off one query over every charge on the card, so this is
    /// what catches a statement being credited with another one's charges.
    #[tokio::test]
    async fn the_list_counts_each_statements_own_charges() {
        let mut db = memory_db().await;
        let (josh, ash) = people(&mut db).await;
        let receipt_id = costco(&mut db, josh, ash).await;

        let july = imported(&mut db, "july.csv", CSV).await;
        let august = "Date,Description,Amount\n08/02/2026,SPOTIFY,11.99\n";
        imported(&mut db, "august.csv", august).await;

        let charge_id = load(&mut db, july).await.unwrap().charges[0].id;
        resolve(&mut db, charge_id, dto::Resolve::Receipt(receipt_id))
            .await
            .unwrap();

        // Newest first, so August leads.
        let rows = list(&mut db).await.unwrap();
        let counts: Vec<_> = rows
            .iter()
            .map(|s| (s.label.as_str(), s.charge_count, s.settled_count))
            .collect();
        assert_eq!(counts, [("august.csv", 1, 0), ("july.csv", 2, 1)]);
    }

    /// The whole path: import, propose, agree, and account for the rest.
    #[tokio::test]
    async fn a_statement_is_reconciled_one_charge_at_a_time() {
        let mut db = memory_db().await;
        let (josh, ash) = people(&mut db).await;
        let receipt_id = costco(&mut db, josh, ash).await;

        let id = imported(&mut db, "july.csv", CSV).await;

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

    /// A refund posts on the statement after the one it came off, and only that
    /// purchase's receipt says whose the money was.
    #[tokio::test]
    async fn a_refund_follows_the_purchase_it_came_off() {
        let mut db = memory_db().await;
        let (josh, ash) = people(&mut db).await;
        let receipt_id = costco(&mut db, josh, ash).await;

        let july = imported(&mut db, "july.csv", CSV).await;
        let august = imported(
            &mut db,
            "august.csv",
            "Date,Description,Amount\n08/02/2026,COSTCO WHSE #1050 RETURN,-13.81\n",
        )
        .await;

        let purchase = load(&mut db, july).await.unwrap().charges[0].id;
        resolve(&mut db, purchase, dto::Resolve::Receipt(receipt_id))
            .await
            .unwrap();

        // No receipt of its own to match, but the purchases are offered — the
        // one the line names first.
        let statement = load(&mut db, august).await.unwrap();
        let refund = &statement.charges[0];
        assert!(refund.suggestions.is_empty());
        let offered: Vec<_> = refund
            .refundable
            .iter()
            .map(|r| r.description.as_str())
            .collect();
        assert_eq!(offered, ["COSTCO WHSE #1050", "NETFLIX.COM"]);

        resolve(&mut db, refund.id, dto::Resolve::Refunds(purchase))
            .await
            .unwrap();

        let statement = load(&mut db, august).await.unwrap();
        let refund = &statement.charges[0];
        assert!(matches!(refund.resolution, dto::Resolution::Refund(_)));
        assert_eq!(statement.settled(), 1, "which accounts for it");
        assert_eq!(statement.outstanding(), (0, Decimal::ZERO));

        // 13.81 is the 12.82 dog food plus the tax it carried, and the dog food
        // was Ash's. People come in name order, so Ash leads.
        let owed: Vec<_> = refund.split.iter().map(|s| s.amount).collect();
        assert_eq!(owed, [dec("-13.81"), dec("0.00")]);
        assert_eq!(owed.iter().sum::<Decimal>(), statement.total());

        // And it shows on the purchase, which is the only place you'd see it —
        // the refund itself is on a statement you aren't looking at.
        let july = load(&mut db, july).await.unwrap();
        assert_eq!(july.charges[0].came_back, dec("-13.81"));
    }

    /// Refunds point one way. Both of these would otherwise settle a row against
    /// something that never paid for it.
    #[tokio::test]
    async fn only_money_coming_back_can_refund_a_purchase() {
        let mut db = memory_db().await;
        people(&mut db).await;

        let id = imported(&mut db, "july.csv", CSV).await;
        let charges = load(&mut db, id).await.unwrap().charges;
        let (costco, netflix) = (charges[0].id, charges[1].id);

        // Netflix is a purchase, not money back.
        let err = resolve(&mut db, netflix, dto::Resolve::Refunds(costco))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("only money coming back"), "{err}");

        let err = resolve(&mut db, costco, dto::Resolve::Refunds(costco))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("only money coming back"), "{err}");
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
        let id = imported(&mut db, "july.csv", twice).await;

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

    /// The reconcile screen stops asking when this says so, so it has to answer
    /// for the receipts its own charges point at, and only those.
    #[tokio::test]
    async fn reading_follows_the_receipts_these_charges_point_at() {
        use crate::server::models::ExtractionStatus;

        let mut db = memory_db().await;
        let (josh, ash) = people(&mut db).await;
        let receipt_id = costco(&mut db, josh, ash).await;

        let id = imported(&mut db, "july.csv", CSV).await;

        // Created without a status, which is `Pending`: still being read, and
        // proposed against the Costco charge.
        assert!(reading(&mut db, id).await.unwrap());

        let mut matched = Receipt::get_by_id(&mut db, &receipt_id).await.unwrap();
        toasty::update!(matched {
            status: ExtractionStatus::Done
        })
        .exec(&mut db)
        .await
        .unwrap();
        assert!(!reading(&mut db, id).await.unwrap());

        // Somebody else's receipt, mid-extraction, accounts for none of these
        // charges — and must not keep this statement polling for ever.
        toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 7, 9),
            merchant: "Elsewhere",
            currency: "USD",
            image_path: "b.jpg",
            status: ExtractionStatus::Extracting,
        })
        .exec(&mut db)
        .await
        .unwrap();
        assert!(
            !reading(&mut db, id).await.unwrap(),
            "not on this statement"
        );
    }

    /// Nothing cascades, so a deleted receipt would otherwise leave a charge
    /// pointing at a row that isn't there.
    #[tokio::test]
    async fn deleting_a_receipt_puts_its_charge_back() {
        let mut db = memory_db().await;
        let (josh, ash) = people(&mut db).await;
        let receipt_id = costco(&mut db, josh, ash).await;

        let id = imported(&mut db, "july.csv", CSV).await;
        let charge_id = load(&mut db, id).await.unwrap().charges[0].id;
        resolve(&mut db, charge_id, dto::Resolve::Receipt(receipt_id))
            .await
            .unwrap();

        receipts::delete(&mut db, receipt_id).await.unwrap();

        let statement = load(&mut db, id).await.unwrap();
        assert!(matches!(
            statement.charges[0].resolution,
            dto::Resolution::Unresolved
        ));
        assert_eq!(statement.settled(), 0);
    }
}
