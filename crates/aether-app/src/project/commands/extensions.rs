use crate::project::config::{ProjectConfig, CONFIG_FILE};
use crate::project::source::{
    build_from_dir_with_config, commit_project_writes, graph_project_write, load_graph_snapshot,
    read_project_bytes, read_project_bytes_bounded, ProjectWrite,
};
use crate::project::validation::{validate_candidate, ValidationStatus};
use aether_ai::{Prompt, TaskClass};
use aether_extensions::{
    content_digest, find_record, generation_system_prompt, install_with_receipts, records,
    set_enabled, uninstall, Capability, ExtensionGrant, ExtensionRecipe, ExtensionRecord,
    ProjectionMode, ProjectionReceipt, MAX_PROJECTION_BYTES,
};
use aether_graph::SemanticGraph;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[cfg(feature = "gui")]
#[derive(Debug, Clone)]
pub(crate) enum ExtensionMutation {
    Install(ExtensionRecipe),
    SetEnabled { extension_id: String, enabled: bool },
    Remove { extension_id: String },
}

#[cfg(feature = "gui")]
pub(crate) struct ExtensionMutationRequest {
    root: PathBuf,
    config: ProjectConfig,
    config_baseline: Option<Vec<u8>>,
    graph: SemanticGraph,
    graph_baseline: Option<Vec<u8>>,
    mutation: ExtensionMutation,
}

#[cfg(feature = "gui")]
pub(crate) struct ExtensionMutationOutcome {
    pub(crate) message: String,
}

#[cfg(feature = "gui")]
impl ExtensionMutationRequest {
    pub(crate) fn new(
        root: PathBuf,
        config: ProjectConfig,
        config_baseline: Option<Vec<u8>>,
        graph: SemanticGraph,
        graph_baseline: Option<Vec<u8>>,
        mutation: ExtensionMutation,
    ) -> Self {
        Self {
            root,
            config,
            config_baseline,
            graph,
            graph_baseline,
            mutation,
        }
    }

    pub(crate) fn run(
        mut self,
        cancel: &Arc<AtomicBool>,
    ) -> std::io::Result<ExtensionMutationOutcome> {
        match self.mutation {
            ExtensionMutation::Install(recipe) => {
                if find_record(&self.graph, &recipe.id)
                    .map_err(extension_error)?
                    .is_some()
                {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!("extension {} is already installed", recipe.id),
                    ));
                }
                let (mut writes, receipts) = projection_install_writes(&self.root, &recipe)?;
                let grant = ExtensionGrant::approve(&recipe).map_err(extension_error)?;
                install_with_receipts(&mut self.graph, recipe.clone(), grant, receipts)
                    .map_err(extension_error)?;
                writes.push(graph_project_write(
                    &self.root,
                    &self.config,
                    &self.graph,
                    self.graph_baseline,
                )?);
                validate_and_commit(
                    &self.root,
                    &self.config,
                    self.config_baseline,
                    writes,
                    cancel,
                )?;
                Ok(ExtensionMutationOutcome {
                    message: format!("Installed and enabled {}", recipe.id),
                })
            }
            ExtensionMutation::SetEnabled {
                extension_id,
                enabled,
            } => {
                set_enabled(&mut self.graph, &extension_id, enabled).map_err(extension_error)?;
                commit_graph_only(
                    &self.root,
                    &self.config,
                    self.config_baseline,
                    self.graph_baseline,
                    &self.graph,
                )?;
                Ok(ExtensionMutationOutcome {
                    message: format!(
                        "{} {extension_id}",
                        if enabled { "Enabled" } else { "Disabled" }
                    ),
                })
            }
            ExtensionMutation::Remove { extension_id } => {
                let record = find_record(&self.graph, &extension_id)
                    .map_err(extension_error)?
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("extension {extension_id} is not installed"),
                        )
                    })?;
                let mut writes = projection_remove_writes(&record);
                uninstall(&mut self.graph, &extension_id).map_err(extension_error)?;
                writes.push(graph_project_write(
                    &self.root,
                    &self.config,
                    &self.graph,
                    self.graph_baseline,
                )?);
                validate_and_commit(
                    &self.root,
                    &self.config,
                    self.config_baseline,
                    writes,
                    cancel,
                )?;
                Ok(ExtensionMutationOutcome {
                    message: format!("Removed {extension_id} and restored its project projections"),
                })
            }
        }
    }
}

