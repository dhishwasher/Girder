//! Semantic similarity over function and type nodes.
//!
//! Two lightweight, dependency-free token-overlap methods, both over the
//! same `tokenize`d bag of meaningful identifier/term tokens:
//!
//! - `compute_similarity_edges` derives `SemanticSimilar` edges between
//!   *functions* from plain Jaccard overlap, for the Refactorer's dedup.
//! - `semantic_search` ("find the code about X") ranks *functions and
//!   types* by an IDF-weighted cosine similarity with a name/path-match
//!   boost (see [`weighted_relevance`]) — plain Jaccard treats every token
//!   equally, so a common word like "function" outweighs a distinctive one
//!   like "discover"; see `docs/description-search-accuracy.md`.
//!
//! The EXTENSION POINT is swapping either for a real embedding model and
//! cosine similarity; everything downstream (edges, search, the
//! Refactorer's dedup) stays the same.

use crate::{Edge, EdgeKind, NodeId, NodeKind, SemanticGraph};
use rayon::prelude::*;
use std::collections::{BTreeSet, HashMap};

/// Common language tokens that carry no semantic signal.
const STOPWORDS: &[&str] = &[
    "let", "mut", "fn", "pub", "return", "self", "for", "while", "loop", "if", "else", "match",
    "the", "and", "def", "class", "import", "from", "i64", "i32", "u64", "u32", "f64", "usize",
    "str", "string", "int", "float", "none", "true", "false", "result", "option", "vec", "new",
];

/// Tokenize source text into a set of meaningful lowercase terms. Case is
/// kept through scanning (unlike a naive lowercase-as-you-go scan) because
/// [`push_token`] needs it to split PascalCase/camelCase identifiers.
pub fn tokenize(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            current.push(ch);
        } else if !current.is_empty() {
            push_token(&mut out, std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        push_token(&mut out, current);
    }
    out
}

/// Strips a trailing plural/3rd-person "-s" (and its "-es" variant after a
/// sibilant) so a query verb like "saves" indexes the same token as the
/// identifier root `save`. Deliberately minimal — no "-ing"/"-ed" handling —
/// because this is the specific mismatch that hid real matches behind a
/// literal, un-stemmed token comparison (a query for "resolves a name" could
/// not match a function named `resolve` at all). Guarded against "-ss"/
/// "-us"/"-is" so "class", "status", and "analysis" are left alone.
fn strip_plural_suffix(word: &str) -> std::borrow::Cow<'_, str> {
    if word.len() > 4 && word.ends_with("ies") {
        return std::borrow::Cow::Owned(format!("{}y", &word[..word.len() - 3]));
    }
    let sibilant_es = ["ches", "shes", "sses", "xes", "zes"];
    if word.len() > 4 && sibilant_es.iter().any(|suffix| word.ends_with(suffix)) {
        return std::borrow::Cow::Borrowed(&word[..word.len() - 2]);
    }
    if word.len() > 3
        && word.ends_with('s')
        && !word.ends_with("ss")
        && !word.ends_with("us")
        && !word.ends_with("is")
    {
        return std::borrow::Cow::Borrowed(&word[..word.len() - 1]);
    }
    std::borrow::Cow::Borrowed(word)
}

fn push_token(set: &mut BTreeSet<String>, token: String) {
    // Index the whole identifier *and* its constituent words, so a query
    // for "sum list" matches `sum_list` and a query for "semantic graph"
    // matches `SemanticGraph` — the two identifier casings this codebase
    // actually uses (snake_case in bodies, PascalCase for types).
    let mut accept = |t: &str| {
        let lower = t.to_ascii_lowercase();
        let stemmed = strip_plural_suffix(&lower);
        let stemmed = stemmed.as_ref();
        if stemmed.len() >= 3
            && !STOPWORDS.contains(&stemmed)
            && !stemmed.chars().all(|c| c.is_numeric())
        {
            set.insert(stemmed.to_string());
        }
    };
    accept(&token);
    for word in split_identifier_words(&token) {
        accept(&word);
    }
}

/// Splits an identifier into its constituent words along snake_case
/// underscores and camelCase/PascalCase boundaries (a lowercase-or-digit
/// character followed by an uppercase one), e.g. `SemanticGraph` ->
/// `["Semantic", "Graph"]`, `sum_list` -> `["sum", "list"]`. An all-caps
/// acronym run (`MCPServer`) is not further split from what follows it —
/// this codebase does not use acronym-prefixed type names, so the simpler
/// rule is enough and avoids guessing at acronym boundaries.
fn split_identifier_words(token: &str) -> Vec<String> {
    let mut words = Vec::new();
    for part in token.split('_') {
        let mut word = String::new();
        let mut prev: Option<char> = None;
        for ch in part.chars() {
            if let Some(p) = prev {
                if (p.is_lowercase() || p.is_ascii_digit()) && ch.is_uppercase() {
                    words.push(std::mem::take(&mut word));
                }
            }
            word.push(ch);
            prev = Some(ch);
        }
        if !word.is_empty() {
            words.push(word);
        }
    }
    words
}

