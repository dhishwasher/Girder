//! Go same-package direct-call proof and lost-evidence safety net (Stage 3, Go).
//!
//! Implements `docs/observations/stage3-go-audit/policy.md`. The pass owns the
//! class of every bare-identifier Go call: it recomputes Must or Unknown from
//! package facts on every resolution, so the result is the same for cold and
//! incremental builds and the refusals apply to same-file calls too. It then
//! gives every call expression that has no claim on a node of its own file an
//! explicit Unknown claim, so a hole the graph cannot represent is never
//! omitted.

use super::{FileState, IncrementalParser, Lang};
use aether_graph::{CallClaim, CallClass, NodeId, NodeKind, SemanticGraph, Span};
use std::collections::{HashMap, HashSet};
use tree_sitter::Node as Syntax;

const REASON: &str = "proven-go-same-package-call";
const REFUSED: &str = "go-same-package-proof-refused";
const ORPHAN: &str = "go-call-evidence-unattributed";
const ASSUMPTION: &str = "go-package-compiles-for-analyzed-platform";

const GOOS: &[&str] = &[
    "aix",
    "android",
    "darwin",
    "dragonfly",
    "freebsd",
    "hurd",
    "illumos",
    "ios",
    "js",
    "linux",
    "nacl",
    "netbsd",
    "openbsd",
    "plan9",
    "solaris",
    "wasip1",
    "windows",
    "zos",
];
const GOARCH: &[&str] = &[
    "386",
    "amd64",
    "amd64p32",
    "arm",
    "armbe",
    "arm64",
    "arm64be",
    "loong64",
    "mips",
    "mipsle",
    "mips64",
    "mips64le",
    "mips64p32",
    "mips64p32le",
    "ppc",
    "ppc64",
    "ppc64le",
    "riscv",
    "riscv64",
    "s390",
    "s390x",
    "sparc",
    "sparc64",
    "wasm",
];

fn text<'a>(node: Syntax<'_>, source: &'a str) -> &'a str {
    &source[node.start_byte()..node.end_byte()]
}

fn walk<'a>(node: Syntax<'a>, all: &mut Vec<Syntax<'a>>) {
    all.push(node);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, all);
    }
}

fn directory(file: &str) -> &str {
    file.rsplit_once('/').map_or(".", |(dir, _)| dir)
}

/// `name_GOOS.go`, `name_GOARCH.go`, `name_GOOS_GOARCH.go` (with an optional
/// `_test` before `.go`). The first component never counts as a constraint.
fn filename_constrained(file: &str) -> bool {
    let base = file.rsplit('/').next().unwrap_or(file);
    let stem = base.strip_suffix(".go").unwrap_or(base);
    let stem = stem.strip_suffix("_test").unwrap_or(stem);
    let parts: Vec<&str> = stem.split('_').collect();
    if parts.len() < 2 {
        return false;
    }
    let last = parts[parts.len() - 1];
    if GOOS.contains(&last) || GOARCH.contains(&last) {
        return true;
    }
    parts.len() >= 3 && GOOS.contains(&parts[parts.len() - 2]) && GOARCH.contains(&last)
}

struct Decl {
    is_func: bool,
    has_body: bool,
    generic: bool,
    span: (usize, usize),
}

struct Call {
    span: Span,
    name: Option<String>,
    enclosed: bool,
}

#[derive(Default)]
struct Facts {
    clause: Option<String>,
    parse_error: bool,
    constrained: bool,
    cgo: bool,
    dot_import: bool,
    test_file: bool,
    decls: HashMap<String, Vec<Decl>>,
    calls: Vec<Call>,
    occurrences: HashMap<String, usize>,
    call_positions: HashMap<String, usize>,
    own_function_names: HashMap<String, usize>,
}

impl Facts {
    /// Every occurrence of `name` in this file is a direct call or a top-level
    /// function declaration name. Rejects shadowing, escaping function values,
    /// field and method names, and unmodeled uses.
    fn clean(&self, name: &str) -> bool {
        let seen = self.occurrences.get(name).copied().unwrap_or(0);
        let calls = self.call_positions.get(name).copied().unwrap_or(0);
        let own = self.own_function_names.get(name).copied().unwrap_or(0);
        seen == calls + own
    }
}

