//! Server-only code: nothing here compiles into the wasm bundle.

pub mod extract;
pub mod image;
pub mod job;
pub mod state;
pub mod store;

use crate::dto;
use crate::models;

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

#[cfg(test)]
mod tests {
    use crate::dto;
    use crate::models::Receipt;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    /// Exercises the range query against real SQLite: the ordering, the line-item
    /// fetch, and the fold into a period figure.
    #[tokio::test]
    async fn a_period_loads_in_order_with_its_items_and_totals() {
        let mut db = crate::db::connect_url("sqlite::memory:").await.unwrap();

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
            .map(|(r, items)| super::to_dto_summary(r, items))
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

    /// The CSV must describe the same period the view does.
    #[tokio::test]
    async fn the_csv_covers_the_same_receipts_as_the_period() {
        let mut db = crate::db::connect_url("sqlite::memory:").await.unwrap();

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
            .map(|(r, items)| super::to_dto_receipt(r, items))
            .collect();

        let csv = dto::receipts_to_csv(&receipts);
        let lines: Vec<_> = csv.lines().collect();
        assert_eq!(lines.len(), 2, "header + one item: {csv}");
        assert!(lines[1].starts_with("2026-07-05,Shop,12.50,USD,Thing,12.50,no,"));
    }
}
