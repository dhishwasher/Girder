//! Conditional, one-hop ESM certificates. Navigation edges are not proofs.
//!
//! Policy: docs/observations/stage3-typescript-audit/esm-import-proof/policy.md.
//! Recompute from the indexed sources and filesystem on every resolution pass.

mod environment;
pub use environment::TypeScriptEsmEnvironment;

use super::{FileState, IncrementalParser, Lang};
use crate::mapper::module_path_for;
use aether_graph::{CallClass, NodeId, NodeKind, SemanticGraph};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tree_sitter::Node as Syntax;

const REASON: &str = "proven-typescript-relative-esm-named-import";
const ASSUMPTIONS: &[&str] = &[
    "indexed-source-snapshot",
    "esm-native-execution",
    "no-unmodeled-module-hooks-loaders-or-mocks-outside-or-inside-snapshot",
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

fn token(node: Syntax<'_>, kind: &str) -> bool {
    (0..node.child_count()).any(|i| node.child(i).is_some_and(|n| n.kind() == kind))
}

fn literal<'a>(node: Syntax<'_>, source: &'a str) -> Option<&'a str> {
    if node.kind() != "string" {
        return None;
    }
    let raw = text(node, source);
    let quote = *raw.as_bytes().first()?;
    if !matches!(quote, b'\'' | b'"') || raw.as_bytes().last() != Some(&quote) {
        return None;
    }
    let inner = &raw[1..raw.len() - 1];
    (!inner.contains('\\')).then_some(inner)
}

fn executable_file(file: &str) -> bool {
    (file.ends_with(".ts") || file.ends_with(".mts"))
        && !file.ends_with(".d.ts")
        && !file.ends_with(".d.mts")
}

/// E3-S: raw == cooked, with precisely the frozen ASCII segment grammar.
fn relative_path(importer: &str, specifier: &str) -> Option<String> {
    let mut rest = specifier;
    let mut parents = 0;
    if let Some(tail) = rest.strip_prefix("./") {
        rest = tail;
    } else {
        while let Some(tail) = rest.strip_prefix("../") {
            parents += 1;
            rest = tail;
        }
        if parents == 0 {
            return None;
        }
    }
    if !executable_file(rest)
        || !rest.split('/').all(|part| {
            part.split('.').all(|word| {
                !word.is_empty()
                    && word
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            })
        })
    {
        return None;
    }
    let mut parts: Vec<_> = importer.split('/').collect();
    parts.pop()?;
    for _ in 0..parents {
        parts.pop()?;
    }
    parts.extend(rest.split('/'));
    Some(parts.join("/"))
}

/// C-2: never follow a symlink in an indexed file's root-relative identity.
fn regular_file(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut absolute = root.to_path_buf();
    let parts: Vec<_> = relative.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() || matches!(*part, "." | "..") || part.contains(['\\', ':']) {
            return None;
        }
        absolute.push(part);
        let metadata = std::fs::symlink_metadata(&absolute).ok()?;
        if metadata.file_type().is_symlink()
            || (i + 1 == parts.len() && !metadata.is_file())
            || (i + 1 < parts.len() && !metadata.is_dir())
        {
            return None;
        }
    }
    Some(absolute)
}

struct Identities<'a> {
    files: &'a HashMap<String, FileState>,
    valid: HashSet<String>,
}

impl<'a> Identities<'a> {
    fn new(root: &Path, files: &'a HashMap<String, FileState>) -> Self {
        let mut modules = HashMap::<String, usize>::new();
        let mut folded = HashMap::<String, usize>::new();
        for file in files.keys() {
            *modules.entry(module_path_for(file)).or_default() += 1;
            *folded.entry(file.to_ascii_lowercase()).or_default() += 1;
        }
        let valid = files
            .iter()
            .filter(|(file, state)| {
                modules.get(&module_path_for(file)) == Some(&1)
                    && folded.get(&file.to_ascii_lowercase()) == Some(&1)
                    && regular_file(root, file)
                        .and_then(|path| std::fs::read(path).ok())
                        .is_some_and(|bytes| bytes == state.source.as_bytes())
            })
            .map(|(file, _)| file.clone())
            .collect();
        Self { files, valid }
    }

