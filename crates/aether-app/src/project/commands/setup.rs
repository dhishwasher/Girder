use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

const STATE_FILE: &str = "girder-setup-state.json";
#[cfg(windows)]
const HOOK_FILE: &str = "girder_context_advisory.py";
#[cfg(windows)]
const HOOK_SOURCE: &str = include_str!("../../../../../npm/hooks/girder_context_advisory.py");
#[cfg(not(windows))]
const HOOK_FILE: &str = "girder_context_advisory.sh";
#[cfg(not(windows))]
const HOOK_SOURCE: &str = include_str!("../../../../../npm/hooks/girder_context_advisory.sh");

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Agent {
    Claude,
    Codex,
    Cursor,
    Generic,
}

impl Agent {
    fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Cursor => "Cursor",
            Self::Generic => "Generic MCP client",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "cursor" => Some(Self::Cursor),
            "generic" | "mcp" => Some(Self::Generic),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct Options {
    agents: BTreeSet<Agent>,
    dry_run: bool,
    force: bool,
    uninstall: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ConfigFormat {
    Json,
    Toml,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct McpState {
    config_path: String,
    format: ConfigFormat,
    config_created: bool,
    previous: Option<Value>,
    installed: Value,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum HookStyle {
    Nested,
    Flat,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct HookState {
    config_path: String,
    config_created: bool,
    event_key: String,
    style: HookStyle,
    previous_matches: Vec<Value>,
    installed: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ScriptState {
    path: String,
    previous: Option<String>,
    installed_sha256: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct SetupState {
    version: u32,
    mcps: Vec<McpState>,
    hooks: Vec<HookState>,
    scripts: Vec<ScriptState>,
}

#[derive(Debug)]
struct Detection {
    agent: Agent,
    config_dir: PathBuf,
    config_path: Option<PathBuf>,
    format: Option<ConfigFormat>,
    evidence: String,
}

#[derive(Debug)]
struct Change {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
}

#[derive(Debug)]
struct Environment {
    home: PathBuf,
    cwd: PathBuf,
    codex_home: Option<PathBuf>,
}

pub fn setup(args: &[String]) -> io::Result<()> {
    let home = home_dir()?;
    let cwd = std::env::current_dir()?;
    let codex_home = std::env::var_os("CODEX_HOME").map(PathBuf::from);
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    run(
        args,
        Environment {
            home,
            cwd,
            codex_home,
        },
        &mut stdout,
    )
}

fn run(args: &[String], env: Environment, out: &mut dyn Write) -> io::Result<()> {
    let options = parse_options(args)?;
    let home = existing_absolute_dir(&env.home, "home directory")?;
    let cwd = env.cwd.canonicalize().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "could not resolve current directory {}: {error}",
                env.cwd.display()
            ),
        )
    })?;

    writeln!(out, "Detection:")?;
    let detections = detect(&home, &cwd, env.codex_home.as_deref(), &options.agents);
    for detection in &detections {
        if let Some(path) = &detection.config_path {
            writeln!(
                out,
                "  {}: detected ({}) -> {}",
                detection.agent.label(),
                detection.evidence,
                path.display()
            )?;
        } else {
            writeln!(
                out,
                "  {}: not detected ({})",
                detection.agent.label(),
                detection.evidence
            )?;
        }
    }
    writeln!(out)?;

    let mut changes = Vec::new();
    let mut notes = Vec::new();
    for detection in detections.iter().filter(|item| item.config_path.is_some()) {
        if options.uninstall {
            plan_uninstall(detection, &home, &mut changes, &mut notes)?;
        } else {
            plan_install(detection, &home, options.force, &mut changes, &mut notes)?;
        }
    }

    if options.dry_run {
        writeln!(out, "Dry run: no files written.")?;
        for change in &changes {
            write_exact_diff(out, change)?;
        }
    } else {
        // Ownership records go first. If a later write is interrupted, a
        // subsequent uninstall can still identify the intended Girder entry.
        changes.sort_by_key(|change| {
            if options.uninstall {
                (is_state_path(&change.path), false)
            } else {
                (
                    !is_state_path(&change.path),
                    !is_hook_script_path(&change.path),
                )
            }
        });
        for change in &changes {
            apply_change(change)?;
        }
    }

    for note in &notes {
        writeln!(out, "{note}")?;
    }
    writeln!(out)?;
    writeln!(out, "Changed:")?;
    if changes.is_empty() {
        writeln!(out, "  nothing")?;
    } else {
        for change in &changes {
            let verb = match (&change.before, &change.after) {
                (None, Some(_)) => "created",
                (Some(_), None) => "removed",
                (Some(_), Some(_)) => "updated",
                (None, None) => "unchanged",
            };
            let prefix = if options.dry_run { "would be " } else { "" };
            writeln!(out, "  {}{} {}", prefix, verb, change.path.display())?;
        }
    }
    writeln!(out, "Undo:")?;
    if options.uninstall {
        writeln!(out, "  girder setup")?;
    } else {
        writeln!(out, "  girder setup --uninstall")?;
    }
    Ok(())
}

fn parse_options(args: &[String]) -> io::Result<Options> {
    let mut requested = None::<BTreeSet<Agent>>;
    let mut dry_run = false;
    let mut force = false;
    let mut uninstall = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" => dry_run = true,
            "--force" => force = true,
            "--uninstall" => uninstall = true,
            "--agents" => {
                index += 1;
                let list = args
                    .get(index)
                    .ok_or_else(|| invalid("--agents needs a list"))?;
                let mut agents = BTreeSet::new();
                for name in list.split(',') {
                    let agent = Agent::parse(name).ok_or_else(|| {
                        invalid(format!(
                            "unknown agent {name:?}; use claude,codex,cursor,generic"
                        ))
                    })?;
                    agents.insert(agent);
                }
                if agents.is_empty() {
                    return Err(invalid("--agents list cannot be empty"));
                }
                requested = Some(agents);
            }
            "--help" | "-h" => {
                return Err(invalid(
                    "usage: girder setup [--agents claude,codex,cursor,generic] [--dry-run] [--force] [--uninstall]",
                ));
            }
            other => return Err(invalid(format!("unknown setup option: {other}"))),
        }
        index += 1;
    }
    Ok(Options {
        agents: requested.unwrap_or_else(|| {
            [Agent::Claude, Agent::Codex, Agent::Cursor, Agent::Generic]
                .into_iter()
                .collect()
        }),
        dry_run,
        force,
        uninstall,
    })
}

fn detect(
    home: &Path,
    cwd: &Path,
    codex_home: Option<&Path>,
    requested: &BTreeSet<Agent>,
) -> Vec<Detection> {
    requested
        .iter()
        .map(|agent| match agent {
            Agent::Claude => {
                let directory = home.join(".claude");
                let project_config = cwd.join(".mcp.json");
                let user_config = home.join(".claude.json");
                if project_config.is_file() && project_config.starts_with(home) {
                    Detection {
                        agent: *agent,
                        config_dir: directory,
                        config_path: Some(project_config),
                        format: Some(ConfigFormat::Json),
                        evidence: "project .mcp.json exists".into(),
                    }
                } else if directory.is_dir() || user_config.is_file() {
                    Detection {
                        agent: *agent,
                        config_dir: directory,
                        config_path: Some(user_config),
                        format: Some(ConfigFormat::Json),
                        evidence: "~/.claude or ~/.claude.json exists".into(),
                    }
                } else {
                    Detection {
                        agent: *agent,
                        config_dir: directory,
                        config_path: None,
                        format: None,
                        evidence: "neither ~/.claude nor an in-home .mcp.json exists".into(),
                    }
                }
            }
            Agent::Codex => {
                let default_directory = home.join(".codex");
                let directory = codex_home
                    .filter(|path| path.is_absolute() && path.starts_with(home))
                    .unwrap_or(&default_directory)
                    .to_path_buf();
                Detection {
                    agent: *agent,
                    config_path: directory.is_dir().then(|| directory.join("config.toml")),
                    format: directory.is_dir().then_some(ConfigFormat::Toml),
                    evidence: if directory.is_dir() {
                        if codex_home.is_some_and(|path| path == directory) {
                            "in-home $CODEX_HOME exists".into()
                        } else {
                            "~/.codex exists".into()
                        }
                    } else if codex_home.is_some() && directory == default_directory {
                        "$CODEX_HOME is outside home, relative, or absent; ~/.codex does not exist"
                            .into()
                    } else {
                        "~/.codex does not exist".into()
                    },
                    config_dir: directory,
                }
            }
            Agent::Cursor => {
                let directory = home.join(".cursor");
                Detection {
                    agent: *agent,
                    config_path: directory.is_dir().then(|| directory.join("mcp.json")),
                    format: directory.is_dir().then_some(ConfigFormat::Json),
                    evidence: if directory.is_dir() {
                        "~/.cursor exists".into()
                    } else {
                        "~/.cursor does not exist".into()
                    },
                    config_dir: directory,
                }
            }
            Agent::Generic => Detection {
                agent: *agent,
                config_dir: home.to_path_buf(),
                config_path: None,
                format: None,
                evidence: "the MCP protocol documents no universal client config path".into(),
            },
        })
        .collect()
}

fn plan_install(
    detection: &Detection,
    home: &Path,
    force: bool,
    changes: &mut Vec<Change>,
    notes: &mut Vec<String>,
) -> io::Result<()> {
    let config_path = safe_target(
        detection.config_path.as_ref().expect("detected config"),
        home,
    )?;
    let state_path = safe_target(&detection.config_dir.join(STATE_FILE), home)?;
    let mut state = read_state(&state_path)?.unwrap_or_default();
    if state.version == 0 {
        state.version = 1;
    }
    let state_before = read_optional(&state_path)?;

    let format = detection.format.expect("detected format");
    let before = read_optional(&config_path)?;
    let desired = mcp_entry();
    let mut document = parse_config(before.as_deref(), format, &config_path)?;
    let current = get_mcp_entry(&document, format)?;
    if current.as_ref() == Some(&desired) {
        notes.push(format!(
            "{}: girder entry already configured.",
            detection.agent.label()
        ));
    } else if current.is_some() && !force {
        notes.push(format!(
            "{}: existing girder entry left unchanged (use --force to replace it).",
            detection.agent.label()
        ));
    } else {
        if let Some(owned) = state
            .mcps
            .iter_mut()
            .find(|owned| owned.config_path == config_path.to_string_lossy())
        {
            owned.installed = desired.clone();
        } else {
            state.mcps.push(McpState {
                config_path: config_path.to_string_lossy().into_owned(),
                format,
                config_created: before.is_none(),
                previous: current,
                installed: desired.clone(),
            });
        }
        let after = if format == ConfigFormat::Toml {
            rewrite_codex_toml(before.as_deref().unwrap_or_default(), Some(&desired))?
        } else {
            set_mcp_entry(&mut document, format, Some(desired))?;
            serialize_config(&document, format)?
        };
        push_change(changes, config_path, before, Some(after));
    }

    plan_hook_install(detection, home, force, &mut state, changes, notes)?;
    if has_ownership(&state) {
        let state_after = serde_json::to_vec_pretty(&state)
            .map(append_newline)
            .map_err(json_error)?;
        push_change(changes, state_path, state_before, Some(state_after));
    }
    Ok(())
}

fn plan_uninstall(
    detection: &Detection,
    home: &Path,
    changes: &mut Vec<Change>,
    notes: &mut Vec<String>,
) -> io::Result<()> {
    let state_path = safe_target(&detection.config_dir.join(STATE_FILE), home)?;
    let state_before = read_optional(&state_path)?;
    let Some(mut state) = read_state_bytes(state_before.as_deref(), &state_path)? else {
        notes.push(format!(
            "{}: no setup-owned entry to remove.",
            detection.agent.label()
        ));
        return Ok(());
    };
    let mut retained = Vec::new();
    for owned in std::mem::take(&mut state.mcps) {
        let config_path = safe_target(Path::new(&owned.config_path), home)?;
        let before = read_optional(&config_path)?;
        if let Some(bytes) = before.as_deref() {
            let mut document = parse_config(Some(bytes), owned.format, &config_path)?;
            let current = get_mcp_entry(&document, owned.format)?;
            if current.as_ref() != Some(&owned.installed) {
                notes.push(format!(
                    "{}: girder entry in {} changed after setup; left it and its ownership record unchanged.",
                    detection.agent.label(),
                    config_path.display()
                ));
                retained.push(owned);
                continue;
            }
            let after = if owned.format == ConfigFormat::Toml {
                rewrite_codex_toml(bytes, owned.previous.as_ref())?
            } else {
                set_mcp_entry(&mut document, owned.format, owned.previous.clone())?;
                serialize_config(&document, owned.format)?
            };
            let after_document = parse_config(Some(&after), owned.format, &config_path)?;
            if owned.config_created && config_is_empty(&after_document, owned.format)? {
                push_change(changes, config_path, before, None);
            } else {
                push_change(changes, config_path, before, Some(after));
            }
        }
    }
    state.mcps = retained;

    uninstall_hooks(detection, home, &mut state, changes, notes)?;

    if state.mcps.is_empty() && state.hooks.is_empty() && state.scripts.is_empty() {
        push_change(changes, state_path, state_before, None);
    } else {
        let after = serde_json::to_vec_pretty(&state)
            .map(append_newline)
            .map_err(json_error)?;
        push_change(changes, state_path, state_before, Some(after));
    }
    Ok(())
}

fn plan_hook_install(
    detection: &Detection,
    home: &Path,
    force: bool,
    state: &mut SetupState,
    changes: &mut Vec<Change>,
    notes: &mut Vec<String>,
) -> io::Result<()> {
    let Some((event_key, style)) = hook_shape(detection.agent) else {
        if detection.agent == Agent::Codex {
            notes.push(
                "Codex: advisory hook not registered because documented file reads arrive as unstructured Bash commands, not a structured Read payload."
                    .into(),
            );
        }
        return Ok(());
    };
    let script_path = safe_target(&detection.config_dir.join("hooks").join(HOOK_FILE), home)?;
    let script_before = read_optional(&script_path)?;
    let script_matches = script_before.as_deref() == Some(HOOK_SOURCE.as_bytes());
    if script_before.is_some() && !script_matches && !force {
        notes.push(format!(
            "{}: existing {} left unchanged; advisory hook was not registered (use --force to replace it).",
            detection.agent.label(),
            script_path.display()
        ));
        return Ok(());
    }
    if !script_matches {
        if let Some(owned) = state
            .scripts
            .iter_mut()
            .find(|owned| owned.path == script_path.to_string_lossy())
        {
            owned.installed_sha256 = sha256(HOOK_SOURCE.as_bytes());
        } else {
            state.scripts.push(ScriptState {
                path: script_path.to_string_lossy().into_owned(),
                previous: script_before
                    .as_deref()
                    .map(std::str::from_utf8)
                    .transpose()
                    .map_err(|error| {
                        invalid(format!(
                            "existing hook {} is not UTF-8: {error}",
                            script_path.display()
                        ))
                    })?
                    .map(str::to_owned),
                installed_sha256: sha256(HOOK_SOURCE.as_bytes()),
            });
        }
        push_change(
            changes,
            script_path.clone(),
            script_before,
            Some(HOOK_SOURCE.as_bytes().to_vec()),
        );
    }

    let config_path = safe_target(&hook_config_path(detection), home)?;
    let before = read_optional(&config_path)?;
    let mut document = parse_config(before.as_deref(), ConfigFormat::Json, &config_path)?;
    let desired = hook_entry(style, &script_path);
    let matches = matching_hooks(&document, event_key, style)?;
    if matches.len() == 1 && matches[0] == desired {
        notes.push(format!(
            "{}: Girder advisory hook already configured.",
            detection.agent.label()
        ));
        return Ok(());
    }
    if !matches.is_empty() && !force {
        notes.push(format!(
            "{}: existing Girder advisory hook left unchanged (use --force to replace it).",
            detection.agent.label()
        ));
        return Ok(());
    }
    if force
        && matches
            .iter()
            .any(|entry| !hook_group_is_safely_replaceable(entry, style))
    {
        notes.push(format!(
            "{}: a Girder advisory handler shares a hook group with other handlers; left the group unchanged.",
            detection.agent.label()
        ));
        return Ok(());
    }

    if let Some(owned) = state
        .hooks
        .iter_mut()
        .find(|owned| owned.config_path == config_path.to_string_lossy())
    {
        owned.installed = desired.clone();
    } else {
        state.hooks.push(HookState {
            config_path: config_path.to_string_lossy().into_owned(),
            config_created: before.is_none(),
            event_key: event_key.into(),
            style,
            previous_matches: matches,
            installed: desired.clone(),
        });
    }
    replace_matching_hooks(&mut document, event_key, style, Some(desired))?;
    let after = serialize_config(&document, ConfigFormat::Json)?;
    push_change(changes, config_path, before, Some(after));
    Ok(())
}

fn uninstall_hooks(
    detection: &Detection,
    home: &Path,
    state: &mut SetupState,
    changes: &mut Vec<Change>,
    notes: &mut Vec<String>,
) -> io::Result<()> {
    let mut retained_hooks = Vec::new();
    for owned in std::mem::take(&mut state.hooks) {
        let config_path = safe_target(Path::new(&owned.config_path), home)?;
        let before = read_optional(&config_path)?;
        if let Some(bytes) = before.as_deref() {
            let mut document = parse_config(Some(bytes), ConfigFormat::Json, &config_path)?;
            let matches = matching_hooks(&document, &owned.event_key, owned.style)?;
            if !matches.iter().any(|entry| entry == &owned.installed) {
                if !matches.is_empty() {
                    notes.push(format!(
                        "{}: advisory hook in {} changed after setup; left it and its ownership record unchanged.",
                        detection.agent.label(),
                        config_path.display()
                    ));
                    retained_hooks.push(owned);
                }
                continue;
            }
            remove_exact_hook(&mut document, &owned.event_key, &owned.installed)?;
            append_hooks(
                &mut document,
                &owned.event_key,
                owned.previous_matches.clone(),
            )?;
            if owned.config_created && config_is_empty(&document, ConfigFormat::Json)? {
                push_change(changes, config_path, before, None);
            } else {
                let after = serialize_config(&document, ConfigFormat::Json)?;
                push_change(changes, config_path, before, Some(after));
            }
        }
    }
    state.hooks = retained_hooks;

    let preserve_scripts = !state.hooks.is_empty();
    let mut retained_scripts = Vec::new();
    for owned in std::mem::take(&mut state.scripts) {
        if preserve_scripts {
            retained_scripts.push(owned);
            continue;
        }
        let script_path = safe_target(Path::new(&owned.path), home)?;
        let before = read_optional(&script_path)?;
        if let Some(bytes) = before.as_deref() {
            if sha256(bytes) != owned.installed_sha256 {
                notes.push(format!(
                    "{}: hook script {} changed after setup; left it and its ownership record unchanged.",
                    detection.agent.label(),
                    script_path.display()
                ));
                retained_scripts.push(owned);
                continue;
            }
            push_change(
                changes,
                script_path,
                before,
                owned.previous.map(String::into_bytes),
            );
        }
    }
    state.scripts = retained_scripts;
    Ok(())
}

fn hook_shape(agent: Agent) -> Option<(&'static str, HookStyle)> {
    match agent {
        Agent::Claude => Some(("PreToolUse", HookStyle::Nested)),
        Agent::Cursor => Some(("preToolUse", HookStyle::Flat)),
        Agent::Codex | Agent::Generic => None,
    }
}

fn hook_config_path(detection: &Detection) -> PathBuf {
    match detection.agent {
        Agent::Claude => detection.config_dir.join("settings.json"),
        Agent::Codex | Agent::Cursor => detection.config_dir.join("hooks.json"),
        Agent::Generic => unreachable!("generic clients have no hook path"),
    }
}

fn hook_entry(style: HookStyle, script_path: &Path) -> Value {
    let interpreter = if cfg!(windows) { "python" } else { "sh" };
    let command = format!(
        "{interpreter} {}",
        shell_quote(&script_path.to_string_lossy())
    );
    match style {
        HookStyle::Nested => json!({
            "matcher": "Read",
            "hooks": [{"type": "command", "command": command}]
        }),
        HookStyle::Flat => json!({"command": command, "matcher": "Read"}),
    }
}

fn shell_quote(value: &str) -> String {
    if cfg!(windows) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn matching_hooks(document: &Value, event_key: &str, style: HookStyle) -> io::Result<Vec<Value>> {
    let Some(entries) = hook_entries(document, event_key)? else {
        return Ok(Vec::new());
    };
    Ok(entries
        .iter()
        .filter(|entry| hook_group_contains_girder(entry, style))
        .cloned()
        .collect())
}

fn hook_entries<'a>(document: &'a Value, event_key: &str) -> io::Result<Option<&'a Vec<Value>>> {
    let root = document
        .as_object()
        .ok_or_else(|| invalid("hook config root must be an object"))?;
    let Some(hooks) = root.get("hooks") else {
        return Ok(None);
    };
    let hooks = hooks
        .as_object()
        .ok_or_else(|| invalid("hooks must be an object"))?;
    let Some(entries) = hooks.get(event_key) else {
        return Ok(None);
    };
    entries
        .as_array()
        .map(Some)
        .ok_or_else(|| invalid(format!("hooks.{event_key} must be an array")))
}

