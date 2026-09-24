//! Independent syntax inventory for call evidence. The legacy name resolver is
//! useful for navigation, but its chosen candidate is not proof of a binding.

use super::{node_text, span_of, BuildOutput};
use crate::parser::Lang;
use aether_graph::{CallClaim, CallClass, CallEvidence, NodeId, NodeKind, Span};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node as TsNode, Tree};

/// The same-file gates `annotate()` already computes, exposed so the
/// crate-wide method-call pass (`sync/rust_methods.rs`) can check them for a
/// call's own caller instead of re-deriving them -- two implementations of
/// `transformed_scope`/`duplicate_paths`/`macro_owners` could drift apart,
/// which is itself a false-Must path.
#[derive(Debug, Default, Clone)]
pub(crate) struct AnnotateGates {
    pub transformed_scope: bool,
    pub duplicate_paths: bool,
    pub parse_error: bool,
    /// Function-node indices (into `BuildOutput::nodes`) whose body contains
    /// some untrusted macro invocation elsewhere -- mirrors `annotate()`'s
    /// own `macro_owners`.
    pub macro_owners: HashSet<usize>,
    /// Every string literal's inner content (Python only) found anywhere in
    /// this file -- exposed so a project-wide pass
    /// (`sync/python_rebinding.rs`) can build a crate-wide string-rebinding
    /// index. This file's own `annotate()` pass already excludes a
    /// same-file string match from `proven` (see `string_rebound` in this
    /// function); a rebinding string can also live in a DIFFERENT file
    /// than the target's own definition (e.g. a test file's
    /// `mocker.patch('pkg.module.target')` naming a function defined in
    /// `pkg/module.py`), which this file-scoped pass alone cannot see --
    /// closed by the project-wide pass instead, not left as a disclosed
    /// gap, after this was checked directly against the real audited
    /// packages and found to actually occur (not just theoretically
    /// possible).
    pub python_string_literals: HashSet<String>,
    /// Bare attribute names (Python only) that are ever the target of an
    /// attribute assignment (`mod.target = ...`), augmented assignment
    /// (`mod.target += ...`), or `del` (`del mod.target`) anywhere in this
    /// file -- the same rebinding hazard as `setattr(mod, "target", ...)`,
    /// just spelled without a string literal. `globals()[...]`/
    /// `vars()[...]` subscript assignment is already covered: the subscript
    /// key is itself a `string` node (`string_content` already collected
    /// above). This set is NOT consulted by this file's own same-file
    /// `proven` computation -- it is already redundant with `clean` there
    /// (the grammar's `attribute` field is itself an `identifier` node, so
    /// `clean`'s blanket identifier scan already rejects it; verified by
    /// mutation, not assumed). It exists purely for the project-wide pass
    /// (`sync/python_rebinding.rs`), which needs this to check a target
    /// rebound from a DIFFERENT file, where no such same-file `clean` scan
    /// exists.
    pub python_attribute_rebind_targets: HashSet<String>,
}

