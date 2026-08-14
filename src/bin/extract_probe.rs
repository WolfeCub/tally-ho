//! Standalone extraction probe. Run this against real receipt photos before
//! wiring any UI — it is the fastest way to find out whether a given Ollama
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
    use rust_decimal::Decimal;
    use tally_ho::server::ask;
    use tally_ho::server::extract::{Config, ExtractedReceipt, OllamaExtractor, ReceiptExtractor};

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: extract_probe <image> [image...]");
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

    let config = Config::from_env();
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
        // below it: wrong here is the photo misread, wrong only below is the
        // structuring.
        if let Some(transcript) = &extraction.ocr_transcript {
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

        // A receipt gives two *independent* equations, and checking both
        // separates the failure modes: a dropped or hallucinated line item
        // breaks the first, a misread money field breaks the second. Checking
        // only items-vs-total would let a compensating pair of errors pass.
        let sum: Decimal = n.line_items.iter().filter_map(|i| i.total).sum();
        println!("  item sum:  {sum}");

        // 1. line items should reconstruct the subtotal (pre-tax).
        match n.subtotal {
            Some(subtotal) if (subtotal - sum).is_zero() => {
                println!("  CHECK 1:   items == subtotal ({subtotal})");
            }
            Some(subtotal) => {
                println!(
                    "  CHECK 1:   MISMATCH items {sum} != subtotal {subtotal} \
                     (diff {})",
                    subtotal - sum
                );
                failures += 1;
            }
            None => {
                // Fall back to comparing against the total, which is all we can
                // do when no subtotal was printed or read.
                match n.total {
                    Some(total) if (total - sum).is_zero() => {
                        println!("  CHECK 1:   no subtotal; items == total ({total})");
                    }
                    Some(total) => {
                        println!("  CHECK 1:   no subtotal; items {sum} != total {total}");
                        failures += 1;
                    }
                    None => {
                        println!("  CHECK 1:   no subtotal and no total extracted");
                        failures += 1;
                    }
                }
            }
        }

        // 2. subtotal + tax should equal the amount actually charged.
        match (n.subtotal, n.tax, n.total) {
            (Some(sub), Some(tax), Some(total)) if (sub + tax - total).is_zero() => {
                println!("  CHECK 2:   subtotal + tax == total ({sub} + {tax} = {total})");
            }
            (Some(sub), Some(tax), Some(total)) => {
                println!(
                    "  CHECK 2:   MISMATCH {sub} + {tax} != {total} (diff {})",
                    sub + tax - total
                );
                failures += 1;
            }
            (sub, tax, total) => {
                println!(
                    "  CHECK 2:   incomplete money fields (subtotal={sub:?} tax={tax:?} \
                     total={total:?})"
                );
                failures += 1;
            }
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
        println!("{failures} check(s) failed across {} image(s)", paths.len());
    }
    Ok(())
}

#[cfg(not(feature = "ssr"))]
fn main() {
    eprintln!("extract_probe requires --features ssr");
    std::process::exit(2);
}
