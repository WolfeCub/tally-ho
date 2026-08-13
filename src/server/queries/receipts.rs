//! Receipts, always with the line items that belong to them.

use crate::server::{mappers, models};
use crate::shared::dto;

/// Receipts with their line items, in the order they came in.
///
/// One query for the lot, grouped in memory. toasty has no join or `IN` loading,
/// so the obvious way to write this is a query per receipt — which the list
/// screen would then run a hundred of, every time it polls while something is
/// being read. Reading the line-item table whole is a few thousand rows against
/// a local SQLite file, and it doesn't grow with the size of the batch.
pub async fn with_items(
    db: &mut toasty::Db,
    receipts: Vec<models::Receipt>,
) -> toasty::Result<Vec<(models::Receipt, Vec<models::LineItem>)>> {
    if receipts.is_empty() {
        return Ok(Vec::new());
    }

    let mut items: std::collections::HashMap<uuid::Uuid, Vec<models::LineItem>> =
        std::collections::HashMap::new();
    for item in models::LineItem::all().exec(db).await? {
        items.entry(item.receipt_id).or_default().push(item);
    }

    Ok(receipts
        .into_iter()
        .map(|receipt| {
            let items = items.remove(&receipt.id).unwrap_or_default();
            (receipt, items)
        })
        .collect())
}

/// The newest receipts, each with its line items. For the list tab.
pub async fn recent(
    db: &mut toasty::Db,
    limit: usize,
) -> toasty::Result<Vec<(models::Receipt, Vec<models::LineItem>)>> {
    let receipts = models::Receipt::all()
        .order_by(models::Receipt::fields().purchased_on().desc())
        .limit(limit)
        .exec(db)
        .await?;

    with_items(db, receipts).await
}

/// Receipts purchased in an inclusive date range, each with its line items.
///
/// `purchased_on` is indexed and stored as ISO-8601 TEXT, which sorts
/// lexicographically, so `>=`/`<=` and `ORDER BY` are both correct on SQLite.
pub async fn load_range(
    db: &mut toasty::Db,
    from: jiff::civil::Date,
    to: jiff::civil::Date,
) -> toasty::Result<Vec<(models::Receipt, Vec<models::LineItem>)>> {
    let receipts = models::Receipt::filter(
        models::Receipt::fields()
            .purchased_on()
            .ge(from)
            .and(models::Receipt::fields().purchased_on().le(to)),
    )
    .order_by(models::Receipt::fields().purchased_on().asc())
    .exec(db)
    .await?;

    with_items(db, receipts).await
}

/// Receipts no charge accounts for, newest first. What the picker offers when
/// nothing was suggested — a receipt whose date was misread is nowhere near the
/// charge that paid for it.
pub async fn spare(db: &mut toasty::Db, limit: usize) -> toasty::Result<Vec<dto::ReceiptSummary>> {
    let taken = super::charges::spoken_for(db).await?;
    let free: Vec<_> = models::Receipt::all()
        .order_by(models::Receipt::fields().purchased_on().desc())
        .exec(db)
        .await?
        .into_iter()
        .filter(|receipt| !taken.contains(&receipt.id))
        .take(limit)
        .collect();

    Ok(with_items(db, free)
        .await?
        .iter()
        .map(|(receipt, items)| mappers::to_dto_summary(receipt, items))
        .collect())
}

/// A review-screen save, with every field already parsed.
///
/// Parsing happens before this is built so a typo in one box can't leave the
/// receipt half-written.
pub struct Save {
    pub merchant: String,
    pub purchased_on: jiff::civil::Date,
    pub currency: String,
    pub subtotal: Option<rust_decimal::Decimal>,
    pub tax: Option<rust_decimal::Decimal>,
    pub total: Option<rust_decimal::Decimal>,
    /// The items the receipt should end up with, in screen order.
    pub items: Vec<SaveItem>,
}

pub struct SaveItem {
    /// `None` for a row the human added.
    pub id: Option<uuid::Uuid>,
    pub description: String,
    pub total: rust_decimal::Decimal,
    pub person_id: Option<uuid::Uuid>,
}

