//! Receipt line-item extraction against local models via Ollama.
//!
//! Two stages, because reading a creased receipt and structuring it are different
//! skills: a small OCR model transcribes the photo, then `OLLAMA_MODEL` turns
//! that text into [`ExtractedReceipt`] without ever seeing the image. Setting
//! `OLLAMA_OCR_MODEL` empty collapses it back to one model reading the photo.
//!
//! A PDF invoice skips the first stage: it carries its own text, so only the
//! structuring model runs, against a prompt written for an invoice rather than a
//! till receipt.
//!
//! # Why not `rig`'s `Extractor`
//!
//! `Extractor::extract()` only yields data when the model emits a tool call
//! matching its synthetic `submit` tool: `output_tool_calls() > 0` is the sole
//! path that returns a value, so in any other output mode it reports `NoData`
//! even when the model returned valid JSON. It also hardcodes
//! `ToolChoice::Required`, which rig's Ollama provider explicitly warns about
//! and drops, so the call can't be compelled.
//!
//! Instead we drive an `Agent` in [`OutputMode::Native`], which rig maps to
//! Ollama's `format` field. That is a grammar constraint: the response is
//! *guaranteed* to match the schema, and no tool-calling support is required of
//! the model.

use std::time::{Duration, Instant};

use base64::Engine as _;
use rig_agent::agent::{Agent, OutputMode};
use rig_core::OneOrMany;
use rig_core::message::{ImageMediaType, Message, UserContent};
use rig_core::providers::ollama;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ask;
use super::image::{self, ImageError};
use super::pdf::{self, PdfError};
use crate::shared::parse;

const DEFAULT_URL: &str = "http://localhost:11434";
const DEFAULT_MODEL: &str = "gemma4:12b";

/// Reads the photo, which [`DEFAULT_MODEL`] then structures. A 1.1B model that
/// only does OCR reads a creased receipt far better than a 12B generalist does:
/// every amount and barcode right in ~2s, against `2.44` coming back as `14.00`.
const DEFAULT_OCR_MODEL: &str = "glm-ocr:q8_0";

/// Ollama's `num_ctx` for the OCR model, which is the one setting here that can't
/// be left to the server default. The photo is most of the prompt and the
/// transcript has to fit in what's left: at 2048 the test receipt stops after
/// three of its five items and loses every total, which downstream reads as a
/// receipt that simply hadn't got any. 4096 transcribes that one whole, so this
/// is double it: a long grocery receipt is a much longer transcript.
const DEFAULT_OCR_CONTEXT: u32 = 8192;

/// Ollama's `num_gpu` for the OCR model, every layer, overriding Ollama's own
/// placement estimate. It sizes a vision model's compute graph against the
/// largest image it might be sent, and at [`DEFAULT_MAX_EDGE`] that projection
/// exceeds what the 12B model leaves free, so it splits the OCR model across
/// CPU and GPU instead. Measured on a 12 GB 3060 with both models resident: the
/// split put 1146 MiB of a ~2.0 GB model on the GPU while 2740 MiB sat unused,
/// so what was short was the projection, not the card. That arithmetic runs on
/// every load, not once, so it was re-decided each time the model came back from
/// an idle unload: see [`DEFAULT_KEEP_ALIVE`], which now stops it unloading at
/// all.
///
/// Forced, the same model loads fully resident at 1.3 GB, *less* than the
/// 1.9 GB the split took, because splitting layers across backends allocates
/// compute buffers on both of them. The configuration Ollama was avoiding is
/// cheaper than the one it chose.
///
/// 99 is the conventional "all of them": Ollama clamps it to the layer count.
/// The tradeoff is that this removes the safety net: if the GPU genuinely
/// hasn't room, the runner now fails to load rather than quietly falling back
/// to a slow CPU split.
const OCR_GPU_LAYERS: u32 = 99;

/// How long Ollama holds these models in VRAM after a request, its `keep_alive`.
/// Negative means never unload, and that is the default here: Ollama's own
/// default drops a model after five minutes idle, so the first receipt after any
/// quiet spell pays a cold load, which for the OCR model is also when its
/// placement is re-decided (see [`OCR_GPU_LAYERS`]).
///
/// Spelled with a unit because the string form is parsed as a duration, so a
/// bare `-1` has nothing to parse; rig requires a string here and rejects a
/// number outright. Set `OLLAMA_KEEP_ALIVE` empty to leave the server's default
/// alone, which is what you want if something else shares the GPU and needs the
/// models to get out of the way.
const DEFAULT_KEEP_ALIVE: &str = "-1m";

