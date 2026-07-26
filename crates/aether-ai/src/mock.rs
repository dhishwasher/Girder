//! A deterministic, offline provider.
//!
//! This is the default so Bit Code's agent swarm and demo run with **no API
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

    /// Returns structured fn specs: one `fn <name> — <description>` line per
    /// function the feature needs. The Planner parses these into [`FnSpec`]s.
    fn plan(user: &str) -> String {
        let lower = user.to_lowercase();
        if lower.contains("auth") {
            "fn validate_credentials — checks that username is non-empty and password has at least 8 chars\n\
             fn generate_token — creates a session token string from a user id\n\
             fn authenticate — orchestrates credential validation and token generation, calling validate_credentials and generate_token"
                .to_string()
        } else if lower.contains("multipl") || lower.contains("product") {
            "fn multiply — multiplies two i64 values together".to_string()
        } else if lower.contains("subtract") || lower.contains("difference") {
            "fn subtract — subtracts the second i64 from the first".to_string()
        } else if lower.contains("divide") || lower.contains("quotient") {
            "fn divide — divides two i64 values, returning 0 on divide-by-zero".to_string()
        } else if lower.contains("add") || lower.contains("sum") {
            "fn add — adds two i64 values together".to_string()
        } else if lower.contains("parse") || lower.contains("read") {
            "fn parse_input — reads and trims raw text input\n\
             fn transform — transforms parsed data into output format, calling parse_input"
                .to_string()
        } else {
            "fn process — processes the input data\n\
             fn validate — validates inputs before processing"
                .to_string()
        }
    }

    /// Synthesize a Rust function. The function name is extracted from the
    /// `"fn <name> — <description>"` format the Coder sends, then code is
    /// generated from both the name and the description keywords.
    fn code(user: &str) -> String {
        // Extract the explicit function name if the prompt starts with "fn <name>"
        let explicit_name: Option<String> = user.trim().strip_prefix("fn ").and_then(|rest| {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() {
                None
            } else {
                Some(name)
            }
        });

        let lower = user.to_lowercase();

        if lower.contains("multipl") || lower.contains("product") {
            let name = explicit_name.as_deref().unwrap_or("multiply");
            format!("fn {name}(a: i64, b: i64) -> i64 {{\n    a * b\n}}")
        } else if lower.contains("subtract") || lower.contains("difference") {
            let name = explicit_name.as_deref().unwrap_or("subtract");
            format!("fn {name}(a: i64, b: i64) -> i64 {{\n    a - b\n}}")
        } else if lower.contains("divide") || lower.contains("quotient") {
            let name = explicit_name.as_deref().unwrap_or("divide");
            format!(
                "fn {name}(a: i64, b: i64) -> i64 {{\n    if b == 0 {{ 0 }} else {{ a / b }}\n}}"
            )
        } else if lower.contains("add") || lower.contains("sum") {
            let name = explicit_name.as_deref().unwrap_or("add");
            format!("fn {name}(a: i64, b: i64) -> i64 {{\n    a + b\n}}")
        } else if lower.contains("auth") || lower.contains("orchestrat") {
            let name = explicit_name.as_deref().unwrap_or("authenticate");
            format!(
                "fn {name}(username: &str, password: &str) -> Option<String> {{\n    \
                 if validate_credentials(username, password) {{\n        \
                 Some(generate_token(0))\n    }} else {{\n        \
                 None\n    }}\n}}"
            )
        } else if lower.contains("validate_credentials")
            || (lower.contains("valid") && lower.contains("credential"))
        {
            let name = explicit_name.as_deref().unwrap_or("validate_credentials");
            format!(
                "fn {name}(username: &str, password: &str) -> bool {{\n    !username.is_empty() && password.len() >= 8\n}}"
            )
        } else if lower.contains("token") || lower.contains("generate_token") {
            let name = explicit_name.as_deref().unwrap_or("generate_token");
            format!("fn {name}(user_id: u64) -> String {{\n    format!(\"token-{{user_id}}\")\n}}")
        } else if lower.contains("valid") {
            let name = explicit_name.as_deref().unwrap_or("validate");
            format!("fn {name}(input: &str) -> bool {{\n    !input.is_empty()\n}}")
        } else if lower.contains("parse") {
            let name = explicit_name.as_deref().unwrap_or("parse_input");
            format!("fn {name}(input: &str) -> Option<&str> {{\n    Some(input.trim())\n}}")
        } else if lower.contains("transform") || lower.contains("process") {
            let name = explicit_name.as_deref().unwrap_or("process");
            format!("fn {name}(input: &str) -> String {{\n    parse_input(input).unwrap_or(\"\").to_string()\n}}")
        } else {
            let name = explicit_name.as_deref().unwrap_or("process");
            format!("fn {name}(input: &str) -> String {{\n    input.to_string()\n}}")
        }
    }

    fn test(user: &str) -> String {
        let lower = user.to_lowercase();
        let name = if lower.contains("multipl") {
            "multiply"
        } else if lower.contains("subtract") {
            "subtract"
        } else if lower.contains("divide") {
            "divide"
        } else if lower.contains("add") || lower.contains("sum") {
            "add"
        } else {
            "process"
        };
        format!(
            "#[test]\nfn test_{name}() {{\n    assert_eq!({name}(2, 3), {});\n}}",
            match name {
                "multiply" => "6".to_string(),
                "subtract" => "-1".to_string(),
                "divide" => "0".to_string(),
                "add" => "5".to_string(),
                _ => "\"23\".to_string()".to_string(),
            }
        )
    }

    fn extension(input: &str) -> String {
        let parsed = serde_json::from_str::<serde_json::Value>(input).unwrap_or_default();
        if let Some(reference) = parsed
            .get("listing")
            .and_then(|listing| listing.get("reference_recipe"))
        {
            return reference.to_string();
        }
        let intent = parsed
            .get("intent")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("workspace insight");
        let mut slug = intent
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>();
        while slug.contains("--") {
            slug = slug.replace("--", "-");
        }
        slug = slug
            .trim_matches('-')
            .chars()
            .take(48)
            .collect::<String>()
            .trim_matches('-')
            .to_string();
        if slug.is_empty() {
            slug = "workspace-insight".into();
        }
        let title = intent
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(60)
            .collect::<String>();
        serde_json::json!({
            "version": 1,
            "id": format!("dev.bitcode.generated.{slug}"),
            "name": if title.is_empty() { "Workspace Insight" } else { &title },
            "description": format!("A graph-backed workspace view for: {intent}"),
            "intent": intent,
            "capabilities": [
                { "kind": "read_graph" },
                { "kind": "contribute_ui" }
            ],
            "contributions": [
                {
                    "kind": "panel",
                    "id": "workspace-view",
                    "title": if title.is_empty() { "Workspace Insight" } else { &title },
                    "location": "right",
                    "view": {
                        "kind": "graph_query",
                        "question": intent
                    }
                }
            ],
            "projections": []
        })
        .to_string()
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
            TaskClass::Extension => Self::extension(&prompt.user),
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

    #[tokio::test(flavor = "current_thread")]
    async fn extension_generation_is_strict_json() {
        let provider = MockProvider::new();
        let completion = provider
            .complete(Prompt::new(
                TaskClass::Extension,
                "",
                r#"{"intent":"show authentication impact"}"#,
            ))
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&completion.text).unwrap();

        assert_eq!(
            value["id"],
            "dev.bitcode.generated.show-authentication-impact"
        );
        assert_eq!(value["capabilities"][0]["kind"], "read_graph");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn marketplace_adaptation_preserves_the_reviewed_identity_offline() {
        let provider = MockProvider::new();
        let completion = provider
            .complete(Prompt::new(
                TaskClass::Extension,
                "",
                r#"{
                    "listing": {
                        "reference_recipe": {
                            "version": 1,
                            "id": "org.bitcode.impact-navigator",
                            "name": "Impact Navigator",
                            "description": "Shows semantic impact.",
                            "intent": "show semantic impact",
                            "capabilities": [],
                            "contributions": [],
                            "projections": []
                        }
                    }
                }"#,
            ))
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&completion.text).unwrap();

        assert_eq!(value["id"], "org.bitcode.impact-navigator");
    }
}
