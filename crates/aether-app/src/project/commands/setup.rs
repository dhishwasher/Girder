use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

const STATE_FILE: &str = "girder-setup-state.json";

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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct SetupState {
    version: u32,
    mcps: Vec<McpState>,
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
                is_state_path(&change.path)
            } else {
                !is_state_path(&change.path)
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

    let format = detection.format.expect("detected format");
    let before = read_optional(&config_path)?;
    let desired = mcp_entry();
    let mut document = parse_config(before.as_deref(), format, &config_path)?;
    let current = get_mcp_entry(&document, format)?;
    if current.is_some() && !force {
        notes.push(format!(
            "{}: existing girder entry left unchanged (use --force to replace it).",
            detection.agent.label()
        ));
        return Ok(());
    }
    if current.as_ref() == Some(&desired) {
        notes.push(format!(
            "{}: girder entry already configured.",
            detection.agent.label()
        ));
        return Ok(());
    }

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
    set_mcp_entry(&mut document, format, Some(desired))?;
    let after = serialize_config(&document, format)?;
    push_change(changes, config_path, before, Some(after));

    let state_before = read_optional(&state_path)?;
    let state_after = serde_json::to_vec_pretty(&state)
        .map(append_newline)
        .map_err(json_error)?;
    push_change(changes, state_path, state_before, Some(state_after));
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
    if state.mcps.is_empty() {
        notes.push(format!(
            "{}: no setup-owned MCP entry to remove.",
            detection.agent.label()
        ));
        return Ok(());
    }

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
            set_mcp_entry(&mut document, owned.format, owned.previous.clone())?;
            if owned.config_created && config_is_empty(&document, owned.format)? {
                push_change(changes, config_path, before, None);
            } else {
                let after = serialize_config(&document, owned.format)?;
                push_change(changes, config_path, before, Some(after));
            }
        }
    }
    state.mcps = retained;

    if state.mcps.is_empty() {
        push_change(changes, state_path, state_before, None);
    } else {
        let after = serde_json::to_vec_pretty(&state)
            .map(append_newline)
            .map_err(json_error)?;
        push_change(changes, state_path, state_before, Some(after));
    }
    Ok(())
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
        assert!(!home.0.join(".cursor").join(STATE_FILE).exists());

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
