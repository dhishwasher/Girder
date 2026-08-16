//! Ollama provider — local models through the `/api/chat` endpoint.
//!
//! Ollama is opt-in: `OLLAMA_HOST` must name the server base URL and the crate
//! must be built with `--features live-providers`. Default builds never route
//! to Ollama or perform network I/O.

use crate::provider::{AiError, AiProvider, Completion, Prompt, TaskClass};
use async_trait::async_trait;
#[cfg(any(feature = "live-providers", test))]
use serde::{Deserialize, Serialize};

const DEFAULT_MODEL: &str = "llama3.1";
/// Falls back to this when `OLLAMA_TIMEOUT_SECS` is unset, blank, zero, or
/// unparseable. A live run against `qwen2.5-coder:1.5b` sat past an hour with
/// no timeout at all (no request-level deadline was ever set), so this exists
/// to guarantee the request is eventually cancelled and surfaces as a normal
/// provider error the router can escalate past, not a hang.
const DEFAULT_TIMEOUT_SECS: u64 = 600;

pub struct OllamaProvider {
    host: Option<String>,
    model: String,
    timeout_secs: u64,
}

#[cfg(any(feature = "live-providers", test))]
#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    messages: Vec<OllamaRequestMessage<'a>>,
    stream: bool,
    options: OllamaOptions,
    /// Grammar-constrains decoding to this JSON Schema via `/api/chat`'s
    /// `format` field — the mechanism `tools/plan_executor_oracle.py`
    /// validated for authoring-cost measurement. Omitted entirely when no
    /// schema was requested, so every existing caller is unaffected.
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<&'a serde_json::Value>,
}

#[cfg(any(feature = "live-providers", test))]
#[derive(Serialize)]
struct OllamaRequestMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[cfg(any(feature = "live-providers", test))]
#[derive(Serialize)]
struct OllamaOptions {
    num_predict: u32,
}

#[cfg(any(feature = "live-providers", test))]
#[derive(Deserialize)]
struct OllamaResponse {
    #[serde(default)]
    model: String,
    message: OllamaResponseMessage,
    #[serde(default)]
    eval_count: u32,
}

#[cfg(any(feature = "live-providers", test))]
#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

