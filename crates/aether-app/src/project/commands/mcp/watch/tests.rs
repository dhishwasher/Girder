use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "girder-watch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        for (name, source) in [
            ("lib.rs", "mod math; pub use math::add;"),
            ("math.rs", "pub fn add(a:i32,b:i32)->i32 { a+b }"),
            ("app.rs", "pub fn run()->i32 { crate::add(1,2) }"),
        ] {
            std::fs::write(path.join(name), source).unwrap();
        }
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn canonical(graph: &SemanticGraph) -> Value {
    let mut nodes: Vec<_> = graph
        .nodes()
        .map(|node| {
            let mut node = node.clone();
            // Persistence canonicalizes attribute allocation order too. Keep
            // every key/value pair while comparing the complete node record.
            node.attributes.sort_by(|left, right| left.0.cmp(&right.0));
            serde_json::to_value(node).unwrap()
        })
        .collect();
    nodes.sort_by_key(|n| n.to_string());
    let mut edges: Vec<_> = graph
        .edge_records()
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect();
    edges.sort_by_key(|e| e.to_string());
    json!({"nodes":nodes,"edges":edges})
}
fn ready(server: &Server) -> Arc<Generation> {
    server
        .clean_generation(Instant::now() + Duration::from_secs(30))
        .unwrap()
}
fn equals_cold(server: &Server) -> Arc<Generation> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let generation = ready(server);
        let config = CachedProject::configuration(&server.shared.root, &exclusions()).unwrap();
        let (cold, _, _) =
            source::build_from_dir_with_config(&server.shared.root, &config).unwrap();
        if generation.snapshot == Snapshot::capture(&server.shared.root, &config).unwrap()
            && canonical(&generation.graph) == canonical(&cold)
        {
            return generation;
        }
        assert!(
            Instant::now() < deadline,
            "watch never reached the complete cold graph"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn watch_atomic_batch_rename_delete_and_empty_save_match_cold() {
    let root = Fixture::new();
    let server = Server::start(&root.0).unwrap();
    let first = equals_cold(&server);
    std::fs::write(
        root.0.join("math.new"),
        "pub fn sum(a:i32,b:i32)->i32 { a+b }",
    )
    .unwrap();
    std::fs::rename(root.0.join("math.new"), root.0.join("math.rs")).unwrap();
    std::fs::write(root.0.join("lib.rs"), "mod math; pub use math::sum;").unwrap();
    let renamed = equals_cold(&server);
    assert!(renamed.number > first.number);
    assert!(renamed.graph.find_by_path("crate::math::add").is_none());
    std::fs::rename(root.0.join("math.rs"), root.0.join("calc.rs")).unwrap();
    std::fs::write(root.0.join("lib.rs"), "mod calc; pub use calc::sum;").unwrap();
    equals_cold(&server);
    std::fs::write(root.0.join("app.rs"), "").unwrap();
    std::fs::remove_file(root.0.join("calc.rs")).unwrap();
    equals_cold(&server);
}

#[test]
fn watch_partial_writes_stay_stale_and_event_storm_is_coalesced() {
    let root = Fixture::new();
    let server = Server::start(&root.0).unwrap();
    let first = ready(&server);
    for n in 0..40 {
        std::fs::write(
            root.0.join("math.rs"),
            format!("pub fn add(a:i32,b:i32)->i32 {{ a+b+{n} }}"),
        )
        .unwrap();
        // Deterministic event injection complements the real native events.
        server
            .shared
            .event(Ok(Event::new(EventKind::Modify(ModifyKind::Data(
                notify::event::DataChange::Content,
            )))
            .add_path(root.0.join("math.rs"))));
        thread::sleep(Duration::from_millis(5));
        assert!(server.shared.state.lock().unwrap().stale);
        assert_eq!(
            server
                .shared
                .state
                .lock()
                .unwrap()
                .generation
                .as_ref()
                .unwrap()
                .number,
            first.number
        );
    }
    let last_write = Instant::now();
    let final_graph = equals_cold(&server);
    assert!(last_write.elapsed() >= QUIET);
    assert!(final_graph.number > first.number);
}

#[test]
fn watch_exclusions_and_configured_scope_do_not_invalidate() {
    let root = Fixture::new();
    std::fs::write(
        root.0.join("girder.toml"),
        "[source]\nroots = [\".\"]\nexclude = [\"skip\"]\n",
    )
    .unwrap();
    let server = Server::start(&root.0).unwrap();
    let first = ready(&server);
    for dir in ["target", ".git", "node_modules", "__pycache__", "skip"] {
        std::fs::create_dir_all(root.0.join(dir)).unwrap();
        std::fs::write(root.0.join(dir).join("noise.rs"), "pub fn noise() {}").unwrap();
    }
    thread::sleep(Duration::from_millis(700));
    let final_graph = ready(&server);
    assert_eq!(first.number, final_graph.number);
    assert!(final_graph.graph.nodes().all(|n| n.name != "noise"));
}

#[test]
fn watch_overflow_and_unknown_rename_force_reconciliation() {
    let root = Fixture::new();
    let server = Server::start(&root.0).unwrap();
    ready(&server);
    let mut event = Event::new(EventKind::Other);
    event.attrs.set_flag(notify::event::Flag::Rescan);
    server.shared.event(Ok(event));
    equals_cold(&server);
    server
        .shared
        .event(Ok(Event::new(EventKind::Modify(ModifyKind::Name(
            RenameMode::Any,
        )))
        .add_path(root.0.join("math.rs"))));
    equals_cold(&server);
    let state = server.shared.state.lock().unwrap();
    assert!(state.metrics.fallbacks >= 2);
    assert_eq!(state.metrics.fallbacks, state.metrics.builds);
}

#[test]
fn watch_root_removal_retains_ownership_until_shutdown() {
    let root = Fixture::new();
    let server = Server::start(&root.0).unwrap();
    ready(&server);
    std::fs::remove_dir_all(&root.0).unwrap();
    server.shared.event(Ok(
        Event::new(EventKind::Remove(RemoveKind::Folder)).add_path(root.0.clone())
    ));
    assert!(server
        .clean_generation(Instant::now() + Duration::from_millis(50))
        .is_err());
    std::fs::create_dir_all(&root.0).unwrap();
    assert!(matches!(Server::start(&root.0),Err(e) if e.kind()==io::ErrorKind::AlreadyExists));
    std::fs::write(root.0.join("new.rs"), "pub fn restored() {}").unwrap();
    equals_cold(&server);
    drop(server);
    let restarted = Server::start(&root.0).unwrap();
    equals_cold(&restarted);
}

#[test]
fn watch_candidate_validation_rejects_transient_source_reads() {
    let root = Fixture::new();
    let config = CachedProject::configuration(&root.0, &exclusions()).unwrap();
    let before = Snapshot::capture(&root.0, &config).unwrap();
    let original = std::fs::read(root.0.join("math.rs")).unwrap();
    std::fs::write(root.0.join("math.rs"), "pub fn wrong() {}").unwrap();
    let wrong = CachedProject::open_with_exclusions(&root.0, &exclusions()).unwrap();
    std::fs::write(root.0.join("math.rs"), original).unwrap();
    assert!(!before.matches_candidate(&wrong));
}

#[test]
fn watch_dot_graph_path_keeps_one_owner_after_first_publication() {
    let root = Fixture::new();
    std::fs::write(
        root.0.join("girder.toml"),
        "[graph]\npath = \"./project.aether\"\n",
    )
    .unwrap();
    let server = Server::start(&root.0).unwrap();
    ready(&server);
    assert!(
        matches!(Server::start(&root.0), Err(error) if error.kind() == io::ErrorKind::AlreadyExists)
    );
}

#[test]
fn watch_releases_obsolete_graph_owner_after_configuration_change() {
    let root = Fixture::new();
    let nested = root.0.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("child.rs"), "pub fn child() {}\n").unwrap();
    std::fs::write(
        root.0.join("girder.toml"),
        "[graph]\npath = \"nested/project.aether\"\n",
    )
    .unwrap();
    let server = Server::start(&root.0).unwrap();
    ready(&server);
    std::fs::write(
        root.0.join("girder.toml"),
        "[graph]\npath = \"parent.aether\"\n",
    )
    .unwrap();
    equals_cold(&server);

    let nested_server = Server::start(&nested).unwrap();
    ready(&nested_server);
}