pub(crate) async fn extensions(args: &[String]) -> std::io::Result<()> {
    let Some(root) = args.first().map(PathBuf::from) else {
        print_usage();
        return Ok(());
    };
    let Some(operation) = args.get(1).map(String::as_str) else {
        print_usage();
        return Ok(());
    };
    let config = ProjectConfig::load(&root)?;
    let config_baseline = read_project_bytes(&root, CONFIG_FILE)?;
    let (mut graph, graph_baseline) = load_reconciled_graph(&root, &config)?;

    match operation {
        "list" => list_extensions(&graph),
        "generate" => {
            let approve = args.iter().any(|argument| argument == "--approve");
            let intent = args
                .iter()
                .skip(2)
                .filter(|argument| argument.as_str() != "--approve")
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            if intent.trim().is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "usage: bitcode extension <dir> generate <intent...> [--approve]",
                ));
            }
            let input = serde_json::json!({
                "intent": intent,
                "graph_context": {
                    "nodes": graph.node_count(),
                    "edges": graph.edge_count(),
                }
            });
            let mut prompt = Prompt::new(
                TaskClass::Extension,
                generation_system_prompt(),
                input.to_string(),
            );
            prompt.max_tokens = 4_096;
            let completion =
                aether_ai::default_router()
                    .complete(prompt)
                    .await
                    .map_err(|error| {
                        std::io::Error::other(format!("extension generation failed: {error}"))
                    })?;
            let recipe = ExtensionRecipe::from_json(&completion.text).map_err(extension_error)?;
            println!("Generated by {}", completion.model);
            preview_or_install(
                &root,
                &config,
                config_baseline,
                &mut graph,
                graph_baseline,
                recipe,
                approve,
            )
        }
        "install" => {
            let Some(recipe_path) = args.get(2) else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "usage: bitcode extension <dir> install <recipe.json> [--approve]",
                ));
            };
            let approve = args.iter().any(|argument| argument == "--approve");
            let json = std::fs::read_to_string(recipe_path).map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("could not read extension recipe {recipe_path}: {error}"),
                )
            })?;
            let recipe = ExtensionRecipe::from_json(&json).map_err(extension_error)?;
            preview_or_install(
                &root,
                &config,
                config_baseline,
                &mut graph,
                graph_baseline,
                recipe,
                approve,
            )
        }
        "enable" | "disable" => {
            let extension_id = required_extension_id(args, operation)?;
            set_enabled(&mut graph, extension_id, operation == "enable")
                .map_err(extension_error)?;
            commit_graph_only(&root, &config, config_baseline, graph_baseline, &graph)?;
            println!(
                "{} {}",
                if operation == "enable" {
                    "Enabled"
                } else {
                    "Disabled"
                },
                extension_id
            );
            Ok(())
        }
        "remove" => {
            let extension_id = required_extension_id(args, operation)?;
            remove_extension(
                &root,
                &config,
                config_baseline,
                &mut graph,
                graph_baseline,
                extension_id,
            )
        }
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("unknown extension operation {operation:?}"),
        )),
    }
}

fn preview_or_install(
    root: &Path,
    config: &ProjectConfig,
    config_baseline: Option<Vec<u8>>,
    graph: &mut SemanticGraph,
    graph_baseline: Option<Vec<u8>>,
    recipe: ExtensionRecipe,
    approve: bool,
) -> std::io::Result<()> {
    print_recipe(&recipe)?;
    if !approve {
        println!("\nPreview only; no project files or graph state were changed.");
        println!("Re-run with --approve to grant these exact capabilities.");
        return Ok(());
    }
    if find_record(graph, &recipe.id)
        .map_err(extension_error)?
        .is_some()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "extension {} is already installed; remove it before replacing the recipe",
                recipe.id
            ),
        ));
    }

    let (mut writes, receipts) = projection_install_writes(root, &recipe)?;
    let grant = ExtensionGrant::approve(&recipe).map_err(extension_error)?;
    install_with_receipts(graph, recipe.clone(), grant, receipts).map_err(extension_error)?;
    writes.push(graph_project_write(root, config, graph, graph_baseline)?);
    validate_and_commit(
        root,
        config,
        config_baseline,
        writes,
        &Arc::new(AtomicBool::new(false)),
    )?;
    println!("\nInstalled and enabled {}", recipe.id);
    Ok(())
}