pub(super) fn annotate(
    tree: &Tree,
    source: &str,
    lang: Lang,
    out: &mut BuildOutput,
) -> AnnotateGates {
    let root = tree.root_node();
    let Some(module) = out
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Module && n.source == source)
    else {
        // No module node to attach evidence to at all: fail closed rather
        // than report gates a caller might trust.
        return AnnotateGates {
            transformed_scope: true,
            ..AnnotateGates::default()
        };
    };
    let mut syntax = Vec::new();
    walk(root, &mut syntax);
    let callable =
        |n: &TsNode<'_>| matches!(n.kind(), "call_expression" | "call" | "new_expression");
    let mut claims: HashMap<usize, Vec<CallClaim>> = HashMap::new();
    let mut seen = HashSet::new();
    let duplicate_paths = out.nodes.iter().any(|n| !seen.insert(n.id));
    let transformed_scope = syntax.iter().any(|n| {
        matches!(n.kind(), "decorator" | "decorated_definition")
            || (matches!(n.kind(), "attribute_item" | "inner_attribute_item")
                && !matches!(node_text(*n, source).trim(), "#[test]" | "#[tokio::test]"))
            || (n.kind() == "macro_invocation" && owner(*n, out, module) == module)
    });
    // Standard library assertion macros have known, fixed semantics: they
    // evaluate their arguments as ordinary expressions and compare/check
    // them, with no other hidden effect on what gets called. A bare call
    // textually inside one of them is therefore provable the same way a
    // same-file top-level direct call is, UNLIKE an arbitrary macro (which
    // could do anything with its arguments, including never evaluating
    // them). Trusted only when the file does not itself define or import
    // something under one of these exact names — checked conservatively
    // (AST-based, whole-file) so an unrecognized shadowing pattern fails
    // closed, not open.
    const TRUSTED_ASSERT_MACROS: &[&str] = &[
        "assert",
        "assert_eq",
        "assert_ne",
        "debug_assert",
        "debug_assert_eq",
        "debug_assert_ne",
    ];
    let shadowed_assert_macros = syntax.iter().any(|n| match n.kind() {
        "macro_definition" => n
            .child_by_field_name("name")
            .is_some_and(|name| TRUSTED_ASSERT_MACROS.contains(&node_text(name, source))),
        "use_declaration" | "use_as_clause" => {
            let text = node_text(*n, source);
            TRUSTED_ASSERT_MACROS
                .iter()
                .any(|macro_name| use_declaration_names(text).any(|name| name == *macro_name))
        }
        _ => false,
    });
    let is_trusted_assert_macro = |n: &TsNode<'_>| -> bool {
        !shadowed_assert_macros
            && lang == Lang::Rust
            && n.child_by_field_name("macro")
                .is_some_and(|m| TRUSTED_ASSERT_MACROS.contains(&node_text(m, source)))
    };
    // Call-shaped bare-identifier occurrences (`name(...)`, not `.name(...)`
    // or `Path::name(...)`) found by walking a trusted, unshadowed assert
    // macro's argument token tree directly -- tree-sitter does not parse
    // macro arguments as ordinary expressions, so there is no call_expression
    // node to find inside one; this walks the raw token leaves instead. Each
    // trusted macro's *entire* argument list is rejected outright (not just
    // the offending spot) when it contains anything this simple structural
    // walk cannot see through: a block (`{`), a closure or bitwise-or (`|`),
    // a `let`/`fn` keyword, or another macro invocation (`ident!`) -- any of
    // which could introduce local shadowing, or rewrite/never-evaluate the
    // arguments, in a way this walk has no way to detect. The scan runs over
    // masked text (comments/string/char literals blanked out, same masking
    // the legacy textual resolver uses) so a keyword or brace appearing only
    // inside a string literal doesn't trigger the rejection. Fails closed:
    // whatever this rejects simply gets no proof, exactly like today.
    let mut trusted_macro_calls: Vec<(TsNode<'_>, TsNode<'_>, usize)> = Vec::new();
    for n in &syntax {
        if n.kind() != "macro_invocation" || !is_trusted_assert_macro(n) {
            continue;
        }
        let mut cursor = n.walk();
        let Some(tokens) = n.children(&mut cursor).find(|c| c.kind() == "token_tree") else {
            continue;
        };
        let masked = super::mask_rust_macro_non_code(node_text(tokens, source));
        if !is_safe_to_trust_macro_arguments(&masked) {
            continue;
        }
        let index = owner(*n, out, module);
        collect_call_shaped_identifiers(tokens, source, index, &mut trusted_macro_calls);
    }
    let exempt_call_shaped_ids: HashSet<usize> = trusted_macro_calls
        .iter()
        .map(|(identifier, _, _)| identifier.id())
        .collect();
    // Direct-call proof is deliberately restricted to top-level declarations.
    // Do not certify nested functions, methods, imports, aliases or factories
    // until their binding/dispatch rules are implemented and measured.
    let mut top = HashMap::<String, Vec<(NodeId, usize)>>::new();
    for n in &syntax {
        if !matches!(
            n.kind(),
            "function_item" | "function_definition" | "function_declaration"
        ) {
            continue;
        }
        let Some(parent) = n.parent() else { continue };
        let top_level = parent.id() == root.id()
            || (parent.kind() == "export_statement"
                && parent.parent().is_some_and(|p| p.id() == root.id()));
        if !top_level || n.child_by_field_name("body").is_none() {
            continue;
        }
        if lang == Lang::Rust
            && node_text(*n, source)
                .split('{')
                .next()
                .unwrap_or("")
                .contains("extern")
        {
            continue;
        }
        let Some(name) = n.child_by_field_name("name") else {
            continue;
        };
        let Some(node) = out.nodes.iter().find(|node| {
            node.kind == NodeKind::Function
                && node.span.start_byte == n.start_byte()
                && node.span.end_byte == n.end_byte()
        }) else {
            continue;
        };
        top.entry(node_text(name, source).to_string())
            .or_default()
            .push((node.id, name.id()));
    }
    let ts_module = !lang.is_typescript()
        || syntax
            .iter()
            .any(|n| matches!(n.kind(), "export_statement" | "import_statement"));
    // Python only: `transformed_scope` (any decorator/decorated_definition
    // ANYWHERE in the file) is too blunt a gate for Python's own
    // same-file Must path -- a single unrelated `@pytest.mark.parametrize`
    // on some other test function disqualified every other Must-eligible
    // call in the file (Stage 3 Python's gate-profile found this blocks
    // 12/13 real conservative audit sites, sole blocker for one of them).
    // The machinery that actually needs to carry the soundness weight once
    // `transformed_scope` stops gating Python -- `top_level`'s existing
    // `parent.id() == root.id()` check (already excludes a candidate whose
    // own parent is `decorated_definition`, so a decorated function was
    // never eligible here to begin with) and the `clean` scan below -- is
    // unit-tested against decorated-file fixtures specifically (not just
    // assumed to still work), since neither had ever run on a file that
    // wasn't already rejected wholesale by `transformed_scope` before this
    // change. Two gaps the *existing* checks can't see, closed here
    // instead: a wildcard import can rebind a name with no `identifier`
    // node the `clean` scan would ever find, and a string-literal
    // rebinding (`setattr(obj, "target", ...)`, `patch("pkg.target")`,
    // `globals()["target"] = ...`) is invisible to the same `identifier`-
    // only scan. Both fail closed at the whole-file or per-name level
    // respectively, matching this design's existing philosophy elsewhere
    // (erring toward Unknown, never toward guessing a target).
    let python_wildcard_import =
        lang == Lang::Python && syntax.iter().any(|n| n.kind() == "wildcard_import");
    let python_exec_eval_compile = lang == Lang::Python
        && syntax.iter().any(|n| {
            n.kind() == "call"
                && n.child_by_field_name("function")
                    .is_some_and(|f| matches!(node_text(f, source), "exec" | "eval" | "compile"))
        });
    // Any string literal anywhere in the file equalling a candidate name,
    // or ending `.name` (the dotted-path form `unittest.mock.patch(...)`
    // uses), excludes that specific name -- deliberately not restricted to
    // arguments of a setattr/patch-*looking* call, since a naive call-site
    // check missed `delattr`, `exec`-built strings, `globals()`/`vars()`/
    // `__dict__` subscript assignment, and multi-line/black-formatted
    // calls in initial drafts of this check. Reads only `string_content`
    // child nodes (the grammar's own non-quote text), not the whole
    // `string` node's text with quotes stripped by hand, which breaks on
    // raw/triple-quoted/f-strings.
    let mut python_string_literals: HashSet<String> = HashSet::new();
    if lang == Lang::Python {
        for n in &syntax {
            if n.kind() != "string" {
                continue;
            }
            let mut cursor = n.walk();
            let mut content = String::new();
            for child in n.named_children(&mut cursor) {
                if child.kind() == "string_content" {
                    content.push_str(node_text(child, source));
                }
            }
            if !content.is_empty() {
                python_string_literals.insert(content);
            }
        }
    }
    // Same disqualifying effect as `setattr(mod, "target", ...)`, spelled as
    // `mod.target = ...` / `mod.target += ...` / `del mod.target` instead --
    // confirmed against the grammar directly: `assignment`/
    // `augmented_assignment` have a `left` field that is an `attribute` node
    // (`object`/`attribute` fields) when the target is dotted, and
    // `delete_statement` contains the deleted `attribute` node the same way.
    // Collected here even though the SAME-FILE case is already redundant
    // with `clean` below (the grammar's `attribute` field is itself an
    // `identifier` node, so `clean`'s blanket identifier scan already
    // rejects `target` in `mod.target = ...` as neither the declaration nor
    // a direct call -- verified by mutation: removing this set from
    // `string_rebound` below left the same-file tests passing). This set
    // exists for the PROJECT-WIDE pass (`sync/python_rebinding.rs`), which
    // has no visibility into another file's `clean` scan at all.
    let mut python_attribute_rebind_targets: HashSet<String> = HashSet::new();
    if lang == Lang::Python {
        for n in &syntax {
            let target = match n.kind() {
                "assignment" | "augmented_assignment" => n
                    .child_by_field_name("left")
                    .filter(|l| l.kind() == "attribute"),
                "delete_statement" => {
                    let mut cursor = n.walk();
                    let children: Vec<_> = n.named_children(&mut cursor).collect();
                    children.into_iter().find(|c| c.kind() == "attribute")
                }
                _ => None,
            };
            if let Some(attr) = target.and_then(|t| t.child_by_field_name("attribute")) {
                python_attribute_rebind_targets.insert(node_text(attr, source).to_string());
            }
        }
    }
    let string_rebound = |name: &str| -> bool {
        lang == Lang::Python
            && (python_string_literals
                .iter()
                .any(|s| s == name || s.ends_with(&format!(".{name}"))))
    };
    let python_whole_file_blocked = python_wildcard_import || python_exec_eval_compile;
    let effective_scope_gate = if lang == Lang::Python {
        python_whole_file_blocked
    } else {
        transformed_scope
    };
    let mut proven = HashMap::new();
    if !root.has_error() && !duplicate_paths && !effective_scope_gate && ts_module {
        for (name, candidates) in top {
            if candidates.len() != 1 {
                continue;
            }
            let (target, declaration) = candidates[0];
            if string_rebound(&name) {
                continue;
            }
            // Every occurrence must be either the declaration or a direct call.
            // This rejects parameter/local/import shadowing, assignments,
            // escaping function values, and unmodeled name uses conservatively.
            let clean = syntax
                .iter()
                .filter(|n| {
                    matches!(
                        n.kind(),
                        "identifier"
                            | "field_identifier"
                            | "property_identifier"
                            | "type_identifier"
                    ) && node_text(**n, source) == name
                })
                .all(|n| {
                    n.id() == declaration
                        || exempt_call_shaped_ids.contains(&n.id())
                        || n.parent().is_some_and(|p| {
                            callable(&p)
                                && p.kind() != "new_expression"
                                && p.child_by_field_name("function")
                                    .is_some_and(|f| f.id() == n.id())
                        })
                });
            if clean {
                proven.insert(name, target);
            }
        }
    }
    let macro_owners: HashSet<usize> = syntax
        .iter()
        .filter(|n| n.kind() == "macro_invocation" && !is_trusted_assert_macro(n))
        .map(|n| owner(*n, out, module))
        .collect();
    // Emit Must claims for the call-shaped identifiers collected above, now
    // that both `proven` and `macro_owners` exist. A call inside a trusted
    // assert is proven exactly like an ordinary same-file direct call: the
    // callee must be the sole in-file top-level definition of that name
    // (`proven`), and the owning function must not also contain some OTHER,
    // untrusted macro invocation elsewhere (`macro_owners`) -- a
    // hygiene-breaking proc or declarative macro there could still rebind
    // the name before this assert runs, even though this assert's own
    // arguments are individually clean.
    for (identifier, call_tokens, index) in &trusted_macro_calls {
        if macro_owners.contains(index) {
            continue;
        }
        let Some(&target) = proven.get(node_text(*identifier, source)) else {
            continue;
        };
        claims.entry(*index).or_default().push(CallClaim {
            site: Span {
                start_byte: identifier.start_byte(),
                end_byte: call_tokens.end_byte(),
                start_row: identifier.start_position().row,
                start_col: identifier.start_position().column,
            },
            class: CallClass::Must,
            targets: vec![target],
            reason: "proven-call-inside-trusted-assertion-macro".into(),
            coverage_gap: false,
        });
    }
    for n in &syntax {
        if callable(n) {
            let index = owner(*n, out, module);
            let function = n.child_by_field_name("function");
            let target = function
                .filter(|f| f.kind() == "identifier")
                .filter(|_| n.kind() != "new_expression" && !macro_owners.contains(&index))
                .and_then(|f| proven.get(node_text(f, source)))
                .copied();
            let reason = if target.is_some() {
                "proven-top-level-lexical-binding"
            } else {
                unresolved_reason(*n, source, lang)
            };
            claims.entry(index).or_default().push(CallClaim {
                site: span_of(*n),
                class: if target.is_some() {
                    CallClass::Must
                } else {
                    CallClass::Unknown
                },
                targets: target.into_iter().collect(),
                reason: reason.into(),
                coverage_gap: false,
            });
        } else if matches!(
            n.kind(),
            "macro_invocation" | "decorator" | "attribute_item" | "inner_attribute_item"
        ) {
            claims
                .entry(owner(*n, out, module))
                .or_default()
                .push(CallClaim {
                    site: span_of(*n),
                    class: CallClass::Unknown,
                    targets: vec![],
                    reason: "unexpanded-macro-or-decorator".into(),
                    coverage_gap: true,
                });
        } else if lang == Lang::Python
            && matches!(
                n.kind(),
                "binary_operator" | "comparison_operator" | "unary_operator" | "not_operator"
            )
        {
            // Each of these can invoke a dunder method implicitly
            // (`__add__`/`__eq__`/`__neg__`/`__bool__`, etc.) whose target
            // depends on the operand's runtime type -- exactly the kind of
            // dispatch the whole-module `implicit-runtime-dispatch-not-
            // certified` gap already discloses, but that gap is excluded
            // from "covering" any specific site by every scorer that reads
            // it (it would otherwise make every byte offset in every
            // Python file always covered, hiding a genuine absence of
            // per-site evidence). Without a per-site claim here, a bare
            // operator expression used as a statement/assert argument gets
            // NO CallClaim at all -- not even an honest Unknown one -- an
            // unsafe_exclusion, not a disclosed gap.
            //
            // Two deliberate imprecisions, both toward over-disclosure
            // (never toward hiding a gap), matching this module's own
            // stated "coarse, textual over-approximation" design:
            // `boolean_operator` (`and`/`or`) is excluded entirely --
            // short-circuit control flow, not an operator-overload
            // dispatch point, so flagging it would misrepresent ordinary
            // control flow as uncertain dispatch. `comparison_operator` is
            // flagged as a whole node even when its only operator is
            // `is`/`is not` (identity comparison, no dunder involved) --
            // Python's grammar bundles a chained comparison
            // (`a == b is None`) into one node, and distinguishing "this
            // node's only operator is is/is not" from "it also has a real
            // comparison" would need per-token inspection this coarse,
            // node-kind-only design doesn't do anywhere else. A pure
            // `is`/`is not` comparison gets an unnecessary but harmless
            // Unknown boundary record, not a wrong answer.
            claims
                .entry(owner(*n, out, module))
                .or_default()
                .push(CallClaim {
                    site: span_of(*n),
                    class: CallClass::Unknown,
                    targets: vec![],
                    reason: "implicit-operator-dispatch-not-certified".into(),
                    coverage_gap: true,
                });
        }
    }
    let mut gaps = Vec::new();
    if root.has_error() {
        gaps.push("parse-error");
    }
    if duplicate_paths {
        gaps.push("duplicate-semantic-path");
    }
    // These are explicit coverage boundaries, never invented missing-call counts.
    // Attribute access, operators, iteration and destruction can invoke code
    // without a syntactic call expression. Their dispatch is not yet certified.
    if lang == Lang::Python || lang.is_typescript() {
        gaps.push("implicit-runtime-dispatch-not-certified");
    }
    if lang == Lang::Rust
        && syntax.iter().any(|n| {
            matches!(
                n.kind(),
                "let_declaration"
                    | "binary_expression"
                    | "unary_expression"
                    | "try_expression"
                    | "index_expression"
                    | "for_expression"
                    | "impl_item"
            )
        })
    {
        gaps.push("implicit-drop-or-operator-dispatch-not-certified");
    }
    if lang == Lang::Go
        && source
            .lines()
            .any(|line| line.trim_start().starts_with("//go:"))
    {
        gaps.push("go-build-or-compiler-directive-not-certified");
    }
    for reason in gaps {
        claims.entry(module).or_default().push(CallClaim {
            site: span_of(root),
            class: CallClass::Unknown,
            targets: vec![],
            reason: reason.into(),
            coverage_gap: true,
        });
    }
    for (index, node) in out
        .nodes
        .iter_mut()
        .enumerate()
        .filter(|(_, node)| node.kind == NodeKind::Function || node.kind == NodeKind::Module)
    {
        let node_claims = claims.remove(&index).unwrap_or_default();
        let mut assumptions = vec!["indexed-source-snapshot".into()];
        if lang == Lang::Python || lang.is_typescript() {
            assumptions.push("no-runtime-rebinding-or-monkey-patching".into());
        }
        if node_claims
            .iter()
            .any(|c| c.reason == "proven-call-inside-trusted-assertion-macro")
        {
            // Shadow detection for assert/assert_eq/etc. is whole-file, not
            // whole-crate: a `macro_rules!` redefinition in a different file
            // brought into scope via textual (`#[macro_use]`) or 2018-style
            // path scoping leaves no trace this file's syntax tree can see.
            // Disclosed here rather than silently assumed away.
            assumptions.push("assert-macro-not-shadowed-crate-wide".into());
        }
        let evidence = CallEvidence::new(node, node_claims, assumptions);
        // All fields are strings/integers/enums supported by RON. Should encoding
        // nevertheless fail, missing evidence is surfaced as Unknown by queries.
        let _ = evidence.attach(node);
    }
    AnnotateGates {
        transformed_scope,
        duplicate_paths,
        parse_error: root.has_error(),
        macro_owners,
        python_string_literals,
        python_attribute_rebind_targets,
    }
}

