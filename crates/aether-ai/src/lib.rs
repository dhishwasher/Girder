//! # aether-ai
//!
//! Bit Code's pluggable AI layer. One [`AiProvider`] trait; a deterministic
//! offline [`MockProvider`] that is always available; implemented OpenAI,
//! Anthropic, and local Ollama providers behind the `live-providers` feature;
//! and compile-clean extension-point structs for Google Gemini and xAI Grok. The
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
/// `live-providers`, `OLLAMA_HOST` enables local Ollama, `OPENAI_API_KEY`
/// enables the OpenAI Responses API provider, and `ANTHROPIC_API_KEY` enables
/// Anthropic. Latency-sensitive and verification tasks prefer Ollama; planning,
/// code generation, and extension authoring retain the remote providers first
/// and use Ollama before the deterministic mock fallback.
pub fn default_router() -> Router {
    let mock: Arc<dyn AiProvider> = Arc::new(MockProvider::new());
    let ollama: Arc<dyn AiProvider> = Arc::new(OllamaProvider::from_env());
    let openai: Arc<dyn AiProvider> = Arc::new(OpenAiProvider::from_env());
    let anthropic: Arc<dyn AiProvider> = Arc::new(AnthropicProvider::from_env());
    Router::new()
        .route(
            TaskClass::Planning,
            vec![openai.clone(), anthropic.clone(), ollama.clone()],
        )
        .route(
            TaskClass::Extension,
            vec![openai.clone(), anthropic.clone(), ollama.clone()],
        )
        .route(
            TaskClass::Codegen,
            vec![openai.clone(), anthropic, ollama.clone()],
        )
        .route(TaskClass::Testing, vec![ollama.clone(), openai.clone()])
        .route(TaskClass::Summarize, vec![ollama.clone(), openai.clone()])
        .route(TaskClass::Quick, vec![ollama, openai])
        .with_fallback(mock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "live-providers"))]
    #[tokio::test(flavor = "current_thread")]
    async fn default_router_is_mock_backed_for_every_task_class() {
        let router = default_router();
        for class in [
            TaskClass::Planning,
            TaskClass::Codegen,
            TaskClass::Testing,
            TaskClass::Summarize,
            TaskClass::Quick,
            TaskClass::Extension,
        ] {
            let completion = router
                .complete(Prompt::new(class, "", "add a multiply function"))
                .await
                .unwrap();
            assert_eq!(completion.model, "mock-deterministic-v1");
        }
    }

    #[test]
    fn default_routing_policy_is_explicit_and_local_first_where_promised() {
        let router = default_router();
        let remote_first = vec![
            "openai:responses",
            "anthropic:claude",
            "ollama:local",
            "mock",
        ];
        for class in [
            TaskClass::Planning,
            TaskClass::Codegen,
            TaskClass::Extension,
        ] {
            assert_eq!(router.configured_provider_names(class), remote_first);
        }

        let local_first = vec!["ollama:local", "openai:responses", "mock"];
        for class in [TaskClass::Quick, TaskClass::Summarize, TaskClass::Testing] {
            assert_eq!(router.configured_provider_names(class), local_first);
        }
    }
}
