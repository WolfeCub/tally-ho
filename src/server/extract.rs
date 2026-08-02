//! Receipt line-item extraction against a local model via Ollama.
//!
//! # Why not `rig`'s `Extractor`
//!
//! `Extractor::extract()` only yields data when the model emits a tool call
//! matching its synthetic `submit` tool — `output_tool_calls() > 0` is the sole
//! path that returns a value, so in any other output mode it reports `NoData`
//! even when the model returned valid JSON. It also hardcodes
//! `ToolChoice::Required`, which rig's Ollama provider explicitly warns about
//! and drops, so the call can't be compelled.
//!
//! Instead we drive an `Agent` in [`OutputMode::Native`], which rig maps to
//! Ollama's `format` field. That is a grammar constraint: the response is
//! *guaranteed* to match the schema, and no tool-calling support is required of
//! the vision model.

use std::time::{Duration, Instant};

use base64::Engine as _;
use rig_agent::agent::{Agent, OutputMode};
// `.agent()` comes from AgentClientExt, which is blanket-implemented for every
// CompletionClient; the prelude brings in both.
use rig_agent::prelude::*;
use rig_core::client::Nothing;
use rig_core::message::{ImageMediaType, Message, UserContent};
use rig_core::providers::ollama;
use rig_core::OneOrMany;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::image::{self, ImageError};

const DEFAULT_URL: &str = "http://localhost:11434";
const DEFAULT_MODEL: &str = "gemma4:12b";
const DEFAULT_MAX_EDGE: u32 = 1600;

/// Ollama's `num_predict`. Measured at ~40 output tokens per line item plus
/// ~130 of fixed JSON overhead, so this covers a long grocery receipt with room
/// to spare. It exists mainly as a backstop: without a cap, a model that fails
/// to terminate blocks the request indefinitely instead of failing.
const MAX_OUTPUT_TOKENS: u64 = 2048;

/// Wall-clock ceiling for one extraction. Decode measured at ~32 tok/s, so a
/// full `MAX_OUTPUT_TOKENS` response is ~65s; the rest is headroom for the
/// request queueing behind other work on the same Ollama instance.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// What the model is asked to produce.
///
/// Every money and date field is a **string**, deliberately. A local model will
/// write "$4.99", "4,99" or "1 234.56", and a typed `number`/`Decimal` schema
/// would either reject the whole payload or silently coerce through a float.
/// Strings keep the grammar simple and move parsing somewhere we can report on
/// it field by field — see [`ExtractedReceipt::normalize`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractedReceipt {
    /// Store or merchant name exactly as printed.
    pub merchant: Option<String>,
    /// Purchase date **verbatim as printed**, e.g. `08/12/21`.
    ///
    /// Deliberately not converted by the model. Asked to produce YYYY-MM-DD it
    /// silently swapped month and day on the test receipt (`08/12/21` came back
    /// as `2021-12-08`), and it did so inconsistently between runs at
    /// temperature 0. A wrong date is the worst failure this app has — it files
    /// a receipt into the wrong statement period while every amount still
    /// balances, so nothing looks wrong. [`parse_date`] applies one documented,
    /// unit-tested convention instead.
    pub purchased_on: Option<String>,
    /// ISO currency code if printed or unambiguous, e.g. USD.
    pub currency: Option<String>,
    pub subtotal: Option<String>,
    pub tax: Option<String>,
    /// The final amount charged.
    pub total: Option<String>,
    pub line_items: Vec<ExtractedLineItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractedLineItem {
    /// Item name as printed on the receipt.
    pub description: String,
    pub quantity: Option<String>,
    pub unit_price: Option<String>,
    /// Line total. Negative for discounts and coupons.
    pub total: Option<String>,
}

/// The field semantics have to live here, not in the struct's doc comments:
/// Ollama compiles the JSON schema into a GBNF grammar and drops `description`
/// fields on the way, so anything explained only in the schema is invisible to
/// the model. Measured behaviour without these instructions was amounts landing
/// in `unit_price` instead of `total`, and `purchased_on` being skipped.
const PROMPT: &str = "\
You transcribe retail receipts into structured data.

