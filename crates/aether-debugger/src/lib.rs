//! # aether-debugger
//!
//! A first-class **time-travel & branching debugger**. It records a full
//! execution [`Trace`] of a toy program, lets you fork the [`Timeline`] at any
//! step with a what-if intervention, and surfaces divergences + AI-guided
//! root-cause analysis. The same model scales to real languages by swapping the
//! interpreter for instrumented execution.

pub mod interp;
pub mod lang;
pub mod timeline;
pub mod trace;

pub use interp::{Intervention, Interpreter};
pub use lang::{bin, call, if_, num, var, Function, Op, Program, Stmt, Value};
pub use timeline::{Branch, Timeline};
pub use trace::{Env, Step, Trace};

/// The demo program used by the app's debugger panel and the headless smoke
/// test. It contains a **deliberate bug**: `rect_area` adds its sides instead of
/// multiplying them, so `area` and everything downstream is wrong.
pub fn buggy_demo_program() -> Program {
    Program::new()
        .function(Function {
            name: "rect_area".to_string(),
            params: vec!["w".to_string(), "h".to_string()],
            body: vec![],
            // BUG: should be `w * h`.
            ret: bin(Op::Add, var("w"), var("h")),
        })
        .stmt("w", num(3))
        .stmt("h", num(4))
        .stmt("area", call("rect_area", vec![var("w"), var("h")]))
        .stmt("scaled", bin(Op::Mul, var("area"), num(2)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_a_full_trace() {
        let tl = Timeline::record(buggy_demo_program());
        let main = tl.branch(0).unwrap();
        assert_eq!(main.trace.len(), 4);
        // The bug manifests: area = 3 + 4 = 7 (should be 12), scaled = 14.
        assert_eq!(main.trace.last_value("area"), Some(7));
        assert_eq!(main.trace.last_value("scaled"), Some(14));
    }

    #[test]
    fn what_if_branch_propagates_the_fix_forward() {
        let mut tl = Timeline::record(buggy_demo_program());
        // "What if area were the correct 12 at step 2?" Downstream recomputes.
        let b = tl.fork_what_if(0, 2, "area", 12, "fix: area = w * h");
        let fixed = tl.branch(b).unwrap();
        assert_eq!(fixed.trace.last_value("area"), Some(12));
        assert_eq!(
            fixed.trace.last_value("scaled"),
            Some(24),
            "scaled must re-derive from the intervened area"
        );
        // The original branch is untouched — branches are independent histories.
        assert_eq!(tl.branch(0).unwrap().trace.last_value("scaled"), Some(14));
    }

    #[test]
    fn divergence_points_at_the_intervened_step() {
        let mut tl = Timeline::record(buggy_demo_program());
        let b = tl.fork_what_if(0, 2, "area", 12, "fix");
        // First difference is exactly the step where we intervened (step 2).
        assert_eq!(tl.first_divergence(0, b), Some(2));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ai_root_cause_returns_an_explanation() {
        let tl = Timeline::record(buggy_demo_program());
        let router = aether_ai::default_router();
        let explanation = tl
            .ai_root_cause(&router, 0, "area is 7 but expected 12")
            .await
            .expect("root cause should produce text");
        assert!(!explanation.is_empty());
    }
}