/// Node kinds `semantic_search` ranks. Functions are the common case; types
/// (structs/enums) are included because a query can name a *definition*
/// rather than any single function over it — e.g. "list of MCP tool
/// definitions with names and schemas" describes the `Tool` struct's fields,
/// not a function. There is no Const/Static `NodeKind`, so a query naming a
/// `static`/`const` value still has no correct answer; that is a real,
/// separate gap (see `docs/description-search-accuracy-policy.json`), not
/// something the scoring below can fix.
fn is_searchable_kind(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::Function | NodeKind::Type)
}

/// Inverse document frequency of each token over `corpus`: rare tokens (e.g.
/// a distinctive identifier fragment like "discover") score high, tokens
/// that appear in most nodes (e.g. "function", which shows up in dozens of
/// bodies via `NodeKind::Function`) score near zero. Smoothed
/// (`ln((n+1)/(df+1)) + 1`) so a token present in every node still gets a
/// small positive weight instead of exactly zero.
fn inverse_document_frequencies<'a>(
    corpus: impl Iterator<Item = &'a BTreeSet<String>>,
) -> HashMap<String, f32> {
    let mut document_frequency: HashMap<&str, usize> = HashMap::new();
    let mut n = 0.0f32;
    for tokens in corpus {
        n += 1.0;
        for token in tokens {
            *document_frequency.entry(token.as_str()).or_insert(0) += 1;
        }
    }
    document_frequency
        .into_iter()
        .map(|(token, df)| {
            (
                token.to_string(),
                ((n + 1.0) / (df as f32 + 1.0)).ln() + 1.0,
            )
        })
        .collect()
}

/// A query's relevance to one node: an IDF-weighted cosine similarity
/// between the query's tokens and the node's tokens, with tokens that also
/// appear in the node's own name or semantic path counted at `NAME_BOOST`
/// weight. Matching a node's identifier (or a segment of its path, e.g. a
/// module name like "mcp") is much stronger evidence of relevance than
/// matching an incidental word inside its body, so it must outweigh a
/// same-IDF body-only match rather than tie with it.
const NAME_BOOST: f32 = 3.0;

fn weighted_relevance(
    query: &BTreeSet<String>,
    name_tokens: &BTreeSet<String>,
    body_tokens: &BTreeSet<String>,
    idf: &HashMap<String, f32>,
) -> f32 {
    let weight_of = |token: &str| idf.get(token).copied().unwrap_or(1.0);
    let boosted_weight = |token: &str| -> f32 {
        let base = weight_of(token);
        if name_tokens.contains(token) {
            base * NAME_BOOST
        } else {
            base
        }
    };

    let mut dot = 0.0f32;
    let mut query_norm_sq = 0.0f32;
    for token in query {
        let w = weight_of(token);
        query_norm_sq += w * w;
        if name_tokens.contains(token) || body_tokens.contains(token) {
            dot += w * boosted_weight(token);
        }
    }

    let mut doc_norm_sq = 0.0f32;
    for token in name_tokens {
        let w = weight_of(token) * NAME_BOOST;
        doc_norm_sq += w * w;
    }
    for token in body_tokens.difference(name_tokens) {
        let w = weight_of(token);
        doc_norm_sq += w * w;
    }

    if query_norm_sq == 0.0 || doc_norm_sq == 0.0 {
        return 0.0;
    }
    dot / (query_norm_sq.sqrt() * doc_norm_sq.sqrt())
}

/// Jaccard similarity of two token sets: |A∩B| / |A∪B|, in `[0, 1]`.
pub fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.len() + b.len() - intersection;
    if union == 0 {
        0.0
    } else {
        intersection as f32 / union as f32
    }
}