fn walk<'tree>(node: TsNode<'tree>, nodes: &mut Vec<TsNode<'tree>>) {
    nodes.push(node);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, nodes);
    }
}

fn owner(node: TsNode<'_>, out: &BuildOutput, module: usize) -> usize {
    out.nodes
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            candidate.kind == NodeKind::Function
                && candidate.span.start_byte <= node.start_byte()
                && candidate.span.end_byte >= node.end_byte()
        })
        .min_by_key(|(_, candidate)| candidate.span.end_byte - candidate.span.start_byte)
        .map_or(module, |(index, _)| index)
}

/// Every identifier a `use` declaration's text could bind into scope --
/// deliberately over-inclusive (splits on `::`, `,`, `{`, `}`, whitespace,
/// and treats every remaining word as a possibly-bound name, including
/// path segments that are not the final binding) so that shadow detection
/// fails closed: a name this misses could wrongly permit trust in a
/// shadowed macro, but a name this over-reports only costs an unrelated
/// file the (safe, unchanged) fallback behavior.
fn use_declaration_names(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| {
        c == ':' || c == ',' || c == '{' || c == '}' || c == ';' || c.is_whitespace()
    })
    .filter(|word| !word.is_empty() && *word != "use" && *word != "as" && *word != "pub")
}

/// Whole-argument-list backstop for trusting calls found by walking a trusted
/// assert macro's token tree: refuses the *entire* macro invocation, not just
/// one occurrence, when its (masked) argument text contains anything this
/// simple structural walk cannot reason about. Over-rejection is safe here --
/// it only means an occurrence falls back to no proof, never a wrong one.
fn is_safe_to_trust_macro_arguments(masked: &str) -> bool {
    if masked.contains('{') || masked.contains('|') {
        return false; // a block or closure/bitwise-or: could bind or rewrite locally.
    }
    if contains_word(masked, "let") || contains_word(masked, "fn") {
        return false; // a local binding or nested item definition.
    }
    !contains_bang_macro_invocation(masked)
}