fn span_of(node: Syntax<'_>) -> Span {
    let start = node.start_position();
    Span {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start_row: start.row,
        start_col: start.column,
    }
}

fn facts(file: &str, state: &FileState) -> Facts {
    let source = &state.source;
    let tree = IncrementalParser::new(Lang::Go).parse(source);
    let root = tree.root_node();
    let mut syntax = Vec::new();
    walk(root, &mut syntax);
    let mut result = Facts {
        parse_error: root.has_error(),
        test_file: file.ends_with("_test.go"),
        constrained: filename_constrained(file)
            || source.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with("//go:build") || line.starts_with("// +build")
            }),
        ..Facts::default()
    };
    let mut cursor = root.walk();
    for top in root.named_children(&mut cursor) {
        match top.kind() {
            "package_clause" => {
                let mut inner = top.walk();
                result.clause = top
                    .named_children(&mut inner)
                    .find(|n| n.kind() == "package_identifier")
                    .map(|n| text(n, source).to_string());
            }
            "function_declaration" => {
                let Some(name) = top.child_by_field_name("name") else {
                    continue;
                };
                let name = text(name, source).to_string();
                *result.own_function_names.entry(name.clone()).or_default() += 1;
                result.decls.entry(name).or_default().push(Decl {
                    is_func: true,
                    has_body: top.child_by_field_name("body").is_some(),
                    generic: top.child_by_field_name("type_parameters").is_some(),
                    span: (top.start_byte(), top.end_byte()),
                });
            }
            "type_declaration" | "var_declaration" | "const_declaration" => {
                let mut specs = Vec::new();
                let mut inner = top.walk();
                for child in top.named_children(&mut inner) {
                    if child.kind().ends_with("_spec_list") {
                        let mut deeper = child.walk();
                        specs.extend(child.named_children(&mut deeper));
                    } else {
                        specs.push(child);
                    }
                }
                for spec in specs {
                    if !spec.kind().ends_with("_spec") && spec.kind() != "type_alias" {
                        continue;
                    }
                    let mut names = spec.walk();
                    for name in spec.children_by_field_name("name", &mut names) {
                        result
                            .decls
                            .entry(text(name, source).to_string())
                            .or_default()
                            .push(Decl {
                                is_func: false,
                                has_body: true,
                                generic: false,
                                span: (spec.start_byte(), spec.end_byte()),
                            });
                    }
                }
            }
            _ => {}
        }
    }
    let function_positions: HashSet<usize> = syntax
        .iter()
        .filter(|n| n.kind() == "call_expression")
        .filter_map(|n| n.child_by_field_name("function"))
        .filter(|f| f.kind() == "identifier")
        .map(|f| f.id())
        .collect();
    for node in &syntax {
        match node.kind() {
            "identifier" | "field_identifier" | "type_identifier" => {
                let name = text(*node, source).to_string();
                *result.occurrences.entry(name.clone()).or_default() += 1;
                if function_positions.contains(&node.id()) {
                    *result.call_positions.entry(name).or_default() += 1;
                }
            }
            "import_spec" => {
                if node
                    .child_by_field_name("path")
                    .is_some_and(|p| text(p, source) == "\"C\"")
                {
                    result.cgo = true;
                }
                if node
                    .child_by_field_name("name")
                    .is_some_and(|n| n.kind() == "dot")
                {
                    result.dot_import = true;
                }
            }
            // `T[X](v)` parses as a conversion to a generic type but is a call
            // whenever `T` names a generic function, which syntax cannot decide.
            "type_conversion_expression" => {
                result.calls.push(Call {
                    span: span_of(*node),
                    name: None,
                    enclosed: true,
                });
            }
            "call_expression" => {
                let function = node.child_by_field_name("function");
                let name = function
                    .filter(|f| f.kind() == "identifier")
                    .map(|f| text(f, source).to_string());
                let mut enclosed = false;
                let mut parent = node.parent();
                while let Some(p) = parent {
                    if matches!(p.kind(), "function_declaration" | "method_declaration") {
                        enclosed = true;
                        break;
                    }
                    parent = p.parent();
                }
                result.calls.push(Call {
                    span: span_of(*node),
                    name,
                    enclosed,
                });
            }
            _ => {}
        }
    }
    result
}