impl SemanticGraph {
    /// Recompute `SemanticSimilar` edges between functions whose token overlap
    /// meets `threshold`. The O(n²) pairwise scan is parallelized with rayon
    /// (read-only); edges are then added sequentially. Returns the edge count.
    pub fn compute_similarity_edges(&mut self, threshold: f32) -> usize {
        self.clear_edges_of_kind(EdgeKind::SemanticSimilar);

        let funcs: Vec<(NodeId, BTreeSet<String>)> = self
            .query_by_kind(NodeKind::Function)
            .iter()
            .map(|n| (n.id, tokenize(&format!("{} {}", n.name, n.source))))
            .collect();

        let pairs: Vec<(NodeId, NodeId, f32)> = (0..funcs.len())
            .into_par_iter()
            .flat_map_iter(|i| {
                let mut local = Vec::new();
                for j in (i + 1)..funcs.len() {
                    let score = jaccard(&funcs[i].1, &funcs[j].1);
                    if score >= threshold {
                        local.push((funcs[i].0, funcs[j].0, score));
                    }
                }
                local
            })
            .collect();

        // Similarity is symmetric — add both directions so a query from either
        // node sees the relationship (neighbors() only walks outgoing edges).
        for (a, b, score) in &pairs {
            let _ = self.add_edge(*a, *b, Edge::with_weight(EdgeKind::SemanticSimilar, *score));
            let _ = self.add_edge(*b, *a, Edge::with_weight(EdgeKind::SemanticSimilar, *score));
        }
        pairs.len()
    }

    /// Concept search: rank functions and types by relevance to a free-text
    /// query — an IDF-weighted cosine similarity over tokens, boosting
    /// matches against a node's own name or semantic path (see
    /// [`weighted_relevance`]). Returns up to `top_k` `(node, score)` pairs,
    /// best first.
    ///
    /// Test-marked nodes (`attr("is_test")`) are excluded from candidates.
    /// This repository names tests as near-full-sentence descriptions of the
    /// behavior they cover (e.g. `loads_a_directory_and_resolves_across_files`),
    /// which is exactly the shape of a natural-language query — a real
    /// implementation's short identifier can only ever share a few tokens
    /// with a query, so an unrelated but verbosely-named test regularly
    /// out-scored the correct answer. Concept search should surface
    /// implementations, not incidentally-worded tests about them.
    pub fn semantic_search(&self, query: &str, top_k: usize) -> Vec<(NodeId, f32)> {
        let q = tokenize(query);
        if q.is_empty() {
            return Vec::new();
        }

        let candidates: Vec<(NodeId, BTreeSet<String>, BTreeSet<String>)> = self
            .nodes()
            .filter(|n| is_searchable_kind(n.kind) && n.attr("is_test").is_none())
            .map(|n| {
                let name_tokens = tokenize(&format!("{} {}", n.name, n.path));
                let body_tokens = tokenize(&n.source);
                (n.id, name_tokens, body_tokens)
            })
            .collect();

        // Document frequency counts each node once per token regardless of
        // whether the token came from its name or its body, so a term that
        // is common only within bodies (or only within names) still gets
        // discounted correctly either way.
        let node_token_sets: Vec<BTreeSet<String>> = candidates
            .iter()
            .map(|(_, name, body)| name.union(body).cloned().collect())
            .collect();
        let idf = inverse_document_frequencies(node_token_sets.iter());

        let mut scored: Vec<(NodeId, f32)> = candidates
            .iter()
            .filter_map(|(id, name_tokens, body_tokens)| {
                let score = weighted_relevance(&q, name_tokens, body_tokens, &idf);
                (score > 0.0).then_some((*id, score))
            })
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        scored.truncate(top_k);
        scored
    }
}

#[cfg(test)]
mod tests {
    use super::tokenize;
    use crate::{Node, NodeKind, SemanticGraph};

    #[test]
    fn tokenize_stems_a_query_verb_to_match_its_identifier_root() {
        // The bug this closes: a query for "resolves" a name found nothing
        // for a function literally named `resolve`, because the two tokens
        // were never equal without stemming.
        assert!(tokenize("resolves").contains("resolve"));
        assert!(tokenize("saves").contains("save"));
        assert!(tokenize("loads").contains("load"));
        assert!(tokenize("runs").contains("run"));
        assert!(tokenize("handles").contains("handle"));
        // "match" is itself a Rust-keyword stopword, so the sibilant-plural
        // ("-ches" -> "-ch") rule is exercised with a non-stopword instead.
        assert!(tokenize("searches").contains("search"));
    }

    #[test]
    fn tokenize_does_not_mangle_words_ending_in_ss_us_or_is() {
        assert!(tokenize("status").contains("status"));
        assert!(tokenize("analysis").contains("analysis"));
        assert!(tokenize("process").contains("process"));
        assert!(tokenize("address").contains("address"));
    }