Always include every field of the object. Use null for anything not printed or \
not readable — never omit a key, never guess, and never invent a line item.

Fields:
- merchant: the store name, e.g. Walmart.
- purchased_on: the transaction date copied EXACTLY as printed, character for \
character, e.g. 08/12/21. Do not convert, reorder or reformat it.
- currency: the ISO code, e.g. USD. Infer it from the currency symbol.
- subtotal: the amount printed as SUBTOTAL.
- tax: the total tax amount printed.
- total: the final amount charged, printed as TOTAL.
- line_items: one entry per purchased item, in the order printed.

For each line item:
- description: the item name exactly as printed. Do not expand abbreviations \
or tidy them up. Do not include the item number, barcode, or the tax-code \
letter that may follow the price.
- total: the amount printed at the end of that item's line. This is the field \
to fill for a normal item — put the printed amount here, not in unit_price.
- quantity and unit_price: only when the line explicitly shows a multiplication \
such as \"2 @ 3.99\". Otherwise null.

Amounts: digits with a period as the decimal separator, no currency symbol and \
no thousands separator, e.g. 1234.56. Discounts and coupons get a negative \
total.

Do not create line items for SUBTOTAL, TAX, TOTAL, tender or change lines, \
loyalty balances, barcodes, or store metadata.";

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("image preparation failed: {0}")]
    Image(#[from] ImageError),
    #[error("ollama request failed: {0}")]
    Prompt(String),
    #[error("ollama did not respond within {0:?}")]
    Timeout(Duration),
    #[error(
        "model returned no content — if the model is thinking-capable, reasoning tokens may have \
         consumed the whole generation budget"
    )]
    EmptyResponse,
    #[error("model returned data that did not match the schema: {0}")]
    Deserialize(#[from] serde_json::Error),
}

/// Outcome of one extraction, including what it cost in wall-clock time — the
/// number that decides whether a given model is usable on your hardware.
#[derive(Debug, Clone)]
pub struct Extraction {
    pub receipt: ExtractedReceipt,
    /// Verbatim model output, retained for debugging bad extractions.
    pub raw: String,
    pub model: String,
    pub elapsed: Duration,
}

#[async_trait::async_trait]
pub trait ReceiptExtractor: Send + Sync {
    /// `bytes` is the original uploaded file in any supported format; the
    /// implementation is responsible for normalizing it.
    async fn extract(&self, bytes: &[u8]) -> Result<Extraction, ExtractError>;
}

#[derive(Debug, Clone)]
pub struct Config {
    pub url: String,
    pub model: String,
    pub max_image_edge: u32,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            url: std::env::var("OLLAMA_URL").unwrap_or_else(|_| DEFAULT_URL.to_string()),
            model: std::env::var("OLLAMA_VISION_MODEL")
                .unwrap_or_else(|_| DEFAULT_MODEL.to_string()),
            max_image_edge: std::env::var("MAX_IMAGE_EDGE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_MAX_EDGE),
        }
    }
}

pub struct OllamaExtractor {
    // `Agent` is generic over the completion model in rig 0.41; the Ollama
    // model type defaults its HTTP backend to `reqwest::Client`.
    agent: Agent<ollama::CompletionModel>,
    config: Config,
}

