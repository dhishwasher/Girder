//! Execution trace model: the recorded history the debugger time-travels over.

use crate::lang::Value;
use std::collections::BTreeMap;

/// Variable environment. `BTreeMap` keeps snapshots ordered & deterministic.
pub type Env = BTreeMap<String, Value>;

/// One recorded execution step (after executing one top-level statement).
#[derive(Debug, Clone)]
pub struct Step {
    /// Position of this step within its branch.
    pub seq: usize,
    /// The variable this statement bound.
    pub var: String,
    /// Human-readable rendering, e.g. `area = rect_area(3, 4)  => 7`.
    pub description: String,
    /// Full variable environment immediately AFTER this step.
    pub env: Env,
    /// True if this step's value was overridden by a what-if intervention.
    pub intervened: bool,
}

/// An ordered list of steps — one linear run of the program.
#[derive(Debug, Clone, Default)]
pub struct Trace {
    pub steps: Vec<Step>,
}

impl Trace {
    pub fn last_value(&self, var: &str) -> Option<Value> {
        self.steps
            .iter()
            .rev()
            .find_map(|s| s.env.get(var).copied())
    }

    pub fn final_env(&self) -> Env {
        self.steps.last().map(|s| s.env.clone()).unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}