fn contains_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
        .any(|token| token == word)
}

/// True if the text contains an `identifier!` sequence (a nested macro
/// invocation), which could rewrite or never evaluate what follows it. `!=`
/// is excluded so a plain inequality comparison is not mistaken for one.
fn contains_bang_macro_invocation(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut chars = text.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if !(ch == '_' || ch.is_ascii_alphabetic()) {
            continue;
        }
        let mut end = start + ch.len_utf8();
        while let Some(&(idx, next)) = chars.peek() {
            if next == '_' || next.is_ascii_alphanumeric() {
                end = idx + next.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        if bytes.get(end) == Some(&b'!') && bytes.get(end + 1) != Some(&b'=') {
            return true;
        }
    }
    false
}

/// Recursively finds call-shaped bare-identifier occurrences (`name(...)`,
/// not `.name(...)` or `Path::name(...)`) among a trusted macro's argument
/// token-tree leaves. Tree-sitter does not parse macro arguments as ordinary
/// expressions, so there is no `call_expression` node to find here; this
/// walks the raw tokens directly, using the surrounding source text (not
/// grammar-specific anonymous-node kinds) to check for a `.`/`::` qualifier
/// immediately before the identifier and a `(...)` token tree immediately
/// after it.
fn collect_call_shaped_identifiers<'a>(
    node: TsNode<'a>,
    source: &str,
    owner_index: usize,
    out: &mut Vec<(TsNode<'a>, TsNode<'a>, usize)>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "identifier" {
            let before = source[..child.start_byte()].trim_end();
            let qualified = before.ends_with('.') || before.ends_with("::");
            if !qualified {
                if let Some(next) = child.next_sibling() {
                    if next.kind() == "token_tree" && node_text(next, source).starts_with('(') {
                        out.push((child, next, owner_index));
                    }
                }
            }
        }
        collect_call_shaped_identifiers(child, source, owner_index, out);
    }
}

