//! A deterministic, offline provider.
//!
//! This is the default so AetherForge's agent swarm and demo run with **no API
//! key and no network**. It is intentionally simple but not a no-op: it produces
//! structured, plausible output per task class (a plan, a Rust function, a test)
//! so the end-to-end pipeline — intent → plan → code → test → graph mutation —
//! actually does something observable.

use crate::provider::{AiError, AiProvider, Completion, Prompt, TaskClass};
use async_trait::async_trait;

#[derive(Default)]
pub struct MockProvider;

impl MockProvider {
    pub fn new() -> Self {
        MockProvider
    }

    fn plan(user: &str) -> String {
        format!(
            "1. Locate the relevant nodes in the semantic graph for: \"{user}\".\n\
             2. Generate or modify the target function.\n\
             3. Wire up call edges and rerun impact analysis.\n\
             4. Synthesize tests and validate."
        )
    }

    /// Synthesize a tiny Rust function. We sniff a verb + operator from the
    /// instruction so different intents yield different (deterministic) code.
    fn code(user: &str) -> String {
        let lower = user.to_lowercase();
        let (name, op) = if lower.contains("multipl") || lower.contains("product") {
            ("multiply", "*")
        } else if lower.contains("subtract") || lower.contains("difference") {
            ("subtract", "-")
        } else if lower.contains("divide") || lower.contains("quotient") {
            ("divide", "/")
        } else {
            ("add", "+")
        };
        format!("fn {name}(a: i64, b: i64) -> i64 {{\n    a {op} b\n}}")
    }

    fn test(user: &str) -> String {
        let name = if user.to_lowercase().contains("multipl") {
            "multiply"
        } else if user.to_lowercase().contains("subtract") {
            "subtract"
        } else {
            "add"
        };
        format!(
            "#[test]\nfn test_{name}() {{\n    assert_eq!({name}(2, 3), {});\n}}",
            match name {
                "multiply" => 6,
                "subtract" => -1,
                _ => 5,
            }
        )
    }
}

#[async_trait]
impl AiProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    async fn complete(&self, prompt: Prompt) -> Result<Completion, AiError> {
        let text = match prompt.class {
            TaskClass::Planning => Self::plan(&prompt.user),
            TaskClass::Codegen => Self::code(&prompt.user),
            TaskClass::Testing => Self::test(&prompt.user),
            TaskClass::Summarize => format!(
                "This unit handles: {}.",
                prompt.user.lines().next().unwrap_or("(unknown)").trim()
            ),
            TaskClass::Quick => prompt.user.chars().take(40).collect(),
        };
        let tokens = (text.len() / 4) as u32;
        Ok(Completion {
            text,
            model: "mock-deterministic-v1".to_string(),
            tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn codegen_reacts_to_intent() {
        let p = MockProvider::new();
        let mult = p
            .complete(Prompt::new(
                TaskClass::Codegen,
                "",
                "add a multiply function",
            ))
            .await
            .unwrap();
        assert!(mult.text.contains("fn multiply"));
        assert!(mult.text.contains("a * b"));
    }
}