impl OllamaProvider {
    /// Reads the Ollama base URL and optional model selection from the
    /// environment. An absent or blank `OLLAMA_HOST` leaves the provider
    /// disabled so the router can continue to its next candidate.
    pub fn from_env() -> Self {
        OllamaProvider {
            host: std::env::var("OLLAMA_HOST")
                .ok()
                .and_then(|host| normalize_host(&host)),
            model: std::env::var("OLLAMA_MODEL")
                .ok()
                .filter(|model| !model.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            timeout_secs: parse_timeout_secs(std::env::var("OLLAMA_TIMEOUT_SECS").ok().as_deref()),
        }
    }

    pub fn with_host(mut self, host: impl AsRef<str>) -> Self {
        self.host = normalize_host(host.as_ref());
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_timeout_secs(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = timeout_secs;
        self
    }

    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    #[cfg(any(feature = "live-providers", test))]
    fn endpoint(&self) -> Option<String> {
        self.host.as_ref().map(|host| format!("{host}/api/chat"))
    }
}

/// Pure so it's testable without mutating process-global env state (tests
/// run in parallel and would otherwise race on `std::env::set_var`).
fn parse_timeout_secs(value: Option<&str>) -> u64 {
    value
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

fn normalize_host(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('/');
    if host.is_empty() {
        return None;
    }
    if host.starts_with("http://") || host.starts_with("https://") {
        Some(host.to_string())
    } else {
        Some(format!("http://{host}"))
    }
}

#[cfg(any(feature = "live-providers", test))]
fn request_body<'a>(model: &'a str, prompt: &'a Prompt) -> OllamaRequest<'a> {
    let mut messages = Vec::with_capacity(2);
    if !prompt.system.is_empty() {
        messages.push(OllamaRequestMessage {
            role: "system",
            content: &prompt.system,
        });
    }
    messages.push(OllamaRequestMessage {
        role: "user",
        content: &prompt.user,
    });
    OllamaRequest {
        model,
        messages,
        stream: false,
        options: OllamaOptions {
            num_predict: prompt.max_tokens.max(1),
        },
        format: prompt.response_schema.as_ref(),
    }
}

#[async_trait]
impl AiProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama:local"
    }

    fn handles(&self, _class: TaskClass) -> bool {
        cfg!(feature = "live-providers") && self.host.is_some()
    }

    #[cfg(feature = "live-providers")]
    async fn complete(&self, prompt: Prompt) -> Result<Completion, AiError> {
        let endpoint = self.endpoint().ok_or_else(|| {
            AiError::Unsupported("ollama:local (set OLLAMA_HOST to the server base URL)".into())
        })?;
        let response = reqwest::Client::new()
            .post(endpoint)
            .timeout(std::time::Duration::from_secs(self.timeout_secs))
            .json(&request_body(&self.model, &prompt))
            .send()
            .await
            .map_err(|error| AiError::Transport(error.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            return Err(AiError::Provider(format!("{status}: {detail}")));
        }
        let parsed: OllamaResponse = response
            .json()
            .await
            .map_err(|error| AiError::Transport(error.to_string()))?;
        Ok(Completion {
            text: parsed.message.content,
            model: if parsed.model.is_empty() {
                self.model.clone()
            } else {
                parsed.model
            },
            tokens: parsed.eval_count,
        })
    }

    #[cfg(not(feature = "live-providers"))]
    async fn complete(&self, _prompt: Prompt) -> Result<Completion, AiError> {
        Err(AiError::Unsupported(
            "ollama:local (build with --features live-providers and set OLLAMA_HOST)".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured_provider() -> OllamaProvider {
        OllamaProvider {
            host: Some("http://127.0.0.1:11434".to_string()),
            model: "test-model".to_string(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }

    #[test]
    fn host_is_a_normalized_base_url() {
        let provider = OllamaProvider {
            host: None,
            model: "test-model".to_string(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
        .with_host(" 127.0.0.1:11434/ ");
        assert_eq!(provider.host(), Some("http://127.0.0.1:11434"));
        assert_eq!(
            provider.endpoint().as_deref(),
            Some("http://127.0.0.1:11434/api/chat")
        );
    }

    #[test]
    fn blank_host_disables_the_provider() {
        let provider = OllamaProvider {
            host: None,
            model: "test-model".to_string(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
        .with_host("  ");
        assert_eq!(provider.host(), None);
        assert!(!provider.handles(TaskClass::Quick));
    }

    #[test]
    fn timeout_secs_falls_back_to_default_when_unset_zero_or_unparseable() {
        assert_eq!(parse_timeout_secs(None), DEFAULT_TIMEOUT_SECS);
        assert_eq!(parse_timeout_secs(Some("")), DEFAULT_TIMEOUT_SECS);
        assert_eq!(parse_timeout_secs(Some("0")), DEFAULT_TIMEOUT_SECS);
        assert_eq!(
            parse_timeout_secs(Some("not-a-number")),
            DEFAULT_TIMEOUT_SECS
        );
    }

    #[test]
    fn timeout_secs_honors_a_positive_override() {
        assert_eq!(parse_timeout_secs(Some("120")), 120);
        assert_eq!(parse_timeout_secs(Some(" 45 ")), 45);
    }

    #[test]
    fn with_timeout_secs_overrides_the_configured_value() {
        let provider = configured_provider().with_timeout_secs(30);
        assert_eq!(provider.timeout_secs, 30);
    }

    #[test]
    fn request_uses_chat_messages_and_output_limit() {
        let mut prompt = Prompt::new(TaskClass::Codegen, "be exact", "write a function");
        prompt.max_tokens = 0;
        let value = serde_json::to_value(request_body("coder", &prompt)).unwrap();
        assert_eq!(value["model"], "coder");
        assert_eq!(value["stream"], false);
        assert_eq!(value["options"]["num_predict"], 1);
        assert_eq!(value["messages"][0]["role"], "system");
        assert_eq!(value["messages"][0]["content"], "be exact");
        assert_eq!(value["messages"][1]["role"], "user");
        assert_eq!(value["messages"][1]["content"], "write a function");
        assert!(value.get("format").is_none());
    }

    #[test]
    fn request_carries_response_schema_as_format_when_present() {
        let schema = serde_json::json!({"type": "object", "required": ["id"]});
        let prompt = Prompt::new(TaskClass::Authoring, "", "author a step")
            .with_response_schema(schema.clone());
        let value = serde_json::to_value(request_body("coder", &prompt)).unwrap();
        assert_eq!(value["format"], schema);
    }

    #[test]
    fn response_exposes_model_content_and_generated_tokens() {
        let parsed: OllamaResponse = serde_json::from_value(serde_json::json!({
            "model": "qwen2.5-coder:7b",
            "message": {"role": "assistant", "content": "done"},
            "eval_count": 37
        }))
        .unwrap();
        assert_eq!(parsed.model, "qwen2.5-coder:7b");
        assert_eq!(parsed.message.content, "done");
        assert_eq!(parsed.eval_count, 37);
    }

    #[cfg(not(feature = "live-providers"))]
    #[tokio::test(flavor = "current_thread")]
    async fn default_build_declines_routing_even_when_configured() {
        let provider = configured_provider();
        assert!(!provider.handles(TaskClass::Quick));
        let error = provider
            .complete(Prompt::new(TaskClass::Quick, "", "hello"))
            .await
            .unwrap_err();
        assert!(matches!(error, AiError::Unsupported(_)));
    }

    #[cfg(feature = "live-providers")]
    #[test]
    fn live_build_handles_every_class_when_host_is_configured() {
        let provider = configured_provider();
        for class in [
            TaskClass::Planning,
            TaskClass::Codegen,
            TaskClass::Testing,
            TaskClass::Summarize,
            TaskClass::Quick,
            TaskClass::Extension,
        ] {
            assert!(provider.handles(class));
        }
    }
}