impl OllamaExtractor {
    pub fn new(config: Config) -> Result<Self, ExtractError> {
        let client = ollama::Client::builder()
            // Ollama needs no auth; `OllamaApiKey` has a `From<Nothing>` impl
            // for exactly this. `build()` won't accept the unset state.
            .api_key(Nothing)
            .base_url(&config.url)
            .build()
            .map_err(|e| ExtractError::Prompt(e.to_string()))?;

        let agent = client
            .agent(&config.model)
            .preamble(PROMPT)
            // Greedy decoding. Measured on gemma4:12b: at the default
            // temperature roughly a third of runs escaped into a string field
            // (`"unit_price": ">{{4.99}}, "`), returned all nulls, or came back
            // empty. At 0 the same prompt was stable across every run. The
            // grammar constrains JSON *shape*, not the content of a string —
            // sampling discipline is what keeps the content sane.
            .temperature(0.0)
            // Disable thinking. This is the single most important setting here.
            // gemma4:12b is thinking-capable and Ollama enables it by default,
            // which was catastrophic: measured on the test receipt, reasoning
            // consumed the entire generation budget (3291 chars of `thinking`,
            // 1500 tokens, 47s) and `content` came back *empty*, so extraction
            // failed with what looked like a parse error. With thinking off the
            // same request produced a fully correct receipt in 335 tokens and
            // 10s. rig lifts `think` out of `additional_params` to the
            // top-level Ollama field.
            .additional_params(serde_json::json!({ "think": false }))
            // Maps to Ollama's `num_predict`.
            .max_tokens(MAX_OUTPUT_TOKENS)
            .output_schema::<ExtractedReceipt>()
            // Native, not Tool: this becomes Ollama's `format`, a hard grammar
            // constraint. Tool mode would depend on the vision model choosing
            // to call a tool, which cannot be forced here.
            .output_mode(OutputMode::Native)
            .build();

        Ok(Self { agent, config })
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }
}

#[async_trait::async_trait]
impl ReceiptExtractor for OllamaExtractor {
    async fn extract(&self, bytes: &[u8]) -> Result<Extraction, ExtractError> {
        let prepared = image::prepare(bytes, self.config.max_image_edge)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&prepared.jpeg);

        // Ollama accepts base64 only: rig's provider hard-errors on the `Raw`
        // and `Url` image source kinds.
        let message = Message::User {
            content: OneOrMany::many([
                UserContent::image_base64(b64, Some(ImageMediaType::JPEG), None),
                UserContent::text("Extract every line item from this receipt."),
            ])
            .expect("two content parts is never empty"),
        };

        let started = Instant::now();
        // Bounded: a model that never terminates would otherwise pin a
        // background task forever.
        let response = tokio::time::timeout(
            REQUEST_TIMEOUT,
            self.agent.runner(message).max_turns(1).run(),
        )
        .await
        .map_err(|_| ExtractError::Timeout(REQUEST_TIMEOUT))?
        .map_err(|e| ExtractError::Prompt(e.to_string()))?;
        let elapsed = started.elapsed();

        let raw = response.output;
        // Distinguish "no content" from "bad JSON" — they have very different
        // causes, and an empty body otherwise surfaces as a confusing
        // "expected value at line 1 column 1".
        if raw.trim().is_empty() {
            return Err(ExtractError::EmptyResponse);
        }
        let receipt: ExtractedReceipt = serde_json::from_str(&raw)?;

        Ok(Extraction {
            receipt,
            raw,
            model: self.config.model.clone(),
            elapsed,
        })
    }
}

// ---------------------------------------------------------------------------
// Normalization
// ---------------------------------------------------------------------------

/// A parsed receipt plus everything that didn't parse. Warnings are surfaced in
/// the review UI rather than thrown away, because a model that mangles one
/// amount usually got the rest right.
#[derive(Debug, Clone, Default)]
pub struct Normalized {
    pub merchant: Option<String>,
    pub purchased_on: Option<jiff::civil::Date>,
    pub currency: Option<String>,
    pub subtotal: Option<Decimal>,
    pub tax: Option<Decimal>,
    pub total: Option<Decimal>,
    pub line_items: Vec<NormalizedLineItem>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NormalizedLineItem {
    pub description: String,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>,
    pub total: Option<Decimal>,
}

impl ExtractedReceipt {
    pub fn normalize(&self) -> Normalized {
        let mut warnings = Vec::new();

        let mut money = |label: &str, raw: &Option<String>| -> Option<Decimal> {
            let raw = raw.as_deref()?;
            match parse_money(raw) {
                Some(d) => Some(d),
                None => {
                    warnings.push(format!("could not parse {label} {raw:?}"));
                    None
                }
            }
        };

        let subtotal = money("subtotal", &self.subtotal);
        let tax = money("tax", &self.tax);
        let total = money("total", &self.total);

        let purchased_on = match self.purchased_on.as_deref() {
            None => None,
            Some(raw) => match parse_date(raw) {
                Some(d) => Some(d),
                None => {
                    warnings.push(format!("could not parse date {raw:?}"));
                    None
                }
            },
        };

        let line_items = self
            .line_items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let mut parse = |label: &str, raw: &Option<String>| -> Option<Decimal> {
                    let raw = raw.as_deref()?;
                    match parse_money(raw) {
                        Some(d) => Some(d),
                        None => {
                            warnings.push(format!("item {i}: could not parse {label} {raw:?}"));
                            None
                        }
                    }
                };
                NormalizedLineItem {
                    description: item.description.trim().to_string(),
                    quantity: parse("quantity", &item.quantity),
                    unit_price: parse("unit price", &item.unit_price),
                    total: parse("total", &item.total),
                }
            })
            .collect();

