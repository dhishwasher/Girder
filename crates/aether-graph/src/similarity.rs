//! Semantic similarity over function nodes.
//!
//! Derives `SemanticSimilar` edges from identifier/term overlap (Jaccard over a
//! bag of meaningful tokens) and powers concept search ("find the code about
//! X"). This is the lightweight, dependency-free stand-in for learned
//! embeddings — the EXTENSION POINT is swapping `tokenize`+Jaccard for a real
//! embedding model and cosine similarity; everything downstream (edges, search,
//! the Refactorer's dedup) stays the same.

use crate::{Edge, EdgeKind, NodeId, NodeKind, SemanticGraph};
use rayon::prelude::*;
use std::collections::BTreeSet;

/// Common language tokens that carry no semantic signal.
const STOPWORDS: &[&str] = &[
    "let", "mut", "fn", "pub", "return", "self", "for", "while", "loop", "if", "else", "match",
    "the", "and", "def", "class", "import", "from", "i64", "i32", "u64", "u32", "f64", "usize",
    "str", "string", "int", "float", "none", "true", "false", "result", "option", "vec", "new",
];

/// Tokenize source text into a set of meaningful lowercase terms.
pub fn tokenize(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            push_token(&mut out, std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        push_token(&mut out, current);
    }
    out
}

fn push_token(set: &mut BTreeSet<String>, token: String) {
    // Index the whole identifier *and* its snake_case parts, so a query for
    // "sum list" matches an identifier `sum_list`.
    let mut accept = |t: &str| {
        if t.len() >= 3 && !STOPWORDS.contains(&t) && !t.chars().all(|c| c.is_numeric()) {
            set.insert(t.to_string());
        }
    };
    accept(&token);
    if token.contains('_') {
        for part in token.split('_') {
            accept(part);
        }
    }
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

    /// Concept search: rank functions by token overlap with a free-text query.
    /// Returns up to `top_k` `(node, score)` pairs, best first.
    pub fn semantic_search(&self, query: &str, top_k: usize) -> Vec<(NodeId, f32)> {
        let q = tokenize(query);
        if q.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(NodeId, f32)> = self
            .query_by_kind(NodeKind::Function)
            .iter()
            .filter_map(|n| {
                let score = jaccard(&q, &tokenize(&format!("{} {}", n.name, n.source)));
                (score > 0.0).then_some((n.id, score))
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }
}

#[cfg(test)]
mod tests {
    use crate::{Node, NodeKind, SemanticGraph};

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
}
