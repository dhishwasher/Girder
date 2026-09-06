use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub(crate) const CONFIG_FILE: &str = "girder.toml";
const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ProjectConfig {
    pub(crate) version: u32,
    pub(crate) source: SourceConfig,
    pub(crate) graph: GraphConfig,
    pub(crate) tests: TestConfig,
    pub(crate) agents: AgentConfig,
    pub(crate) validation: ValidationConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SourceConfig {
    pub(crate) roots: Vec<String>,
    pub(crate) exclude: Vec<String>,
    pub(crate) follow_symlinks: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct GraphConfig {
    pub(crate) path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TestConfig {
    pub(crate) rust: Option<Vec<String>>,
    pub(crate) python: Option<Vec<String>>,
    pub(crate) go: Option<Vec<String>>,
    /// Hard wall-clock budget for each `test-impact --run` child.
    pub(crate) run_timeout_seconds: u64,
    /// Hard cap on bytes a `--run` child may stream before it is killed.
    pub(crate) run_max_output_bytes: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct AgentConfig {
    pub(crate) output_module: String,
    pub(crate) output_file: String,
    pub(crate) timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ValidationConfig {
    pub(crate) enabled: bool,
    pub(crate) run_tests: bool,
    pub(crate) commands: Vec<Vec<String>>,
    pub(crate) timeout_seconds: u64,
    pub(crate) max_output_bytes: usize,
    pub(crate) max_copy_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfiguredCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            source: SourceConfig::default(),
            graph: GraphConfig::default(),
            tests: TestConfig::default(),
            agents: AgentConfig::default(),
            validation: ValidationConfig::default(),
        }
    }
}

impl Default for SourceConfig {
    fn default() -> Self {
        Self {
            roots: vec![".".into()],
            exclude: [
                "**/.git",
                "**/.git/**",
                "**/.aether-cache",
                "**/.aether-cache/**",
                "**/target",
                "**/target/**",
                "**/node_modules",
                "**/node_modules/**",
                "**/__pycache__",
                "**/__pycache__/**",
                "**/.venv",
                "**/.venv/**",
                "**/venv",
                "**/venv/**",
                "**/.*",
                "**/.*/**",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            follow_symlinks: false,
        }
    }
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            path: "project.aether".into(),
        }
    }
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            rust: Some(
                ["cargo", "test", "--workspace", "{test}"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            ),
            python: Some(
                ["pytest", "-k", "{filter}"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            ),
            go: Some(
                ["go", "test", "./...", "-run", "{filter}"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            ),
            run_timeout_seconds: 1800,
            run_max_output_bytes: 8 * 1024 * 1024,
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            output_module: "crate::forge".into(),
            output_file: "src/forge.rs".into(),
            timeout_seconds: 10,
        }
    }
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            run_tests: true,
            commands: Vec::new(),
            timeout_seconds: 300,
            max_output_bytes: 128 * 1024,
            max_copy_bytes: 512 * 1024 * 1024,
        }
    }
}

impl ProjectConfig {
    pub(crate) fn load(root: &Path) -> std::io::Result<Self> {
        let path = root.join(CONFIG_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                config.validate()?;
                return Ok(config);
            }
            Err(error) => return Err(error),
        };
        let config: Self = toml::from_str(&text).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid {}: {error}", path.display()),
            )
        })?;
        config.validate()?;
        Ok(config)
    }

    pub(crate) fn write_default(root: &Path) -> std::io::Result<PathBuf> {
        let path = root.join(CONFIG_FILE);
        let config = Self::default();
        let text = config.to_toml()?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    std::io::Error::new(
                        error.kind(),
                        format!(
                            "{} already exists; refusing to overwrite it",
                            path.display()
                        ),
                    )
                } else {
                    error
                }
            })?;
        file.write_all(text.as_bytes())?;
        Ok(path)
    }

    pub(crate) fn to_toml(&self) -> std::io::Result<String> {
        toml::to_string_pretty(self)
            .map_err(|error| std::io::Error::other(format!("could not render config: {error}")))
    }

    pub(crate) fn source_excludes(&self) -> std::io::Result<GlobSet> {
        let mut builder = GlobSetBuilder::new();
        for pattern in &self.source.exclude {
            let glob = GlobBuilder::new(pattern)
                .literal_separator(true)
                .backslash_escape(false)
                .build()
                .map_err(|error| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("invalid source.exclude glob {pattern:?}: {error}"),
                    )
                })?;
            builder.add(glob);
        }
        builder.build().map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("could not compile source.exclude globs: {error}"),
            )
        })
    }

    pub(crate) fn rust_test_command(&self, test: &str) -> Option<ConfiguredCommand> {
        render_command(self.tests.rust.as_deref()?, "{test}", test)
    }

    pub(crate) fn python_test_command(&self, filter: &str) -> Option<ConfiguredCommand> {
        render_command(self.tests.python.as_deref()?, "{filter}", filter)
    }

    // EXTENSION POINT: test_impact.rs's `run` path only builds commands for
    // "rust"/"python" test nodes (mirroring its `--quiet` name filter); Go
    // test execution is configurable here but not yet wired into that loop.
    #[allow(dead_code)]
    pub(crate) fn go_test_command(&self, filter: &str) -> Option<ConfiguredCommand> {
        render_command(self.tests.go.as_deref()?, "{filter}", filter)
    }

    fn validate(&self) -> std::io::Result<()> {
        if self.version != CONFIG_VERSION {
            return Err(invalid_config(format!(
                "unsupported config version {}; expected {CONFIG_VERSION}",
                self.version
            )));
        }
        if self.source.roots.is_empty() {
            return Err(invalid_config("source.roots must not be empty"));
        }
        for root in &self.source.roots {
            validate_relative_path("source root", root)?;
        }
        self.source_excludes()?;
        validate_relative_path("graph.path", &self.graph.path)?;
        if !matches!(
            Path::new(&self.graph.path)
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("aether" | "aetherb")
        ) {
            return Err(invalid_config(
                "graph.path must end with .aether or .aetherb",
            ));
        }
        validate_command("tests.rust", self.tests.rust.as_deref(), "{test}")?;
        validate_command("tests.python", self.tests.python.as_deref(), "{filter}")?;
        if !(1..=86_400).contains(&self.tests.run_timeout_seconds) {
            return Err(invalid_config(
                "tests.run_timeout_seconds must be between 1 and 86400",
            ));
        }
        if !(4 * 1024..=1024 * 1024 * 1024).contains(&self.tests.run_max_output_bytes) {
            return Err(invalid_config(
                "tests.run_max_output_bytes must be between 4096 and 1073741824",
            ));
        }
        if self.agents.output_module.trim().is_empty() {
            return Err(invalid_config("agents.output_module must not be empty"));
        }
        validate_relative_path("agents.output_file", &self.agents.output_file)?;
        if Path::new(&self.agents.output_file)
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("rs")
        {
            return Err(invalid_config("agents.output_file must be a Rust file"));
        }
        if !(1..=3600).contains(&self.agents.timeout_seconds) {
            return Err(invalid_config(
                "agents.timeout_seconds must be between 1 and 3600",
            ));
        }
        if !(1..=3600).contains(&self.validation.timeout_seconds) {
            return Err(invalid_config(
                "validation.timeout_seconds must be between 1 and 3600",
            ));
        }
        if !(4 * 1024..=4 * 1024 * 1024).contains(&self.validation.max_output_bytes) {
            return Err(invalid_config(
                "validation.max_output_bytes must be between 4096 and 4194304",
            ));
        }
        if !(1024 * 1024..=16 * 1024 * 1024 * 1024).contains(&self.validation.max_copy_bytes) {
            return Err(invalid_config(
                "validation.max_copy_bytes must be between 1048576 and 17179869184",
            ));
        }
        for (index, command) in self.validation.commands.iter().enumerate() {
            if command.is_empty() || command[0].trim().is_empty() {
                return Err(invalid_config(format!(
                    "validation.commands[{index}] must name a program"
                )));
            }
        }
        Ok(())
    }
}