fn hook_group_contains_girder(entry: &Value, style: HookStyle) -> bool {
    match style {
        HookStyle::Flat => entry
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(is_girder_hook_command),
        HookStyle::Nested => entry
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|handlers| {
                handlers.iter().any(|handler| {
                    handler
                        .get("command")
                        .and_then(Value::as_str)
                        .is_some_and(is_girder_hook_command)
                })
            }),
    }
}

fn hook_group_is_safely_replaceable(entry: &Value, style: HookStyle) -> bool {
    match style {
        HookStyle::Flat => hook_group_contains_girder(entry, style),
        HookStyle::Nested => entry
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|handlers| {
                !handlers.is_empty()
                    && handlers.iter().all(|handler| {
                        handler
                            .get("command")
                            .and_then(Value::as_str)
                            .is_some_and(is_girder_hook_command)
                    })
            }),
    }
}

fn replace_matching_hooks(
    document: &mut Value,
    event_key: &str,
    style: HookStyle,
    installed: Option<Value>,
) -> io::Result<()> {
    let entries = hook_entries_mut(document, event_key, installed.is_some())?;
    if let Some(entries) = entries {
        entries.retain(|entry| {
            !(hook_group_contains_girder(entry, style)
                && hook_group_is_safely_replaceable(entry, style))
        });
        if let Some(installed) = installed {
            entries.push(installed);
        }
    }
    prune_empty_hooks(document, event_key)?;
    Ok(())
}