#[test]
fn watch_restricted_source_roots_and_live_scope_changes_match_cold() {
    let root = Fixture::new();
    std::fs::create_dir_all(root.0.join("src")).unwrap();
    for name in ["lib.rs", "math.rs", "app.rs"] {
        std::fs::rename(root.0.join(name), root.0.join("src").join(name)).unwrap();
    }
    std::fs::write(root.0.join("girder.toml"), "[source]\nroots = [\"src\"]\n").unwrap();
    let server = Server::start(&root.0).unwrap();
    let first = equals_cold(&server);
    std::fs::write(root.0.join("outside.rs"), "pub fn outside() {}\n").unwrap();
    thread::sleep(Duration::from_millis(700));
    assert_eq!(first.number, ready(&server).number);
    std::fs::write(root.0.join("girder.toml"), "[source]\nroots = [\".\"]\n").unwrap();
    let expanded = equals_cold(&server);
    assert!(expanded.graph.nodes().any(|node| node.name == "outside"));
    let state = server.shared.state.lock().unwrap();
    assert!(state.metrics.fallbacks > 0);
    assert!(state.metrics.parsed >= 4);
}

#[test]
fn watch_populated_child_directory_deletion_forces_full_reconciliation() {
    let root = Fixture::new();
    std::fs::create_dir_all(root.0.join("nested")).unwrap();
    std::fs::write(
        root.0.join("nested/helper.rs"),
        "pub fn nested_helper() {}\n",
    )
    .unwrap();
    let server = Server::start(&root.0).unwrap();
    ready(&server);
    std::fs::remove_dir_all(root.0.join("nested")).unwrap();
    let updated = equals_cold(&server);
    assert!(!updated
        .graph
        .nodes()
        .any(|node| node.name == "nested_helper"));
    let state = server.shared.state.lock().unwrap();
    assert!(state.metrics.fallbacks > 0);
    assert!(state.metrics.parsed >= 3);
}

mod measurement;
