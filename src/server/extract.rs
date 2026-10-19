//! Receipt line-item extraction against local models via llama-server.
//!
//! Two stages, because reading a creased receipt and structuring it are different
//! skills: a small OCR model transcribes the photo, then `LLAMA_MODEL` turns
//! that text into [`ExtractedReceipt`] without ever seeing the image. Setting
//! `LLAMA_OCR_MODEL` empty collapses it back to one model reading the photo.
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
//! even when the model returned valid JSON, and a small local model can't be
//! relied on to choose to call a tool.
//!
//! Instead we drive a plain `Agent` with the schema as the request's
//! `response_format`, which llama-server compiles into a grammar: the response
//! is *guaranteed* to match the schema, and no tool-calling support is required
//! of the model. See [`ask::shaped`] for why that isn't rig's `output_schema`.

use std::time::{Duration, Instant};

use base64::Engine as _;
use rig_agent::Agent;
use rig_core::message::{ImageMediaType, Message, UserContent};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ask;
use super::image::{self, ImageError};
use super::pdf::{self, PdfError};
use crate::shared::parse;

/// llama-server's own default port. The root, not `/v1`: see [`ask::client`].
const DEFAULT_URL: &str = "http://localhost:8080";

/// What the server routes on, so these have to match its model names. The
/// context, GPU layers and residency each model runs with are the server's to
/// set.
const DEFAULT_MODEL: &str = "gemma4:12b";

/// Reads the photo, which [`DEFAULT_MODEL`] then structures. A 1.1B model that
/// only does OCR reads a creased receipt far better than a 12B generalist does:
/// every amount and barcode right in ~2s, against `2.44` coming back as `14.00`.
const DEFAULT_OCR_MODEL: &str = "glm-ocr:q8_0";

/// Downscale only, and about cost rather than accuracy: the OCR model reads this
/// receipt anywhere from 1024 to 5096 pixels. Past this it's just tiled into more
/// pieces: 2.1s here against 10s at native, for the same text. Below it the
/// transcript gets noisier, fenced and repeating, which lost a subtotal at 1024.
const DEFAULT_MAX_EDGE: u32 = 1600;

/// The `max_tokens` of every request, for the transcript as much as the JSON.
/// Measured at ~40 output tokens per line item plus ~130 of fixed JSON
/// overhead, so this covers a long grocery receipt with room to spare. It
/// exists mainly as a backstop: without a cap, a model that fails to terminate
/// blocks the request indefinitely instead of failing. Hitting it fails the
/// read rather than keeping what was written: see [`ask::Error::Truncated`].
const MAX_OUTPUT_TOKENS: u64 = 2048;

/// Wall-clock ceiling for one request, so a two-stage extraction can take twice
/// it. Decode measured at ~32 tok/s, so a full `MAX_OUTPUT_TOKENS` response is
/// ~65s; the rest is headroom for the request queueing behind other work on the
/// same server.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// What the model is asked to produce.
///
/// Every money and date field comes back a **string**, deliberately. A local
/// model will write "$4.99", "4,99" or "1 234.56", and a typed `number`/`Decimal`
/// schema would either reject the whole payload or silently coerce through a
/// float. Strings move parsing somewhere we can report on it field by field: see
/// [`ExtractedReceipt::normalize`].
///
/// The model may still *write* an amount as a bare number: see [`Amount`].
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
    pub subtotal: Option<Amount>,
    pub tax: Option<Amount>,
    /// The final amount charged.
    pub total: Option<Amount>,
    pub line_items: Vec<ExtractedLineItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractedLineItem {
    /// Item name as printed on the receipt.
    pub description: String,
    pub quantity: Option<Amount>,
    pub unit_price: Option<Amount>,
    /// Line total. Negative for discounts and coupons.
    ///
    /// Not an `Option`, which is what puts it in the nested object's `required`
    /// list; [`ask::schema_of`] only does that for the top level. Left optional,
    /// the grammar lets the model omit the key, and on an invoice it omitted
    /// this one on every line. An amount it can't read comes back empty.
    pub total: Amount,
}