fn remove_exact_hook(document: &mut Value, event_key: &str, installed: &Value) -> io::Result<()> {
    if let Some(entries) = hook_entries_mut(document, event_key, false)? {
        if let Some(index) = entries.iter().position(|entry| entry == installed) {
            entries.remove(index);
        }
    }
    prune_empty_hooks(document, event_key)
}

fn is_girder_hook_command(command: &str) -> bool {
    command.contains("girder_context_advisory.py") || command.contains("girder_context_advisory.sh")
}

fn append_hooks(document: &mut Value, event_key: &str, entries: Vec<Value>) -> io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    hook_entries_mut(document, event_key, true)?
        .expect("created hook array")
        .extend(entries);
    Ok(())
}

fn hook_entries_mut<'a>(
    document: &'a mut Value,
    event_key: &str,
    create: bool,
) -> io::Result<Option<&'a mut Vec<Value>>> {
    let root = document
        .as_object_mut()
        .ok_or_else(|| invalid("hook config root must be an object"))?;
    if create && !root.contains_key("hooks") {
        root.insert("hooks".into(), Value::Object(Map::new()));
    }
    let Some(hooks) = root.get_mut("hooks") else {
        return Ok(None);
    };
    let hooks = hooks
        .as_object_mut()
        .ok_or_else(|| invalid("hooks must be an object"))?;
    if create && !hooks.contains_key(event_key) {
        hooks.insert(event_key.into(), Value::Array(Vec::new()));
    }
    let Some(entries) = hooks.get_mut(event_key) else {
        return Ok(None);
    };
    entries
        .as_array_mut()
        .map(Some)
        .ok_or_else(|| invalid(format!("hooks.{event_key} must be an array")))
}