    #[test]
    fn tokenize_splits_pascal_case_type_names_into_words() {
        // The bug this closes: a query for "semantic graph" could not match
        // a path segment `SemanticGraph`, because only snake_case was split
        // and `semanticgraph` (one un-split token) never equals either
        // query word on its own.
        let t = tokenize("SemanticGraph");
        assert!(t.contains("semantic"), "{t:?}");
        assert!(t.contains("graph"), "{t:?}");

        let t = tokenize("AnthropicProvider");
        assert!(t.contains("anthropic"), "{t:?}");
        assert!(t.contains("provider"), "{t:?}");
    }

    fn graph() -> SemanticGraph {
        let mut g = SemanticGraph::new();
        g.upsert_node(
            Node::new(
                NodeKind::Function,
                "encrypt_password",
                "crate::auth::encrypt_password",
            )
            .with_source("fn encrypt_password(password: String) -> String { hash(password) }"),
        );
        g.upsert_node(
            Node::new(
                NodeKind::Function,
                "hash_password",
                "crate::auth::hash_password",
            )
            .with_source("fn hash_password(password: String) -> String { hash(password) }"),
        );
        g.upsert_node(
            Node::new(
                NodeKind::Function,
                "render_pixel",
                "crate::gfx::render_pixel",
            )
            .with_source("fn render_pixel(buffer: Frame) { buffer.draw() }"),
        );
        g
    }

    #[test]
    fn similar_functions_get_linked() {
        let mut g = graph();
        let n = g.compute_similarity_edges(0.3);
        assert!(n >= 1, "the two password functions should be linked");
        // The graphics function should not be similar to the auth ones.
        let enc = crate::NodeId::from_path("crate::auth::encrypt_password");
        let similar: Vec<_> = g
            .neighbors(enc, Some(crate::EdgeKind::SemanticSimilar))
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert!(similar.contains(&crate::NodeId::from_path("crate::auth::hash_password")));
        assert!(!similar.contains(&crate::NodeId::from_path("crate::gfx::render_pixel")));
    }

    #[test]
    fn concept_search_finds_relevant_functions() {
        let g = graph();
        let hits = g.semantic_search("password hashing", 5);
        assert!(!hits.is_empty());
        // Top hit is one of the password functions, not the graphics one.
        let top = hits[0].0;
        assert_ne!(top, crate::NodeId::from_path("crate::gfx::render_pixel"));
    }

    /// Reproduces the bug this module was rewritten to fix: a query built
    /// from a distinctive word ("discover") and a common one ("function",
    /// which appears in many bodies here via the literal word "function")
    /// must rank the node whose *name* matches the distinctive word above
    /// nodes that only share the common word.
    #[test]
    fn a_name_match_on_a_distinctive_term_beats_a_body_match_on_a_common_term() {
        let mut g = SemanticGraph::new();
        g.upsert_node(
            Node::new(
                NodeKind::Function,
                "discover_result",
                "crate::mcp::discover_result",
            )
            .with_source("fn discover_result() -> Value { json!({\"resultType\": \"complete\"}) }"),
        );
        // Several unrelated functions all mention "function" in their body,
        // the way this repository's own source does via `NodeKind::Function`
        // pattern matches — the plain-Jaccard bug weighted that shared word
        // the same as a real match.
        for i in 0..5 {
            g.upsert_node(
                Node::new(
                    NodeKind::Function,
                    format!("render_function_{i}"),
                    format!("crate::gfx::render_function_{i}"),
                )
                .with_source("fn render_function() { call_function(); }"),
            );
        }

        let hits = g.semantic_search("the function that builds the discover response", 5);
        assert!(!hits.is_empty());
        assert_eq!(
            hits[0].0,
            crate::NodeId::from_path("crate::mcp::discover_result"),
            "{hits:?}"
        );
    }

    /// `semantic_search` must also rank `Type` nodes (structs/enums), since a
    /// query can name a definition's fields rather than any function over
    /// it — e.g. "tool definitions with names and schemas" describing a
    /// `Tool` struct's fields, not a function.
    #[test]
    fn concept_search_finds_a_matching_struct_definition() {
        let mut g = SemanticGraph::new();
        g.upsert_node(
            Node::new(NodeKind::Type, "Tool", "crate::mcp::Tool").with_source(
                "struct Tool { name: &'static str, description: &'static str, schema: fn() -> Value }",
            ),
        );
        g.upsert_node(
            Node::new(NodeKind::Function, "unrelated", "crate::other::unrelated")
                .with_source("fn unrelated() -> i64 { 1 }"),
        );

        let hits = g.semantic_search("tool definitions with names and schemas", 5);
        assert!(
            hits.iter()
                .any(|(id, _)| *id == crate::NodeId::from_path("crate::mcp::Tool")),
            "{hits:?}"
        );
    }
}