    fn resolve(&self, importer: &str, specifier: &str) -> Option<String> {
        let target = relative_path(importer, specifier)?;
        (self.files.contains_key(&target) && self.valid.contains(&target)).then_some(target)
    }
}

#[derive(Default)]
struct Facts {
    eligible: bool,
    exports: HashMap<String, NodeId>,
    imports: Vec<Import>,
    calls: HashMap<(usize, usize), String>,
    dependencies: Vec<String>,
    mocks: Vec<String>,
    blocked: bool,
}

struct Import {
    local: String,
    name: String,
    specifier: String,
}

fn clean_binding(syntax: &[Syntax<'_>], source: &str, name: Syntax<'_>) -> bool {
    let spelling = text(name, source);
    syntax
        .iter()
        .filter(|n| n.kind().contains("identifier") && text(**n, source) == spelling)
        .all(|n| {
            n.id() == name.id()
                || (n.kind() == "identifier"
                    && n.parent().is_some_and(|p| {
                        p.kind() == "call_expression"
                            && p.child_by_field_name("function")
                                .is_some_and(|f| f.id() == n.id())
                    }))
        })
}

fn facts(file: &str, state: &FileState) -> Facts {
    let Some(lang) = Lang::from_path(file).filter(|lang| lang.is_typescript()) else {
        return Facts::default();
    };
    let source = &state.source;
    let tree = IncrementalParser::new(lang).parse(source);
    let root = tree.root_node();
    let mut syntax = Vec::new();
    walk(root, &mut syntax);
    let mut ids = HashSet::new();
    let duplicate = state.nodes.iter().any(|n| !ids.insert(n.id));
    let mut cursor = root.walk();
    let module = root
        .named_children(&mut cursor)
        .any(|n| matches!(n.kind(), "import_statement" | "export_statement"));
    let eligible = executable_file(file)
        && module
        && !root.has_error()
        && !duplicate
        && !syntax.iter().any(|n| {
            matches!(n.kind(), "decorator" | "with_statement")
                || (n.kind().contains("identifier")
                    && (text(*n, source) == "eval" || text(*n, source).contains('\\')))
        });
    let mut result = Facts {
        eligible,
        ..Facts::default()
    };

    // HOOK-1/2 also inspect files ineligible to receive certificates.
    const LIBRARIES: &[&str] = &[
        "esmock",
        "testdouble",
        "quibble",
        "proxyquire",
        "mock-require",
        "rewire",
        "rewiremock",
        "mockery",
        "jest-mock",
        "@babel/register",
        "ts-node",
        "tsx",
        "jiti",
        "@swc-node/register",
        "esbuild-register",
        "vitest",
        "@jest/globals",
        "bun:test",
    ];
    let module_api = syntax
        .iter()
        .any(|n| literal(*n, source).is_some_and(|s| matches!(s, "module" | "node:module")));
    for n in &syntax {
        // Refuse spellings that could conceal a modeled API until decoded.
        // Computed literal mock methods cannot establish a direct-call identity.
        if n.kind() == "subscript_expression"
            || (n.kind() == "string" && text(*n, source).contains('\\'))
            || literal(*n, source).is_some_and(|s| {
                matches!(
                    s,
                    "mock"
                        | "doMock"
                        | "unmock"
                        | "doUnmock"
                        | "setMock"
                        | "unstable_mockModule"
                        | "module"
                )
            })
        {
            result.blocked = true;
        }
        if literal(*n, source).is_some_and(|s| {
            LIBRARIES.iter().any(|lib| {
                s == *lib
                    || s.strip_prefix(lib)
                        .is_some_and(|tail| tail.starts_with('/'))
            })
        }) || (module_api
            && ((n.kind().contains("identifier")
                && (matches!(text(*n, source), "register" | "registerHooks")
                    || text(*n, source).contains('\\')))
                || literal(*n, source).is_some_and(|s| matches!(s, "register" | "registerHooks"))))
        {
            result.blocked = true;
        }
        // Match mock methods on any object, including aliases. A method
        // reference outside a direct call cannot establish module identity.
        let mock_name = matches!(
            text(*n, source),
            "mock"
                | "doMock"
                | "unmock"
                | "doUnmock"
                | "setMock"
                | "unstable_mockModule"
                | "module"
        );
        if !mock_name
            || !matches!(
                n.kind(),
                "property_identifier" | "shorthand_property_identifier_pattern"
            )
        {
            continue;
        }
        let Some(member) = n.parent().filter(|p| p.kind() == "member_expression") else {
            // A destructured mock API may be invoked under an arbitrary name.
            if n.kind() == "shorthand_property_identifier_pattern"
                || n.parent().is_some_and(|p| p.kind() == "pair_pattern")
            {
                result.blocked = true;
            }
            continue;
        };
        let Some(call) = member.parent().filter(|p| {
            p.kind() == "call_expression"
                && p.child_by_field_name("function")
                    .is_some_and(|f| f.id() == member.id())
        }) else {
            // x.mock.module is the containing API object, not a method alias.
            if !(text(*n, source) == "mock"
                && member.parent().is_some_and(|p| {
                    p.kind() == "member_expression"
                        && p.child_by_field_name("property")
                            .is_some_and(|p| text(p, source) == "module")
                }))
            {
                result.blocked = true;
            }
            continue;
        };
        let arg = call
            .child_by_field_name("arguments")
            .and_then(|a| a.named_child(0));
        let specifier = arg.and_then(|arg| {
            literal(arg, source).or_else(|| {
                (arg.kind() == "call_expression"
                    && arg
                        .child_by_field_name("function")
                        .is_some_and(|f| f.kind() == "import"))
                .then(|| {
                    arg.child_by_field_name("arguments")
                        .and_then(|a| a.named_child(0))
                        .and_then(|a| literal(a, source))
                })
                .flatten()
            })
        });
        match specifier {
            Some(specifier) => result.mocks.push(specifier.to_string()),
            None => result.blocked = true,
        }
    }

    let export_barrel = syntax.iter().any(|n| {
        n.kind() == "export_statement"
            && (n.child_by_field_name("source").is_some() || token(*n, "*"))
    });
    for n in &syntax {
        if n.kind() == "import_statement" {
            let Some(specifier) = n
                .child_by_field_name("source")
                .and_then(|s| literal(s, source))
            else {
                continue;
            };
            if !token(*n, "type") {
                result.dependencies.push(specifier.to_string());
            }
            if token(*n, "type")
                || token(*n, "with")
                || (0..n.named_child_count()).any(|i| {
                    n.named_child(i)
                        .is_some_and(|c| c.kind() == "import_attribute")
                })
                || n.parent().is_none_or(|p| p.id() != root.id())
            {
                continue;
            }
            let mut c = n.walk();
            let Some(clause) = n
                .named_children(&mut c)
                .find(|c| c.kind() == "import_clause")
            else {
                continue;
            };
            let mut c = clause.walk();
            let children: Vec<_> = clause.named_children(&mut c).collect();
            if children.len() != 1 || children[0].kind() != "named_imports" {
                continue;
            }
            let mut c = children[0].walk();
            for spec in children[0].named_children(&mut c) {
                if spec.kind() != "import_specifier" || token(spec, "type") {
                    continue;
                }
                let Some(name) = spec
                    .child_by_field_name("name")
                    .filter(|n| n.kind() == "identifier")
                else {
                    continue;
                };
                let local = spec.child_by_field_name("alias").unwrap_or(name);
                if local.kind() == "identifier" && clean_binding(&syntax, source, local) {
                    result.imports.push(Import {
                        local: text(local, source).into(),
                        name: text(name, source).into(),
                        specifier: specifier.into(),
                    });
                }
            }
        }
        if n.kind() == "function_declaration"
            && n.child_by_field_name("body").is_some()
            && !export_barrel
        {
            let Some(export) = n.parent().filter(|p| {
                p.kind() == "export_statement"
                    && !token(*p, "default")
                    && p.parent().is_some_and(|p| p.id() == root.id())
            }) else {
                continue;
            };
            let _ = export;
            let Some(name) = n.child_by_field_name("name") else {
                continue;
            };
            if !clean_binding(&syntax, source, name) {
                continue;
            }
            let candidates: Vec<_> = state
                .nodes
                .iter()
                .filter(|node| {
                    node.kind == NodeKind::Function
                        && node.span.start_byte == n.start_byte()
                        && node.span.end_byte == n.end_byte()
                })
                .collect();
            if let [node] = candidates.as_slice() {
                result.exports.insert(text(name, source).into(), node.id);
            }
        }
        if n.kind() == "call_expression"
            && !token(*n, "?.")
            && n.child_by_field_name("optional_chain").is_none()
            && n.child_by_field_name("arguments")
                .is_some_and(|a| a.kind() == "arguments")
            && !state
                .refused_structural_subtrees
                .iter()
                .any(|s| s.start_byte <= n.start_byte() && n.end_byte() <= s.end_byte)
        {
            if let Some(callee) = n
                .child_by_field_name("function")
                .filter(|c| c.kind() == "identifier")
            {
                result
                    .calls
                    .insert((n.start_byte(), n.end_byte()), text(callee, source).into());
            }
        }
    }
    result
}

fn reaches(start: &str, target: &str, dependencies: &HashMap<String, Vec<String>>) -> bool {
    let mut pending = vec![start];
    let mut seen = HashSet::new();
    while let Some(file) = pending.pop() {
        if file == target {
            return true;
        }
        if seen.insert(file) {
            if let Some(next) = dependencies.get(file) {
                pending.extend(next.iter().map(String::as_str));
            }
        }
    }
    false
}

pub(super) fn resolve(
    root: Option<&Path>,
    files: &HashMap<String, FileState>,
    graph: &mut SemanticGraph,
    environment: &mut Option<TypeScriptEsmEnvironment>,
) {
    *environment = None;
    // Revoke the previous generation first, including single-file load/delete
    // routes that do not reconstruct every node before resolving.
    let previous: Vec<_> = graph
        .nodes()
        .filter(|node| node.language == "typescript")
        .filter_map(|node| {
            graph
                .call_evidence(node.id)
                .ok()
                .filter(|e| e.calls.iter().any(|c| c.reason == REASON))
                .map(|e| (node.id, e))
        })
        .collect();
    for (id, mut evidence) in previous {
        for call in &mut evidence.calls {
            if call.reason == REASON {
                call.class = CallClass::Unknown;
                call.targets.clear();
                call.reason = "typescript-binding-or-structural-dispatch-unproven".into();
            }
        }
        if let Some(node) = graph.get_mut(id) {
            let _ = evidence.attach(node);
        }
    }
    let Some(root) = root else { return };
    if !files.keys().any(|f| executable_file(f)) {
        return;
    }
    let mut all_facts = HashMap::new();
    for (file, state) in files {
        let file_facts = facts(file, state);
        if file_facts.blocked {
            return;
        }
        all_facts.insert(file.clone(), file_facts);
    }
    let facts = all_facts;
    if !facts.values().any(|f| f.eligible && !f.imports.is_empty()) {
        return;
    }
    let Ok(current_environment) = TypeScriptEsmEnvironment::capture(root) else {
        return;
    };
    let clear = current_environment.is_clear();
    *environment = Some(current_environment);
    if !clear {
        return;
    }
    let identity = Identities::new(root, files);
    let mut mocked = HashSet::new();
    for (file, f) in &facts {
        if !f.mocks.is_empty() && !identity.valid.contains(file) {
            return;
        }
        for specifier in &f.mocks {
            let Some(target) = identity.resolve(file, specifier) else {
                return;
            };
            mocked.insert(target);
        }
    }
    let dependencies: HashMap<_, _> = facts
        .iter()
        .map(|(file, f)| {
            (
                file.clone(),
                f.dependencies
                    .iter()
                    .filter_map(|specifier| identity.resolve(file, specifier))
                    .collect(),
            )
        })
        .collect();
    for (file, f) in &facts {
        if !f.eligible || !identity.valid.contains(file) {
            continue;
        }
        let mut bindings = HashMap::new();
        for import in &f.imports {
            let Some(target_file) = identity.resolve(file, &import.specifier) else {
                continue;
            };
            if mocked.contains(&target_file) || reaches(&target_file, file, &dependencies) {
                continue;
            }
            let Some(target_facts) = facts.get(&target_file).filter(|f| f.eligible) else {
                continue;
            };
            let Some(target) = target_facts.exports.get(&import.name) else {
                continue;
            };
            let Some(node) = graph.get(*target) else {
                continue;
            };
            if !files[&target_file].nodes.iter().any(|original| {
                original.id == *target
                    && original.source == node.source
                    && original.span == node.span
            }) {
                continue;
            }
            if node.file.as_deref() != Some(&target_file)
                || !files[file]
                    .rust_imports
                    .iter()
                    .any(|i| i.local == import.local && i.target == node.path)
            {
                continue;
            }
            bindings.insert(import.local.as_str(), *target);
        }
        for extracted in &files[file].nodes {
            if !graph.get(extracted.id).is_some_and(|node| {
                node.file.as_deref() == Some(file)
                    && node.source == extracted.source
                    && node.span == extracted.span
            }) {
                continue;
            }
            let Ok(mut evidence) = graph.call_evidence(extracted.id) else {
                continue;
            };
            let mut changed = false;
            for claim in &mut evidence.calls {
                if claim.class != CallClass::Unknown
                    || claim.coverage_gap
                    || !claim.targets.is_empty()
                {
                    continue;
                }
                let Some(name) = f.calls.get(&(claim.site.start_byte, claim.site.end_byte)) else {
                    continue;
                };
                let Some(target) = bindings.get(name.as_str()) else {
                    continue;
                };
                claim.class = CallClass::Must;
                claim.targets = vec![*target];
                claim.reason = REASON.into();
                changed = true;
            }
            if changed {
                for assumption in ASSUMPTIONS {
                    if !evidence.assumptions.iter().any(|a| a == assumption) {
                        evidence.assumptions.push((*assumption).into());
                    }
                }
                if let Some(node) = graph.get_mut(extracted.id) {
                    let _ = evidence.attach(node);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::relative_path;

    #[test]
    fn url_and_lexical_identity_must_agree() {
        assert_eq!(
            relative_path("test/app.test.ts", "../app.ts"),
            Some("app.ts".into())
        );
        assert_eq!(
            relative_path("test.ts", "./some.dir/a-b_2.mts"),
            Some("some.dir/a-b_2.mts".into())
        );
        for specifier in [
            "./%61pp.ts",
            "./ap\\p.ts",
            "./app.ts ",
            "./ap\tp.ts",
            "./äpp.ts",
            "./app.ts?q",
            "./app.ts#x",
            "./app.d.ts",
            "./app.d.mts",
            "./app.tsx",
            "./app.cts",
            "./app.js",
            "./app",
            "../app.ts",
            "./a/../app.ts",
            "./a//app.ts",
            "./.app.ts",
            "app.ts",
        ] {
            assert_eq!(relative_path("test.ts", specifier), None, "{specifier}");
        }
    }
}