fn prune_empty_hooks(document: &mut Value, event_key: &str) -> io::Result<()> {
    let root = document
        .as_object_mut()
        .ok_or_else(|| invalid("hook config root must be an object"))?;
    let Some(hooks) = root.get_mut("hooks") else {
        return Ok(());
    };
    let hooks = hooks
        .as_object_mut()
        .ok_or_else(|| invalid("hooks must be an object"))?;
    if hooks
        .get(event_key)
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        hooks.remove(event_key);
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn has_ownership(state: &SetupState) -> bool {
    !(state.mcps.is_empty() && state.hooks.is_empty() && state.scripts.is_empty())
}

fn mcp_entry() -> Value {
    json!({"command": "npx", "args": ["-y", "girder-mcp", "."]})
}

fn parse_config(bytes: Option<&[u8]>, format: ConfigFormat, path: &Path) -> io::Result<Value> {
    let text = match bytes {
        Some(bytes) => std::str::from_utf8(bytes)
            .map_err(|error| invalid(format!("{} is not valid UTF-8: {error}", path.display())))?,
        None => "",
    };
    match format {
        ConfigFormat::Json => {
            if text.trim().is_empty() {
                Ok(Value::Object(Map::new()))
            } else {
                serde_json::from_str(text).map_err(|error| {
                    invalid(format!(
                        "could not parse {} as JSON: {error}",
                        path.display()
                    ))
                })
            }
        }
        ConfigFormat::Toml => {
            let value = if text.trim().is_empty() {
                toml::Value::Table(toml::map::Map::new())
            } else {
                toml::from_str::<toml::Value>(text).map_err(|error| {
                    invalid(format!(
                        "could not parse {} as TOML: {error}",
                        path.display()
                    ))
                })?
            };
            serde_json::to_value(value).map_err(json_error)
        }
    }
}

fn serialize_config(document: &Value, format: ConfigFormat) -> io::Result<Vec<u8>> {
    match format {
        ConfigFormat::Json => serde_json::to_vec_pretty(document)
            .map(append_newline)
            .map_err(json_error),
        ConfigFormat::Toml => toml::to_string_pretty(document)
            .map(|text| text.into_bytes())
            .map_err(|error| invalid(format!("could not encode TOML: {error}"))),
    }
}

fn rewrite_codex_toml(before: &[u8], entry: Option<&Value>) -> io::Result<Vec<u8>> {
    let text = std::str::from_utf8(before)
        .map_err(|error| invalid(format!("Codex config is not UTF-8: {error}")))?;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let start = lines
        .iter()
        .position(|line| line.trim() == "[mcp_servers.girder]");
    let existing = parse_config(Some(before), ConfigFormat::Toml, Path::new("config.toml"))?;
    if get_mcp_entry(&existing, ConfigFormat::Toml)?.is_some() && start.is_none() {
        return Err(invalid(
            "Codex girder entry uses an inline or unsupported TOML shape; left config unchanged",
        ));
    }

    let mut after = String::new();
    if let Some(start) = start {
        let end = lines[start + 1..]
            .iter()
            .position(|line| line.trim_start().starts_with('['))
            .map_or(lines.len(), |relative| start + 1 + relative);
        for line in &lines[..start] {
            after.push_str(line);
        }
        if let Some(entry) = entry {
            after.push_str(&codex_girder_table(entry)?);
        }
        for line in &lines[end..] {
            after.push_str(line);
        }
    } else {
        after.push_str(text);
        if let Some(entry) = entry {
            if !after.is_empty() && !after.ends_with('\n') {
                after.push('\n');
            }
            after.push_str(&codex_girder_table(entry)?);
        }
    }
    let after_document = parse_config(
        Some(after.as_bytes()),
        ConfigFormat::Toml,
        Path::new("config.toml"),
    )?;
    if get_mcp_entry(&after_document, ConfigFormat::Toml)?.as_ref() != entry {
        return Err(invalid(
            "Codex TOML rewrite did not produce the requested entry",
        ));
    }
    Ok(after.into_bytes())
}

fn codex_girder_table(entry: &Value) -> io::Result<String> {
    toml::to_string_pretty(&json!({"mcp_servers": {"girder": entry}}))
        .map_err(|error| invalid(format!("could not encode Codex girder entry: {error}")))
}

fn get_mcp_entry(document: &Value, format: ConfigFormat) -> io::Result<Option<Value>> {
    let key = match format {
        ConfigFormat::Json => "mcpServers",
        ConfigFormat::Toml => "mcp_servers",
    };
    let root = document
        .as_object()
        .ok_or_else(|| invalid("agent config root must be an object/table"))?;
    let Some(servers) = root.get(key) else {
        return Ok(None);
    };
    let servers = servers
        .as_object()
        .ok_or_else(|| invalid(format!("{key} must be an object/table")))?;
    Ok(servers.get("girder").cloned())
}

fn set_mcp_entry(
    document: &mut Value,
    format: ConfigFormat,
    entry: Option<Value>,
) -> io::Result<()> {
    let key = match format {
        ConfigFormat::Json => "mcpServers",
        ConfigFormat::Toml => "mcp_servers",
    };
    let root = document
        .as_object_mut()
        .ok_or_else(|| invalid("agent config root must be an object/table"))?;
    if entry.is_some() && !root.contains_key(key) {
        root.insert(key.into(), Value::Object(Map::new()));
    }
    if let Some(servers) = root.get_mut(key) {
        let servers = servers
            .as_object_mut()
            .ok_or_else(|| invalid(format!("{key} must be an object/table")))?;
        match entry {
            Some(value) => {
                servers.insert("girder".into(), value);
            }
            None => {
                servers.remove("girder");
            }
        }
        if servers.is_empty() {
            root.remove(key);
        }
    }
    Ok(())
}

fn config_is_empty(document: &Value, _format: ConfigFormat) -> io::Result<bool> {
    document
        .as_object()
        .map(Map::is_empty)
        .ok_or_else(|| invalid("agent config root must be an object/table"))
}

fn read_state(path: &Path) -> io::Result<Option<SetupState>> {
    let bytes = read_optional(path)?;
    read_state_bytes(bytes.as_deref(), path)
}

fn read_state_bytes(bytes: Option<&[u8]>, path: &Path) -> io::Result<Option<SetupState>> {
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    serde_json::from_slice(bytes).map(Some).map_err(|error| {
        invalid(format!(
            "could not parse Girder ownership record {}: {error}",
            path.display()
        ))
    })
}

fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn push_change(
    changes: &mut Vec<Change>,
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
) {
    if before != after {
        changes.push(Change {
            path,
            before,
            after,
        });
    }
}

fn apply_change(change: &Change) -> io::Result<()> {
    let current = read_optional(&change.path)?;
    if current != change.before {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            format!(
                "{} changed while setup was planning; no overwrite was attempted",
                change.path.display()
            ),
        ));
    }
    match &change.after {
        Some(bytes) => atomic_write(&change.path, bytes, is_state_path(&change.path)),
        None => match std::fs::remove_file(&change.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        },
    }
}

