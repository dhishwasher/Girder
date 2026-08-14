//! The provider abstraction: one clean trait every model backend implements.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Broad task classes the router uses to pick the best-suited model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskClass {
    /// High-level decomposition / reasoning (Planner).
    Planning,
    /// Code generation / editing (Coder, Refactorer).
    Codegen,
    /// Test synthesis & evaluation (Tester).
    Testing,
    /// Natural-language summaries / docs (Documenter).
    Summarize,
    /// Cheap, latency-sensitive completions (inline hints).
    Quick,
    /// Strict declarative extension-recipe generation.
    Extension,
    /// Graph-addressed plan authoring for `bitcode do`. Routed local-first
    /// (unlike `Planning`) because it is the task the authoring-cost
    /// measurement in `docs/authoring-cost.md` is about.
    Authoring,
}

/// A request to a model. Deliberately minimal but provider-agnostic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt {
    /// System/role framing.
    pub system: String,
    /// The user/instruction content.
    pub user: String,
    /// What kind of work this is (drives routing & temperature).
    pub class: TaskClass,
    /// Soft cap on response length.
    pub max_tokens: u32,
    /// A JSON Schema the response must satisfy, when the provider is able to
    /// grammar-constrain decoding to it (Ollama's `/api/chat` `format`,
    /// OpenAI's structured outputs). `None` — the default — is the only
    /// value any caller passed before `bitcode do` existed, so no existing
    /// provider behavior changes unless a caller opts in.
    pub response_schema: Option<serde_json::Value>,
}

impl Prompt {
    pub fn new(class: TaskClass, system: impl Into<String>, user: impl Into<String>) -> Self {
        Prompt {
            system: system.into(),
            user: user.into(),
            class,
            max_tokens: 1024,
            response_schema: None,
        }
    }

    /// Attach a JSON Schema the response must satisfy. Providers that can
    /// grammar-constrain decoding to it will; providers that can't (or a
    /// build without `live-providers`) simply ignore it.
    pub fn with_response_schema(mut self, schema: serde_json::Value) -> Self {
        self.response_schema = Some(schema);
        self
    }
}

/// A model response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Completion {
    pub text: String,
    /// Which provider/model produced this (for the agent console & telemetry).
    pub model: String,
    /// Rough token accounting if the backend reports it.
    pub tokens: u32,
}

/// Errors a provider can surface.
#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error(
        "provider '{0}' is not enabled (build with --features live-providers and set an API key)"
    )]
    Unsupported(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("provider returned an error: {0}")]
    Provider(String),
}

/// Anything that can turn a [`Prompt`] into a [`Completion`].
///
/// `Send + Sync` so the orchestrator can share one provider across parallel
/// agent tasks behind an `Arc`.
#[async_trait]
pub trait AiProvider: Send + Sync {
    /// Human-readable id, e.g. `"mock"`, `"anthropic:claude"`.
    fn name(&self) -> &str;

    /// Whether this provider can serve a given task class. The router uses this
    /// to skip providers that don't fit (e.g. a tiny local model for Planning).
    fn handles(&self, _class: TaskClass) -> bool {
        true
    }

    async fn complete(&self, prompt: Prompt) -> Result<Completion, AiError>;
}
