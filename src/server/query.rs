//! Queries that touch more than one row: the statement period, which both the
//! period view and the CSV export go through, and throwing a receipt away.

use crate::server::models;
use crate::shared::dto;

/// Fills in either end of a requested statement period.
///
/// Both ends are optional so the client can ask for "the usual" without owning a
/// clock, and the response echoes back which dates were actually used.
pub fn resolve_range(
    from: Option<jiff::civil::Date>,
    to: Option<jiff::civil::Date>,
) -> (jiff::civil::Date, jiff::civil::Date) {
    let (default_from, default_to) = dto::last_full_month(jiff::Zoned::now().date());
    (from.unwrap_or(default_from), to.unwrap_or(default_to))
}

/// Receipts purchased in an inclusive date range, each with its line items.
///
/// `purchased_on` is indexed and stored as ISO-8601 TEXT, which sorts
/// lexicographically, so `>=`/`<=` and `ORDER BY` are both correct on SQLite.
///
/// The line items are fetched per receipt — one query each. toasty has no join
/// or `IN` loading, and a statement period holds tens of receipts against a
/// local SQLite file, so the N+1 is cheaper than the raw-SQL escape hatch it
/// would take to avoid.
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

    let mut out = Vec::with_capacity(receipts.len());
    for receipt in receipts {
        let items = receipt.line_items().exec(db).await?;
        out.push((receipt, items));
    }
    Ok(out)
}

/// A review-screen save, with every field already parsed.
///
/// Parsing happens before this is built so a typo in one box can't leave the
/// receipt half-written.
pub struct ReceiptSave {
    pub merchant: String,
    pub purchased_on: jiff::civil::Date,
    pub currency: String,
    pub subtotal: Option<rust_decimal::Decimal>,
    pub tax: Option<rust_decimal::Decimal>,
    pub total: Option<rust_decimal::Decimal>,
    /// The items the receipt should end up with, in screen order.
    pub items: Vec<ItemSave>,
}

pub struct ItemSave {
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
pub async fn save_receipt(
    db: &mut toasty::Db,
    id: uuid::Uuid,
    save: ReceiptSave,
) -> toasty::Result<()> {
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
                toasty::update!(row {
                    description: item.description,
                    total: item.total,
                    position: position,
                    edited: edited,
                    person_id: item.person_id,
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
pub async fn delete_receipt(db: &mut toasty::Db, id: uuid::Uuid) -> toasty::Result<String> {
    let mut tx = db.transaction().await?;

    let receipt = models::Receipt::get_by_id(&mut tx, &id).await?;
    let image_path = receipt.image_path.clone();

    for item in receipt.line_items().exec(&mut tx).await? {
        item.delete().exec(&mut tx).await?;
    }
    receipt.delete().exec(&mut tx).await?;

    tx.commit().await?;
    Ok(image_path)
}

/// A person as the settings screen left them, already parsed.
pub struct PersonSave {
    /// `None` for someone the human added.
    pub id: Option<uuid::Uuid>,
    pub name: String,
    pub description: Option<String>,
}

/// Writes the settings screen in one go: everyone it ended up with, as a
/// complete replacement for whoever is stored.
///
/// One transaction — a half-applied save could leave items charged to somebody
/// who was meant to be gone.
pub async fn save_people(db: &mut toasty::Db, people: Vec<PersonSave>) -> toasty::Result<()> {
    let mut tx = db.transaction().await?;

    let mut existing: std::collections::HashMap<_, _> = models::Person::all()
        .exec(&mut tx)
        .await?
        .into_iter()
        .map(|person| (person.id, person))
        .collect();

    for person in people {
        // Removing as we go, so what's left over is who the human removed. An
        // id that isn't there any more falls through to a create.
        match person.id.and_then(|id| existing.remove(&id)) {
            Some(mut row) => {
                toasty::update!(row {
                    name: person.name,
                    description: person.description,
                })
                .exec(&mut tx)
                .await?;
            }
            None => {
                toasty::create!(models::Person {
                    name: person.name,
                    description: person.description,
                })
                .exec(&mut tx)
                .await?;
            }
        }
    }

    for person in existing.into_values() {
        // Nothing cascades, and an item pointing at somebody who isn't there
        // would be neither assigned nor unassigned — it would drop out of both
        // halves of the split.
        for mut item in
            models::LineItem::filter(models::LineItem::fields().person_id().eq(person.id))
                .exec(&mut tx)
                .await?
        {
            toasty::update!(item { person_id: None })
                .exec(&mut tx)
                .await?;
        }
        person.delete().exec(&mut tx).await?;
    }

    tx.commit().await
}

#[cfg(test)]
mod tests {
    use crate::server::mappers;
    use crate::server::models::Receipt;
    use crate::shared::dto;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    /// Exercises the range query against real SQLite: the ordering, the line-item
    /// fetch, and the fold into a period figure.
    #[tokio::test]
    async fn a_period_loads_in_order_with_its_items_and_totals() {
        let mut db = crate::server::db::connect_url("sqlite::memory:")
            .await
            .unwrap();

        // Inserted newest-first so a passing order assertion means `order_by`
        // did the work, not insertion order.
        toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 7, 20),
            merchant: "Costco",
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

        // No total: must make the period partial rather than contributing zero.
        toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 7, 2),
            merchant: "Unreadable",
            currency: "USD",
            image_path: "a.jpg",
        })
        .exec(&mut db)
        .await
        .unwrap();

        // Outside the period.
        toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 8, 1),
            merchant: "Next month",
            total: dec("999.00"),
            currency: "USD",
            image_path: "c.jpg",
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