fn atomic_write(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid(format!("{} has no parent directory", path.display())))?;
    std::fs::create_dir_all(parent)?;
    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(format!(".girder-setup-{}.tmp", std::process::id()));
    let temporary = parent.join(name);
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        let permissions = match std::fs::metadata(path) {
            Ok(metadata) => metadata.permissions(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                new_file_permissions(&file, private)?
            }
            Err(error) => return Err(error),
        };
        file.set_permissions(permissions)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn new_file_permissions(_file: &std::fs::File, _private: bool) -> io::Result<std::fs::Permissions> {
    use std::os::unix::fs::PermissionsExt;
    Ok(std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn new_file_permissions(file: &std::fs::File, _private: bool) -> io::Result<std::fs::Permissions> {
    file.metadata().map(|metadata| metadata.permissions())
}

fn write_exact_diff(out: &mut dyn Write, change: &Change) -> io::Result<()> {
    let old_label = if change.before.is_some() {
        change.path.display().to_string()
    } else {
        "/dev/null".into()
    };
    let new_label = if change.after.is_some() {
        change.path.display().to_string()
    } else {
        "/dev/null".into()
    };
    writeln!(out, "--- {old_label}")?;
    writeln!(out, "+++ {new_label}")?;
    let before = change.before.as_deref().unwrap_or_default();
    let after = change.after.as_deref().unwrap_or_default();
    let old_count = diff_line_count(before);
    let new_count = diff_line_count(after);
    writeln!(out, "@@ -1,{old_count} +1,{new_count} @@")?;
    write_diff_lines(out, b'-', before)?;
    write_diff_lines(out, b'+', after)?;
    Ok(())
}

fn diff_line_count(bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        0
    } else {
        bytes.iter().filter(|byte| **byte == b'\n').count() + usize::from(!bytes.ends_with(b"\n"))
    }
}

fn write_diff_lines(out: &mut dyn Write, prefix: u8, bytes: &[u8]) -> io::Result<()> {
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        out.write_all(&[prefix])?;
        out.write_all(line)?;
        if !line.ends_with(b"\n") {
            out.write_all(b"\n\\ No newline at end of file\n")?;
        }
    }
    Ok(())
}

