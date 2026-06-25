//! EXTENSION POINT — OpenAi provider.
//!
//! A real, compile-clean struct that slots into the [`AiProvider`] trait. The
//! HTTP body is intentionally not implemented in the prototype: building with
//! `--features live-providers` and setting `OPENAI_API_KEY` is where you'd wire the real
//! API call (reqwest + the provider's JSON schema). Until then it reports
//! [`AiError::Unsupported`] so the swarm transparently falls back to the mock.

use crate::provider::{AiError, AiProvider, Completion, Prompt, TaskClass};
use async_trait::async_trait;

pub struct OpenAiProvider {
    api_key: Option<String>,
    model: String,
}

impl OpenAiProvider {
    /// Reads the key from `OPENAI_API_KEY` if present.
    pub fn from_env() -> Self {
        OpenAiProvider {
            api_key: std::env::var("OPENAI_API_KEY").ok(),
            model: "openai:gpt".to_string(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }
}

#[async_trait]
impl AiProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai:gpt"
    }

    fn handles(&self, _class: TaskClass) -> bool {
        // Once live, this provider could decline classes it's poorly suited to.
        self.api_key.is_some()
    }

    async fn complete(&self, _prompt: Prompt) -> Result<Completion, AiError> {
        #[cfg(feature = "live-providers")]
        {
            // EXTENSION POINT: real request goes here, e.g.
            //   let client = reqwest::Client::new();
            //   let resp = client.post(ENDPOINT).bearer_auth(key).json(&body).send().await?;
            // For now even the live build is unimplemented.
            let _ = &self.api_key;
            let _ = &self.model;
            return Err(AiError::Unsupported("openai:gpt".into()));
        }
        #[cfg(not(feature = "live-providers"))]
        Err(AiError::Unsupported("openai:gpt".into()))
    }
}
