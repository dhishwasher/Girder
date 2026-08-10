//! `bitcode plan validate|explain|run` — thin CLI-arg layer over
//! `crate::project::planfile`. Per the plan format spec, these subcommands
//! take only a `<plan.json>` path (no project `<dir>` argument); the
//! project root is the current working directory.

use crate::project::planfile;
use std::path::{Path, PathBuf};

pub fn plan_validate(args: &[String]) -> std::io::Result<()> {
    let Some(plan_path) = args.first() else {
        eprintln!("usage: bitcode plan validate <plan.json>");
        return Ok(());
    };
    planfile::validate(&PathBuf::from("."), Path::new(plan_path))
}

pub fn plan_explain(args: &[String]) -> std::io::Result<()> {
    let Some(plan_path) = args.first() else {
        eprintln!("usage: bitcode plan explain <plan.json>");
        return Ok(());
    };
    planfile::explain(Path::new(plan_path))
}

pub fn plan_run(args: &[String]) -> std::io::Result<()> {
    let Some(plan_path) = args.first() else {
        eprintln!("usage: bitcode plan run <plan.json> [--dry]");
        return Ok(());
    };
    let dry = args.iter().any(|arg| arg == "--dry");
    planfile::run(&PathBuf::from("."), Path::new(plan_path), dry)
}
