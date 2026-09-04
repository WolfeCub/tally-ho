//! Standalone extraction probe. Run this against real receipt photos before
//! wiring any UI: it is the fastest way to find out whether a given Ollama
//! model is good enough, and to iterate on the prompt.
//!
//!   OLLAMA_URL=http://host.orb.internal:11434 \
//!   OLLAMA_MODEL=gemma4:12b \
//!   cargo run --no-default-features --features ssr --bin extract_probe -- path/to/receipt.webp
//!
//! Add `RUST_LOG=rig=debug` to see the request rig actually sends (confirm it
//! carries a `format` object and an `images` array).

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use tally_ho::server::ask;
    use tally_ho::server::extract::{Config, ExtractedReceipt, OllamaExtractor, ReceiptExtractor};
    use tally_ho::shared::dto::LineItem;
    use tally_ho::shared::problems::{line_item_sum, problems_of};

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: extract_probe <image|pdf> [image|pdf...]");
        eprintln!("       extract_probe --schema   # dump the JSON schema sent to Ollama");
        std::process::exit(2);
    }

    // Dumping the schema is the first thing to check when output is garbage:
    // Ollama turns it into a GBNF grammar, and a schema it can't convert
    // cleanly yields structurally-valid-but-nonsense JSON.
    if paths[0] == "--schema" {
        let schema = ask::schema_of::<ExtractedReceipt>();
        println!("{}", serde_json::to_string_pretty(&schema)?);
        return Ok(());
    }

    let config = Config::load().await;
    println!(
        "model={} url={} max_edge={} ocr_context={} keep_alive={}\n",
        config.label(),
        config.url,
        config.max_image_edge,
        config.ocr_context,
        config.keep_alive.as_deref().unwrap_or("<server default>")
    );

    let extractor = OllamaExtractor::new(config)?;
    let mut failures = 0;

    for path in &paths {
        println!("=== {path} ===");
        let bytes = std::fs::read(path)?;

        let extraction = match extractor.extract(&bytes).await {
            Ok(e) => e,
            Err(e) => {
                println!("  EXTRACTION FAILED: {e}\n");
                failures += 1;
                continue;
            }
        };

        let n = extraction.receipt.normalize();

        // First, so a wrong item can be read straight off against the fields
        // below it, wrong here is the photo misread, wrong only below is the
        // structuring.
        if let Some(transcript) = &extraction.source_text {
            println!("  transcript:");
            for line in transcript.lines() {
                println!("    | {line}");
            }
        }

        println!("  elapsed:   {:.1}s", extraction.elapsed.as_secs_f32());
        println!("  merchant:  {:?}", n.merchant);
        println!("  date:      {:?}", n.purchased_on);
        println!("  subtotal:  {:?}", n.subtotal);
        println!("  tax:       {:?}", n.tax);
        println!(
            "  total:     {:?} {}",
            n.total,
            n.currency.as_deref().unwrap_or("")
        );
        println!("  items:     {}", n.line_items.len());

        for item in &n.line_items {
            println!(
                "    {:>10}  {}",
                item.total
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "?".into()),
                item.description
            );
        }

        // The app's own checks, so this says what the review screen will say
        // about the same receipt. An amount that couldn't be read is stored as
        // zero here too, which is what makes the zero-item warning match.
        let items: Vec<LineItem> = n
            .line_items
            .iter()
            .enumerate()
            .map(|(position, item)| LineItem {
                id: uuid::Uuid::nil(),
                description: item.description.clone(),
                quantity: item.quantity,
                unit_price: item.unit_price,
                total: item.total.unwrap_or_default(),
                position: position as i64,
                edited: false,
                person_id: None,
                guessed_why: None,
            })
            .collect();

        println!("  item sum:  {}", line_item_sum(&items));

        let problems = problems_of(n.subtotal, n.tax, n.total, &items);
        if problems.is_empty() {
            println!("  problems:  none");
        } else {
            println!("  problems:");
            for problem in &problems {
                println!("    - {problem}");
            }
            failures += problems.len();
        }

        if !n.warnings.is_empty() {
            println!("  warnings:");
            for w in &n.warnings {
                println!("    - {w}");
            }
        }
        println!();
    }

    if failures > 0 {
        println!("{failures} problem(s) across {} file(s)", paths.len());
    }
    Ok(())
}

#[cfg(not(feature = "ssr"))]
fn main() {
    eprintln!("extract_probe requires --features ssr");
    std::process::exit(2);
}
