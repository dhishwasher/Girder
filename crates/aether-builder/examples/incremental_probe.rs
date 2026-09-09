//! Isolated cold/update worker for the frozen incremental measurement harness.
#[allow(dead_code)]
#[path = "../tests/support/incremental.rs"]
mod support;

use serde_json::{json, Value};
use std::time::Instant;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        3,
        "usage: incremental_probe cold|update INPUT.json"
    );
    let input: Value = serde_json::from_slice(&std::fs::read(&args[2]).unwrap()).unwrap();
    let mut files = support::sources(&input);
    if args[1] == "cold" {
        for step in input["mutations"].as_array().unwrap() {
            support::changes(step, &mut files);
        }
        let started = Instant::now();
        let (graph, _) = support::cold(&files);
        let seconds = started.elapsed().as_secs_f64();
        println!(
            "{}",
            json!({"graph":support::canonical(&graph),"elapsed_seconds":seconds,"parsed_files":files.len(),"reused_files":0})
        );
    } else {
        assert_eq!(args[1], "update");
        let started = Instant::now();
        let (mut graph, mut builder) = support::cold(&files);
        let cache_build_seconds = started.elapsed().as_secs_f64();
        let mut report = None;
        for step in input["mutations"].as_array().unwrap() {
            let changes = support::changes(step, &mut files);
            report = Some(
                builder
                    .update_files(&mut graph, &changes, &support::reasons(step))
                    .unwrap(),
            );
        }
        let report = report.expect("at least one mutation is required");
        println!(
            "{}",
            json!({"graph":support::canonical(&graph),"report":support::report_json(&report),"cache_build_seconds":cache_build_seconds,"elapsed_seconds":report.elapsed_seconds})
        );
    }
}
