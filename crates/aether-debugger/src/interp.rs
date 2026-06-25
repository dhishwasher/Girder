//! The recording interpreter.
//!
//! Executes a [`Program`]'s `main` statement-by-statement, snapshotting the
//! environment after each into a [`Trace`]. Supports a single *intervention*
//! (the "what-if": force a variable to a value at a chosen step and let the
//! consequences propagate) — the primitive the branching timeline is built on.

use crate::lang::{Expr, Op, Program, Value};
use crate::trace::{Env, Step, Trace};
use std::collections::BTreeMap;

/// How many times each function was *executed* during a run — the raw hot-path
/// signal the Optimizer mines (`function name -> invocation count`).
pub type CallCounts = BTreeMap<String, usize>;

/// An optional what-if override applied during a run.
#[derive(Debug, Clone)]
pub struct Intervention {
    /// After this step index executes, force `var` to `value`.
    pub at_step: usize,
    pub var: String,
    pub value: Value,
}

pub struct Interpreter<'a> {
    program: &'a Program,
}

impl<'a> Interpreter<'a> {
    pub fn new(program: &'a Program) -> Self {
        Interpreter { program }
    }

    /// Run `main` straight through, recording a [`Trace`].
    pub fn run(&self) -> Trace {
        self.run_with(None)
    }

    /// Run `main`, optionally applying a single [`Intervention`].
    pub fn run_with(&self, intervention: Option<&Intervention>) -> Trace {
        self.run_with_counts(intervention).0
    }

    /// Run `main` and also return the per-function execution counts (hot-path
    /// profile). Counts reflect what actually executed, so an intervention that
    /// changes a branch changes the profile too.
    pub fn run_with_counts(&self, intervention: Option<&Intervention>) -> (Trace, CallCounts) {
        let mut env: Env = Env::new();
        let mut trace = Trace::default();
        let mut counts: CallCounts = CallCounts::new();

        for (i, stmt) in self.program.main.iter().enumerate() {
            let mut value = self.eval(&stmt.expr, &env, &mut counts);
            let mut intervened = false;

            // Apply a what-if to this very binding if it targets this step+var.
            if let Some(iv) = intervention {
                if iv.at_step == i && iv.var == stmt.var {
                    value = iv.value;
                    intervened = true;
                }
            }

            env.insert(stmt.var.clone(), value);
            let description = format!(
                "{} = {}  => {}{}",
                stmt.var,
                render(&stmt.expr),
                value,
                if intervened { "  [what-if]" } else { "" }
            );
            trace.steps.push(Step {
                seq: i,
                var: stmt.var.clone(),
                description,
                env: env.clone(),
                intervened,
            });
        }
        (trace, counts)
    }

    /// Evaluate an expression in `env`, tallying each executed call into `counts`.
    /// Function calls recurse with a fresh scope; these inner steps are not
    /// recorded (top-level stepping only) but their calls *are* counted.
    fn eval(&self, expr: &Expr, env: &Env, counts: &mut CallCounts) -> Value {
        match expr {
            Expr::Num(n) => *n,
            Expr::Var(name) => env.get(name).copied().unwrap_or(0),
            Expr::Bin(op, a, b) => {
                let x = self.eval(a, env, counts);
                let y = self.eval(b, env, counts);
                apply_op(*op, x, y)
            }
            Expr::If(cond, then, els) => {
                if self.eval(cond, env, counts) != 0 {
                    self.eval(then, env, counts)
                } else {
                    self.eval(els, env, counts)
                }
            }
            Expr::Call(name, args) => {
                let Some(func) = self.program.funcs.get(name) else {
                    return 0;
                };
                *counts.entry(name.clone()).or_insert(0) += 1;
                let mut scope: Env = Env::new();
                for (param, arg) in func.params.iter().zip(args) {
                    let v = self.eval(arg, env, counts);
                    scope.insert(param.clone(), v);
                }
                for stmt in &func.body {
                    let v = self.eval(&stmt.expr, &scope, counts);
                    scope.insert(stmt.var.clone(), v);
                }
                self.eval(&func.ret, &scope, counts)
            }
        }
    }
}

fn apply_op(op: Op, x: Value, y: Value) -> Value {
    match op {
        Op::Add => x + y,
        Op::Sub => x - y,
        Op::Mul => x * y,
        // Integer division guards against divide-by-zero (a classic bug to debug).
        Op::Div => {
            if y == 0 {
                0
            } else {
                x / y
            }
        }
        Op::Lt => (x < y) as Value,
        Op::Gt => (x > y) as Value,
        Op::Eq => (x == y) as Value,
    }
}

/// Render an expression back to readable source for step descriptions.
pub fn render(expr: &Expr) -> String {
    match expr {
        Expr::Num(n) => n.to_string(),
        Expr::Var(name) => name.clone(),
        Expr::Bin(op, a, b) => format!("{} {} {}", render(a), op.symbol(), render(b)),
        Expr::If(c, t, e) => format!("if {} then {} else {}", render(c), render(t), render(e)),
        Expr::Call(name, args) => {
            let rendered: Vec<String> = args.iter().map(render).collect();
            format!("{}({})", name, rendered.join(", "))
        }
    }
}
