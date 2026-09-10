//! Explicitly invoked probe: routine CI does not run the timing campaign.
use super::*;
use sha2::{Digest, Sha256};

fn digest(graph: &SemanticGraph) -> String {
    format!("{:x}", Sha256::digest(graph.to_ron().unwrap().as_bytes()))
}
fn record(file: &mut std::fs::File, value: Value) {
    writeln!(file, "{value}").unwrap();
    file.sync_all().unwrap();
}
fn oracle(
    root: &Path,
    generation: &Generation,
    previous_cold: Option<&SemanticGraph>,
) -> (Value, SemanticGraph) {
    let started = Instant::now();
    let (source_graph, _, _) =
        source::build_from_dir_with_config(root, &generation.config).unwrap();
    let cold_seconds = started.elapsed().as_secs_f64();
    let persisted = source::load_graph_snapshot(root, &generation.config)
        .unwrap()
        .graph
        .unwrap();
    // The watcher output cannot become its own expected durable input. Carry
    // the oracle's preceding cold generation through the mutation sequence.
    let reconciled = match previous_cold {
        Some(previous) => SemanticGraph::reconcile_persisted(source_graph.clone(), previous).0,
        None => source_graph.clone(),
    };
    let result = json!({"cold_seconds":cold_seconds,"source_sha256":digest(&generation.graph),"cold_source_sha256":digest(&source_graph),
        "persisted_sha256":digest(&persisted),"cold_reconciled_sha256":digest(&reconciled),
        "equal_source":canonical(&generation.graph)==canonical(&source_graph),"equal_persisted":canonical(&persisted)==canonical(&reconciled)});
    (result, reconciled)
}
fn metrics_delta(before: &Metrics, after: &Metrics) -> Value {
    let reasons: BTreeMap<_, _> = after
        .reasons
        .iter()
        .map(|(r, n)| (r.clone(), n - before.reasons.get(r).copied().unwrap_or(0)))
        .filter(|(_, n)| *n > 0)
        .collect();
    json!({"attempts":after.attempts-before.attempts,"builds":after.builds-before.builds,"published":after.published-before.published,
        "discarded_candidates":after.discarded-before.discarded,"failed_attempts":after.failed_attempts-before.failed_attempts,
        "full_parsing":after.fallbacks-before.fallbacks,"fallback_reasons":reasons,"parsed_files":after.parsed-before.parsed,"reused_files":after.reused-before.reused})
}

#[test]
fn persistence_oracle_rejects_metadata_invented_by_the_watcher() {
    let root = Fixture::new();
    let server = Server::start(&root.0).unwrap();
    let generation = ready(&server);
    drop(server);
    let mut saved = source::load_graph_snapshot(&root.0, &generation.config)
        .unwrap()
        .graph
        .unwrap();
    let id = saved.nodes().next().unwrap().id;
    saved
        .get_mut(id)
        .unwrap()
        .set_attr("summary", "unexpected output metadata");
    source::save_graph(&root.0, &generation.config, &saved).unwrap();
    // Reusing the observed output as the expected durable input would accept
    // this invented metadata. The independently carried cold graph rejects it.
    let circular = SemanticGraph::reconcile_persisted(generation.graph.clone(), &saved).0;
    assert_eq!(canonical(&circular), canonical(&saved));
    let (result, _) = oracle(&root.0, &generation, None);
    assert_eq!(result["equal_source"], true);
    assert_eq!(result["equal_persisted"], false);
}

#[test]
#[ignore = "recorded watcher timing campaign; invoke the compiled test binary outside Cargo"]
fn watch_measurement() {
    let input = std::env::var_os("GIRDER_WATCH_MEASUREMENT_INPUT").expect("measurement input");
    let output = std::env::var_os("GIRDER_WATCH_MEASUREMENT_OUTPUT").expect("measurement output");
    let corpus: Value = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .expect("refuse existing observation");
    for case in corpus["cases"].as_array().unwrap() {
        let root = Fixture::new();
        for name in ["lib.rs", "math.rs", "app.rs"] {
            std::fs::remove_file(root.0.join(name)).unwrap();
        }
        for (path, source) in case["files"].as_object().unwrap() {
            let target = root.0.join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, source.as_str().unwrap()).unwrap();
        }
        record(
            &mut output,
            json!({"kind":"initial_started","case":case["id"]}),
        );
        let started = Instant::now();
        let server = Server::start(&root.0).unwrap();
        let mut generation = ready(&server);
        let initial_seconds = started.elapsed().as_secs_f64();
        let (initial_oracle, mut previous_cold) = oracle(&root.0, &generation, None);
        record(
            &mut output,
            json!({"kind":"initial","case":case["id"],"language":case["language"],"files":case["initial_file_count"],"seconds":initial_seconds,"oracle":initial_oracle}),
        );
        assert_eq!(initial_oracle["equal_source"], true);
        assert_eq!(initial_oracle["equal_persisted"], true);
        for mutation in case["mutations"].as_array().unwrap() {
            let id = format!(
                "{}/{}",
                case["id"].as_str().unwrap(),
                mutation["id"].as_str().unwrap()
            );
            record(&mut output, json!({"kind":"started","id":id}));
            let before = server.shared.state.lock().unwrap().metrics.clone();
            let started = Instant::now();
            for change in mutation["changes"].as_array().unwrap() {
                let target = root.0.join(change["path"].as_str().unwrap());
                if let Some(source) = change["source"].as_str() {
                    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                    std::fs::write(target, source).unwrap();
                } else {
                    std::fs::remove_file(target).unwrap();
                }
            }
            let deadline = Instant::now() + Duration::from_secs(60);
            let mut state = server.shared.state.lock().unwrap();
            loop {
                if !state.stale {
                    if let Some(next) = &state.generation {
                        let matches = mutation["changes"].as_array().unwrap().iter().all(|c| {
                            let path = c["path"].as_str().unwrap();
                            if let Some(source) = c["source"].as_str() {
                                next.snapshot.matches_bytes(path, Some(source.as_bytes()))
                            } else {
                                !next.snapshot.files.contains_key(path)
                            }
                        });
                        if next.number > generation.number && matches {
                            generation = next.clone();
                            break;
                        }
                    }
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(!remaining.is_zero(), "mutation timed out: {id}");
                state = server
                    .shared
                    .changed
                    .wait_timeout(state, remaining)
                    .unwrap()
                    .0;
            }
            let latency = started.elapsed().as_secs_f64();
            let metrics = metrics_delta(&before, &state.metrics);
            drop(state);
            let (result, cold) = oracle(&root.0, &generation, Some(&previous_cold));
            record(
                &mut output,
                json!({"kind":"completed","id":id,"case":case["id"],"language":case["language"],"files":case["initial_file_count"],
                "mutation_to_publication_seconds":latency,"metrics":metrics,"oracle":result}),
            );
            assert_eq!(result["equal_source"], true, "{id}");
            assert_eq!(result["equal_persisted"], true, "{id}");
            previous_cold = cold;
        }
    }
}