        let summaries = rows
            .iter()
            .map(|(r, items)| mappers::to_dto_summary(r, items))
            .collect();
        let period = dto::PeriodSummary::new(from, to, summaries);

        assert_eq!(period.totals.len(), 1, "both receipts are USD");
        assert_eq!(period.totals[0].currency, "USD");
        assert_eq!(period.totals[0].total.known(), dec("32.00"));
        assert_eq!(
            period.totals[0].total.missing(),
            1,
            "the unreadable receipt"
        );
        assert!(!period.totals[0].total.is_complete());

        // Costco balances (30 + 2 = 32, items = 30); the other is only missing a
        // total, so exactly one problem each way.
        assert!(
            period.receipts[1].problems.is_empty(),
            "{:?}",
            period.receipts[1].problems
        );
        assert_eq!(period.receipts[0].problems.len(), 1);
        assert_eq!(period.needing_attention(), 1);
    }

    /// The line items are the part that can be left behind: nothing cascades,
    /// and an orphan would still be counted by anything loading items directly.
    #[tokio::test]
    async fn deleting_a_receipt_takes_its_line_items_with_it() {
        use crate::server::models::LineItem;

        let mut db = crate::server::db::connect_url("sqlite::memory:")
            .await
            .unwrap();

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

        let image_path = super::delete_receipt(&mut db, doomed.id).await.unwrap();
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
        use crate::server::models::LineItem;

        let mut db = crate::server::db::connect_url("sqlite::memory:")
            .await
            .unwrap();

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

        super::save_receipt(
            &mut db,
            receipt.id,
            super::ReceiptSave {
                merchant: "Costco".into(),
                purchased_on: jiff::civil::date(2026, 7, 21),
                currency: "USD".into(),
                subtotal: Some(dec("30.00")),
                tax: Some(dec("2.50")),
                total: Some(dec("32.50")),
                items: vec![
                    // Untouched.
                    super::ItemSave {
                        id: Some(milk.id),
                        description: "Milk".into(),
                        total: dec("10.00"),
                        person_id: None,
                    },
                    // Corrected.
                    super::ItemSave {
                        id: Some(eggs.id),
                        description: "Eggs".into(),
                        total: dec("12.00"),
                        person_id: None,
                    },
                    // Added on screen. "Misread" is absent, so it goes.
                    super::ItemSave {
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

    /// One save has to rename, add and remove people at once — the settings
    /// screen sends the list it ended up with, not a diff.
    #[tokio::test]
    async fn saving_people_replaces_the_whole_list() {
        use crate::server::models::Person;

        let mut db = crate::server::db::connect_url("sqlite::memory:")
            .await
            .unwrap();

        let josh = toasty::create!(Person { name: "Josh" })
            .exec(&mut db)
            .await
            .unwrap();
        let typo = toasty::create!(Person { name: "Asj" })
            .exec(&mut db)
            .await
            .unwrap();

        super::save_people(
            &mut db,
            vec![
                // Untouched.
                super::PersonSave {
                    id: Some(josh.id),
                    name: "Josh".into(),
                    description: None,
                },
                // Corrected, and described.
                super::PersonSave {
                    id: Some(typo.id),
                    name: "Ash".into(),
                    description: Some("the other card".into()),
                },
                // Added on screen.
                super::PersonSave {
                    id: None,
                    name: "Guest".into(),
                    description: None,
                },
            ],
        )
        .await
        .unwrap();

        let mut people = Person::all().exec(&mut db).await.unwrap();
        people.sort_by(|a, b| a.name.cmp(&b.name));
        let got: Vec<_> = people
            .iter()
            .map(|p| (p.name.as_str(), p.description.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("Ash", Some("the other card")),
                ("Guest", None),
                ("Josh", None),
            ]
        );
        // Renaming edits the row rather than replacing it, so anything charged
        // to them stays charged to them.
        assert_eq!(people[0].id, typo.id);
    }

    /// Assignments survive a save, and dropping someone hands their items back
    /// rather than leaving them pointed at a person who no longer exists.
    #[tokio::test]
    async fn removing_a_person_unassigns_their_items() {
        use crate::server::models::Person;

        let mut db = crate::server::db::connect_url("sqlite::memory:")
            .await
            .unwrap();

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

        super::save_receipt(
            &mut db,
            receipt.id,
            super::ReceiptSave {
                merchant: "Costco".into(),
                purchased_on: jiff::civil::date(2026, 7, 20),
                currency: "USD".into(),
                subtotal: None,
                tax: None,
                total: Some(dec("30.00")),
                items: vec![
                    super::ItemSave {
                        id: Some(milk.id),
                        description: "Milk".into(),
                        total: dec("10.00"),
                        person_id: Some(josh.id),
                    },
                    super::ItemSave {
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

        let assigned = |items: &[crate::server::models::LineItem], description: &str| {
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
        super::save_people(
            &mut db,
            vec![super::PersonSave {
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

    /// The CSV must describe the same period the view does.
    #[tokio::test]
    async fn the_csv_covers_the_same_receipts_as_the_period() {
        let mut db = crate::server::db::connect_url("sqlite::memory:")
            .await
            .unwrap();

        toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 7, 5),
            merchant: "Shop",
            total: dec("12.50"),
            currency: "USD",
            image_path: "a.jpg",
            line_items: [{ description: "Thing", total: dec("12.50"), position: 0 }],
        })
        .exec(&mut db)
        .await
        .unwrap();

        let rows = super::load_range(
            &mut db,
            jiff::civil::date(2026, 7, 1),
            jiff::civil::date(2026, 7, 31),
        )
        .await
        .unwrap();
        let receipts: Vec<_> = rows
            .iter()
            .map(|(r, items)| mappers::to_dto_receipt(r, items))
            .collect();

        let csv = dto::receipts_to_csv(&receipts);
        let lines: Vec<_> = csv.lines().collect();
        assert_eq!(lines.len(), 2, "header + one item: {csv}");
        assert!(lines[1].starts_with("2026-07-05,Shop,12.50,USD,Thing,12.50,no,"));
    }
}
