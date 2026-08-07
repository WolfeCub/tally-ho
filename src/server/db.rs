//! Database connection and schema management.
//!
//! `toasty::Db` is cheap to clone (an `Arc` over a shared pool), but every
//! query takes `&mut db`. So the app holds one `Db` and each request clones it:
//! `let mut db = state.db.clone();`

use std::path::Path;

const DEFAULT_URL: &str = "sqlite:./data/tally-ho.db";

fn url_from_env() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_URL.to_string())
}

/// Where the migrations, snapshots, and history file live.
///
/// toasty-cli resolves this relative to the working directory, which is fine in
/// the repo but not for an installed binary — hence `MIGRATIONS_DIR`, which the
/// Nix wrapper points at the copy in the store.
///
/// Built in code rather than read from a `Toasty.toml`: the defaults are already
/// what we want, and `MigrationConfig` has no serde defaults, so a config file
/// would have to spell out every field.
pub fn migration_config() -> toasty_cli::Config {
    let path = std::env::var("MIGRATIONS_DIR").unwrap_or_else(|_| "toasty".to_string());
    toasty_cli::Config::new().migration(toasty_cli::MigrationConfig::new().path(path))
}

/// Connects without touching the schema.
///
/// For the migrate binary, which is the thing that manages the schema and so
/// must not have migrations applied underneath it on the way in.
pub async fn connect_raw() -> toasty::Result<toasty::Db> {
    connect_raw_url(&url_from_env()).await
}

pub async fn connect_raw_url(url: &str) -> toasty::Result<toasty::Db> {
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

    toasty::Db::builder()
        // `crate::*` is the only whole-crate form the macro takes — there's no
        // arm for a module glob like `server::models::*`. It finds models by
        // CARGO_PKG_NAME rather than module path, so wherever they live is fine.
        .models(toasty::models!(crate::*))
        .max_pool_size(max_pool_size)
        .connect(url)
        .await
}

/// Connects and brings the schema up to date.
pub async fn connect() -> anyhow::Result<toasty::Db> {
    connect_url(&url_from_env()).await
}

pub async fn connect_url(url: &str) -> anyhow::Result<toasty::Db> {
    let db = connect_raw_url(url).await?;

    if url.contains(":memory:") {
        // Migrations cannot reach an in-memory database: applying them opens a
        // fresh driver connection, and for `:memory:` that is a *different*
        // empty database than the pool's. So tests build the schema straight
        // from the models — which is also what makes the drift test below worth
        // having, since it is what ties the two paths together.
        db.push_schema().await?;
        return Ok(db);
    }

    apply_migrations(&db).await?;
    Ok(db)
}

/// Runs any migrations in `toasty/history.toml` that this database hasn't seen.
///
/// Done at startup rather than as a separate deploy step: this is a single-user
/// app started with `cargo leptos watch`, and the alternative failure mode —
/// forgetting to migrate and then hitting missing-column errors at runtime — is
/// far more confusing than the migration output in the log.
///
/// The driver records applied ids in `__toasty_migrations` and wraps each
/// migration in a transaction, so this is idempotent and a failure leaves the
/// schema where it was.
async fn apply_migrations(db: &toasty::Db) -> anyhow::Result<()> {
    toasty_cli::ToastyCli::with_config(db.clone(), migration_config())
        .parse_from(["toasty", "migration", "apply"])
        .await
        .map_err(|e| {
            // The one predictable way this fails: a database created by an
            // older build that used `push_schema`, which left no migration
            // history, so migration 0001 tries to create tables that exist.
            if format!("{e:#}").contains("already exists") {
                e.context(
                    "this database has tables but no migration history, so it predates \
                     migrations — delete it and let them rebuild it: rm -rf ./data",
                )
            } else {
                e
            }
        })?;

    // `apply` reports "no migrations found" and succeeds when the history file
    // is missing, which would otherwise leave an empty database to fail on its
    // first query.
    crate::server::models::Receipt::all()
        .first()
        .exec(&mut db.clone())
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "the schema is missing after applying migrations ({e}) — if toasty/ is \
                 empty, generate the first migration: \
                 cargo run --features ssr --bin migrate -- migration generate --name init"
            )
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::server::models::{ExtractionStatus, Receipt};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    async fn test_db() -> toasty::Db {
        super::connect_url("sqlite::memory:").await.unwrap()
    }

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    /// Catches the mistake the migration workflow makes easy: editing a model
    /// and forgetting to generate the migration that goes with it. Tests build
    /// their schema from the models and the real database builds it from
    /// migrations, so without this the two could silently diverge.
    #[tokio::test]
    async fn the_models_match_the_latest_migration() {
        let db = super::connect_raw_url("sqlite::memory:").await.unwrap();
        let config = super::migration_config();

        let history =
            toasty::migration::History::load_or_default(config.migration.get_history_file_path())
                .unwrap();
        let latest = history
            .entries()
            .last()
            .expect("no migrations yet — run: migration generate --name init");
        let snapshot = toasty::migration::Snapshot::load(
            config
                .migration
                .get_snapshots_dir()
                .join(&latest.snapshot_name),
        )
        .unwrap();

        let drift = toasty::migration::generate(
            db.driver(),
            &snapshot.schema,
            &db.schema().db,
            &Default::default(),
        );

        assert!(
            drift.is_none(),
            "src/models.rs has drifted from {}. Run: \
             cargo run --features ssr --bin migrate -- migration generate --name <what-changed>",
            latest.name
        );
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
