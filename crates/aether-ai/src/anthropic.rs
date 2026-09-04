//! Anthropic Claude provider.
//!
//! Rust has no official Anthropic SDK, so this talks to the Messages API over
//! raw HTTPS (`POST https://api.anthropic.com/v1/messages`) per the documented
//! wire format: `x-api-key` + `anthropic-version: 2023-06-01` headers, a
//! `{model, max_tokens, system, messages}` body, and a response whose answer is
//! the first `text` block of `content`. The real request is compiled only with
//! `--features live-providers`; the default build returns [`AiError::Unsupported`]
//! so the swarm transparently falls back to the offline mock.

use crate::provider::{AiError, AiProvider, Completion, Prompt, TaskClass};
use async_trait::async_trait;

/// Default model. Override with `ANTHROPIC_MODEL` or
/// [`AnthropicProvider::with_model`].
const DEFAULT_MODEL: &str = "claude-opus-5";
#[cfg(feature = "live-providers")]
const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
#[cfg(feature = "live-providers")]
const API_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    api_key: Option<String>,
    /// The Anthropic model id used on the wire.
    model: String,
}

impl AnthropicProvider {
    /// Reads `ANTHROPIC_API_KEY` and optional `ANTHROPIC_MODEL` if present.
    pub fn from_env() -> Self {
        AnthropicProvider {
            api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            model: std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string()),
        }
    }

    /// Override the model id.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }
}

// ---- wire types (only needed for the live build) ----
#[cfg(feature = "live-providers")]
#[derive(serde::Serialize)]
struct Request<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<WireMessage<'a>>,
}

#[cfg(feature = "live-providers")]
#[derive(serde::Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[cfg(feature = "live-providers")]
#[derive(serde::Deserialize)]
struct Response {
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    usage: Usage,
}

#[cfg(feature = "live-providers")]
#[derive(serde::Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

#[cfg(feature = "live-providers")]
#[derive(serde::Deserialize, Default)]
struct Usage {
    #[serde(default)]
    output_tokens: u32,
}

#[async_trait]
impl AiProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic:claude"
    }

    fn handles(&self, _class: TaskClass) -> bool {
        // Without a key (or without the live build) it can't serve anything, so
        // the router skips it and falls through to the mock.
        cfg!(feature = "live-providers") && self.api_key.is_some()
    }

    #[cfg(feature = "live-providers")]
    async fn complete(&self, prompt: Prompt) -> Result<Completion, AiError> {
        let key = self.api_key.as_ref().ok_or_else(|| {
            AiError::Unsupported("anthropic:claude (no ANTHROPIC_API_KEY)".into())
        })?;

        // EXTENSION POINT: `prompt.response_schema` is not wired here. The
        // Messages API has no `response_format`/json-schema parameter like
        // Ollama's `format` or OpenAI's `text.format`; grammar-constraining
        // Anthropic would require forced tool-use (`tool_choice: {type:
        // "tool", ...}` with an `input_schema`, then reading the answer off
        // a `tool_use` content block instead of a `text` block) — a
        // different response-parsing path, deferred until a caller actually
        // needs Anthropic in the loop. `girder do` still lists Anthropic in
        // its provider chain; it just gets a best-effort free-text
        // completion here rather than a grammar-guaranteed one, same as
        // before this field existed.
        let body = Request {
            model: &self.model,
            max_tokens: prompt.max_tokens.max(1),
            system: &prompt.system,
            messages: vec![WireMessage {
                role: "user",
                content: &prompt.user,
            }],
        };

        let resp = reqwest::Client::new()
            .post(ENDPOINT)
            .header("x-api-key", key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Transport(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            return Err(AiError::Provider(format!("{status}: {detail}")));
        }

        let parsed: Response = resp
            .json()
            .await
            .map_err(|e| AiError::Transport(e.to_string()))?;

        let text = parsed
            .content
            .into_iter()
            .find(|b| b.kind == "text")
            .map(|b| b.text)
            .unwrap_or_default();

        Ok(Completion {
            text,
            model: if parsed.model.is_empty() {
                self.model.clone()
            } else {
                parsed.model
            },
            tokens: parsed.usage.output_tokens,
        })
    }

    #[cfg(not(feature = "live-providers"))]
    async fn complete(&self, _prompt: Prompt) -> Result<Completion, AiError> {
        Err(AiError::Unsupported(
            "anthropic:claude (build with --features live-providers)".into(),
        ))
    }
}
