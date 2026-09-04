//! Talking to Ollama: the client, the per-request options, one bounded turn, and
//! the one question rig has no wrapper for.
//!
//! Reading a receipt and working out whose the items are both go through here, so
//! the failure modes and the residency policy are settled in one place.

use std::time::Duration;

use rig_agent::agent::{Agent, AgentBuilder};
use rig_agent::prelude::*;
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
        "the document is {used} tokens against a context of {limit}, so it would be read with \
         its middle missing. Raise the context window for this model."
    )]
    TooLong { used: u64, limit: u32 },
    #[error(
        "the model's context filled: {used} tokens against a limit of {limit}, so the answer was \
         cut short. Raise the context window for this model."
    )]
    ContextFull { used: u64, limit: u32 },
    #[error(
        "model returned no content: if the model is thinking-capable, reasoning tokens may have \
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

/// An agent against one Ollama model, greedy and staying loaded. The caller adds
/// what makes it its own: a preamble, a schema, a token ceiling.
pub fn agent(
    client: &ollama::Client,
    model: &str,
    keep_alive: Option<&str>,
    mut options: serde_json::Value,
) -> AgentBuilder<ollama::CompletionModel> {
    // `keep_alive` restarts the clock per call rather than sticking to the loaded
    // model, so it has to ride along on every request.
    if let Some(keep_alive) = keep_alive {
        options["keep_alive"] = serde_json::Value::String(keep_alive.to_string());
    }

    client
        .agent(model)
        // At the default temperature about a third of gemma4:12b runs came back
        // empty, all null, or with junk inside a string field. The grammar
        // constrains the shape, not what goes in a string.
        .temperature(0.0)
        .additional_params(options)
}

/// What Ollama is actually running `model` at, so a response's token count has a
/// ceiling to be checked against.
///
/// Only `/api/ps` knows the number in force, and only for a loaded model, so
/// this preloads it with an empty prompt first. `/api/show` answers a different
/// question: the context the model was *trained* for, 262144 for gemma4 against
/// the 4096 its runner gets.
///
/// `None` when Ollama can't be reached or says nothing useful, which leaves the
/// caller without a ceiling rather than refusing to work.
pub async fn context_length(url: &str, model: &str, keep_alive: Option<&str>) -> Option<u32> {
    /// Long enough to load a 12B model off a cold disk, short enough that an
    /// Ollama at an unroutable address doesn't hold up the whole startup.
    const PRELOAD_TIMEOUT: Duration = Duration::from_secs(60);

    let url = url.trim_end_matches('/');
    let http = reqwest::Client::builder()
        .timeout(PRELOAD_TIMEOUT)
        .build()
        .ok()?;

    let mut preload = serde_json::json!({ "model": model });
    if let Some(keep_alive) = keep_alive {
        preload["keep_alive"] = keep_alive.into();
    }
    http.post(format!("{url}/api/generate"))
        .json(&preload)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;

    let running: serde_json::Value = http
        .get(format!("{url}/api/ps"))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    running["models"]
        .as_array()?
        .iter()
        .find(|loaded| loaded["model"] == model)?["context_length"]
        .as_u64()?
        .try_into()
        .ok()
}

/// Whether Ollama will take this prompt whole.
///
/// It otherwise trims an over-long one to about half the context, keeps the
/// tail, and answers as though nothing happened: well-formed JSON describing a
/// document with its head missing, which nothing downstream can tell from a good
/// read. `truncate` makes it refuse and say how long the prompt was.
///
/// A bare request because rig promotes only `think` and `keep_alive` out of the
/// options object, so a top-level field can't go through it. Nothing is
/// generated, and the real request then finds the prompt in Ollama's cache.
pub async fn fits(
    url: &str,
    model: &str,
    keep_alive: Option<&str>,
    system: &str,
    user: &str,
) -> Result<(), Error> {
    /// Long enough to count a large document, short enough that a wedged Ollama
    /// fails here rather than holding the upload open.
    const FIT_TIMEOUT: Duration = Duration::from_secs(30);

    let mut body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
        "stream": false,
        "truncate": false,
        // Changes the template, so it changes the length.
        "think": false,
        "options": { "num_predict": 1 },
    });
    if let Some(keep_alive) = keep_alive {
        body["keep_alive"] = keep_alive.into();
    }

    let http = reqwest::Client::builder()
        .timeout(FIT_TIMEOUT)
        .build()
        .map_err(|e| Error::Prompt(e.to_string()))?;
    let response = http
        .post(format!("{}/api/chat", url.trim_end_matches('/')))
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Prompt(e.to_string()))?;

    if response.status().is_success() {
        return Ok(());
    }

    // Anything else that went wrong is left to the real request, a moment away,
    // rather than made into a second way for an upload to fail.
    match refused(&response.text().await.unwrap_or_default()) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The refusal's numbers, which Ollama passes on as a JSON string inside its own
/// JSON, so they sit one parse further down.
fn refused(body: &str) -> Option<Error> {
    let response: serde_json::Value = serde_json::from_str(body).ok()?;
    let refusal: serde_json::Value = serde_json::from_str(response["error"].as_str()?).ok()?;

    let refusal = &refusal["error"];
    Some(Error::TooLong {
        used: refusal["n_prompt_tokens"].as_u64()?,
        limit: refusal["n_ctx"].as_u64()?.try_into().ok()?,
    })
}

/// Thinking off, for [`agent`]'s options.
///
/// Ollama turns it on by default for gemma4:12b, and the reasoning ate the whole
/// token budget: 47s of it, then empty content and what looked like a parse
/// error. Only for models that have it: Ollama rejects the field on ones that
/// don't, which is why the OCR model goes without.
pub fn no_thinking() -> serde_json::Value {
    serde_json::json!({ "think": false })
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

/// One turn, bounded, with the failure modes kept apart. `context` is the
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

    // Ollama trims an oversized prompt and cuts an overlong answer, reporting
    // neither: the tell is the token count landing on the limit. Refused rather
    // than returned, because what comes back is a plausible document with its
    // middle missing, which nothing downstream can catch. Zero means Ollama sent
    // no count.
    let used = response.usage.total_tokens;
    if let Some(limit) = context
        && used > 0
        && used >= u64::from(limit)
    {
        return Err(Error::ContextFull { used, limit });
    }

    // Distinguish "no content" from "bad JSON": very different causes, and an
    // empty body otherwise surfaces as "expected value at line 1 column 1".
    if response.output.trim().is_empty() {
        return Err(Error::EmptyResponse);
    }
    Ok(response.output)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape Ollama refuses an over-long prompt in, verbatim off the wire.
    #[test]
    fn a_refusal_reports_the_length_and_the_context() {
        let body = r#"{"error":"{\"error\":{\"code\":400,\"message\":\"request (36742 tokens) exceeds the available context size (32768 tokens), try increasing it\",\"type\":\"exceed_context_size_error\",\"n_prompt_tokens\":36742,\"n_ctx\":32768}}"}"#;
        let Some(Error::TooLong { used, limit }) = refused(body) else {
            panic!("not read as a refusal: {:?}", refused(body));
        };
        assert_eq!((used, limit), (36742, 32768));
    }

    /// Everything else is somebody else's problem, and must not be read as one.
    #[test]
    fn an_unrelated_error_is_left_alone() {
        assert!(refused(r#"{"error":"model 'nope' not found"}"#).is_none());
        assert!(refused("not json at all").is_none());
    }
}
