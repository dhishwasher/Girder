//! Real project loading, CLI subcommands, and `.aether` persistence.

mod commands;
mod config;
mod git;
mod projection;
mod source;
mod summary;
mod validation;
#[cfg(any(feature = "gui", test))]
mod workspace;

#[cfg(feature = "gui")]
pub(crate) use commands::extensions::{
    ExtensionMutation, ExtensionMutationOutcome, ExtensionMutationRequest,
};
pub(crate) use commands::{
    analyze, config, dap, debug, extensions, forge, inspect, plan, query, refactor, review, search,
    test_impact,
};
#[cfg(feature = "gui")]
pub(crate) use validation::{ValidationReport, ValidationStatus};
#[cfg(feature = "gui")]
pub(crate) use workspace::{
    AgentValidationOutcome, ExtensionCommandRequest, ProjectWorkspace, SyncImpact,
};