fn remove_extension(
    root: &Path,
    config: &ProjectConfig,
    config_baseline: Option<Vec<u8>>,
    graph: &mut SemanticGraph,
    graph_baseline: Option<Vec<u8>>,
    extension_id: &str,
) -> std::io::Result<()> {
    let record = find_record(graph, extension_id)
        .map_err(extension_error)?
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("extension {extension_id} is not installed"),
            )
        })?;
    let mut writes = projection_remove_writes(&record);
    if !uninstall(graph, extension_id).map_err(extension_error)? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("extension {extension_id} is not installed"),
        ));
    }
    writes.push(graph_project_write(root, config, graph, graph_baseline)?);
    validate_and_commit(
        root,
        config,
        config_baseline,
        writes,
        &Arc::new(AtomicBool::new(false)),
    )?;
    println!("Removed {extension_id} and restored its project projections");
    Ok(())
}

fn load_reconciled_graph(
    root: &Path,
    config: &ProjectConfig,
) -> std::io::Result<(SemanticGraph, Option<Vec<u8>>)> {
    let (source, _, _) = build_from_dir_with_config(root, config)?;
    let persisted = load_graph_snapshot(root, config)?;
    if let Some(error) = persisted.error {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("persisted graph is invalid; refusing extension changes: {error}"),
        ));
    }
    let graph = match persisted.graph {
        Some(durable) => SemanticGraph::reconcile_persisted(source, &durable).0,
        None => source,
    };
    Ok((graph, persisted.bytes))
}

fn projection_install_writes(
    root: &Path,
    recipe: &ExtensionRecipe,
) -> std::io::Result<(Vec<ProjectWrite>, Vec<ProjectionReceipt>)> {
    let mut writes = Vec::with_capacity(recipe.projections.len());
    let mut receipts = Vec::with_capacity(recipe.projections.len());
    for projection in &recipe.projections {
        let current = read_project_bytes_bounded(root, &projection.path, MAX_PROJECTION_BYTES)?;
        match (projection.mode, current.as_ref()) {
            (ProjectionMode::Create, Some(_)) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!(
                        "extension projection {} uses create mode but already exists",
                        projection.path
                    ),
                ));
            }
            (ProjectionMode::Replace, None) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!(
                        "extension projection {} uses replace mode but does not exist",
                        projection.path
                    ),
                ));
            }
            _ => {}
        }
        let previous = current
            .as_ref()
            .map(|bytes| {
                String::from_utf8(bytes.clone()).map_err(|error| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "extension projection baseline {} is not UTF-8: {error}",
                            projection.path
                        ),
                    )
                })
            })
            .transpose()?;
        receipts.push(ProjectionReceipt {
            path: projection.path.clone(),
            previous,
            applied_digest: content_digest(projection.contents.as_bytes()),
        });
        writes.push(ProjectWrite::text(
            &projection.path,
            current,
            projection.contents.clone(),
        ));
    }
    Ok((writes, receipts))
}

fn projection_remove_writes(record: &ExtensionRecord) -> Vec<ProjectWrite> {
    record
        .recipe
        .projections
        .iter()
        .zip(&record.projection_receipts)
        .map(|(projection, receipt)| {
            let applied = projection.contents.as_bytes().to_vec();
            match &receipt.previous {
                Some(previous) => {
                    ProjectWrite::text(&projection.path, Some(applied), previous.clone())
                }
                None => ProjectWrite::delete(&projection.path, applied),
            }
        })
        .collect()
}