        Normalized {
            merchant: self.merchant.as_deref().map(|m| m.trim().to_string()),
            purchased_on,
            currency: self.currency.as_deref().map(|c| c.trim().to_uppercase()),
            subtotal,
            tax,
            total,
            line_items,
            warnings,
        }
    }
}

/// Money as printed on receipts, which is not the same as money as `Decimal`
/// parses it: currency symbols, thousands separators, trailing minus signs and
/// parenthesised negatives all show up.
pub fn parse_money(raw: &str) -> Option<Decimal> {
    let mut s = raw.trim().to_string();

    // "(4.99)" and "4.99-" both mean negative on receipts.
    let mut negative = false;
    if s.starts_with('(') && s.ends_with(')') {
        negative = true;
        s = s[1..s.len() - 1].to_string();
    }
    if let Some(stripped) = s.strip_suffix('-') {
        negative = true;
        s = stripped.to_string();
    }

    // Drop currency symbols, codes and spaces; keep digits, separators, sign.
    s.retain(|c| c.is_ascii_digit() || c == '.' || c == ',' || c == '-');

    // Comma disambiguation: with both separators the last one is the decimal
    // point ("1,234.56" vs "1.234,56"). With only commas, treat a single comma
    // followed by exactly two digits as a decimal point, otherwise as grouping.
    let s = match (s.rfind('.'), s.rfind(',')) {
        (Some(dot), Some(comma)) if comma > dot => s.replace('.', "").replace(',', "."),
        (Some(_), Some(_)) => s.replace(',', ""),
        (None, Some(comma)) if s.len() - comma == 3 && s.matches(',').count() == 1 => {
            s.replace(',', ".")
        }
        (None, Some(_)) => s.replace(',', ""),
        _ => s,
    };

    if s.is_empty() || s == "-" {
        return None;
    }

    let d: Decimal = s.parse().ok()?;
    Some(if negative { -d } else { d })
}