/// Writes the review screen in one go: the header fields, plus the line items as
/// a complete replacement for whatever is stored.
///
/// One transaction — items dropped without the total that justified them reads
/// as a genuine receipt, and nothing would flag it.
pub async fn save(db: &mut toasty::Db, id: uuid::Uuid, save: Save) -> toasty::Result<()> {
    let mut tx = db.transaction().await?;

    let mut receipt = models::Receipt::get_by_id(&mut tx, &id).await?;
    let mut existing: std::collections::HashMap<_, _> = receipt
        .line_items()
        .exec(&mut tx)
        .await?
        .into_iter()
        .map(|item| (item.id, item))
        .collect();

    toasty::update!(receipt {
        merchant: save.merchant,
        purchased_on: save.purchased_on,
        currency: save.currency,
        subtotal: save.subtotal,
        tax: save.tax,
        total: save.total,
    })
    .exec(&mut tx)
    .await?;

    for (position, item) in save.items.into_iter().enumerate() {
        let position = position as i64;
        // Removing as we go, so what's left over at the end is what the human
        // deleted. An id that isn't there any more falls through to a create.
        match item.id.and_then(|id| existing.remove(&id)) {
            Some(mut row) => {
                // `edited` marks a row a human actually changed, so a later
                // re-extraction knows not to clobber it.
                let edited =
                    row.edited || row.description != item.description || row.total != item.total;
                // Picking somebody makes it your answer rather than the model's.
                let guessed_why = if row.person_id == item.person_id {
                    row.guessed_why.clone()
                } else {
                    None
                };
                toasty::update!(row {
                    description: item.description,
                    total: item.total,
                    position: position,
                    edited: edited,
                    person_id: item.person_id,
                    guessed_why: guessed_why,
                })
                .exec(&mut tx)
                .await?;
            }
            None => {
                toasty::create!(models::LineItem {
                    receipt_id: id,
                    description: item.description,
                    total: item.total,
                    position: position,
                    edited: true,
                    person_id: item.person_id,
                })
                .exec(&mut tx)
                .await?;
            }
        }
    }

    for row in existing.into_values() {
        row.delete().exec(&mut tx).await?;
    }

    tx.commit().await
}

/// Deletes a receipt and its line items, handing back the image path so the
/// caller can remove the photo once the rows are actually gone.
///
/// Nothing cascades, so the items go first.
pub async fn delete(db: &mut toasty::Db, id: uuid::Uuid) -> toasty::Result<String> {
    let mut tx = db.transaction().await?;

    let receipt = models::Receipt::get_by_id(&mut tx, &id).await?;
    let image_path = receipt.image_path.clone();

    for item in receipt.line_items().exec(&mut tx).await? {
        item.delete().exec(&mut tx).await?;
    }

    // A charge pointing at a receipt that isn't there would look settled against
    // nothing, so it goes back to unresolved in the same transaction.
    for mut charge in models::Charge::filter(models::Charge::fields().receipt_id().eq(id))
        .exec(&mut tx)
        .await?
    {
        toasty::update!(charge {
            receipt_id: None,
            confirmed: false
        })
        .exec(&mut tx)
        .await?;
    }

    receipt.delete().exec(&mut tx).await?;

    tx.commit().await?;
    Ok(image_path)
}

#[cfg(test)]
mod tests {
    use crate::server::mappers;
    use crate::server::models::{ExtractionStatus, LineItem, Receipt};
    use crate::server::testing::{dec, memory_db};

