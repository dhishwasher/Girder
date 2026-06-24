//! # aether-ai
//!
//! AetherForge's pluggable, multi-provider AI layer. One [`AiProvider`] trait;
//! many backends (a deterministic offline [`MockProvider`] plus compile-clean
//! EXTENSION POINTs for Anthropic Claude, OpenAI, Google Gemini, xAI Grok, and
//! local Ollama models). A [`Router`] picks the best provider per [`TaskClass`]
//! and falls back gracefully — *local-first by default*.

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

/// Build the default local-first router used by the demo: prefer real providers
/// per task class (they self-disable without keys), always backed by the mock.
pub fn default_router() -> Router {
    let mock: Arc<dyn AiProvider> = Arc::new(MockProvider::new());
    Router::new()
        .route(
            TaskClass::Planning,
            vec![Arc::new(AnthropicProvider::from_env())],
        )
        .route(
            TaskClass::Codegen,
            vec![
                Arc::new(AnthropicProvider::from_env()),
                Arc::new(OpenAiProvider::from_env()),
            ],
        )
        .route(TaskClass::Quick, vec![Arc::new(OllamaProvider::from_env())])
        .with_fallback(mock)
}