type Package = (String, String);
/// Package-scope declarations by package, then by name: (file, declaration).
type PackageDecls<'a> = HashMap<Package, HashMap<&'a str, Vec<(&'a str, &'a Decl)>>>;

pub(super) fn resolve(files: &HashMap<String, FileState>, graph: &mut SemanticGraph) {
    let mut names: Vec<&String> = files
        .keys()
        .filter(|f| Lang::from_path(f) == Some(Lang::Go))
        .collect();
    names.sort();
    // Revoke the previous generation's lost-evidence claims first.
    let go_nodes: Vec<NodeId> = graph
        .nodes()
        .filter(|node| node.language == "go")
        .map(|node| node.id)
        .collect();
    for id in &go_nodes {
        let Ok(mut evidence) = graph.call_evidence(*id) else {
            continue;
        };
        let before = evidence.calls.len();
        evidence.calls.retain(|c| c.reason != ORPHAN);
        if evidence.calls.len() != before {
            if let Some(node) = graph.get_mut(*id) {
                let _ = evidence.attach(node);
            }
        }
    }
    if names.is_empty() {
        return;
    }
    let all: HashMap<&str, Facts> = names
        .iter()
        .map(|f| (f.as_str(), facts(f, &files[f.as_str()])))
        .collect();
    // Package-scope declarations per package, and the clauses declaring a name per directory.
    let mut package_decls: PackageDecls = HashMap::new();
    let mut clauses_for: HashMap<(&str, &str), HashSet<&str>> = HashMap::new();
    for name in &names {
        let f = &all[name.as_str()];
        let clause = f.clause.as_deref().unwrap_or("");
        let dir = directory(name);
        for (decl_name, decls) in &f.decls {
            for decl in decls {
                package_decls
                    .entry((dir.to_string(), clause.to_string()))
                    .or_default()
                    .entry(decl_name.as_str())
                    .or_default()
                    .push((name.as_str(), decl));
            }
            clauses_for
                .entry((dir, decl_name.as_str()))
                .or_default()
                .insert(clause);
        }
    }
    let target_for = |caller: &str, call: &Call, graph: &SemanticGraph| -> Option<NodeId> {
        let f = &all[caller];
        let name = call.name.as_deref()?;
        let clause = f.clause.as_deref()?;
        if f.parse_error
            || f.constrained
            || f.cgo
            || f.dot_import
            || !call.enclosed
            || !f.clean(name)
        {
            return None;
        }
        let dir = directory(caller);
        let declared = package_decls
            .get(&(dir.to_string(), clause.to_string()))?
            .get(name)?;
        let [(target_file, decl)] = declared.as_slice() else {
            return None;
        };
        if !decl.is_func || !decl.has_body || decl.generic {
            return None;
        }
        if clauses_for.get(&(dir, name)).is_none_or(|c| c.len() != 1) {
            return None;
        }
        let t = &all[*target_file];
        if t.parse_error || t.constrained || t.cgo || (t.test_file && !f.test_file) {
            return None;
        }
        let extracted = files[*target_file].nodes.iter().find(|n| {
            n.kind == NodeKind::Function
                && n.span.start_byte == decl.span.0
                && n.span.end_byte == decl.span.1
        })?;
        let node = graph.get(extracted.id)?;
        (node.file.as_deref() == Some(*target_file)
            && node.span == extracted.span
            && node.source == extracted.source)
            .then_some(extracted.id)
    };
    let mut proven: HashMap<(&str, usize, usize), NodeId> = HashMap::new();
    for name in &names {
        for call in all[name.as_str()].calls.iter().filter(|c| c.name.is_some()) {
            if let Some(target) = target_for(name.as_str(), call, &*graph) {
                proven.insert(
                    (name.as_str(), call.span.start_byte, call.span.end_byte),
                    target,
                );
            }
        }
    }
    for name in &names {
        let f = &all[name.as_str()];
        let by_site: HashMap<(usize, usize), &Call> = f
            .calls
            .iter()
            .filter(|c| c.name.is_some())
            .map(|c| ((c.span.start_byte, c.span.end_byte), c))
            .collect();
        for extracted in &files[name.as_str()].nodes {
            if !matches!(extracted.kind, NodeKind::Function | NodeKind::Module) {
                continue;
            }
            if !graph.get(extracted.id).is_some_and(|node| {
                node.file.as_deref() == Some(name.as_str())
                    && node.source == extracted.source
                    && node.span == extracted.span
            }) {
                continue;
            }
            let Ok(mut evidence) = graph.call_evidence(extracted.id) else {
                continue;
            };
            let mut changed = false;
            let mut any_proven = false;
            for claim in &mut evidence.calls {
                if claim.coverage_gap {
                    continue;
                }
                if !by_site.contains_key(&(claim.site.start_byte, claim.site.end_byte)) {
                    continue;
                }
                let target = proven
                    .get(&(name.as_str(), claim.site.start_byte, claim.site.end_byte))
                    .copied();
                if let Some(target) = target {
                    if claim.class != CallClass::Must
                        || claim.targets != vec![target]
                        || claim.reason != REASON
                    {
                        claim.class = CallClass::Must;
                        claim.targets = vec![target];
                        claim.reason = REASON.into();
                        changed = true;
                    }
                    any_proven = true;
                } else if claim.class == CallClass::Must {
                    claim.class = CallClass::Unknown;
                    claim.targets.clear();
                    claim.reason = REFUSED.into();
                    changed = true;
                }
            }
            if any_proven && !evidence.assumptions.iter().any(|a| a == ASSUMPTION) {
                evidence.assumptions.push(ASSUMPTION.into());
                changed = true;
            }
            if changed {
                if let Some(node) = graph.get_mut(extracted.id) {
                    let _ = evidence.attach(node);
                }
            }
        }
    }
    // Lost-evidence safety net: every call expression needs a claim on a node of
    // its own file. Anchor missing ones on the file's first surviving function.
    let mut by_file: HashMap<String, Vec<NodeId>> = HashMap::new();
    for node in graph.nodes().filter(|n| n.language == "go") {
        if let Some(file) = node.file.as_deref() {
            by_file.entry(file.to_string()).or_default().push(node.id);
        }
    }
    for name in &names {
        let f = &all[name.as_str()];
        let ids = by_file.get(name.as_str()).cloned().unwrap_or_default();
        let mut present: HashSet<(usize, usize)> = HashSet::new();
        for id in &ids {
            if let Ok(evidence) = graph.call_evidence(*id) {
                present.extend(
                    evidence
                        .calls
                        .iter()
                        .filter(|c| !c.coverage_gap)
                        .map(|c| (c.site.start_byte, c.site.end_byte)),
                );
            }
        }
        let anchor = ids
            .iter()
            .filter_map(|id| graph.get(*id))
            .filter(|n| n.kind == NodeKind::Function)
            .min_by_key(|n| (n.span.start_byte, n.id))
            .map(|n| n.id);
        let Some(anchor) = anchor else { continue };
        let Ok(mut evidence) = graph.call_evidence(anchor) else {
            continue;
        };
        let mut missing: Vec<&Call> = f
            .calls
            .iter()
            .filter(|c| !present.contains(&(c.span.start_byte, c.span.end_byte)))
            .collect();
        missing.sort_by_key(|c| (c.span.start_byte, c.span.end_byte));
        if missing.is_empty() {
            continue;
        }
        for call in missing {
            evidence.calls.push(CallClaim {
                site: call.span,
                class: CallClass::Unknown,
                targets: vec![],
                reason: ORPHAN.into(),
                coverage_gap: false,
            });
        }
        if let Some(node) = graph.get_mut(anchor) {
            let _ = evidence.attach(node);
        }
    }
}
