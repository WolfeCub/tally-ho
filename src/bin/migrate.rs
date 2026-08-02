//! Schema management: `cargo run --features ssr --bin migrate -- migration <cmd>`.
//!
//! Generation has to live in a binary that links `crate::models`, since it works
//! by diffing those models against the newest snapshot. After changing a model:
//!
//! ```text
//! cargo run --features ssr --bin migrate -- migration generate --name add_something
//! ```
//!
//! That writes SQL to `toasty/migrations/`, a snapshot to `toasty/snapshots/`,
//! and an entry in `toasty/history.toml`. Review the SQL and commit all three.
//! The app applies pending migrations on its next start, so `apply` is only
//! needed if you want it to happen now. `drop` and `reset` are also available.
//!
//! No `Toasty.toml`: toasty-cli's defaults are already what we want (`toasty/`,
//! sequential prefixes, statement breakpoints on), and its config struct has no
//! serde defaults, so a config file would have to spell out every field —
//! including `checksums`, which toasty 0.9 declares but never reads.

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tally_ho=debug".into()),
        )
        .init();

    // `connect_raw`, not `connect`: this binary manages the schema, so it must
    // not trigger the startup migration on the way in.
    let db = tally_ho::db::connect_raw().await?;
    toasty_cli::ToastyCli::new(db).parse_and_run().await
}

#[cfg(not(feature = "ssr"))]
fn main() {
    eprintln!("build with --features ssr");
    std::process::exit(1);
}
