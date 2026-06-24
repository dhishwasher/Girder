//! The branching timeline: first-class time-travel over execution traces.
//!
//! Branch 0 is the original run. From any step of any branch you can spawn a new
//! branch with a what-if intervention; the consequences re-propagate forward.
//! Branches form a tree, enabling side-by-side "what-if" exploration and
//! divergence analysis — the backbone of AI-guided root-cause debugging.

use crate::interp::{Interpreter, Intervention};
use crate::lang::{Program, Value};
use crate::trace::Trace;
use aether_ai::{Prompt, Router, TaskClass};

/// A single timeline branch.
#[derive(Debug, Clone)]
pub struct Branch {
    pub id: usize,
    pub label: String,
    /// Where this branch was forked from: `(parent_branch_id, step_index)`.
    pub origin: Option<(usize, usize)>,
    pub trace: Trace,
}

/// A tree of execution branches over one program.
pub struct Timeline {
    program: Program,
    branches: Vec<Branch>,
}

impl Timeline {
    /// Record the baseline run as branch 0.
    pub fn record(program: Program) -> Self {
        let trace = Interpreter::new(&program).run();
        let root = Branch {
            id: 0,
            label: "main".to_string(),
            origin: None,
            trace,
        };
        Timeline {
            program,
            branches: vec![root],
        }
    }

    pub fn branches(&self) -> &[Branch] {
        &self.branches
    }

    pub fn branch(&self, id: usize) -> Option<&Branch> {
        self.branches.get(id)
    }

    /// Fork a new branch: re-run with a what-if forcing `var = value` at
    /// `at_step`. Returns the new branch id.
    pub fn fork_what_if(
        &mut self,
        from_branch: usize,
        at_step: usize,
        var: &str,
        value: Value,
        label: &str,
    ) -> usize {
        let intervention = Intervention {
            at_step,
            var: var.to_string(),
            value,
        };
        let trace = Interpreter::new(&self.program).run_with(Some(&intervention));
        let id = self.branches.len();
        self.branches.push(Branch {
            id,
            label: label.to_string(),
            origin: Some((from_branch, at_step)),
            trace,
        });
        id
    }

    /// First step index at which two branches' environments differ. This is the
    /// raw signal for "where did things start to go wrong".
    pub fn first_divergence(&self, a: usize, b: usize) -> Option<usize> {
        let (ta, tb) = (&self.branches.get(a)?.trace, &self.branches.get(b)?.trace);
        let n = ta.steps.len().min(tb.steps.len());
        (0..n).find(|&i| ta.steps[i].env != tb.steps[i].env)
    }

    /// AI-guided root-cause analysis. Builds a prompt from the (allegedly buggy)
    /// branch's trace and asks the router to explain it. Uses whatever provider
    /// the router resolves — the offline mock by default.
    pub async fn ai_root_cause(
        &self,
        router: &Router,
        branch: usize,
        symptom: &str,
    ) -> Option<String> {
        let trace = &self.branches.get(branch)?.trace;
        let rendered: Vec<String> = trace.steps.iter().map(|s| s.description.clone()).collect();
        let prompt = Prompt::new(
            TaskClass::Planning,
            "You are the Debugger. Given an execution trace and a symptom, \
             identify the earliest suspicious step and explain the likely root cause.",
            format!("Symptom: {symptom}\nTrace:\n{}", rendered.join("\n")),
        );
        router.complete(prompt).await.ok().map(|c| c.text)
    }
}
