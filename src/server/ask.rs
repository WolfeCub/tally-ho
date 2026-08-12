//! Talking to Ollama: the client, the per-request options, and one bounded turn.
//!
//! Reading a receipt and working out whose the items are both go through here, so
//! the two failure modes and the residency policy are settled in one place.

use std::time::Duration;

use rig_agent::agent::Agent;
use rig_core::client::Nothing;
use rig_core::message::Message;
use rig_core::providers::ollama;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("ollama request failed: {0}")]
    Prompt(String),
    #[error("ollama did not respond within {0:?}")]
    Timeout(Duration),
    #[error(
        "model returned no content — if the model is thinking-capable, reasoning tokens may have \
         consumed the whole generation budget"
    )]
    EmptyResponse,
}

/// Ollama needs no auth; `OllamaApiKey` has a `From<Nothing>` impl for exactly
/// this, and `build()` won't accept the unset state.
pub fn client(url: &str) -> Result<ollama::Client, Error> {
    ollama::Client::builder()
        .api_key(Nothing)
        .base_url(url)
        .build()
        .map_err(|e| Error::Prompt(e.to_string()))
}

/// `options` with the residency policy added, which has to ride along on every
/// request: `keep_alive` restarts the clock per call rather than being a property
/// of the loaded model, so sending it once wouldn't hold.
pub fn options(keep_alive: Option<&str>, mut options: serde_json::Value) -> serde_json::Value {
    if let Some(keep_alive) = keep_alive {
        options["keep_alive"] = serde_json::Value::String(keep_alive.to_string());
    }
    options
}

/// The JSON schema for `T`, with every field of the top-level object required.
///
/// schemars leaves an `Option` out of `required`, and Ollama turns the schema
/// into a grammar that then lets the model skip the key. It takes that way out
/// for exactly the fields worth having: it omitted `currency` rather than work
/// out which country the receipt came from, while writing `"subtotal": null`
/// next to it. Null is still available, it just has to be said.
///
/// Only the top level, because doing this to the nested objects too made things
/// worse: a line item's quantity is null on nearly every line, and having to
/// write that out just before the amount had the model carry the null straight
/// on into it. Every price on a Walmart receipt came back null that way.
pub fn schema_of<T: schemars::JsonSchema>() -> schemars::Schema {
    let mut schema = schemars::generate::SchemaGenerator::default().into_root_schema_for::<T>();
    if let Some(fields) = schema.get("properties").and_then(|p| p.as_object()) {
        let names: Vec<String> = fields.keys().cloned().collect();
        schema.insert("required".to_string(), names.into());
    }
    schema
}

/// One turn, bounded, with the two failure modes kept apart. `context` is the
/// ceiling we set on this agent, where we set one.
pub async fn once(
    agent: &Agent<ollama::CompletionModel>,
    message: Message,
    timeout: Duration,
    context: Option<u32>,
) -> Result<String, Error> {
    let response = tokio::time::timeout(timeout, agent.runner(message).max_turns(1).run())
        .await
        .map_err(|_| Error::Timeout(timeout))?
        .map_err(|e| Error::Prompt(e.to_string()))?;

    // A filled context cuts the answer off mid-way and reports no error; the tell
    // is the token count landing on the limit. Zero means Ollama sent no count.
    let used = response.usage.total_tokens;
    if used > 0 && context.is_some_and(|limit| used >= u64::from(limit)) {
        tracing::error!(
            tokens = used,
            "the context filled, so the answer stops part way — raise it"
        );
    }

    // Distinguish "no content" from "bad JSON" — very different causes, and an
    // empty body otherwise surfaces as "expected value at line 1 column 1".
    if response.output.trim().is_empty() {
        return Err(Error::EmptyResponse);
    }
    Ok(response.output)
}