    /// Exercises the range matching searches through, against real SQLite: the
    /// ordering, the line-item fetch, and the problems that come with them.
    #[tokio::test]
    async fn a_date_range_loads_in_order_with_its_items() {
        let mut db = memory_db().await;

        // Inserted newest-first so a passing order assertion means `order_by`
        // did the work, not insertion order. Both are marked read, because the
        // problem checks are held back until a receipt has been: the default
        // status is `Pending`, which reports nothing by design.
        toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 7, 20),
            merchant: "Costco",
            status: ExtractionStatus::Done,
            subtotal: dec("30.00"),
            tax: dec("2.00"),
            total: dec("32.00"),
            currency: "USD",
            image_path: "b.jpg",
            line_items: [
                { description: "Milk", total: dec("10.00"), position: 0 },
                { description: "Eggs", total: dec("20.00"), position: 1 },
            ],
        })
        .exec(&mut db)
        .await
        .unwrap();

        // Read, but no total came out of it, so nothing can match it until a
        // human fixes it — one problem.
        toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 7, 2),
            merchant: "Unreadable",
            status: ExtractionStatus::Done,
            currency: "USD",
            image_path: "a.jpg",
        })
        .exec(&mut db)
        .await
        .unwrap();

        // Outside the range, and with items of its own: they are loaded in the
        // same sweep as everyone else's, so this is what catches them being
        // handed to the wrong receipt.
        toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 8, 1),
            merchant: "Next month",
            total: dec("999.00"),
            currency: "USD",
            image_path: "c.jpg",
            line_items: [{ description: "Not ours", total: dec("999.00"), position: 0 }],
        })
        .exec(&mut db)
        .await
        .unwrap();

        let from = jiff::civil::date(2026, 7, 1);
        let to = jiff::civil::date(2026, 7, 31);
        let rows = super::load_range(&mut db, from, to).await.unwrap();

        let merchants: Vec<_> = rows.iter().map(|(r, _)| r.merchant.as_str()).collect();
        assert_eq!(merchants, ["Unreadable", "Costco"], "ascending by date");
        assert_eq!(rows[0].1.len(), 0);
        assert_eq!(rows[1].1.len(), 2);

        let summaries: Vec<_> = rows
            .iter()
            .map(|(r, items)| mappers::to_dto_summary(r, items))
            .collect();

        // Costco balances (30 + 2 = 32, items = 30); the other is only missing a
        // total, so exactly one problem each way.
        assert!(
            summaries[1].problems.is_empty(),
            "{:?}",
            summaries[1].problems
        );
        assert_eq!(summaries[0].problems.len(), 1);
    }

    /// The line items are the part that can be left behind: nothing cascades,
    /// and an orphan would still be counted by anything loading items directly.
    #[tokio::test]
    async fn deleting_a_receipt_takes_its_line_items_with_it() {
        let mut db = memory_db().await;

        let doomed = toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 7, 20),
            merchant: "Duplicate",
            total: dec("32.00"),
            currency: "USD",
            image_path: "images/2026/07/doomed.jpg",
            line_items: [
                { description: "Milk", total: dec("10.00"), position: 0 },
                { description: "Eggs", total: dec("22.00"), position: 1 },
            ],
        })
        .exec(&mut db)
        .await
        .unwrap();

        // A second receipt, to catch a delete that is too enthusiastic.
        toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 7, 21),
            merchant: "Keeper",
            total: dec("5.00"),
            currency: "USD",
            image_path: "images/2026/07/keeper.jpg",
            line_items: [{ description: "Coffee", total: dec("5.00"), position: 0 }],
        })
        .exec(&mut db)
        .await
        .unwrap();

        let image_path = super::delete(&mut db, doomed.id).await.unwrap();
        assert_eq!(
            image_path, "images/2026/07/doomed.jpg",
            "for the file delete"
        );

        assert!(
            Receipt::get_by_id(&mut db, &doomed.id).await.is_err(),
            "receipt still there"
        );

        let items = LineItem::filter(LineItem::fields().receipt_id().eq(doomed.id))
            .exec(&mut db)
            .await
            .unwrap();
        assert!(items.is_empty(), "orphaned line items: {}", items.len());

        let survivors = super::load_range(
            &mut db,
            jiff::civil::date(2026, 7, 1),
            jiff::civil::date(2026, 7, 31),
        )
        .await
        .unwrap();
        assert_eq!(survivors.len(), 1);
        assert_eq!(survivors[0].0.merchant, "Keeper");
        assert_eq!(survivors[0].1.len(), 1);
    }

    /// One save has to update, add and delete line items at once — the review
    /// screen sends the list it ended up with, not a diff.
    #[tokio::test]
    async fn saving_a_receipt_replaces_its_line_items() {
        let mut db = memory_db().await;

        let receipt = toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 7, 20),
            merchant: "Cotsco",
            total: dec("32.00"),
            currency: "usd",
            image_path: "a.jpg",
            line_items: [
                { description: "Milk", total: dec("10.00"), position: 0 },
                { description: "Eggs", total: dec("20.00"), position: 1 },
                { description: "Misread", total: dec("99.00"), position: 2 },
            ],
        })
        .exec(&mut db)
        .await
        .unwrap();

        let before = receipt.line_items().exec(&mut db).await.unwrap();
        let milk = before.iter().find(|i| i.description == "Milk").unwrap();
        let eggs = before.iter().find(|i| i.description == "Eggs").unwrap();

        super::save(
            &mut db,
            receipt.id,
            super::Save {
                merchant: "Costco".into(),
                purchased_on: jiff::civil::date(2026, 7, 21),
                currency: "USD".into(),
                subtotal: Some(dec("30.00")),
                tax: Some(dec("2.50")),
                total: Some(dec("32.50")),
                items: vec![
                    // Untouched.
                    super::SaveItem {
                        id: Some(milk.id),
                        description: "Milk".into(),
                        total: dec("10.00"),
                        person_id: None,
                    },
                    // Corrected.
                    super::SaveItem {
                        id: Some(eggs.id),
                        description: "Eggs".into(),
                        total: dec("12.00"),
                        person_id: None,
                    },
                    // Added on screen. "Misread" is absent, so it goes.
                    super::SaveItem {
                        id: None,
                        description: "Bread".into(),
                        total: dec("8.00"),
                        person_id: None,
                    },
                ],
            },
        )
        .await
        .unwrap();

        let receipt = Receipt::get_by_id(&mut db, &receipt.id).await.unwrap();
        assert_eq!(receipt.merchant, "Costco");
        assert_eq!(receipt.purchased_on, jiff::civil::date(2026, 7, 21));
        assert_eq!(receipt.total, Some(dec("32.50")));

        let mut items = receipt.line_items().exec(&mut db).await.unwrap();
        items.sort_by_key(|i| i.position);
        let got: Vec<_> = items
            .iter()
            .map(|i| (i.description.as_str(), i.total, i.edited))
            .collect();
        assert_eq!(
            got,
            [
                ("Milk", dec("10.00"), false),
                ("Eggs", dec("12.00"), true),
                ("Bread", dec("8.00"), true),
            ],
            "only the rows a human changed are marked edited"
        );

        // Dropping a row has to take it out of the table, not just the screen —
        // anything loading items directly would still count an orphan.
        let all = LineItem::filter(LineItem::fields().receipt_id().eq(receipt.id))
            .exec(&mut db)
            .await
            .unwrap();
        assert_eq!(all.len(), 3, "the misread row is gone");
    }

    /// A guess is only a guess until somebody says otherwise, and saving is how
    /// they say it, including saying "actually, nobody's".
    #[tokio::test]
    async fn picking_somebody_yourself_settles_a_guess() {
        use crate::server::models::Person;

        let mut db = memory_db().await;

        let josh = toasty::create!(Person { name: "Josh" })
            .exec(&mut db)
            .await
            .unwrap();
        let ash = toasty::create!(Person { name: "Ash" })
            .exec(&mut db)
            .await
            .unwrap();

        let receipt = toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 7, 20),
            merchant: "Costco",
            total: dec("30.00"),
            currency: "USD",
            image_path: "a.jpg",
            line_items: [
                { description: "Beer", total: dec("18.00"), position: 0, person_id: josh.id, guessed_why: "he drinks it" },
                { description: "Steak", total: dec("8.00"), position: 1, person_id: josh.id, guessed_why: "Ash is vegetarian" },
                { description: "Milk", total: dec("4.00"), position: 2, person_id: josh.id, guessed_why: "a guess" },
            ],
        })
        .exec(&mut db)
        .await
        .unwrap();

        let mut before = receipt.line_items().exec(&mut db).await.unwrap();
        before.sort_by_key(|item| item.position);
        let save = |item: &LineItem, person_id| super::SaveItem {
            id: Some(item.id),
            description: item.description.clone(),
            total: item.total,
            person_id,
        };

        super::save(
            &mut db,
            receipt.id,
            super::Save {
                merchant: "Costco".into(),
                purchased_on: jiff::civil::date(2026, 7, 20),
                currency: "USD".into(),
                subtotal: None,
                tax: None,
                total: Some(dec("30.00")),
                items: vec![
                    // Left alone, so still the model's word.
                    save(&before[0], Some(josh.id)),
                    // Corrected.
                    save(&before[1], Some(ash.id)),
                    // Handed back to the even split, which is an answer too.
                    save(&before[2], None),
                ],
            },
        )
        .await
        .unwrap();

        let mut after = receipt.line_items().exec(&mut db).await.unwrap();
        after.sort_by_key(|item| item.position);
        let got: Vec<_> = after
            .iter()
            .map(|item| (item.person_id, item.guessed_why.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                (Some(josh.id), Some("he drinks it")),
                (Some(ash.id), None),
                (None, None),
            ]
        );
    }

    /// Assignments survive a save, and dropping someone hands their items back
    /// rather than leaving them pointed at a person who no longer exists.
    #[tokio::test]
    async fn removing_a_person_unassigns_their_items() {
        use crate::server::models::Person;
        use crate::server::queries::people;

        let mut db = memory_db().await;

        let josh = toasty::create!(Person { name: "Josh" })
            .exec(&mut db)
            .await
            .unwrap();
        let ash = toasty::create!(Person {
            name: "Ash",
            description: "the other card",
        })
        .exec(&mut db)
        .await
        .unwrap();

        let receipt = toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 7, 20),
            merchant: "Costco",
            total: dec("30.00"),
            currency: "USD",
            image_path: "a.jpg",
            line_items: [
                { description: "Milk", total: dec("10.00"), position: 0 },
                { description: "Dog food", total: dec("20.00"), position: 1 },
            ],
        })
        .exec(&mut db)
        .await
        .unwrap();

        let before = receipt.line_items().exec(&mut db).await.unwrap();
        let milk = before.iter().find(|i| i.description == "Milk").unwrap();
        let food = before.iter().find(|i| i.description == "Dog food").unwrap();

        super::save(
            &mut db,
            receipt.id,
            super::Save {
                merchant: "Costco".into(),
                purchased_on: jiff::civil::date(2026, 7, 20),
                currency: "USD".into(),
                subtotal: None,
                tax: None,
                total: Some(dec("30.00")),
                items: vec![
                    super::SaveItem {
                        id: Some(milk.id),
                        description: "Milk".into(),
                        total: dec("10.00"),
                        person_id: Some(josh.id),
                    },
                    super::SaveItem {
                        id: Some(food.id),
                        description: "Dog food".into(),
                        total: dec("20.00"),
                        person_id: Some(ash.id),
                    },
                ],
            },
        )
        .await
        .unwrap();

        let assigned = |items: &[LineItem], description: &str| {
            items
                .iter()
                .find(|i| i.description == description)
                .unwrap()
                .person_id
        };

        let items = receipt.line_items().exec(&mut db).await.unwrap();
        assert_eq!(assigned(&items, "Milk"), Some(josh.id));
        assert_eq!(assigned(&items, "Dog food"), Some(ash.id));

        // Josh is absent from the save, so he goes.
        people::save(
            &mut db,
            vec![people::Save {
                id: Some(ash.id),
                name: "Ash".into(),
                description: None,
            }],
        )
        .await
        .unwrap();

        let items = receipt.line_items().exec(&mut db).await.unwrap();
        assert_eq!(assigned(&items, "Milk"), None, "back to unassigned");
        assert_eq!(assigned(&items, "Dog food"), Some(ash.id), "left alone");
        assert!(Person::get_by_id(&mut db, &josh.id).await.is_err());
    }
}