/// An amount as the model wrote it, a string or a bare number.
///
/// gemma4:12b writes amounts as JSON numbers whatever the prompt says. Under a
/// string-only grammar that leaves it a second choice, and on llama-server that
/// choice was `null`: a fifth of receipts came back with no total, against 6%
/// on Ollama, which took the quote instead and sometimes wrote junk inside it
/// (`"}}17.37"`). Allowing the number takes away the fight.
///
/// Either way it's kept as text. A number comes back through `f64`, whose
/// shortest round trip is exact for anything with fewer than 16 significant
/// digits, which every amount on a receipt has.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Amount(pub String);

impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Written {
            Text(String),
            Number(serde_json::Number),
        }

        Ok(Self(match Written::deserialize(d)? {
            Written::Text(text) => text,
            Written::Number(number) => number.to_string(),
        }))
    }
}

impl std::ops::Deref for Amount {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl JsonSchema for Amount {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Amount".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": ["string", "number"] })
    }
}

/// The field semantics have to live here, not in the struct's doc comments:
/// the server compiles the JSON schema into a GBNF grammar and drops `description`
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
- subtotal, tax and total again: one document can hold several invoices, each \
with its own invoice number and its own total. Then none of them covers the whole \
order, so these three are null. Do not add the invoices' figures together.
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
    /// llama-server's root, routing to either model by name.
    pub url: String,
    pub model: String,
    /// Transcribes the photo before [`Self::model`] structures it. Set
    /// `LLAMA_OCR_MODEL` empty to skip that and hand the photo to [`Self::model`]
    /// instead, which then has to be able to see: worse, but one model to serve.
    pub ocr_model: Option<String>,
    pub max_image_edge: u32,
}

