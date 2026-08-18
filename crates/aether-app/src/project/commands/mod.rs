mod agents;
mod author;
mod authoring_context;
mod collaboration;
mod config;
mod context_cmd;
mod dap;
mod debug;
pub(crate) mod extensions;
mod graph;
mod planfile_cmd;
mod query;
mod review;
mod test_impact;

pub(crate) use agents::{forge, swarm_plan};
pub(crate) use author::do_intent;
#[cfg(feature = "gui")]
pub(crate) use author::{author, AuthorEvent, AuthorOutcome, DEFAULT_MAX_REPAIRS};
#[cfg(feature = "gui")]
pub(crate) use authoring_context::search_nodes_for_authoring;
pub(crate) use collaboration::collaboration;
pub(crate) use config::config;
#[cfg(feature = "gui")]
pub(crate) use context_cmd::build_context_json;
pub(crate) use context_cmd::context;
pub(crate) use dap::dap;
pub(crate) use debug::debug;
pub(crate) use extensions::extensions;
pub(crate) use graph::{analyze, inspect, refactor, search};
pub(crate) use planfile_cmd::{plan_explain, plan_run, plan_validate};
pub(crate) use query::query;
pub(crate) use review::review;
pub(crate) use test_impact::test_impact;
