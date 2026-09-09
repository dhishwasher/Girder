#[allow(dead_code)]
#[path = "support/incremental.rs"]
mod support;

use aether_builder::FileChange;
use serde_json::Value;
use support::{assert_edges, canonical, changes, cold, corpus, reasons, sources};

fn run_fixture(fixture: &Value) {
    let mut files = sources(fixture);
    let (mut graph, mut builder) = cold(&files);
    assert_edges(&graph, &fixture["initial_edges"]);
    for step in fixture["mutations"].as_array().unwrap() {
        let changes = changes(step, &mut files);
        let report = builder
            .update_files(&mut graph, &changes, &reasons(step))
            .unwrap();
        let (expected, _) = cold(&files);
        assert_eq!(
            canonical(&graph),
            canonical(&expected),
            "{} / {}",
            fixture["id"],
            step["id"]
        );
        assert_edges(&graph, &step["edges"]);
        for (key, observed) in [
            ("expected_parsed", report.parsed_files.len()),
            ("expected_reused", report.reused_files.len()),
        ] {
            if let Some(expected) = step[key].as_u64() {
                assert_eq!(
                    observed as u64, expected,
                    "{} / {} / {key}",
                    fixture["id"], step["id"]
                );
            }
        }
        if let Some(expected) = step["expected_fallback"].as_bool() {
            assert_eq!(
                !report.full_rebuild_reasons.is_empty(),
                expected,
                "{} / {}",
                fixture["id"],
                step["id"]
            );
        }
        assert_eq!(
            report.parsed_files.len() + report.reused_files.len(),
            files.len()
        );
    }
}

#[test]
fn frozen_incremental_corpus_matches_complete_cold_records_after_every_mutation() {
    for fixture in corpus()["fixtures"].as_array().unwrap() {
        run_fixture(fixture);
    }
}

#[test]
fn frozen_normalized_batches_and_rejections_are_atomic() {
    let corpus = corpus();
    let fixture = corpus["project_cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "normalized-batches")
        .unwrap();
    let mut files = sources(fixture);
    let (mut graph, mut builder) = cold(&files);
    let update = changes(&fixture["mutations"][0], &mut files);
    let report = builder.update_files(&mut graph, &update, &[]).unwrap();
    assert_eq!(report.dirty_files, ["src/a.rs"]);
    assert_eq!(report.parsed_files.len(), 1);
    assert_eq!(canonical(&graph), canonical(&cold(&files).0));
    for path in fixture["rejected_paths"].as_array().unwrap() {
        let previous = canonical(&graph);
        let changes = [
            FileChange::replace("src/a.rs", "pub fn forbidden_partial_update() {}"),
            FileChange::delete(path.as_str().unwrap()),
        ];
        assert!(builder.update_files(&mut graph, &changes, &[]).is_err());
        assert_eq!(canonical(&graph), previous);
        assert_eq!(
            builder.source_of("src/a.rs"),
            files.get("src/a.rs").map(String::as_str)
        );
    }
}
