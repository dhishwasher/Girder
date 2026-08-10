//! Real project loading, CLI subcommands, and `.aether` persistence.

mod collaboration_discovery;
mod collaboration_identity;
mod collaboration_projection;
mod collaboration_transport;
mod commands;
mod config;
mod git;
mod planfile;
mod process;
mod projection;
mod source;
mod summary;
mod validation;
#[cfg(any(feature = "gui", test))]
mod workspace;

#[cfg(feature = "gui")]
pub(crate) use collaboration_discovery::{
    discover as discover_collaboration_peers, DiscoveredPeer,
};
#[cfg(feature = "gui")]
pub(crate) use collaboration_identity::{
    generate_identity as generate_collaboration_identity,
    inspect_public_identity as inspect_collaboration_public_identity,
    trust_identity as trust_collaboration_identity,
    trusted_identities as trusted_collaboration_identities, IdentitySummary, TrustChange,
};
#[cfg(feature = "gui")]
pub(crate) use collaboration_projection::{
    apply_reviewed_collaboration_projection, review_collaboration_projection,
    CollaborationProjectionReview,
};
#[cfg(feature = "gui")]
pub(crate) use collaboration_transport::{
    generate_secret as generate_collaboration_secret, join as join_collaboration, LiveSyncReport,
};
#[cfg(feature = "gui")]
pub(crate) use commands::extensions::{
    ExtensionMutation, ExtensionMutationOutcome, ExtensionMutationRequest,
};
pub(crate) use commands::{
    analyze, collaboration, config, dap, debug, extensions, forge, inspect, plan_explain, plan_run,
    plan_validate, query, refactor, review, search, swarm_plan, test_impact,
};
#[cfg(feature = "gui")]
pub(crate) use validation::{ValidationReport, ValidationStatus};
#[cfg(feature = "gui")]
pub(crate) use workspace::{
    AgentValidationOutcome, ExtensionCommandRequest, ProjectWorkspace, SyncImpact,
};
