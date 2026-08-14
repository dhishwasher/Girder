//! Intelligent multi-provider routing.
//!
//! Agents don't talk to a model directly; they ask the [`Router`], which picks
//! the best available provider for a task class and transparently falls back
//! (e.g. a remote provider that's `Unsupported` because no key is set drops to
//! the local mock). This is what makes Bit Code *local-first by default* yet
//! able to escalate to frontier models when configured.

use crate::provider::{AiError, AiProvider, Completion, Prompt, TaskClass};
use std::sync::Arc;

/// A routing preference: for a task class, try these providers in order.
struct Route {
    class: TaskClass,
    providers: Vec<Arc<dyn AiProvider>>,
}

/// Routes prompts to providers. Cheap to clone (everything is `Arc`).
#[derive(Clone, Default)]
pub struct Router {
    routes: Vec<Arc<Route>>,
    fallback: Option<Arc<dyn AiProvider>>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    /// A sensible default: everything routed to a single local provider.
    pub fn local_only(provider: Arc<dyn AiProvider>) -> Self {
        Router {
            routes: Vec::new(),
            fallback: Some(provider),
        }
    }

    /// Set the last-resort provider used when no route matches or all preferred
    /// providers decline/error.
    pub fn with_fallback(mut self, provider: Arc<dyn AiProvider>) -> Self {
        self.fallback = Some(provider);
        self
    }

    /// Register an ordered provider preference for a task class.
    pub fn route(mut self, class: TaskClass, providers: Vec<Arc<dyn AiProvider>>) -> Self {
        self.routes.push(Arc::new(Route { class, providers }));
        self
    }

    /// Resolve the ordered candidate list for a class (preferred first, then
    /// fallback), filtered to providers that `handles(class)`. Exposed so a
    /// caller like `bitcode do` can walk providers one at a time itself
    /// (e.g. to run a repair loop against each before moving to the next),
    /// rather than only getting `complete`'s single resolved outcome.
    pub fn candidates(&self, class: TaskClass) -> Vec<Arc<dyn AiProvider>> {
        let mut out: Vec<Arc<dyn AiProvider>> = self
            .routes
            .iter()
            .filter(|r| r.class == class)
            .flat_map(|r| r.providers.iter().cloned())
            .filter(|p| p.handles(class))
            .collect();
        if let Some(fb) = &self.fallback {
            out.push(fb.clone());
        }
        out
    }

    /// Return the declared route before availability filtering. This keeps the
    /// default policy assertable without requiring API keys or a live Ollama
    /// server in tests.
    #[cfg(test)]
    pub(crate) fn configured_provider_names(&self, class: TaskClass) -> Vec<&str> {
        let mut out: Vec<&str> = self
            .routes
            .iter()
            .filter(|route| route.class == class)
            .flat_map(|route| route.providers.iter().map(|provider| provider.name()))
            .collect();
        if let Some(fallback) = &self.fallback {
            out.push(fallback.name());
        }
        out
    }

    /// Complete a prompt, trying candidates in order until one succeeds.
    pub async fn complete(&self, prompt: Prompt) -> Result<Completion, AiError> {
        let class = prompt.class;
        let candidates = self.candidates(class);
        if candidates.is_empty() {
            return Err(AiError::Unsupported("no provider configured".into()));
        }
        let mut last_err = AiError::Unsupported("no provider configured".into());
        for provider in candidates {
            // Each attempt gets its own clone of the prompt (cheap; small structs).
            match provider.complete(prompt.clone()).await {
                Ok(c) => return Ok(c),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthropic::AnthropicProvider;
    use crate::mock::MockProvider;

    #[tokio::test(flavor = "current_thread")]
    async fn falls_back_to_mock_when_remote_unsupported() {
        // Prefer Anthropic for planning, but no key is set -> it declines, and
        // the router transparently falls back to the local mock.
        let router = Router::new()
            .route(
                TaskClass::Planning,
                vec![Arc::new(AnthropicProvider::from_env())],
            )
            .with_fallback(Arc::new(MockProvider::new()));

        let res = router
            .complete(Prompt::new(TaskClass::Planning, "", "do a thing"))
            .await
            .unwrap();
        assert_eq!(res.model, "mock-deterministic-v1");
    }
}
