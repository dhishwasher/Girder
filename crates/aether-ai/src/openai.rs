//! OpenAI provider.
//!
//! Uses the Responses API (`POST /v1/responses`) in live builds. The default
//! build stays offline and reports [`AiError::Unsupported`] so Bit Code falls
//! back to the deterministic mock.

use crate::provider::{AiError, AiProvider, Completion, Prompt, TaskClass};
use async_trait::async_trait;

const DEFAULT_MODEL: &str = "gpt-5.6";
const DEFAULT_ENDPOINT: &str = "https://api.openai.com/v1/responses";

pub struct OpenAiProvider {
    api_key: Option<String>,
    endpoint: String,
    model: String,
}

impl OpenAiProvider {
    /// Reads `OPENAI_API_KEY`, optional `OPENAI_MODEL`, and optional
    /// `OPENAI_RESPONSES_URL` from the environment.
    pub fn from_env() -> Self {
        OpenAiProvider {
            api_key: std::env::var("OPENAI_API_KEY").ok(),
            endpoint: std::env::var("OPENAI_RESPONSES_URL")
                .unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string()),
            model: std::env::var("OPENAI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string()),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }
}

#[async_trait]
impl AiProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai:responses"
    }

    fn handles(&self, _class: TaskClass) -> bool {
        cfg!(feature = "live-providers") && self.api_key.is_some()
    }

    #[cfg(feature = "live-providers")]
    async fn complete(&self, prompt: Prompt) -> Result<Completion, AiError> {
        let key = self
            .api_key
            .as_ref()
            .ok_or_else(|| AiError::Unsupported("openai:responses (no OPENAI_API_KEY)".into()))?;

        let body = request_body(&self.model, &prompt);

        let resp = reqwest::Client::new()
            .post(&self.endpoint)
            .bearer_auth(key)
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

        let parsed: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AiError::Transport(e.to_string()))?;
        let text = response_text(&parsed);
        let tokens = parsed
            .pointer("/usage/output_tokens")
            .or_else(|| parsed.pointer("/usage/total_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let model = parsed
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.model)
            .to_string();

        Ok(Completion {
            text,
            model,
            tokens,
        })
    }

    #[cfg(not(feature = "live-providers"))]
    async fn complete(&self, _prompt: Prompt) -> Result<Completion, AiError> {
        Err(AiError::Unsupported(
            "openai:responses (build with --features live-providers)".into(),
        ))
    }
}

#[cfg(any(feature = "live-providers", test))]
fn request_body(model: &str, prompt: &Prompt) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "instructions": prompt.system,
        "input": prompt.user,
        "max_output_tokens": prompt.max_tokens.max(1),
    });
    if let Some(schema) = &prompt.response_schema {
        body["text"] = serde_json::json!({
            "format": {
                "type": "json_schema",
                "name": "bitcode_plan_step",
                "schema": schema,
                "strict": true,
            }
        });
    }
    body
}

#[cfg(feature = "live-providers")]
fn response_text(value: &serde_json::Value) -> String {
    if let Some(text) = value.get("output_text").and_then(|v| v.as_str()) {
        return text.to_string();
    }

    let Some(output) = value.get("output").and_then(|v| v.as_array()) else {
        return String::new();
    };
    output
        .iter()
        .filter_map(|item| item.get("content").and_then(|v| v.as_array()))
        .flatten()
        .filter_map(|content| {
            content
                .get("text")
                .or_else(|| content.get("output_text"))
                .and_then(|v| v.as_str())
        })
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(feature = "live-providers"))]
    fn configured_provider_declines_routing_without_live_feature() {
        let provider = OpenAiProvider {
            api_key: Some("test".to_string()),
            endpoint: "http://127.0.0.1".to_string(),
            model: "test-model".to_string(),
        };
        assert!(!provider.handles(TaskClass::Codegen));
    }

    #[test]
    #[cfg(feature = "live-providers")]
    fn configured_provider_handles_routing_with_live_feature() {
        let provider = OpenAiProvider {
            api_key: Some("test".to_string()),
            endpoint: "http://127.0.0.1".to_string(),
            model: "test-model".to_string(),
        };
        assert!(provider.handles(TaskClass::Codegen));
    }

    #[test]
    fn request_omits_text_format_when_no_schema_is_requested() {
        let prompt = Prompt::new(TaskClass::Codegen, "", "write a function");
        let body = request_body("test-model", &prompt);
        assert!(body.get("text").is_none());
    }

    #[test]
    fn response_schema_becomes_strict_structured_output_format() {
        let schema = serde_json::json!({"type": "object", "required": ["id"]});
        let prompt = Prompt::new(TaskClass::Authoring, "", "author a step")
            .with_response_schema(schema.clone());
        let body = request_body("test-model", &prompt);
        assert_eq!(body["text"]["format"]["type"], "json_schema");
        assert_eq!(body["text"]["format"]["schema"], schema);
        assert_eq!(body["text"]["format"]["strict"], true);
    }

    #[test]
    #[cfg(feature = "live-providers")]
    fn extracts_response_api_output_text() {
        let response = serde_json::json!({
            "output": [
                {
                    "type": "message",
                    "content": [
                        {"type": "output_text", "text": "hello"},
                        {"type": "output_text", "text": " world"}
                    ]
                }
            ]
        });
        assert_eq!(response_text(&response), "hello world");
    }
}