/// ISO first, then the formats receipts actually print.
///
/// **Ambiguous numeric dates are read as MM/DD/YY** (US convention), because
/// that is what the receipts this is built for print. `08/12/21` is therefore
/// 2021-08-12, not 2021-12-08. This is deliberately a fixed rule rather than a
/// guess: being consistently wrong on a non-US receipt is recoverable in the
/// review screen, whereas being unpredictably wrong is not detectable at all.
pub fn parse_date(raw: &str) -> Option<jiff::civil::Date> {
    let s = raw.trim();

    if let Ok(d) = s.parse::<jiff::civil::Date>() {
        return Some(d);
    }

    let parts: Vec<&str> = s
        .split(['/', '-', '.'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let a: i32 = parts[0].parse().ok()?;
    let b: i8 = parts[1].parse().ok()?;
    let c: i32 = parts[2].parse().ok()?;

    // YYYY/MM/DD
    if parts[0].len() == 4 {
        return jiff::civil::Date::new(a as i16, b, c as i8).ok();
    }

    // MM/DD/YY(YY). Two-digit years are assumed current-century; receipts are
    // not historical documents.
    let year = if parts[2].len() <= 2 { 2000 + c } else { c };
    jiff::civil::Date::new(year as i16, a as i8, b).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn parses_plain_and_decorated_amounts() {
        assert_eq!(parse_money("4.99"), Some(dec("4.99")));
        assert_eq!(parse_money("$4.99"), Some(dec("4.99")));
        assert_eq!(parse_money(" USD 4.99 "), Some(dec("4.99")));
        assert_eq!(parse_money("1,234.56"), Some(dec("1234.56")));
        assert_eq!(parse_money("1.234,56"), Some(dec("1234.56")));
        assert_eq!(parse_money("4,99"), Some(dec("4.99")));
        assert_eq!(parse_money("1,234"), Some(dec("1234")));
    }

    #[test]
    fn parses_receipt_style_negatives() {
        assert_eq!(parse_money("-2.00"), Some(dec("-2.00")));
        assert_eq!(parse_money("(2.00)"), Some(dec("-2.00")));
        assert_eq!(parse_money("2.00-"), Some(dec("-2.00")));
    }

    #[test]
    fn rejects_unparseable_amounts() {
        assert_eq!(parse_money(""), None);
        assert_eq!(parse_money("n/a"), None);
        assert_eq!(parse_money("-"), None);
    }

    /// Exactness is the whole reason for `Decimal`; a float round-trip would
    /// lose this.
    #[test]
    fn amounts_are_exact() {
        let sum: Decimal = ["0.10", "0.20"].iter().map(|s| dec(s)).sum();
        assert_eq!(sum, dec("0.30"));
        assert_eq!(parse_money("19.99").unwrap() * dec("3"), dec("59.97"));
    }

    #[test]
    fn parses_common_receipt_date_formats() {
        let expected = jiff::civil::date(2026, 7, 14);
        assert_eq!(parse_date("2026-07-14"), Some(expected));
        assert_eq!(parse_date("07/14/2026"), Some(expected));
        assert_eq!(parse_date("7/14/26"), Some(expected));
        assert_eq!(parse_date("2026/07/14"), Some(expected));
        assert_eq!(parse_date("07.14.2026"), Some(expected));
    }

    /// Regression guard. The model returned `2021-12-08` for a receipt printed
    /// `08/12/21`, silently moving it four months and into a different
    /// statement period. Both digits are valid months, so nothing downstream
    /// could have caught it — the ordering rule has to be enforced here.
    #[test]
    fn ambiguous_numeric_dates_are_month_first() {
        assert_eq!(parse_date("08/12/21"), Some(jiff::civil::date(2021, 8, 12)));
        assert_eq!(parse_date("01/02/26"), Some(jiff::civil::date(2026, 1, 2)));
        // Unambiguous: 13 cannot be a month, so this is not silently accepted
        // as December 13th.
        assert_eq!(parse_date("13/02/26"), None);
    }

    #[test]
    fn rejects_unparseable_dates() {
        assert_eq!(parse_date("last Tuesday"), None);
        assert_eq!(parse_date("13/45/2026"), None);
        assert_eq!(parse_date(""), None);
    }

    /// A model that mangles one field should not discard the rest.
    #[test]
    fn normalize_reports_bad_fields_without_dropping_good_ones() {
        let raw = ExtractedReceipt {
            merchant: Some("  Walmart  ".to_string()),
            purchased_on: Some("nonsense".to_string()),
            currency: Some("usd".to_string()),
            subtotal: None,
            tax: Some("about three dollars".to_string()),
            total: Some("$12.34".to_string()),
            line_items: vec![ExtractedLineItem {
                description: " MILK 2% ".to_string(),
                quantity: None,
                unit_price: None,
                total: Some("4.99".to_string()),
            }],
        };

        let n = raw.normalize();
        assert_eq!(n.merchant.as_deref(), Some("Walmart"));
        assert_eq!(n.currency.as_deref(), Some("USD"));
        assert_eq!(n.total, Some(dec("12.34")));
        assert_eq!(n.purchased_on, None);
        assert_eq!(n.tax, None);
        assert_eq!(n.line_items[0].description, "MILK 2%");
        assert_eq!(n.line_items[0].total, Some(dec("4.99")));
        assert_eq!(n.warnings.len(), 2, "bad date and bad tax should warn");
    }
}
