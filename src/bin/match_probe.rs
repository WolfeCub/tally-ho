//! Dry-runs a statement CSV against the real database: what the importer would
//! read, which receipts it would have to choose from, and what it would attach
//! without asking. Nothing is written.
//!
//!   CURRENCY=CAD cargo run --no-default-features --features ssr \
//!     --bin match_probe -- some-statement.csv
//!
//! The unit tests cover the matching rules against fixtures. This covers the
//! part they can't: the pool. A charge matches nothing for reasons that are
//! invisible on the reconcile screen — the statement is in a currency none of
//! your receipts are in, the receipt sits outside the date range the pool is
//! drawn from, or an earlier statement already spoke for it — and all three
//! look identical to a matcher that simply didn't find anything.
//!
//! It calls [`queries::statements::pool`] rather than working the pool out for
//! itself, so it cannot quietly start disagreeing with what an import does.

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::HashSet;

    use tally_ho::server::{db, matching, queries, state, statement_csv};

    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: match_probe <statement.csv> [statement.csv...]");
        std::process::exit(2);
    }

    // The same currency the server would import under, read the same way.
    let currency = state::currency_from_env()?;
    let mut db = db::connect().await?;

    for path in files {
        println!("\n=== {path} ===");

        let parsed = match statement_csv::charges(&std::fs::read(&path)?) {
            Ok(parsed) => parsed,
            Err(why) => {
                println!("  refused: {why}");
                continue;
            }
        };

        let (begins_on, ends_on) = parsed.range();
        println!(
            "  read as: {} / {} / {}",
            parsed.layout.date.name, parsed.layout.description.name, parsed.layout.amount.name
        );
        println!("  {begins_on} .. {ends_on} in {currency}");
        for skipped in &parsed.skipped {
            println!("  skipped {skipped}");
        }

        let free = queries::statements::pool(&mut db, begins_on, ends_on).await?;
        println!("  pool: {} receipts", free.len());
        // Worth saying out loud: every charge below will match nothing, and the
        // screen gives no hint as to why.
        if free.is_empty() {
            println!("  nothing to match against — wrong dates, or all spoken for");
        }

        // Same walk as the importer: a receipt it proposes is gone for the
        // charges after it.
        let mut taken = HashSet::new();
        for charge in &parsed.charges {
            let untaken = free.iter().filter(|receipt| !taken.contains(&receipt.id));
            let offered = matching::candidates(
                charge.charged_on,
                &charge.description,
                charge.amount,
                &currency,
                untaken,
            );
            let proposed = matching::automatic(&offered);
            if let Some(receipt) = proposed {
                taken.insert(receipt);
            }

            println!(
                "\n  {} {:>9} {}",
                charge.charged_on, charge.amount, charge.description
            );
            if offered.is_empty() {
                println!("      nothing offered");
            }
            for candidate in &offered {
                // Three states worth telling apart: taken unasked, good enough
                // to have been taken had it been alone, and merely offered.
                let mark = match (Some(candidate.receipt_id) == proposed, candidate.confident) {
                    (true, _) => "attached",
                    (_, true) => "confident",
                    _ => "offered",
                };
                println!(
                    "      {mark:>9}  {}  {} {} — {}",
                    &candidate.receipt_id.to_string()[..8],
                    candidate.merchant,
                    candidate.total,
                    candidate.why
                );
            }
            // The case that costs money silently: several receipts any of which
            // would have done, so the importer rightly picks none.
            if proposed.is_none() && offered.iter().filter(|c| c.confident).count() > 1 {
                println!("      ^ more than one would fit; left for a human on purpose");
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "ssr"))]
fn main() {
    eprintln!("match_probe requires --features ssr");
    std::process::exit(2);
}