fn unresolved_reason(node: TsNode<'_>, source: &str, lang: Lang) -> &'static str {
    let Some(function) = node.child_by_field_name("function") else {
        return "unresolved-call-syntax";
    };
    let text = node_text(function, source);
    match lang {
        Lang::Python if matches!(text, "getattr" | "setattr" | "eval" | "exec" | "__import__") => {
            "python-reflection"
        }
        Lang::Python => "python-binding-or-dispatch-unproven",
        Lang::TypeScript | Lang::Tsx => "typescript-binding-or-structural-dispatch-unproven",
        Lang::Go if text.starts_with("reflect.") || text.starts_with("C.") => {
            "go-reflection-or-ffi"
        }
        Lang::Go => "go-binding-or-interface-dispatch-unproven",
        Lang::Rust => "rust-binding-or-dispatch-unproven",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GraphBuilder;
    use aether_graph::SemanticGraph;

    fn graph(file: &str, source: &str) -> SemanticGraph {
        let mut graph = SemanticGraph::new();
        GraphBuilder::new().load_file(&mut graph, file, source);
        graph
    }

    #[test]
    fn direct_local_bindings_have_evidence_in_all_four_languages() {
        for (file, source) in [
            ("src/lib.rs", "fn target() {} fn caller() { target(); }"),
            (
                "app.py",
                "def target():\n    pass\ndef caller():\n    target()\n",
            ),
            (
                "app.ts",
                "export function target() {} export function caller() { target(); }",
            ),
            (
                "app.go",
                "package app\nfunc target() {}\nfunc caller() { target() }\n",
            ),
        ] {
            let g = graph(file, source);
            let caller = g.nodes().find(|n| n.name == "caller").unwrap();
            let claims = g.call_evidence(caller.id).unwrap();
            assert_eq!(claims.calls.len(), 1, "{file}: {claims:?}");
            assert_eq!(claims.calls[0].class, CallClass::Must, "{file}: {claims:?}");
        }
    }

    #[test]
    fn a_unique_name_does_not_certify_a_shadowed_call() {
        for (file, source) in [
            (
                "src/lib.rs",
                "fn target() {} fn caller(target: fn()) { target(); }",
            ),
            (
                "app.py",
                "def target():\n    pass\ndef caller(target):\n    target()\n",
            ),
            (
                "app.ts",
                "export function target() {} function caller(target: () => void) { target(); }",
            ),
            (
                "app.go",
                "package app\nfunc target() {}\nfunc caller(target func()) { target() }\n",
            ),
        ] {
            let g = graph(file, source);
            let caller = g.nodes().find(|n| n.name == "caller").unwrap();
            assert!(
                g.call_evidence(caller.id)
                    .unwrap()
                    .calls
                    .iter()
                    .all(|c| c.class == CallClass::Unknown),
                "{file}"
            );
        }
    }

    #[test]
    fn reflection_macro_and_parse_gaps_are_recorded_without_targets() {
        for (file, source, reason) in [
            (
                "app.py",
                "def caller(x):\n    getattr(x, 'run')()\n",
                "python-reflection",
            ),
            (
                "src/lib.rs",
                "fn caller() { opaque!(); }",
                "unexpanded-macro-or-decorator",
            ),
            ("app.ts", "function caller( {", "parse-error"),
        ] {
            let g = graph(file, source);
            let claims: Vec<_> = g
                .nodes()
                .filter_map(|n| g.call_evidence(n.id).ok())
                .flat_map(|e| e.calls)
                .collect();
            assert!(
                claims
                    .iter()
                    .any(|c| c.reason == reason && c.targets.is_empty()),
                "{file}: {claims:?}"
            );
        }
    }

    #[test]
    fn a_decorator_elsewhere_in_a_python_file_no_longer_blocks_an_unrelated_same_file_call() {
        // Reproduces the exact shape Stage 3 Python's gate-profile found:
        // one real audit site (deprecated_from_orm) and one dispatch-corpus
        // fixture (decorator-elsewhere-in-file) were blocked SOLELY by the
        // whole-file transformed_scope gate tripping on an unrelated
        // decorator. This is the positive case this change targets.
        let g = graph(
            "app.py",
            "import functools\n\
             def target():\n\
             \x20\x20\x20\x20return 42\n\
             def caller():\n\
             \x20\x20\x20\x20return target()\n\
             @functools.lru_cache\n\
             def unrelated_cached():\n\
             \x20\x20\x20\x20return 1\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims.calls.iter().any(|c| c.class == CallClass::Must),
            "{claims:?}"
        );
    }

    #[test]
    fn a_decorated_target_itself_is_still_never_proven_must() {
        // The machinery this relies on now that transformed_scope no
        // longer gates Python: `top_level`'s existing
        // `parent.id() == root.id()` check already excludes a candidate
        // whose own parent is `decorated_definition` -- a decorated
        // function was never eligible for `top` at all. This is the FIRST
        // time this specific protection is exercised on a file that isn't
        // already rejected wholesale by transformed_scope -- mutation-
        // verified below.
        let g = graph(
            "app.py",
            "def dec(f):\n\
             \x20\x20\x20\x20return f\n\
             @dec\n\
             def target():\n\
             \x20\x20\x20\x20return 42\n\
             def caller():\n\
             \x20\x20\x20\x20return target()\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims.calls.iter().all(|c| c.class != CallClass::Must),
            "{claims:?}"
        );
    }

    #[test]
    fn a_later_decorated_redefinition_blocks_proof_of_the_earlier_plain_one() {
        // `def target()` then later `@dec def target()` -- the decorated
        // redefinition is excluded from `top` (never eligible), so only
        // the first, undecorated `target` is a `top` candidate with a
        // unique count of 1. Protection here is actually `duplicate_paths`,
        // not `clean` as an earlier version of this comment claimed
        // (verified by mutation: forcing `clean = true` left this test
        // passing) -- both `target` definitions share the same path-derived
        // NodeId, so `out.nodes.iter().any(|n| !seen.insert(n.id))` trips
        // `duplicate_paths` for the whole file, which gates `proven`
        // entirely before `clean` is ever reached for this name.
        let g = graph(
            "app.py",
            "def dec(f):\n\
             \x20\x20\x20\x20return f\n\
             def target():\n\
             \x20\x20\x20\x20return 42\n\
             @dec\n\
             def target():\n\
             \x20\x20\x20\x20return 43\n\
             def caller():\n\
             \x20\x20\x20\x20return target()\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims.calls.iter().all(|c| c.class != CallClass::Must),
            "{claims:?}"
        );
    }

    #[test]
    fn a_name_used_as_a_decorator_argument_value_blocks_proof() {
        // `@pytest.mark.parametrize("f", [target])` -- target appears as a
        // VALUE inside decorator arguments, an identifier occurrence that
        // is neither the declaration nor a direct call callee.
        let g = graph(
            "app.py",
            "def target():\n\
             \x20\x20\x20\x20return 42\n\
             @mark.parametrize('f', [target])\n\
             def test_x():\n\
             \x20\x20\x20\x20pass\n\
             def caller():\n\
             \x20\x20\x20\x20return target()\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims.calls.iter().all(|c| c.class != CallClass::Must),
            "{claims:?}"
        );
    }

    #[test]
    fn a_name_used_bare_as_a_decorator_blocks_proof() {
        // `@target` (no parens) -- target used directly as a decorator on
        // some other function, an identifier occurrence inside a
        // `decorator` node, not a `call` node's function field.
        let g = graph(
            "app.py",
            "def target(f):\n\
             \x20\x20\x20\x20return f\n\
             @target\n\
             def wrapped():\n\
             \x20\x20\x20\x20pass\n\
             def caller():\n\
             \x20\x20\x20\x20return target(wrapped)\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims.calls.iter().all(|c| c.class != CallClass::Must),
            "{claims:?}"
        );
    }

    #[test]
    fn global_del_and_import_as_all_block_proof_in_a_decorated_file() {
        for source in [
            "def target():\n    return 42\n@mark.thing\ndef other():\n    global target\n    target = None\ndef caller():\n    return target()\n",
            "def target():\n    return 42\n@mark.thing\ndef other():\n    del target\ndef caller():\n    return target()\n",
            "def target():\n    return 42\n@mark.thing\ndef other():\n    pass\nimport helper as target\ndef caller():\n    return target()\n",
        ] {
            let g = graph("app.py", source);
            let caller = g.nodes().find(|n| n.name == "caller").unwrap();
            let claims = g.call_evidence(caller.id).unwrap();
            assert!(
                claims.calls.iter().all(|c| c.class != CallClass::Must),
                "{source}: {claims:?}"
            );
        }
    }

    #[test]
    fn a_wildcard_import_blocks_every_proof_in_that_file() {
        let g = graph(
            "app.py",
            "from somewhere import *\n\
             def target():\n\
             \x20\x20\x20\x20return 42\n\
             def caller():\n\
             \x20\x20\x20\x20return target()\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims.calls.iter().all(|c| c.class != CallClass::Must),
            "{claims:?}"
        );
    }

    #[test]
    fn a_string_literal_matching_the_name_blocks_proof_setattr_shape() {
        let g = graph(
            "app.py",
            "def target():\n\
             \x20\x20\x20\x20return 42\n\
             def other():\n\
             \x20\x20\x20\x20setattr(mod, 'target', None)\n\
             def caller():\n\
             \x20\x20\x20\x20return target()\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims.calls.iter().all(|c| c.class != CallClass::Must),
            "{claims:?}"
        );
    }

    #[test]
    fn a_dotted_string_literal_ending_in_the_name_blocks_proof_patch_shape() {
        let g = graph(
            "app.py",
            "def target():\n\
             \x20\x20\x20\x20return 42\n\
             def other():\n\
             \x20\x20\x20\x20patch('pkg.mod.target')\n\
             def caller():\n\
             \x20\x20\x20\x20return target()\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims.calls.iter().all(|c| c.class != CallClass::Must),
            "{claims:?}"
        );
    }

    #[test]
    fn a_same_file_attribute_assignment_matching_the_name_blocks_proof_via_clean() {
        // Exercises `clean`'s pre-existing identifier scan on a shape it had
        // never been tested against, not the new
        // `python_attribute_rebind_targets` set: the grammar's `attribute`
        // field is itself an `identifier` node, so `clean` already rejects
        // this (verified: this test still passes with
        // `python_attribute_rebind_targets` removed from `string_rebound`).
        let g = graph(
            "app.py",
            "def target():\n\
             \x20\x20\x20\x20return 42\n\
             def other():\n\
             \x20\x20\x20\x20mod.target = None\n\
             def caller():\n\
             \x20\x20\x20\x20return target()\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims.calls.iter().all(|c| c.class != CallClass::Must),
            "{claims:?}"
        );
    }

    #[test]
    fn a_same_file_del_of_a_dotted_attribute_matching_the_name_blocks_proof_via_clean() {
        // Same as above: exercises `clean`, not the new project-wide set.
        let g = graph(
            "app.py",
            "def target():\n\
             \x20\x20\x20\x20return 42\n\
             def other():\n\
             \x20\x20\x20\x20del mod.target\n\
             def caller():\n\
             \x20\x20\x20\x20return target()\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims.calls.iter().all(|c| c.class != CallClass::Must),
            "{claims:?}"
        );
    }

    #[test]
    fn an_exec_or_eval_call_anywhere_blocks_every_proof_in_that_file() {
        let g = graph(
            "app.py",
            "def target():\n\
             \x20\x20\x20\x20return 42\n\
             def other():\n\
             \x20\x20\x20\x20eval('1 + 1')\n\
             def caller():\n\
             \x20\x20\x20\x20return target()\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims.calls.iter().all(|c| c.class != CallClass::Must),
            "{claims:?}"
        );
    }

    #[test]
    fn a_typescript_file_with_the_same_decorator_elsewhere_shape_is_unaffected() {
        // This change is deliberately gated to Python only. A TypeScript
        // file with the identical "unrelated decorator elsewhere in the
        // file" shape must NOT newly prove Must -- TypeScript's own
        // transformed_scope gating is untouched, and Stage 3 TypeScript
        // hasn't started (the roadmap's own language-order rule).
        let g = graph(
            "app.ts",
            "export function target() { return 42; }\n\
             export function caller() { return target(); }\n\
             class Foo {\n\
             \x20\x20@bar\n\
             \x20\x20unrelated() {}\n\
             }\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims.calls.iter().all(|c| c.class != CallClass::Must),
            "{claims:?}"
        );
    }

    #[test]
    fn a_bare_comparison_expression_gets_an_unknown_claim_not_no_claim_at_all() {
        // Reproduces the exact real-world gap Stage 3 Python's before-
        // observation found (docs/observations/stage3-python-audit/
        // before-observation.md): a function with two real calls and a
        // bare `assert x == y` between them used to leave NO CallClaim at
        // all covering the comparison -- not even an honest Unknown one --
        // scoring as unsafe_exclusion rather than a disclosed gap.
        let g = graph(
            "app.py",
            "def make():\n    pass\ndef dump(m):\n    pass\ndef caller():\n    m = make()\n    assert m.a == 3\n    dump(m)\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        let operator_claims: Vec<_> = claims
            .calls
            .iter()
            .filter(|c| c.reason == "implicit-operator-dispatch-not-certified")
            .collect();
        assert_eq!(operator_claims.len(), 1, "{claims:?}");
        assert_eq!(operator_claims[0].class, CallClass::Unknown);
        assert!(operator_claims[0].targets.is_empty());
        assert!(operator_claims[0].coverage_gap);
    }

    #[test]
    fn boolean_and_or_are_not_flagged_as_operator_dispatch() {
        // `and`/`or` are short-circuit control flow, not an
        // operator-overload dispatch point (no `__and__`/`__or__` call
        // involved) -- flagging them would misrepresent ordinary control
        // flow as uncertain dispatch.
        let g = graph("app.py", "def caller(a, b):\n    return a and b\n");
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims
                .calls
                .iter()
                .all(|c| c.reason != "implicit-operator-dispatch-not-certified"),
            "{claims:?}"
        );
    }

    #[test]
    fn unary_and_not_operators_are_flagged_as_operator_dispatch() {
        let g = graph(
            "app.py",
            "def caller(a, b):\n    x = -a\n    y = not b\n    return x, y\n",
        );
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        let count = claims
            .calls
            .iter()
            .filter(|c| c.reason == "implicit-operator-dispatch-not-certified")
            .count();
        assert_eq!(count, 2, "{claims:?}");
    }

    #[test]
    fn methods_do_not_inherit_the_legacy_resolvers_confidence() {
        let g = graph("app.py", "class Base:\n    def run(self):\n        pass\n    def caller(self):\n        self.run()\nclass Child(Base):\n    def run(self):\n        pass\n");
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        assert!(g
            .call_evidence(caller.id)
            .unwrap()
            .calls
            .iter()
            .all(|c| c.class == CallClass::Unknown));
    }

    #[test]
    fn a_bare_call_inside_assert_eq_is_proven_must() {
        let g = graph(
            "src/lib.rs",
            "fn target() -> i32 { 42 }\n#[test]\nfn test_caller() { assert_eq!(target(), 42); }\n",
        );
        let target = g.nodes().find(|n| n.name == "target").unwrap();
        let caller = g.nodes().find(|n| n.name == "test_caller").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        assert!(
            claims.calls.iter().any(|c| c.class == CallClass::Must
                && c.reason == "proven-call-inside-trusted-assertion-macro"
                && c.targets == vec![target.id]),
            "{claims:?}"
        );
    }

    #[test]
    fn a_local_binding_inside_the_assert_itself_blocks_must() {
        // `target` here is a closure bound INSIDE the assert's own arguments,
        // not a call to the top-level fn of the same name -- the whole-block
        // backstop (rejects on `{`) must refuse the entire macro invocation.
        let g = graph(
            "src/lib.rs",
            "fn target() -> i32 { 1 }\n#[test]\nfn test_x() { assert!({ let target = || 2; target() } == 2); }\n",
        );
        let claims: Vec<_> = g
            .nodes()
            .filter_map(|n| g.call_evidence(n.id).ok())
            .flat_map(|e| e.calls)
            .collect();
        assert!(
            claims
                .iter()
                .all(|c| c.reason != "proven-call-inside-trusted-assertion-macro"),
            "{claims:?}"
        );
    }

    #[test]
    fn a_nested_macro_invocation_inside_the_assert_blocks_must() {
        // `m!` could rewrite or never evaluate `target()` inside it -- the
        // nested-`ident!` backstop must refuse the whole macro invocation.
        let g = graph(
            "src/lib.rs",
            "fn target() -> i32 { 1 }\n#[test]\nfn test_x() { assert!(m!(target()) == 1); }\n",
        );
        let claims: Vec<_> = g
            .nodes()
            .filter_map(|n| g.call_evidence(n.id).ok())
            .flat_map(|e| e.calls)
            .collect();
        assert!(
            claims
                .iter()
                .all(|c| c.reason != "proven-call-inside-trusted-assertion-macro"),
            "{claims:?}"
        );
    }

    #[test]
    fn an_untrusted_macro_elsewhere_in_the_same_function_blocks_must() {
        // Even though assert_eq!'s own arguments are clean, `opaque!()`
        // elsewhere in test_x's body means the owning function is in
        // macro_owners -- an unrelated hygiene-breaking macro there could
        // still rebind `target` before the assert runs.
        let g = graph(
            "src/lib.rs",
            "fn target() -> i32 { 1 }\n#[test]\nfn test_x() { opaque!(); assert_eq!(target(), 1); }\n",
        );
        let claims: Vec<_> = g
            .nodes()
            .filter_map(|n| g.call_evidence(n.id).ok())
            .flat_map(|e| e.calls)
            .collect();
        assert!(
            claims
                .iter()
                .all(|c| c.reason != "proven-call-inside-trusted-assertion-macro"),
            "{claims:?}"
        );
    }

    #[test]
    fn the_must_claim_span_is_the_call_not_the_whole_macro() {
        // A second, unproven call in the same assert (`other()`, a
        // never-defined-in-file bare call so it stays unresolved either
        // way) must not fall inside the tight Must span meant for `target`.
        let source =
            "fn target() -> i32 { 1 }\n#[test]\nfn test_x() { assert_eq!(target(), other()); }\n";
        let g = graph("src/lib.rs", source);
        let caller = g.nodes().find(|n| n.name == "test_x").unwrap();
        let claims = g.call_evidence(caller.id).unwrap();
        let must = claims
            .calls
            .iter()
            .find(|c| c.reason == "proven-call-inside-trusted-assertion-macro")
            .unwrap_or_else(|| panic!("{claims:?}"));
        let call_text = &source[must.site.start_byte..must.site.end_byte];
        assert_eq!(call_text, "target()", "{claims:?}");
        let whole_macro_span = claims
            .calls
            .iter()
            .find(|c| c.reason == "unexpanded-macro-or-decorator")
            .unwrap_or_else(|| panic!("{claims:?}"));
        assert!(
            must.site.start_byte > whole_macro_span.site.start_byte
                && must.site.end_byte < whole_macro_span.site.end_byte,
            "{claims:?}"
        );
    }

    #[test]
    fn a_local_macro_rules_shadowing_assert_eq_blocks_must() {
        let g = graph(
            "src/lib.rs",
            "macro_rules! assert_eq { () => {} }\nfn target() {}\n#[test]\nfn test_caller() { assert_eq!(target(), 42); }\n",
        );
        let claims: Vec<_> = g
            .nodes()
            .filter_map(|n| g.call_evidence(n.id).ok())
            .flat_map(|e| e.calls)
            .collect();
        assert!(
            claims
                .iter()
                .all(|c| c.reason != "proven-call-inside-trusted-assertion-macro"),
            "{claims:?}"
        );
    }

    #[test]
    fn a_use_import_aliased_to_assert_eq_blocks_must() {
        let g = graph(
            "src/lib.rs",
            "use custom::foo as assert_eq;\nfn target() {}\n#[test]\nfn test_caller() { assert_eq!(target(), 42); }\n",
        );
        let claims: Vec<_> = g
            .nodes()
            .filter_map(|n| g.call_evidence(n.id).ok())
            .flat_map(|e| e.calls)
            .collect();
        assert!(
            claims
                .iter()
                .all(|c| c.reason != "proven-call-inside-trusted-assertion-macro"),
            "{claims:?}"
        );
    }

    #[test]
    fn a_cfg_attribute_anywhere_in_the_file_blocks_must_even_inside_assert_eq() {
        let g = graph(
            "src/lib.rs",
            "#[cfg(feature = \"x\")]\nfn unused() {}\nfn target() {}\n#[test]\nfn test_caller() { assert_eq!(target(), 42); }\n",
        );
        let claims: Vec<_> = g
            .nodes()
            .filter_map(|n| g.call_evidence(n.id).ok())
            .flat_map(|e| e.calls)
            .collect();
        assert!(
            claims
                .iter()
                .all(|c| c.reason != "proven-call-inside-trusted-assertion-macro"),
            "{claims:?}"
        );
    }

    #[test]
    fn an_attribute_macro_anywhere_in_the_file_blocks_must_even_inside_assert_eq() {
        let g = graph(
            "src/lib.rs",
            "#[derive(Debug)]\nstruct S;\nfn target() {}\n#[test]\nfn test_caller() { assert_eq!(target(), 42); }\n",
        );
        let claims: Vec<_> = g
            .nodes()
            .filter_map(|n| g.call_evidence(n.id).ok())
            .flat_map(|e| e.calls)
            .collect();
        assert!(
            claims
                .iter()
                .all(|c| c.reason != "proven-call-inside-trusted-assertion-macro"),
            "{claims:?}"
        );
    }
}