impl Config {
    pub fn from_env() -> Self {
        use crate::server::env;

        Self {
            url: env::string("LLAMA_URL", DEFAULT_URL),
            model: env::string("LLAMA_MODEL", DEFAULT_MODEL),
            ocr_model: env::optional("LLAMA_OCR_MODEL", DEFAULT_OCR_MODEL),
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

pub struct LlamaExtractor {
    /// Structures a till receipt read off a photo.
    receipt: Agent,
    /// Structures a vendor's invoice read out of a PDF. Same model and schema as
    /// [`Self::receipt`], differing only in the preamble, which has to be a
    /// second agent, because rig fixes the preamble when one is built.
    invoice: Agent,
    /// Transcribes a photo for [`Self::receipt`]. Absent when there's no OCR
    /// model, in which case `receipt` reads the photo itself.
    ocr: Option<Agent>,
    config: Config,
}

impl LlamaExtractor {
    pub fn new(config: Config) -> Self {
        let client = ask::client(&config.url);

        let structuring = |preamble: &str| {
            ask::agent(&client, &config.model)
                .additional_params(ask::shaped::<ExtractedReceipt>())
                .preamble(preamble)
                .max_tokens(MAX_OUTPUT_TOKENS)
                .build()
        };

        // No preamble and no schema: it does one thing, and its chat template is
        // a bare prompt, so a system message has nowhere to go.
        let ocr = config.ocr_model.as_deref().map(|model| {
            ask::agent(&client, model)
                .max_tokens(MAX_OUTPUT_TOKENS)
                .build()
        });

        Self {
            receipt: structuring(PROMPT),
            invoice: structuring(INVOICE_PROMPT),
            ocr,
            config,
        }
    }

    /// A photo as something [`Self::receipt`] can structure: the OCR model's
    /// transcript, or the image itself when there is no OCR model to read it.
    async fn read_photo(&self, bytes: &[u8]) -> Result<(Message, Option<String>), ExtractError> {
        let prepared = image::prepare(bytes, self.config.max_image_edge)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&prepared.jpeg);

        // Sent inline as a data URI, which needs the media type said.
        let photo = |instruction: &str| Message::User {
            content: vec![
                UserContent::image_base64(b64.clone(), Some(ImageMediaType::JPEG), None),
                UserContent::text(instruction),
            ],
        };

        let Some(ocr) = &self.ocr else {
            return Ok((photo("Extract every line item from this receipt."), None));
        };

        let transcript = ask::once(ocr, photo(OCR_PROMPT), REQUEST_TIMEOUT).await?;
        let message = Message::user(format!("The receipt, transcribed:\n\n{transcript}"));
        Ok((message, Some(transcript)))
    }
}

#[async_trait::async_trait]
impl ReceiptExtractor for LlamaExtractor {
    async fn extract(&self, bytes: &[u8]) -> Result<Extraction, ExtractError> {
        let started = Instant::now();

        // The two paths differ in how the text is got and which agent structures
        // it. A PDF carries its own text, so the OCR model never runs: naming it
        // would credit a model that never saw this one.
        let (agent, message, source_text, model) = if pdf::is_pdf(bytes) {
            let text = pdf::text(bytes)?;
            (
                &self.invoice,
                Message::user(format!("The invoice, as printed:\n\n{text}")),
                Some(text),
                self.config.model.clone(),
            )
        } else {
            let (message, transcript) = self.read_photo(bytes).await?;
            (&self.receipt, message, transcript, self.config.label())
        };

        let structuring_raw = ask::once(agent, message, REQUEST_TIMEOUT).await?;

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
    /// `today` settles an ambiguous date: see [`parse::date_not_after`].
    pub fn normalize(&self, today: jiff::civil::Date) -> Normalized {
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
            |raw| parse::date_not_after(raw, today),
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
                    total: money(Some(&*item.total), &format!("item {i} total")),
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
            tax: Some(Amount("about three dollars".into())),
            total: Some(Amount("$12.34".into())),
            line_items: vec![ExtractedLineItem {
                description: " MILK 2% ".to_string(),
                quantity: None,
                unit_price: None,
                total: Amount("4.99".into()),
            }],
        };

        let n = raw.normalize(jiff::civil::date(2026, 7, 20));
        assert_eq!(n.merchant.as_deref(), Some("Walmart"));
        assert_eq!(n.currency.as_deref(), Some("USD"));
        assert_eq!(n.total, Some(dec("12.34")));
        assert_eq!(n.purchased_on, None);
        assert_eq!(n.tax, None);
        assert_eq!(n.line_items[0].description, "MILK 2%");
        assert_eq!(n.line_items[0].total, Some(dec("4.99")));
        assert_eq!(n.warnings.len(), 2, "bad date and bad tax should warn");
    }

    /// The model writes an amount as a number as readily as a string, and both
    /// have to arrive as the same text.
    #[test]
    fn an_amount_reads_as_a_string_or_a_number() {
        let raw: ExtractedReceipt = serde_json::from_str(
            r#"{"merchant":null,"purchased_on":null,"currency":null,
                "subtotal":3.95,"tax":"0.28","total":null,
                "line_items":[{"description":"COFFEE","quantity":2,"unit_price":null,"total":-1.5}]}"#,
        )
        .unwrap();

        assert_eq!(raw.subtotal.as_deref(), Some("3.95"));
        assert_eq!(raw.tax.as_deref(), Some("0.28"));
        assert_eq!(raw.total, None);
        assert_eq!(raw.line_items[0].quantity.as_deref(), Some("2"));
        assert_eq!(&*raw.line_items[0].total, "-1.5");
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
                total: Amount("  ".into()),
            }],
        };

        let n = raw.normalize(jiff::civil::date(2026, 7, 20));
        assert_eq!(n.line_items[0].total, None);
        assert!(n.warnings.is_empty(), "got {:?}", n.warnings);
    }
}