fn safe_target(path: &Path, home: &Path) -> io::Result<PathBuf> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(invalid(format!(
            "refusing to access non-normalized path {}",
            path.display()
        )));
    }
    if !path.is_absolute() || !path.starts_with(home) {
        return Err(invalid(format!(
            "refusing to access {} outside home {}",
            path.display(),
            home.display()
        )));
    }
    let mut ancestor = path;
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| invalid(format!("could not resolve {}", path.display())))?;
    }
    let canonical_ancestor = ancestor.canonicalize()?;
    if !canonical_ancestor.starts_with(home) {
        return Err(invalid(format!(
            "refusing to follow {} outside home {}",
            path.display(),
            home.display()
        )));
    }
    Ok(path.to_path_buf())
}

fn existing_absolute_dir(path: &Path, label: &str) -> io::Result<PathBuf> {
    let canonical = path.canonicalize().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("could not resolve {label} {}: {error}", path.display()),
        )
    })?;
    if !canonical.is_dir() {
        return Err(invalid(format!("{label} is not a directory")));
    }
    Ok(canonical)
}

fn home_dir() -> io::Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| invalid("HOME is not set"))
}

fn append_newline(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.push(b'\n');
    bytes
}

fn is_state_path(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == STATE_FILE)
}

fn is_hook_script_path(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == HOOK_FILE)
}

