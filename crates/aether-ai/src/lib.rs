//! # aether-ai
//!
//! Bit Code's pluggable AI layer. One [`AiProvider`] trait; a deterministic
//! offline [`MockProvider`] that is always available; implemented OpenAI and
//! Anthropic providers behind the `live-providers` feature; and compile-clean
//! extension-point structs for Google Gemini, xAI Grok, and local Ollama. The
//! [`Router`] selects among implemented providers per [`TaskClass`] and falls
//! back gracefully — local-first by default.

pub mod anthropic;
pub mod gemini;
pub mod grok;
pub mod mock;
pub mod ollama;
pub mod openai;
pub mod provider;
pub mod router;

pub use mock::MockProvider;
pub use provider::{AiError, AiProvider, Completion, Prompt, TaskClass};
pub use router::Router;

pub use anthropic::AnthropicProvider;
pub use gemini::GeminiProvider;
pub use grok::GrokProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAiProvider;

use std::sync::Arc;

/// Build the default local-first router used by the demo.
///
/// The deterministic mock is always the fallback. When the crate is built with
/// `live-providers`, `OPENAI_API_KEY` enables the OpenAI Responses API provider
/// and `ANTHROPIC_API_KEY` enables Anthropic. OpenAI is tried first so users can
/// opt into it directly; Anthropic remains a secondary live provider for
/// planning and code generation. Gemini/Grok/Ollama are intentionally not part
/// of default routing until their HTTP bodies are implemented.
pub fn default_router() -> Router {
    let mock: Arc<dyn AiProvider> = Arc::new(MockProvider::new());
    let openai: Arc<dyn AiProvider> = Arc::new(OpenAiProvider::from_env());
    let anthropic: Arc<dyn AiProvider> = Arc::new(AnthropicProvider::from_env());
    Router::new()
        .route(TaskClass::Planning, vec![openai.clone(), anthropic.clone()])
        .route(TaskClass::Codegen, vec![openai.clone(), anthropic])
        .route(TaskClass::Testing, vec![openai.clone()])
        .route(TaskClass::Summarize, vec![openai.clone()])
        .route(TaskClass::Quick, vec![openai])
        .with_fallback(mock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn default_router_is_mock_backed_for_every_task_class() {
        let router = default_router();
        for class in [
            TaskClass::Planning,
            TaskClass::Codegen,
            TaskClass::Testing,
            TaskClass::Summarize,
            TaskClass::Quick,
        ] {
            let completion = router
                .complete(Prompt::new(class, "", "add a multiply function"))
                .await
                .unwrap();
            assert_eq!(completion.model, "mock-deterministic-v1");
        }
    }
}