/// Downscale only, and about cost rather than accuracy: the OCR model reads this
/// receipt anywhere from 1024 to 5096 pixels. Past this it's just tiled into more
/// pieces: 2.1s here against 10s at native, for the same text. Below it the
/// transcript gets noisier, fenced and repeating, which lost a subtotal at 1024.
const DEFAULT_MAX_EDGE: u32 = 1600;

/// Ollama's `num_predict`, for the transcript as much as the JSON. Measured at
/// ~40 output tokens per line item plus ~130 of fixed JSON overhead, so this
/// covers a long grocery receipt with room to spare. It exists mainly as a
/// backstop: without a cap, a model that fails to terminate blocks the request
/// indefinitely instead of failing.
const MAX_OUTPUT_TOKENS: u64 = 2048;

/// Wall-clock ceiling for one request, so a two-stage extraction can take twice
/// it. Decode measured at ~32 tok/s, so a full `MAX_OUTPUT_TOKENS` response is
/// ~65s; the rest is headroom for the request queueing behind other work on the
/// same Ollama instance.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// What the model is asked to produce.
///
/// Every money and date field is a **string**, deliberately. A local model will
/// write "$4.99", "4,99" or "1 234.56", and a typed `number`/`Decimal` schema
/// would either reject the whole payload or silently coerce through a float.
/// Strings keep the grammar simple and move parsing somewhere we can report on
/// it field by field: see [`ExtractedReceipt::normalize`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractedReceipt {
    /// Store or merchant name exactly as printed.
    pub merchant: Option<String>,
    /// Purchase date **verbatim as printed**, e.g. `08/12/21`.
    ///
    /// Deliberately not converted by the model. Asked to produce YYYY-MM-DD it
    /// silently swapped month and day on the test receipt (`08/12/21` came back
    /// as `2021-12-08`), and it did so inconsistently between runs at
    /// temperature 0. A wrong date is the worst failure this app has: it files
    /// a receipt into the wrong statement period while every amount still
    /// balances, so nothing looks wrong. [`parse::date`] applies one documented,
    /// unit-tested convention instead.
    pub purchased_on: Option<String>,
    /// ISO currency code, e.g. USD.
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
    ///
    /// Not an `Option`, which is what puts it in the nested object's `required`
    /// list; [`ask::schema_of`] only does that for the top level. Left optional,
    /// the grammar lets the model omit the key, and on an invoice it omitted
    /// this one on every line. An amount it can't read comes back empty.
    pub total: String,
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
- currency: the ISO code, e.g. CAD. The one field to work out rather than copy: \
the address, the phone number and the name of the tax charged say which country \
the receipt was printed in, and the country says which currency. A dollar sign on \
its own settles nothing, since plenty of countries write their money with one. \
With nothing on the receipt to say which country, the answer is null rather than \
the code you see most often.
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

/// The same schema off a vendor's invoice, which goes wrong in its own ways: the
/// customer's name is printed larger than the seller's, several dates are printed
/// and only one is the purchase, and anything that shipped in two parcels lists
/// its items twice.
///
/// The last line stands alone: folded into the exclusion list above it, the same
/// two rules were ignored, and a $0.00 fee line and a tax line came back as items.
const INVOICE_PROMPT: &str = "\
You transcribe online vendors' invoices and order confirmations into structured \
data.

Always include every field of the object. Use null for anything not printed or \
not readable — never omit a key, never guess, and never invent a line item.

Fields:
- merchant: the company the order was placed with, the one that took the money, \
named in the invoice header or footer. NOT the customer: \"bill to\", \"ship \
to\", \"sold to\" and the delivery address all name the buyer. NOT a \"sold by\" \
line either, which on a marketplace names a third party trading through it. One \
name, never two joined together.
- purchased_on: the order date, copied EXACTLY as printed, character for \
character. Do not convert, reorder or reformat it. Where several dates are \
printed, the one to take is the order or invoice date — not the due date or the \
payment date. Where no order date is printed at all, the ship or delivery date is \
the nearest thing to one and is what to take.
- currency: the ISO code. Invoices usually print it outright, next to an amount \
or in the payment summary; take that. Failing that the seller's address says \
which country. With nothing to go on, null.
- subtotal: the pre-tax total, the figure the tax is charged on top of, so \
shipping, fees and discounts are already inside it. An invoice may print several \
running totals before the tax: take the last one. Where the only figure printed as \
SUBTOTAL has further charges or credits listed below it, it is not this field, so \
null.
- tax: the total tax printed, however it's named — VAT, GST, HST, sales tax. Add \
nothing up: where several are printed separately and no combined figure is, null.
- total: the final amount charged, after tax and shipping.
- line_items: one entry per thing bought, in the order listed.

