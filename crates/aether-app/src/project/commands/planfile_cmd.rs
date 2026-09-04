//! `girder plan validate|explain|run` — thin CLI-arg layer over
//! `crate::project::planfile`. Per the plan format spec, these subcommands
//! take only a `<plan.json>` path (no project `<dir>` argument); the
//! project root is the current working directory.

use crate::project::planfile;
use std::path::{Path, PathBuf};

pub fn plan_validate(args: &[String]) -> std::io::Result<()> {
    let Some(plan_path) = args.first() else {
        eprintln!("usage: girder plan validate <plan.json>");
        return Ok(());
    };
    planfile::validate(&PathBuf::from("."), Path::new(plan_path))
}

pub fn plan_explain(args: &[String]) -> std::io::Result<()> {
    let Some(plan_path) = args.first() else {
        eprintln!("usage: girder plan explain <plan.json>");
        return Ok(());
    };
    planfile::explain(Path::new(plan_path))
}

pub fn plan_run(args: &[String]) -> std::io::Result<()> {
    let Some(plan_path) = args.first() else {
        eprintln!(
            "usage: girder plan run <plan.json|-> [--dry] [--out <path>] \
             [--authoring-receipt <receipt.json>] [--authored [--authored-by <name>]]"
        );
        return Ok(());
    };
    let dry = args.iter().any(|arg| arg == "--dry");
    let out_path = args
        .windows(2)
        .find(|window| window[0] == "--out")
        .map(|window| Path::new(window[1].as_str()));
    if args.iter().any(|arg| arg == "--out") && out_path.is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "--out requires a path",
        ));
    }
    let authoring_receipt = args
        .windows(2)
        .find(|window| window[0] == "--authoring-receipt")
        .map(|window| Path::new(window[1].as_str()));
    if args.iter().any(|arg| arg == "--authoring-receipt") && authoring_receipt.is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "--authoring-receipt requires a receipt path",
        ));
    }
    let authored = args.iter().any(|arg| arg == "--authored");
    let authored_by = args
        .windows(2)
        .find(|window| window[0] == "--authored-by")
        .map(|window| window[1].as_str());
    if args.iter().any(|arg| arg == "--authored-by") && authored_by.is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "--authored-by requires a name",
        ));
    }
    if authored_by.is_some() && !authored {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "--authored-by requires --authored",
        ));
    }
    planfile::run(
        &PathBuf::from("."),
        Path::new(plan_path),
        dry,
        authoring_receipt,
        authored,
        authored_by,
        out_path,
    )
}
