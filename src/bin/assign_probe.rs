//! Asks the assigner about reviewed receipts in a real database and lines its
//! guesses up against what is stored there. Nothing is written.
//!
//!   DATABASE_URL=sqlite:tmp/tally-ho.db \
//!   LLAMA_URL=http://host.orb.internal:8080 \
//!   cargo run --no-default-features --features ssr --bin assign_probe -- [receipt id...]
//!
//! With no ids it asks about every reviewed receipt. Stored is what a person
//! chose, or a guess nobody overruled, so a disagreement is worth reading but
//! is not always the model's mistake.

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::BTreeMap;

    use tally_ho::server::assign::{self, ItemAssigner as _, LlamaAssigner};
    use tally_ho::server::{db, extract, queries};

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let mut db = db::connect().await?;
    let people = queries::people::list(&mut db).await?;
    let name_of = |id| {
        people
            .iter()
            .find(|p| p.id == id)
            .map_or("?", |p| p.name.as_str())
    };

    let ids: Vec<uuid::Uuid> = match std::env::args().skip(1).collect::<Vec<_>>() {
        args if args.is_empty() => queries::receipts::recent(&mut db, i64::MAX as usize)
            .await?
            .into_iter()
            .filter(|r| r.reviewed)
            .map(|r| r.id)
            .collect(),
        args => args.iter().map(|a| a.parse()).collect::<Result<_, _>>()?,
    };

    let config = assign::Config::from_extraction(&extract::Config::from_env());
    println!("model={} url={}\n", config.model, config.url);
    let assigner = LlamaAssigner::new(config);

    // (what's stored, what the model said) -> items
    let mut tally: BTreeMap<(&str, &str), usize> = BTreeMap::new();

    for id in ids {
        let Some(mut receipt) = queries::receipts::load(&mut db, id).await? else {
            println!("=== {id}: no such receipt ===\n");
            continue;
        };
        receipt.line_items.sort_by_key(|item| item.position);
        println!(
            "=== {} {} ({id}) ===",
            receipt.purchased_on, receipt.merchant
        );

        let descriptions: Vec<&str> = receipt
            .line_items
            .iter()
            .map(|i| i.description.as_str())
            .collect();
        let started = std::time::Instant::now();
        let said = match assigner
            .assign(&receipt.merchant, &people, &descriptions)
            .await
        {
            Ok(said) => said,
            Err(e) => {
                println!("  FAILED: {e:#}\n");
                continue;
            }
        };
        println!("  elapsed: {:.1}s", started.elapsed().as_secs_f32());
        let guesses = assign::resolve(&people, descriptions.len(), &said);

        for (item, guess) in receipt.line_items.iter().zip(&guesses) {
            let stored = item.person_id.map(name_of);
            let guessed = guess.as_ref().map(|g| name_of(g.person_id));

            let kind = match (item.person_id, &item.guessed_why) {
                (None, _) => "nobody",
                (Some(_), None) => "chosen",
                (Some(_), Some(_)) => "guess kept",
            };
            let verdict = match (stored, guessed) {
                (a, b) if a == b => "same",
                (_, None) => "null",
                (None, Some(_)) => "named",
                (Some(_), Some(_)) => "other",
            };
            *tally.entry((kind, verdict)).or_default() += 1;

            let mark = if matches!(verdict, "named" | "other") {
                "!"
            } else {
                " "
            };
            println!(
                " {mark} {:<40} {kind:>10} {:<6} -> {}",
                item.description,
                stored.unwrap_or("-"),
                guessed.unwrap_or("-"),
            );
            if let Some(guess) = guess {
                println!("      {}", guess.why);
            }
        }
        println!();
    }

    for ((kind, verdict), n) in &tally {
        println!("{kind:>10} {verdict:<6} {n}");
    }
    Ok(())
}

#[cfg(not(feature = "ssr"))]
fn main() {
    eprintln!("assign_probe requires --features ssr");
    std::process::exit(2);
}
