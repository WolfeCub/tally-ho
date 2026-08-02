//! Database connection.
//!
//! `toasty::Db` is cheap to clone (an `Arc` over a shared pool), but every
//! query takes `&mut db`. So the app holds one `Db` and each request clones it:
//! `let mut db = state.db.clone();`

use std::path::Path;

const DEFAULT_URL: &str = "sqlite:./data/tally-ho.db";

/// Connects and, in dev, creates the schema directly from the models if it is
/// not already there.
///
/// `push_schema` is the fast path while the schema is still moving; the
/// `migrate` binary takes over once there is data worth preserving.
pub async fn connect() -> toasty::Result<toasty::Db> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    connect_url(&url).await
}

pub async fn connect_url(url: &str) -> toasty::Result<toasty::Db> {
    // A file-backed SQLite URL fails to open if the directory is missing, and
    // `DATA_DIR` won't exist on a first run.
    if let Some(path) = url.strip_prefix("sqlite:")
        && path != ":memory:"
        && let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).ok();
    }

    // In-memory SQLite gives each connection its own empty database, so a pool
    // larger than one would hand out connections that can't see the schema.
    let max_pool_size = if url.contains(":memory:") { 1 } else { 16 };

    let mut db = toasty::Db::builder()
        // `crate::*` is the only supported whole-crate form; the macro has no
        // arm for a module glob like `crate::models::*`. It discovers by
        // CARGO_PKG_NAME rather than module path, so models in `crate::models`
        // are picked up regardless.
        .models(toasty::models!(crate::*))
        .max_pool_size(max_pool_size)
        .connect(url)
        .await?;

    // `push_schema` issues bare CREATE TABLEs and fails with `table already
    // exists` on the second run, so it cannot be called unconditionally — that
    // would make the app start exactly once per database. Probe with the
    // cheapest possible query and only create the schema when it is genuinely
    // absent. Replaced wholesale by the migration workflow.
    let schema_present = crate::models::Receipt::all()
        .first()
        .exec(&mut db)
        .await
        .is_ok();

    if schema_present {
        tracing::debug!("schema already present; skipping push_schema");
    } else {
        tracing::info!("no schema found; creating it from the models");
        db.push_schema().await?;
    }

    Ok(db)
}

#[cfg(test)]
mod tests {
    use crate::models::{ExtractionStatus, Receipt};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    async fn test_db() -> toasty::Db {
        super::connect_url("sqlite::memory:").await.unwrap()
    }

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[tokio::test]
    async fn creates_a_receipt_with_line_items_and_reads_them_back() {
        let mut db = test_db().await;

        let receipt = toasty::create!(Receipt {
            purchased_on: jiff::civil::date(2026, 7, 14),
            merchant: "Costco",
            total: dec("142.83"),
            currency: "USD",
            image_path: "2026/07/abc.jpg",
            line_items: [
                { description: "Milk 2%", total: dec("4.99"), position: 0 },
                { description: "Dog food", total: dec("38.50"), position: 1 },
            ],
        })
        .exec(&mut db)
        .await
        .unwrap();

        // Defaults applied on insert rather than by us.
        assert_eq!(receipt.status, ExtractionStatus::Pending);
        assert!(receipt.extraction_error.is_none());

        let items = receipt.line_items().exec(&mut db).await.unwrap();
        assert_eq!(items.len(), 2);
        // Decimal round-trips exactly through SQLite TEXT.
        assert_eq!(items.iter().map(|i| i.total).sum::<Decimal>(), dec("43.49"));
        assert!(items.iter().all(|i| !i.edited));
    }

    /// The app's core query. Boundary dates must be inclusive at both ends, and
    /// `jiff` dates are stored as ISO-8601 TEXT, which sorts lexicographically —
    /// this is what makes `.ge()/.le()` correct rather than merely plausible.
    #[tokio::test]
    async fn date_range_filter_is_inclusive_at_both_ends() {
        let mut db = test_db().await;

        // One day either side of the period, plus both boundaries and a day in
        // the middle.
        let fixtures = [
            ((2026, 6, 14), "day-before"),
            ((2026, 6, 15), "start-boundary"),
            ((2026, 6, 30), "middle"),
            ((2026, 7, 14), "end-boundary"),
            ((2026, 7, 15), "day-after"),
        ];

        for ((y, m, d), label) in fixtures {
            toasty::create!(Receipt {
                purchased_on: jiff::civil::date(y, m, d),
                merchant: label,
                total: dec("1.00"),
                currency: "USD",
                image_path: "x.jpg",
            })
            .exec(&mut db)
            .await
            .unwrap();
        }

        let from = jiff::civil::date(2026, 6, 15);
        let to = jiff::civil::date(2026, 7, 14);

        let found = Receipt::filter(
            Receipt::fields()
                .purchased_on()
                .ge(from)
                .and(Receipt::fields().purchased_on().le(to)),
        )
        .exec(&mut db)
        .await
        .unwrap();

        let mut names: Vec<_> = found.iter().map(|r| r.merchant.clone()).collect();
        names.sort();
        assert_eq!(
            names,
            ["end-boundary", "middle", "start-boundary"],
            "range must include both boundary dates and exclude the days either side"
        );
    }
}