impl ConfiguredCommand {
    pub(crate) fn display(&self) -> String {
        std::iter::once(&self.program)
            .chain(&self.args)
            .map(|part| shell_quote(part))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn render_command(
    template: &[String],
    placeholder: &str,
    value: &str,
) -> Option<ConfiguredCommand> {
    let mut rendered = template.iter().map(|part| part.replace(placeholder, value));
    Some(ConfiguredCommand {
        program: rendered.next()?,
        args: rendered.collect(),
    })
}

fn validate_command(
    label: &str,
    command: Option<&[String]>,
    placeholder: &str,
) -> std::io::Result<()> {
    let Some(command) = command else {
        return Ok(());
    };
    if command.is_empty() || command[0].trim().is_empty() {
        return Err(invalid_config(format!("{label} must name a program")));
    }
    if !command.iter().any(|part| part.contains(placeholder)) {
        return Err(invalid_config(format!(
            "{label} must contain the {placeholder} placeholder"
        )));
    }
    Ok(())
}

fn validate_relative_path(label: &str, value: &str) -> std::io::Result<()> {
    let path = Path::new(value);
    if value.trim().is_empty() || path.is_absolute() {
        return Err(invalid_config(format!(
            "{label} must be a non-empty project-relative path"
        )));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(invalid_config(format!(
            "{label} must not escape the project root: {value}"
        )));
    }
    Ok(())
}

fn invalid_config(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_./:=+".contains(&byte))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_config_inherits_defaults() {
        let config: ProjectConfig = toml::from_str(
            r#"
version = 1

[source]
roots = ["src", "tests"]
"#,
        )
        .unwrap();

        assert_eq!(config.source.roots, ["src", "tests"]);
        assert_eq!(config.graph.path, "project.aether");
        assert_eq!(config.agents.output_file, "src/forge.rs");
        assert!(config.tests.rust.is_some());
        assert!(config.validation.enabled);
        assert!(config.validation.run_tests);
        config.validate().unwrap();
    }

    #[test]
    fn validation_commands_must_name_a_program() {
        let mut config = ProjectConfig::default();
        config.validation.commands = vec![Vec::new()];

        let error = config.validate().unwrap_err();

        assert!(error
            .to_string()
            .contains("validation.commands[0] must name a program"));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let error = toml::from_str::<ProjectConfig>(
            r#"
version = 1
surprise = true
"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn paths_cannot_escape_the_project() {
        let mut config = ProjectConfig::default();
        config.graph.path = "../outside.aether".into();
        let error = config.validate().unwrap_err();
        assert!(error.to_string().contains("must not escape"));
    }

    #[test]
    fn graph_path_requires_a_supported_format() {
        let mut config = ProjectConfig::default();
        config.graph.path = "semantic.json".into();

        let error = config.validate().unwrap_err();

        assert!(error.to_string().contains(".aether or .aetherb"));
    }

    #[test]
    fn commands_are_substituted_without_a_shell() {
        let config = ProjectConfig::default();
        let command = config.rust_test_command("name with spaces").unwrap();
        assert_eq!(command.program, "cargo");
        assert_eq!(command.args, ["test", "--workspace", "name with spaces"]);
        assert_eq!(
            command.display(),
            "cargo test --workspace 'name with spaces'"
        );
    }
}
