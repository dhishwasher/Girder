//! A tiny toy language the time-travel debugger executes.
//!
//! It's an embedded AST (no text parser — that's an EXTENSION POINT) with just
//! enough to demonstrate stepwise tracing, branching, and what-if interventions:
//! integer variables, arithmetic/comparison, `if`, and (recursive) function
//! calls.

use std::collections::HashMap;

/// Runtime value. Integers only; booleans are encoded as 0/1.
pub type Value = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Lt,
    Gt,
    Eq,
}

impl Op {
    pub fn symbol(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Lt => "<",
            Op::Gt => ">",
            Op::Eq => "==",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    Num(Value),
    Var(String),
    Bin(Op, Box<Expr>, Box<Expr>),
    /// `if cond != 0 then a else b`.
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

/// A top-level / function-body statement: bind (or rebind) a variable.
#[derive(Debug, Clone)]
pub struct Stmt {
    pub var: String,
    pub expr: Expr,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    /// The function's return expression, evaluated after `body`.
    pub ret: Expr,
}

/// A whole program: named functions plus a `main` statement sequence.
#[derive(Debug, Clone, Default)]
pub struct Program {
    pub funcs: HashMap<String, Function>,
    pub main: Vec<Stmt>,
}

impl Program {
    pub fn new() -> Self {
        Program::default()
    }

    pub fn function(mut self, f: Function) -> Self {
        self.funcs.insert(f.name.clone(), f);
        self
    }

    pub fn stmt(mut self, var: &str, expr: Expr) -> Self {
        self.main.push(Stmt {
            var: var.to_string(),
            expr,
        });
        self
    }
}

// ---- ergonomic AST builders (used by the demo & tests) ----

pub fn num(n: Value) -> Expr {
    Expr::Num(n)
}
pub fn var(name: &str) -> Expr {
    Expr::Var(name.to_string())
}
pub fn bin(op: Op, a: Expr, b: Expr) -> Expr {
    Expr::Bin(op, Box::new(a), Box::new(b))
}
pub fn call(name: &str, args: Vec<Expr>) -> Expr {
    Expr::Call(name.to_string(), args)
}
pub fn if_(cond: Expr, then: Expr, els: Expr) -> Expr {
    Expr::If(Box::new(cond), Box::new(then), Box::new(els))
}
