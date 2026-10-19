//! Talking to llama-server: the client, the per-request options, and one
//! bounded turn.
//!
//! Reading a receipt and working out whose the items are both go through here, so
//! the failure modes are settled in one place.

use std::time::Duration;

use rig_agent::{Agent, AgentBuilder};
use rig_core::completion::FinishReason;
use rig_core::message::Message;
use rig_core::providers::llamacpp::extension::LlamaCppOptions;
use rig_core::providers::openai::{self, OpenAIConfig, wire::LLAMACPP};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("model request failed: {0}")]
    Prompt(String),
    #[error("the model did not respond within {0:?}")]
    Timeout(Duration),
    #[error(
        "the document is {used} tokens against a context of {limit}, so it would be read with \
         its middle missing. Raise the context window for this model."
    )]
    TooLong { used: u64, limit: u32 },
    #[error("the answer was cut short: the model reached its output limit or filled its context")]
    Truncated,
    #[error(
        "model returned no content: if the model is thinking-capable, reasoning tokens may have \
         consumed the whole generation budget"
    )]
    EmptyResponse,
}

/// A client for the llama-server at `url`, its root rather than `/v1`. No key:
/// a server started without one rejects a request that carries one.
pub fn client(url: &str) -> openai::OpenAI {
    OpenAIConfig::with_key(&LLAMACPP, "")
        .with_base_url(format!("{}/v1", url.trim_end_matches('/')))
        .client()
}

/// An agent against one model, greedy and not thinking. The caller adds what
/// makes it its own: a preamble, a token ceiling, [`shaped`].
pub fn agent(client: &openai::OpenAI, model: &str) -> AgentBuilder {
    AgentBuilder::new(client.chat(model))
        // At the default temperature about a third of gemma4:12b runs came back
        // empty, all null, or with junk inside a string field. The grammar
        // constrains the shape, not what goes in a string.
        .temperature(0.0)
        // gemma4:12b thinks by default, which eats the token budget. A template
        // without the switch ignores it.
        .provider_option(
            LlamaCppOptions::new().chat_template_kwarg("enable_thinking", false.into()),
        )
}

/// The request parameters that hold an answer to the shape of `T`, for an
/// agent's `additional_params`.
///
/// Our own `response_format` rather than rig's `output_schema`, which rewrites
/// the schema to require every field of every nested object. See [`schema_of`]
/// for why only the top level is.
pub fn shaped<T: schemars::JsonSchema>() -> serde_json::Value {
    serde_json::json!({
        "response_format": {
            "type": "json_schema",
            "json_schema": { "name": "response", "schema": schema_of::<T>() },
        },
    })
}

/// llama-server's refusal of a prompt longer than the context, which it sends
/// rather than trimming the prompt and answering as though nothing happened.
fn refused(body: &serde_json::Value) -> Option<Error> {
    let refusal = &body["error"];
    if refusal["type"] != "exceed_context_size_error" {
        return None;
    }
    Some(Error::TooLong {
        used: refusal["n_prompt_tokens"].as_u64()?,
        limit: refusal["n_ctx"].as_u64()?.try_into().ok()?,
    })
}

/// The JSON schema for `T`, with every field of the top-level object required.
///
/// schemars leaves an `Option` out of `required`, and the server turns the
/// schema into a grammar that then lets the model skip the key. It takes that
/// way out for exactly the fields worth having: it omitted `currency` rather
/// than work out which country the receipt came from, while writing
/// `"subtotal": null` next to it. Null is still available, it just has to be
/// said.
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

/// One turn, bounded, with the failure modes kept apart.
pub async fn once(agent: &Agent, message: Message, timeout: Duration) -> Result<String, Error> {
    let response = tokio::time::timeout(timeout, agent.prompt(message).max_turns(1).run())
        .await
        .map_err(|_| Error::Timeout(timeout))?
        .map_err(|e| {
            e.provider_response_json()
                .ok()
                .flatten()
                .and_then(|body| refused(&body))
                .unwrap_or_else(|| Error::Prompt(e.to_string()))
        })?;

    // rig hands back whatever was written before the cut: a plausible document
    // with its middle missing, which nothing downstream can catch. Refused
    // rather than returned.
    let stopped = response
        .completion_calls
        .last()
        .and_then(|call| call.finish_reason.as_ref());
    if stopped == Some(&FinishReason::Length) {
        return Err(Error::Truncated);
    }

    // Distinguish "no content" from "bad JSON": very different causes, and an
    // empty body otherwise surfaces as "expected value at line 1 column 1".
    let output = response.output();
    if output.trim().is_empty() {
        return Err(Error::EmptyResponse);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape llama-server refuses an over-long prompt in, verbatim off the
    /// wire.
    #[test]
    fn a_refusal_reports_the_length_and_the_context() {
        let body = serde_json::json!({"error":{"code":400,"message":"request (36742 tokens) exceeds the available context size (32768 tokens), try increasing it","type":"exceed_context_size_error","n_prompt_tokens":36742,"n_ctx":32768}});
        let Some(Error::TooLong { used, limit }) = refused(&body) else {
            panic!("not read as a refusal: {:?}", refused(&body));
        };
        assert_eq!((used, limit), (36742, 32768));
    }

    /// Everything else is somebody else's problem, and must not be read as one.
    #[test]
    fn an_unrelated_error_is_left_alone() {
        let missing = serde_json::json!({"error":{"code":404,"message":"model not found","type":"not_found_error"}});
        assert!(refused(&missing).is_none());
        assert!(refused(&serde_json::json!("not an object")).is_none());
    }
}