fn json_error(error: serde_json::Error) -> io::Error {
    invalid(format!("JSON error: {error}"))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempHome(PathBuf);

    impl TempHome {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "girder-setup-{name}-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }

        fn mkdir(&self, relative: &str) {
            std::fs::create_dir_all(self.0.join(relative)).unwrap();
        }

        fn write(&self, relative: &str, text: &str) {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, text).unwrap();
        }

        fn run(&self, args: &[&str]) -> String {
            let mut output = Vec::new();
            run(
                &args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>(),
                Environment {
                    home: self.0.clone(),
                    cwd: self.0.clone(),
                    codex_home: None,
                },
                &mut output,
            )
            .unwrap();
            String::from_utf8(output).unwrap()
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn dry_run_reports_exact_diff_and_writes_nothing() {
        let home = TempHome::new("dry-run");
        home.mkdir(".codex");
        home.write(".codex/config.toml", "model = \"gpt\"\n");

        let output = home.run(&["--agents", "codex", "--dry-run"]);

        assert!(output.contains("Codex: detected"));
        assert!(output.contains("--- "));
        assert!(output.contains("+[mcp_servers.girder]"));
        assert!(output.contains("+command = \"npx\""));
        assert!(output.contains("Dry run: no files written."));
        assert_eq!(
            std::fs::read_to_string(home.0.join(".codex/config.toml")).unwrap(),
            "model = \"gpt\"\n"
        );
        assert!(!home.0.join(".codex").join(STATE_FILE).exists());
    }

    #[test]
    fn setup_merges_idempotently_and_uninstall_restores_each_agent() {
        let home = TempHome::new("round-trip");
        home.mkdir(".claude");
        home.mkdir(".codex");
        home.mkdir(".cursor");
        home.write(
            ".claude.json",
            "{\"mcpServers\":{\"other\":{\"command\":\"keep\"}}}\n",
        );
        home.write(".codex/config.toml", "model = \"gpt\"\n");
        home.write(".cursor/mcp.json", "{\"theme\":\"dark\"}\n");

        let first = home.run(&["--agents", "claude,codex,cursor"]);
        assert!(first.contains("girder setup --uninstall"));
        let claude_after = std::fs::read(home.0.join(".claude.json")).unwrap();
        let codex_after = std::fs::read(home.0.join(".codex/config.toml")).unwrap();
        let cursor_after = std::fs::read(home.0.join(".cursor/mcp.json")).unwrap();
        let claude_hooks: Value =
            serde_json::from_slice(&std::fs::read(home.0.join(".claude/settings.json")).unwrap())
                .unwrap();
        let cursor_hooks: Value =
            serde_json::from_slice(&std::fs::read(home.0.join(".cursor/hooks.json")).unwrap())
                .unwrap();
        assert_eq!(claude_hooks["hooks"]["PreToolUse"][0]["matcher"], "Read");
        assert_eq!(cursor_hooks["hooks"]["preToolUse"][0]["matcher"], "Read");
        assert!(first.contains("Codex: advisory hook not registered"));
        for directory in [".claude", ".cursor"] {
            assert_eq!(
                std::fs::read_to_string(home.0.join(directory).join("hooks").join(HOOK_FILE))
                    .unwrap(),
                HOOK_SOURCE
            );
        }
        let second = home.run(&["--agents", "claude,codex,cursor"]);
        assert!(second.contains("Changed:\n  nothing"));
        assert_eq!(
            std::fs::read(home.0.join(".claude.json")).unwrap(),
            claude_after
        );
        assert_eq!(
            std::fs::read(home.0.join(".codex/config.toml")).unwrap(),
            codex_after
        );
        assert_eq!(
            std::fs::read(home.0.join(".cursor/mcp.json")).unwrap(),
            cursor_after
        );

        home.run(&["--agents", "claude,codex,cursor", "--uninstall"]);
        assert_eq!(
            std::fs::read_to_string(home.0.join(".claude.json")).unwrap(),
            "{\n  \"mcpServers\": {\n    \"other\": {\n      \"command\": \"keep\"\n    }\n  }\n}\n"
        );
        assert_eq!(
            std::fs::read_to_string(home.0.join(".codex/config.toml")).unwrap(),
            "model = \"gpt\"\n"
        );
        assert_eq!(
            std::fs::read_to_string(home.0.join(".cursor/mcp.json")).unwrap(),
            "{\n  \"theme\": \"dark\"\n}\n"
        );
        for path in [
            home.0.join(".claude/settings.json"),
            home.0.join(".cursor/hooks.json"),
            home.0.join(".claude/hooks").join(HOOK_FILE),
            home.0.join(".cursor/hooks").join(HOOK_FILE),
        ] {
            assert!(
                !path.exists(),
                "setup-owned file survived: {}",
                path.display()
            );
        }
    }

    #[test]
    fn existing_entry_is_untouched_without_force_and_restored_after_force() {
        let home = TempHome::new("force");
        home.mkdir(".cursor");
        home.write(
            ".cursor/mcp.json",
            "{\"mcpServers\":{\"girder\":{\"command\":\"custom\"}}}\n",
        );

        let untouched = home.run(&["--agents", "cursor"]);
        assert!(untouched.contains("existing girder entry left unchanged"));
        let untouched_config: Value =
            serde_json::from_slice(&std::fs::read(home.0.join(".cursor/mcp.json")).unwrap())
                .unwrap();
        assert_eq!(
            untouched_config["mcpServers"]["girder"]["command"],
            "custom"
        );
        assert!(home.0.join(".cursor").join(STATE_FILE).exists());

        home.run(&["--agents", "cursor", "--force"]);
        let configured: Value =
            serde_json::from_slice(&std::fs::read(home.0.join(".cursor/mcp.json")).unwrap())
                .unwrap();
        assert_eq!(configured["mcpServers"]["girder"], mcp_entry());

        home.run(&["--agents", "cursor", "--uninstall"]);
        let restored: Value =
            serde_json::from_slice(&std::fs::read(home.0.join(".cursor/mcp.json")).unwrap())
                .unwrap();
        assert_eq!(restored["mcpServers"]["girder"]["command"], "custom");
    }

    #[test]
    fn uninstall_preserves_a_setup_entry_modified_later() {
        let home = TempHome::new("changed-after");
        home.mkdir(".cursor");
        home.run(&["--agents", "cursor"]);
        home.write(
            ".cursor/mcp.json",
            "{\"mcpServers\":{\"girder\":{\"command\":\"mine\"}}}\n",
        );

        let output = home.run(&["--agents", "cursor", "--uninstall"]);
        assert!(output.contains("changed after setup; left it"));
        assert!(home.0.join(".cursor").join(STATE_FILE).exists());
        let config = std::fs::read_to_string(home.0.join(".cursor/mcp.json")).unwrap();
        assert!(config.contains("mine"));
    }

    #[test]
    fn project_mcp_config_is_used_only_when_it_is_inside_home() {
        let home = TempHome::new("project-config");
        home.write(".mcp.json", "{}\n");
        let output = home.run(&["--agents", "claude"]);
        assert!(output.contains("project .mcp.json exists"));
        let config: Value =
            serde_json::from_slice(&std::fs::read(home.0.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["girder"], mcp_entry());
    }

    #[test]
    fn claude_tracks_and_uninstalls_more_than_one_project_config() {
        let home = TempHome::new("multiple-projects");
        home.mkdir(".claude");
        home.mkdir("one");
        home.mkdir("two");
        home.write("one/.mcp.json", "{}\n");
        home.write("two/.mcp.json", "{}\n");
        let args = vec!["--agents".to_owned(), "claude".to_owned()];
        for project in ["one", "two"] {
            let mut output = Vec::new();
            run(
                &args,
                Environment {
                    home: home.0.clone(),
                    cwd: home.0.join(project),
                    codex_home: None,
                },
                &mut output,
            )
            .unwrap();
        }

        home.run(&["--agents", "claude", "--uninstall"]);
        for project in ["one", "two"] {
            let config: Value = serde_json::from_slice(
                &std::fs::read(home.0.join(project).join(".mcp.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(config, json!({}));
        }
    }

    #[test]
    fn generic_config_is_reported_as_unavailable_without_guessing() {
        let home = TempHome::new("generic");
        let output = home.run(&["--agents", "generic"]);
        assert!(output.contains("Generic MCP client: not detected"));
        assert!(output.contains("no universal client config path"));
        assert!(output.contains("Changed:\n  nothing"));
    }

    #[test]
    fn force_replaces_and_uninstall_restores_an_existing_hook() {
        let home = TempHome::new("force-hook");
        home.mkdir(".cursor");
        home.write(
            ".cursor/hooks.json",
            "{\"hooks\":{\"preToolUse\":[{\"command\":\"python old/girder_context_advisory.py\",\"matcher\":\"Read\"},{\"command\":\"keep\",\"matcher\":\"Shell\"}]}}\n",
        );
        let untouched = home.run(&["--agents", "cursor"]);
        assert!(untouched.contains("existing Girder advisory hook left unchanged"));

        home.run(&["--agents", "cursor", "--force"]);
        let configured: Value =
            serde_json::from_slice(&std::fs::read(home.0.join(".cursor/hooks.json")).unwrap())
                .unwrap();
        let configured_entries = configured["hooks"]["preToolUse"].as_array().unwrap();
        assert_eq!(configured_entries.len(), 2);
        assert!(configured_entries
            .iter()
            .any(|entry| entry["command"] == "keep"));
        assert!(configured_entries.iter().any(|entry| {
            entry["command"]
                .as_str()
                .is_some_and(|command| command.contains(&home.0.to_string_lossy().to_string()))
        }));

        home.run(&["--agents", "cursor", "--uninstall"]);
        let restored: Value =
            serde_json::from_slice(&std::fs::read(home.0.join(".cursor/hooks.json")).unwrap())
                .unwrap();
        let restored_entries = restored["hooks"]["preToolUse"].as_array().unwrap();
        assert!(restored_entries
            .iter()
            .any(|entry| entry["command"] == "python old/girder_context_advisory.py"));
        assert!(restored_entries
            .iter()
            .any(|entry| entry["command"] == "keep"));
    }

    #[test]
    fn uninstall_removes_only_the_exact_owned_hook_entry() {
        let home = TempHome::new("exact-hook-uninstall");
        home.mkdir(".cursor");
        home.run(&["--agents", "cursor"]);
        let hooks_path = home.0.join(".cursor/hooks.json");
        let mut document: Value =
            serde_json::from_slice(&std::fs::read(&hooks_path).unwrap()).unwrap();
        document["hooks"]["preToolUse"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "command": "python custom/girder_context_advisory.py",
                "matcher": "Read"
            }));
        std::fs::write(
            &hooks_path,
            append_newline(serde_json::to_vec_pretty(&document).unwrap()),
        )
        .unwrap();

        home.run(&["--agents", "cursor", "--uninstall"]);
        let remaining: Value =
            serde_json::from_slice(&std::fs::read(&hooks_path).unwrap()).unwrap();
        let entries = remaining["hooks"]["preToolUse"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0]["command"],
            "python custom/girder_context_advisory.py"
        );
    }

    #[test]
    fn uninstall_keeps_the_script_when_its_registration_was_modified() {
        let home = TempHome::new("modified-hook");
        home.mkdir(".cursor");
        home.run(&["--agents", "cursor"]);
        let hooks_path = home.0.join(".cursor/hooks.json");
        let mut document: Value =
            serde_json::from_slice(&std::fs::read(&hooks_path).unwrap()).unwrap();
        document["hooks"]["preToolUse"][0]["timeout"] = json!(5);
        std::fs::write(
            &hooks_path,
            append_newline(serde_json::to_vec_pretty(&document).unwrap()),
        )
        .unwrap();

        let output = home.run(&["--agents", "cursor", "--uninstall"]);
        assert!(output.contains("advisory hook") && output.contains("changed after setup"));
        assert!(home.0.join(".cursor/hooks").join(HOOK_FILE).exists());
        assert!(home.0.join(".cursor").join(STATE_FILE).exists());
    }

    #[test]
    fn force_does_not_replace_a_nested_group_with_unrelated_handlers() {
        let home = TempHome::new("mixed-hook-group");
        home.mkdir(".claude");
        home.write(
            ".claude/settings.json",
            "{\"hooks\":{\"PreToolUse\":[{\"matcher\":\"Read\",\"hooks\":[{\"type\":\"command\",\"command\":\"keep\"},{\"type\":\"command\",\"command\":\"python old/girder_context_advisory.py\"}]}]}}\n",
        );

        let output = home.run(&["--agents", "claude", "--force"]);
        assert!(output.contains("shares a hook group with other handlers"));
        let document: Value =
            serde_json::from_slice(&std::fs::read(home.0.join(".claude/settings.json")).unwrap())
                .unwrap();
        let handlers = document["hooks"]["PreToolUse"][0]["hooks"]
            .as_array()
            .unwrap();
        assert_eq!(handlers.len(), 2);
        assert!(handlers.iter().any(|handler| handler["command"] == "keep"));
    }

    #[test]
    fn codex_home_override_is_used_only_inside_home() {
        let home = TempHome::new("codex-home");
        home.mkdir("custom-codex");
        let mut output = Vec::new();
        run(
            &["--agents".into(), "codex".into()],
            Environment {
                home: home.0.clone(),
                cwd: home.0.clone(),
                codex_home: Some(home.0.join("custom-codex")),
            },
            &mut output,
        )
        .unwrap();
        assert!(home.0.join("custom-codex/config.toml").exists());
        assert!(!home.0.join(".codex/config.toml").exists());
        assert!(String::from_utf8(output)
            .unwrap()
            .contains("in-home $CODEX_HOME exists"));
    }

    #[test]
    fn codex_merge_preserves_comments_and_other_tables_byte_for_byte() {
        let home = TempHome::new("codex-comments");
        home.mkdir(".codex");
        let original = "# private agent preferences\nmodel = \"gpt\"\n\n[features]\nweb_search = true # keep this\n";
        home.write(".codex/config.toml", original);

        home.run(&["--agents", "codex"]);
        let installed = std::fs::read_to_string(home.0.join(".codex/config.toml")).unwrap();
        assert!(installed.starts_with(original));
        assert!(installed.contains("[mcp_servers.girder]"));

        home.run(&["--agents", "codex", "--uninstall"]);
        assert_eq!(
            std::fs::read_to_string(home.0.join(".codex/config.toml")).unwrap(),
            original
        );
    }

    #[test]
    fn exact_diff_marks_a_missing_final_newline_and_preserves_crlf() {
        let change = Change {
            path: PathBuf::from("config.json"),
            before: Some(b"old\r\nlast".to_vec()),
            after: Some(b"new\r\n".to_vec()),
        };
        let mut output = Vec::new();
        write_exact_diff(&mut output, &change).unwrap();
        assert!(output
            .windows(b"-old\r\n".len())
            .any(|part| part == b"-old\r\n"));
        assert!(output
            .windows(b"-last\n\\ No newline at end of file\n".len())
            .any(|part| part == b"-last\n\\ No newline at end of file\n"));
        assert!(output
            .windows(b"+new\r\n".len())
            .any(|part| part == b"+new\r\n"));
    }

    #[cfg(unix)]
    #[test]
    fn new_config_and_ownership_files_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let home = TempHome::new("permissions");
        home.mkdir(".cursor");
        home.run(&["--agents", "cursor"]);
        for path in [
            home.0.join(".cursor/mcp.json"),
            home.0.join(".cursor").join(STATE_FILE),
        ] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