For each line item:
- description: the product name as printed. Leave out the order number, SKU, \
ASIN, item number and seller name that may sit around it.
- total: the amount printed at the end of that item's line. This is the field to \
fill for every item — put the printed amount here, not in unit_price.
- quantity and unit_price: the count and the each-price, where the line prints \
them beside the amount. Where a line prints both an ordered and a shipped or \
delivered count, the quantity is the one the amount was charged for. Filling these \
never means leaving total empty.

Shipping, handling, delivery and postage are line items. So are discounts, \
promotions and credits, whose total is negative. Somebody has to pay for them, so \
they cannot be dropped.

Amounts: digits with a period as the decimal separator, no currency symbol and \
no thousands separator, e.g. 1234.56.

An item listed once per parcel, or again in a summary at the end, is still one \
item — list it once. Do not create line items for subtotals, the total, order \
numbers, tracking numbers, addresses, payment methods or card digits.

Never output a line item for tax, or one whose amount is zero.";

/// All the OCR model is asked for. It has no chat template to speak of and one
/// job, so there's nothing to gain by saying more.
const OCR_PROMPT: &str = "Transcribe this receipt exactly as printed, line by line.";

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("image preparation failed: {0}")]
    Image(#[from] ImageError),
    #[error(transparent)]
    Pdf(#[from] PdfError),
    #[error(transparent)]
    Ask(#[from] ask::Error),
    #[error("model returned data that did not match the schema: {0}")]
    Deserialize(#[from] serde_json::Error),
}

/// Outcome of one extraction, including what it cost in wall-clock time: the
/// number that decides whether a given model is usable on your hardware.
#[derive(Debug, Clone)]
pub struct Extraction {
    pub receipt: ExtractedReceipt,
    /// The text the structuring model was given: a photo's OCR transcript, or a
    /// PDF's own text. `None` when one model read the photo itself.
    pub source_text: Option<String>,
    /// Verbatim output of the model that turned that text into JSON.
    ///
    /// Kept beside [`Self::source_text`] because between them they say which of
    /// the two stages got an item wrong, which is otherwise guesswork.
    pub structuring_raw: String,
    pub model: String,
    pub elapsed: Duration,
}

#[async_trait::async_trait]
pub trait ReceiptExtractor: Send + Sync {
    /// `bytes` is the original uploaded file, a photo or a PDF; the
    /// implementation is responsible for normalizing it.
    async fn extract(&self, bytes: &[u8]) -> Result<Extraction, ExtractError>;
}

#[derive(Debug, Clone)]
pub struct Config {
    pub url: String,
    pub model: String,
    /// What Ollama runs [`Self::model`] at, and `None` when it wouldn't say.
    /// Asked for rather than configured: Ollama already knows, and a number
    /// typed into the environment could disagree with it.
    pub context: Option<u32>,
    /// Transcribes the photo before [`Self::model`] structures it. Set
    /// `OLLAMA_OCR_MODEL` empty to skip that and hand the photo to [`Self::model`]
    /// instead, which then has to be able to see: worse, but one model to pull.
    pub ocr_model: Option<String>,
    /// Context window for [`Self::ocr_model`], big enough for the photo and the
    /// transcript it comes back as.
    pub ocr_context: u32,
    /// Ollama's `keep_alive` for both models. `None` leaves the server's default
    /// in place; otherwise it's a duration string, negative to never unload.
    pub keep_alive: Option<String>,
    pub max_image_edge: u32,
}

impl Config {
    /// The environment, plus the one setting only Ollama can answer for. Async
    /// because finding that out means loading the model, which the keep-alive
    /// policy wants done at startup anyway.
    pub async fn load() -> Self {
        let mut config = Self::from_env();
        let model = &config.model;

        config.context =
            ask::context_length(&config.url, model, config.keep_alive.as_deref()).await;
        match config.context {
            Some(context) => tracing::info!(%model, context, "ollama context length"),
            None => tracing::warn!(%model, "ollama would not say its context length"),
        }

        config
    }

    fn from_env() -> Self {
        use crate::server::env;

        Self {
            url: env::string("OLLAMA_URL", DEFAULT_URL),
            model: env::string("OLLAMA_MODEL", DEFAULT_MODEL),
            context: None,
            ocr_model: env::optional("OLLAMA_OCR_MODEL", DEFAULT_OCR_MODEL),
            ocr_context: env::number("OLLAMA_OCR_CONTEXT", DEFAULT_OCR_CONTEXT),
            keep_alive: env::optional("OLLAMA_KEEP_ALIVE", DEFAULT_KEEP_ALIVE),
            max_image_edge: env::number("MAX_IMAGE_EDGE", DEFAULT_MAX_EDGE),
        }
    }

    /// What produced a receipt, for the `model_used` column: both models when
    /// there are two, so a bad batch is traceable to the pair that made it.
    pub fn label(&self) -> String {
        match &self.ocr_model {
            Some(ocr) => format!("{ocr} + {}", self.model),
            None => self.model.clone(),
        }
    }
}

pub struct OllamaExtractor {
    /// Structures a till receipt read off a photo.
    ///
    /// `Agent` is generic over the completion model in rig 0.41; the Ollama model
    /// type defaults its HTTP backend to `reqwest::Client`.
    receipt: Agent<ollama::CompletionModel>,
    /// Structures a vendor's invoice read out of a PDF. Same model and schema as
    /// [`Self::receipt`], differing only in the preamble, which has to be a
    /// second agent, because rig fixes the preamble when one is built.
    invoice: Agent<ollama::CompletionModel>,
    /// Transcribes a photo for [`Self::receipt`]. Absent when there's no OCR
    /// model, in which case `receipt` reads the photo itself.
    ocr: Option<Agent<ollama::CompletionModel>>,
    config: Config,
}

impl OllamaExtractor {
    pub fn new(config: Config) -> Result<Self, ExtractError> {
        let client = ask::client(&config.url)?;
        let keep_alive = config.keep_alive.as_deref();

        let structuring = |preamble: &str| {
            ask::agent(&client, &config.model, keep_alive, ask::no_thinking())
                .preamble(preamble)
                // Maps to Ollama's `num_predict`.
                .max_tokens(MAX_OUTPUT_TOKENS)
                .output_schema_raw(ask::schema_of::<ExtractedReceipt>())
                // Native, not Tool: this becomes Ollama's `format`, a hard
                // grammar constraint. Tool mode would depend on the model
                // choosing to call a tool, which cannot be forced here.
                .output_mode(OutputMode::Native)
                .build()
        };

        // No preamble and no schema: it does one thing, and its Ollama template is
        // a bare prompt, so a system message has nowhere to go.
        let ocr = config.ocr_model.as_deref().map(|model| {
            // rig lifts `num_ctx` into Ollama's `options`.
            let options = serde_json::json!({
                "num_ctx": config.ocr_context,
                "num_gpu": OCR_GPU_LAYERS,
            });
            ask::agent(&client, model, keep_alive, options)
                .max_tokens(MAX_OUTPUT_TOKENS)
                .build()
        });

        Ok(Self {
            receipt: structuring(PROMPT),
            invoice: structuring(INVOICE_PROMPT),
            ocr,
            config,
        })
    }

    /// A photo as something [`Self::receipt`] can structure: the OCR model's
    /// transcript, or the image itself when there is no OCR model to read it.
    async fn read_photo(&self, bytes: &[u8]) -> Result<(Message, Option<String>), ExtractError> {
        let prepared = image::prepare(bytes, self.config.max_image_edge)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&prepared.jpeg);

        // Ollama accepts base64 only: rig's provider hard-errors on the `Raw`
        // and `Url` image source kinds.
        let photo = |instruction: &str| Message::User {
            content: OneOrMany::many([
                UserContent::image_base64(b64.clone(), Some(ImageMediaType::JPEG), None),
                UserContent::text(instruction),
            ])
            .expect("two content parts is never empty"),
        };

        let Some(ocr) = &self.ocr else {
            return Ok((photo("Extract every line item from this receipt."), None));
        };

        let transcript = ask::once(
            ocr,
            photo(OCR_PROMPT),
            REQUEST_TIMEOUT,
            Some(self.config.ocr_context),
        )
        .await?;
        let message = Message::user(format!("The receipt, transcribed:\n\n{transcript}"));
        Ok((message, Some(transcript)))
    }
}

#[async_trait::async_trait]
impl ReceiptExtractor for OllamaExtractor {
    async fn extract(&self, bytes: &[u8]) -> Result<Extraction, ExtractError> {
        let started = Instant::now();

        // The two paths differ in how the text is got and which agent structures
        // it. A PDF carries its own text, so the OCR model never runs: naming it
        // would credit a model that never saw this one.
        let (agent, message, source_text, model) = if pdf::is_pdf(bytes) {
            let text = pdf::text(bytes)?;
            let printed = format!("The invoice, as printed:\n\n{text}");
            // A photo needs no such check: its transcript is capped by
            // `MAX_OUTPUT_TOKENS`, so it can't outgrow what reads it.
            ask::fits(
                &self.config.url,
                &self.config.model,
                self.config.keep_alive.as_deref(),
                INVOICE_PROMPT,
                &printed,
            )
            .await?;
            (
                &self.invoice,
                Message::user(printed),
                Some(text),
                self.config.model.clone(),
            )
        } else {
            let (message, transcript) = self.read_photo(bytes).await?;
            (&self.receipt, message, transcript, self.config.label())
        };

        let structuring_raw =
            ask::once(agent, message, REQUEST_TIMEOUT, self.config.context).await?;

        Ok(Extraction {
            receipt: serde_json::from_str(&structuring_raw)?,
            source_text,
            structuring_raw,
            model,
            elapsed: started.elapsed(),
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

/// Parses one field the model wrote, noting what it couldn't read rather than
/// losing the whole receipt over it. The note names the field, so whoever
/// reviews it knows where to look.
fn parse_or_note<T>(
    raw: Option<&str>,
    label: &str,
    parse: impl FnOnce(&str) -> Option<T>,
    notes: &mut Vec<String>,
) -> Option<T> {
    // Empty is a field the grammar made the model write but the document didn't
    // give it, which is the same as one it was free to leave out.
    let raw = raw.map(str::trim).filter(|raw| !raw.is_empty())?;
    parse(raw).or_else(|| {
        notes.push(format!("could not parse {label} {raw:?}"));
        None
    })
}

impl ExtractedReceipt {
    pub fn normalize(&self) -> Normalized {
        let mut notes = Vec::new();

        let subtotal = parse_or_note(
            self.subtotal.as_deref(),
            "subtotal",
            parse::money,
            &mut notes,
        );
        let tax = parse_or_note(self.tax.as_deref(), "tax", parse::money, &mut notes);
        let total = parse_or_note(self.total.as_deref(), "total", parse::money, &mut notes);
        let purchased_on = parse_or_note(
            self.purchased_on.as_deref(),
            "date",
            parse::date,
            &mut notes,
        );

        let line_items = self
            .line_items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let mut money =
                    |raw, label: &str| parse_or_note(raw, label, parse::money, &mut notes);
                NormalizedLineItem {
                    description: item.description.trim().to_string(),
                    quantity: money(item.quantity.as_deref(), &format!("item {i} quantity")),
                    unit_price: money(item.unit_price.as_deref(), &format!("item {i} unit price")),
                    total: money(Some(&item.total), &format!("item {i} total")),
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
            warnings: notes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::dec;

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
                total: "4.99".to_string(),
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

    /// The grammar makes the model write a `total` for every item, so an amount
    /// it couldn't read arrives empty rather than absent. That's the same thing
    /// as unprinted, and must not be reported as unparseable.
    #[test]
    fn an_empty_amount_is_missing_rather_than_malformed() {
        let raw = ExtractedReceipt {
            merchant: None,
            purchased_on: None,
            currency: None,
            subtotal: None,
            tax: None,
            total: None,
            line_items: vec![ExtractedLineItem {
                description: "SMUDGED".to_string(),
                quantity: None,
                unit_price: None,
                total: "  ".to_string(),
            }],
        };

        let n = raw.normalize();
        assert_eq!(n.line_items[0].total, None);
        assert!(n.warnings.is_empty(), "got {:?}", n.warnings);
    }
}