fn commit_graph_only(
    root: &Path,
    config: &ProjectConfig,
    config_baseline: Option<Vec<u8>>,
    graph_baseline: Option<Vec<u8>>,
    graph: &SemanticGraph,
) -> std::io::Result<()> {
    let graph_write = graph_project_write(root, config, graph, graph_baseline)?;
    ensure_config_unchanged(root, config_baseline)?;
    commit_project_writes(root, vec![graph_write])?;
    Ok(())
}

fn validate_and_commit(
    root: &Path,
    config: &ProjectConfig,
    config_baseline: Option<Vec<u8>>,
    writes: Vec<ProjectWrite>,
    cancel: &Arc<AtomicBool>,
) -> std::io::Result<()> {
    let report = validate_candidate(root, config, &writes, cancel)?;
    for step in &report.steps {
        let command = step
            .command
            .as_deref()
            .map(|command| format!(": {command}"))
            .unwrap_or_default();
        println!(
            "  [{}] {}{} ({:.2}s)",
            step.status.label(),
            step.label,
            command,
            step.duration.as_secs_f32()
        );
        if step.status != ValidationStatus::Passed && !step.output.trim().is_empty() {
            for line in step.output.lines() {
                println!("    {line}");
            }
        }
    }
    if !report.passed() {
        return Err(std::io::Error::other(format!(
            "{}; project was not modified",
            report.summary()
        )));
    }
    ensure_config_unchanged(root, config_baseline)?;
    commit_project_writes(root, writes)?;
    Ok(())
}

fn ensure_config_unchanged(root: &Path, baseline: Option<Vec<u8>>) -> std::io::Result<()> {
    if read_project_bytes(root, CONFIG_FILE)? != baseline {
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{CONFIG_FILE} changed while the extension operation was running"),
        ))
    } else {
        Ok(())
    }
}

fn list_extensions(graph: &SemanticGraph) -> std::io::Result<()> {
    let records = records(graph);
    if records.is_empty() {
        println!("No extensions installed.");
        return Ok(());
    }
    for record in records {
        let record = record.map_err(extension_error)?;
        println!(
            "{}\t{:?}\t{}\t{} capability(s)",
            record.recipe.id,
            record.state,
            record.recipe.name,
            record.grant.capabilities.len()
        );
    }
    Ok(())
}

fn print_recipe(recipe: &ExtensionRecipe) -> std::io::Result<()> {
    println!("\nExtension: {} ({})", recipe.name, recipe.id);
    println!("  {}", recipe.description);
    println!("  digest: {}", recipe.digest().map_err(extension_error)?);
    println!("  requested capabilities:");
    if recipe.capabilities.is_empty() {
        println!("    (none)");
    }
    for capability in &recipe.capabilities {
        println!("    - {}", capability_label(capability));
    }
    println!("  contributions: {}", recipe.contributions.len());
    println!("  project projections: {}", recipe.projections.len());
    println!("\n{}", recipe.to_json_pretty().map_err(extension_error)?);
    Ok(())
}

fn capability_label(capability: &Capability) -> String {
    match capability {
        Capability::ReadGraph => "read graph".into(),
        Capability::WriteGraph { namespaces } => {
            format!("write graph: {}", namespaces.join(", "))
        }
        Capability::ReadProject { paths } => format!("read project: {}", paths.join(", ")),
        Capability::WriteProject { paths } => format!("write project: {}", paths.join(", ")),
        Capability::RunValidation { programs } => {
            format!("run validation: {}", programs.join(", "))
        }
        Capability::Network { hosts } => format!("network: {}", hosts.join(", ")),
        Capability::ContributeUi => "contribute UI".into(),
    }
}

fn required_extension_id<'a>(args: &'a [String], operation: &str) -> std::io::Result<&'a str> {
    args.get(2).map(String::as_str).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("usage: bitcode extension <dir> {operation} <extension-id>"),
        )
    })
}

fn extension_error(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
}

fn print_usage() {
    eprintln!(
        "usage: bitcode extension <dir> \
         list|generate|install|enable|disable|remove ..."
    );
}
