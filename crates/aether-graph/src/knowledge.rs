//! Natural-language graph queries.
//!
//! Converts a free-text question into a typed [`KnowledgeQuery`] and dispatches
//! it to the appropriate graph traversal, returning a ranked [`QueryResult`]
//! that can be printed directly or embedded in agent transcripts.
//!
//! Supported query shapes:
//! - **Concept** — "what functions handle authentication?" → Jaccard search
//! - **Impact** — "what would break if I change X?" → BFS impact set
//! - **Callers** — "what calls X?" → incoming Calls edges
//! - **Callees** — "what does X depend on?" → outgoing Calls edges
//! - **Explain** — "explain X" → node details + immediate neighbourhood
//! - **Neighborhood** — "show subgraph around X" → multi-hop BFS

use crate::{EdgeKind, NodeId, SemanticGraph};
use std::collections::HashMap;

/// A structured graph query, parsed from a natural-language question.
#[derive(Debug, Clone, PartialEq)]
pub enum KnowledgeQuery {
    /// "what functions handle X?" / "find code about X"
    Concept(String),
    /// "what would break if I change X?" / "impact of X"
    Impact(String),
    /// "what calls X?" / "who calls X?" / "callers of X"
    Callers(String),
    /// "what does X call?" / "what does X depend on?"
    Callees(String),
    /// "explain X" / "what is X?" / "describe X"
    Explain(String),
    /// "subgraph around X" / "neighborhood of X"
    Neighborhood { center: String, hops: u32 },
}

/// The answer to a `KnowledgeQuery`.
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub question: String,
    pub kind: &'static str,
    /// Ranked list of (node_id, per-node explanation line).
    pub nodes: Vec<(NodeId, String)>,
    pub summary: String,
}

impl QueryResult {
    /// Format as readable CLI / transcript text.
    pub fn display(&self) -> String {
        let mut out = format!("Q: {}\n", self.question);
        out.push_str(&format!("[{}] {}\n", self.kind, self.summary));
        for (_, line) in &self.nodes {
            out.push_str(&format!("  · {line}\n"));
        }
        out
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Parse a natural-language question into a [`KnowledgeQuery`].
///
/// The heuristic is keyword-first: recognise question shapes before falling
/// back to a broad concept search over the full text.
pub fn parse_query(text: &str) -> KnowledgeQuery {
    let lower = text.to_lowercase();
    let lower = lower.trim_end_matches('?').trim();

    // Impact: "break", "impact", "affect" → what changes when X changes?
    if lower.contains("break") || lower.contains("impact") || lower.contains("affect") {
        let subject = after_keywords(lower, &["change", "modify", "edit", "of"]);
        return KnowledgeQuery::Impact(subject.to_string());
    }

    // Callers: "what calls X", "who calls X", "callers of X"
    // Guard: must NOT contain "does … call" (that's Callees)
    if (lower.starts_with("what calls")
        || lower.starts_with("who calls")
        || lower.contains("callers of"))
        && !lower.contains("does")
    {
        return KnowledgeQuery::Callers(last_ident(lower).to_string());
    }

    // Callees: "what does X call", "what does X depend on", "callees of X"
    if (lower.contains("does") && (lower.contains(" call") || lower.contains("depend")))
        || lower.contains("callees of")
        || lower.contains("calls made by")
    {
        let subject = between(lower, "does", "call")
            .or_else(|| between(lower, "does", "depend"))
            .unwrap_or_else(|| last_ident(lower));
        return KnowledgeQuery::Callees(subject.to_string());
    }

    // Explain: "explain X", "describe X", "what is X" (not "what is the best…")
    if lower.starts_with("explain")
        || lower.starts_with("describe")
        || (lower.starts_with("what is") && !lower.contains("best") && !lower.contains("code"))
    {
        return KnowledgeQuery::Explain(last_ident(lower).to_string());
    }

    // Neighborhood: "subgraph around X", "neighborhood of X", "around X"
    if lower.contains("subgraph") || lower.contains("neighborhood") || lower.contains("around") {
        let center = after_keywords(lower, &["around", "of", "near"]);
        return KnowledgeQuery::Neighborhood {
            center: center.to_string(),
            hops: 2,
        };
    }

    // Default: concept search over the full (original-case) text.
    KnowledgeQuery::Concept(text.trim().to_string())
}

// ── Graph traversals ──────────────────────────────────────────────────────────

impl SemanticGraph {
    /// Answer a structured [`KnowledgeQuery`] by traversing the graph.
    pub fn answer_query(&self, q: &KnowledgeQuery) -> QueryResult {
        match q {
            KnowledgeQuery::Concept(text) => self.ans_concept(text),
            KnowledgeQuery::Impact(subject) => self.ans_impact(subject),
            KnowledgeQuery::Callers(subject) => self.ans_callers(subject),
            KnowledgeQuery::Callees(subject) => self.ans_callees(subject),
            KnowledgeQuery::Explain(subject) => self.ans_explain(subject),
            KnowledgeQuery::Neighborhood { center, hops } => self.ans_neighborhood(center, *hops),
        }
    }

    /// Resolve a subject string: try exact path, then name prefix/suffix match.
    fn resolve(&self, subject: &str) -> Option<NodeId> {
        let subject = clean_subject(subject);
        if subject.is_empty() {
            return None;
        }
        if let Some(node) = self.find_by_path(subject) {
            return Some(node.id);
        }

        let lower = subject.to_lowercase();
        let suffix = format!("::{lower}");
        let mut candidates: Vec<_> = self
            .nodes()
            .filter(|n| n.name.to_lowercase() == lower)
            .collect();
        if candidates.is_empty() {
            candidates = self
                .nodes()
                .filter(|n| n.path.to_lowercase().ends_with(&suffix))
                .collect();
        }
        if candidates.is_empty() {
            candidates = self
                .nodes()
                .filter(|n| {
                    let name = n.name.to_lowercase();
                    let path = n.path.to_lowercase();
                    name.contains(&lower) || path.contains(&lower)
                })
                .collect();
        }

        candidates.sort_by(|a, b| a.path.cmp(&b.path));
        candidates.first().map(|n| n.id)
    }

    fn not_found(question: String, kind: &'static str, subject: &str) -> QueryResult {
        QueryResult {
            question,
            kind,
            nodes: Vec::new(),
            summary: format!("'{subject}' not found in graph"),
        }
    }

    fn ans_concept(&self, text: &str) -> QueryResult {
        let hits = self.semantic_search(text, 8);

        let mut nodes: Vec<(NodeId, String)> = hits
            .iter()
            .filter_map(|(id, score)| {
                self.get(*id).map(|n| {
                    (
                        *id,
                        format!("{} [{:?}] (score {:.2})", n.path, n.kind, score),
                    )
                })
            })
            .collect();

        // Extend with SemanticSimilar neighbours of the top hit.
        if let Some((top_id, _)) = hits.first() {
            for nb in self
                .neighbors(*top_id, Some(EdgeKind::SemanticSimilar))
                .into_iter()
                .take(3)
            {
                if !nodes.iter().any(|(id, _)| id == &nb.id) {
                    if let Some(n) = self.get(nb.id) {
                        nodes.push((nb.id, format!("{} [similar, {:.2}]", n.path, nb.weight)));
                    }
                }
            }
        }

        let summary = if nodes.is_empty() {
            format!("no matches for \"{text}\"")
        } else {
            format!("{} result(s) for \"{text}\"", nodes.len())
        };
        QueryResult {
            question: format!("concept: {text}"),
            kind: "concept",
            nodes,
            summary,
        }
    }

    fn ans_impact(&self, subject: &str) -> QueryResult {
        let question = format!("impact of {subject}");
        let Some(id) = self.resolve(subject) else {
            return Self::not_found(question, "impact", subject);
        };
        let node_path = self.get(id).map(|n| n.path.clone()).unwrap_or_default();
        let impact = self.impact_of(id);
        let nodes: Vec<(NodeId, String)> = impact
            .ranked()
            .into_iter()
            .filter_map(|(nid, dist)| {
                self.get(nid)
                    .map(|n| (nid, format!("{} (distance {})", n.path, dist)))
            })
            .collect();
        let summary = if nodes.is_empty() {
            format!("changing '{node_path}' affects no other recorded nodes")
        } else {
            format!(
                "changing '{node_path}' directly affects {} node(s)",
                nodes.len()
            )
        };
        QueryResult {
            question,
            kind: "impact",
            nodes,
            summary,
        }
    }

    fn ans_callers(&self, subject: &str) -> QueryResult {
        let question = format!("callers of {subject}");
        let Some(id) = self.resolve(subject) else {
            return Self::not_found(question, "callers", subject);
        };
        let node_path = self.get(id).map(|n| n.path.clone()).unwrap_or_default();
        let callers = self.callers(id);
        let mut nodes: Vec<(NodeId, String)> = callers
            .iter()
            .filter_map(|nb| self.get(nb.id).map(|n| (nb.id, n.path.clone())))
            .collect();
        nodes.sort_by(|a, b| a.1.cmp(&b.1));
        let summary = if nodes.is_empty() {
            format!("'{node_path}' has no recorded callers")
        } else {
            format!("{} function(s) call '{node_path}'", nodes.len())
        };
        QueryResult {
            question,
            kind: "callers",
            nodes,
            summary,
        }
    }

    fn ans_callees(&self, subject: &str) -> QueryResult {
        let question = format!("callees of {subject}");
        let Some(id) = self.resolve(subject) else {
            return Self::not_found(question, "callees", subject);
        };
        let node_path = self.get(id).map(|n| n.path.clone()).unwrap_or_default();
        let callees = self.neighbors(id, Some(EdgeKind::Calls));
        let mut nodes: Vec<(NodeId, String)> = callees
            .iter()
            .filter_map(|nb| self.get(nb.id).map(|n| (nb.id, n.path.clone())))
            .collect();
        nodes.sort_by(|a, b| a.1.cmp(&b.1));
        let summary = if nodes.is_empty() {
            format!("'{node_path}' calls no recorded functions")
        } else {
            format!("'{node_path}' calls {} function(s)", nodes.len())
        };
        QueryResult {
            question,
            kind: "callees",
            nodes,
            summary,
        }
    }

    fn ans_explain(&self, subject: &str) -> QueryResult {
        let question = format!("explain {subject}");
        let Some(id) = self.resolve(subject) else {
            return Self::not_found(question, "explain", subject);
        };
        let node = self.get(id).unwrap();
        let path = node.path.clone();
        let kind = node.kind;
        let source_first_line = node.source.lines().next().unwrap_or("").to_string();
        let lang = node.language.clone();

        let mut lines: Vec<(NodeId, String)> = vec![
            (id, format!("{path} [{kind:?}] ({lang})")),
            (id, format!("  source: {source_first_line}")),
        ];
        for (k, v) in &node.attributes {
            lines.push((id, format!("  {k}: {v}")));
        }
        let callees = self.neighbors(id, Some(EdgeKind::Calls));
        if !callees.is_empty() {
            let names: Vec<String> = callees
                .iter()
                .filter_map(|nb| self.get(nb.id).map(|n| n.name.clone()))
                .collect();
            lines.push((id, format!("  calls: {}", names.join(", "))));
        }
        let callers = self.callers(id);
        if !callers.is_empty() {
            let names: Vec<String> = callers
                .iter()
                .filter_map(|nb| self.get(nb.id).map(|n| n.name.clone()))
                .collect();
            lines.push((id, format!("  called by: {}", names.join(", "))));
        }
        QueryResult {
            question,
            kind: "explain",
            nodes: lines,
            summary: format!("{path} [{kind:?}]"),
        }
    }

    fn ans_neighborhood(&self, center: &str, hops: u32) -> QueryResult {
        let question = format!("neighborhood of {center}");
        let Some(id) = self.resolve(center) else {
            return Self::not_found(question, "neighborhood", center);
        };
        let center_path = self.get(id).map(|n| n.path.clone()).unwrap_or_default();

        // BFS collecting both outgoing and incoming edges.
        let mut visited: HashMap<NodeId, u32> = HashMap::new();
        let mut frontier = vec![id];
        visited.insert(id, 0);

        for hop in 1..=hops {
            let mut next = Vec::new();
            for &cur in &frontier {
                for nb in self.neighbors(cur, None) {
                    if let std::collections::hash_map::Entry::Vacant(entry) = visited.entry(nb.id) {
                        entry.insert(hop);
                        next.push(nb.id);
                    }
                }
                for nb in self.callers(cur) {
                    if let std::collections::hash_map::Entry::Vacant(entry) = visited.entry(nb.id) {
                        entry.insert(hop);
                        next.push(nb.id);
                    }
                }
            }
            frontier = next;
        }

        let mut nodes: Vec<(NodeId, String)> = visited
            .iter()
            .filter(|(&nid, _)| nid != id)
            .filter_map(|(&nid, &dist)| {
                self.get(nid)
                    .map(|n| (nid, format!("{} (hop {})", n.path, dist)))
            })
            .collect();
        nodes.sort_by(|a, b| a.1.cmp(&b.1));

        let summary = format!(
            "{} node(s) within {} hop(s) of '{center_path}'",
            nodes.len(),
            hops
        );
        QueryResult {
            question,
            kind: "neighborhood",
            nodes,
            summary,
        }
    }
}

// ── Text extraction helpers ───────────────────────────────────────────────────

/// First non-stopword identifier-like token found after any of the keywords.
fn after_keywords<'a>(text: &'a str, keywords: &[&str]) -> &'a str {
    for kw in keywords {
        if let Some(pos) = text.find(kw) {
            let rest = text[pos + kw.len()..].trim_start();
            let word = rest
                .split(|c: char| !c.is_alphanumeric() && c != '_' && c != ':')
                .next()
                .unwrap_or("")
                .trim();
            if !word.is_empty() && !is_stop(word) {
                return word;
            }
        }
    }
    last_ident(text)
}

/// Find the token between two keywords — returns `None` if not found / stopword.
fn between<'a>(text: &'a str, before: &str, after: &str) -> Option<&'a str> {
    let start = text.find(before)? + before.len();
    let slice = text[start..].trim_start();
    let end = slice.find(after).unwrap_or(slice.len());
    let word = slice[..end]
        .trim()
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != ':')
        .next()
        .unwrap_or("")
        .trim();
    (!word.is_empty() && !is_stop(word)).then_some(word)
}

/// Last token that looks like a code identifier (has `_`, `::`, or is not a stopword).
fn last_ident(text: &str) -> &str {
    let tokens: Vec<&str> = text
        .split([' ', '\t', ',', '?', '!'])
        .filter(|t| !t.is_empty())
        .collect();

    // Prefer tokens with underscores or colons (code-like).
    for tok in tokens.iter().rev() {
        let clean = tok.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != ':');
        if !clean.is_empty() && !is_stop(clean) && (clean.contains('_') || clean.contains(':')) {
            return clean;
        }
    }
    // Fall back to any non-stopword of length ≥ 2.
    for tok in tokens.iter().rev() {
        let clean = tok.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if !clean.is_empty() && !is_stop(clean) && clean.len() >= 2 {
            return clean;
        }
    }
    text
}

fn is_stop(w: &str) -> bool {
    matches!(
        w,
        "a" | "i"
            | "if"
            | "in"
            | "is"
            | "it"
            | "me"
            | "my"
            | "of"
            | "on"
            | "or"
            | "to"
            | "us"
            | "we"
            | "at"
            | "be"
            | "by"
            | "do"
            | "fn"
            | "he"
            | "no"
            | "up"
            | "an"
            | "as"
            | "so"
            | "the"
            | "and"
            | "but"
            | "for"
            | "not"
            | "she"
            | "was"
            | "are"
            | "let"
            | "mut"
            | "pub"
            | "who"
            | "what"
            | "how"
            | "when"
            | "which"
            | "show"
            | "find"
            | "get"
            | "does"
            | "call"
            | "calls"
            | "would"
            | "could"
            | "should"
    )
}

fn clean_subject(subject: &str) -> &str {
    subject
        .trim()
        .trim_matches(|c: char| matches!(c, '?' | '!' | ',' | '.' | ';' | ':' | '"' | '\'' | '`'))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, EdgeKind, Node, NodeKind, SemanticGraph};

    fn auth_graph() -> SemanticGraph {
        let mut g = SemanticGraph::new();
        let vc = g.upsert_node(
            Node::new(NodeKind::Function, "validate_credentials", "crate::auth::validate_credentials")
                .with_source("fn validate_credentials(u: &str, p: &str) -> bool { !u.is_empty() && p.len() >= 8 }"),
        );
        let gt = g.upsert_node(
            Node::new(
                NodeKind::Function,
                "generate_token",
                "crate::auth::generate_token",
            )
            .with_source(
                "fn generate_token(user_id: u64) -> String { format!(\"tok_{user_id}\") }",
            ),
        );
        let auth = g.upsert_node(
            Node::new(NodeKind::Function, "authenticate", "crate::auth::authenticate")
                .with_source("fn authenticate(u: &str, p: &str) -> Option<String> { if validate_credentials(u, p) { Some(generate_token(42)) } else { None } }"),
        );
        // authenticate → validate_credentials, authenticate → generate_token
        let _ = g.add_edge(auth, vc, Edge::new(EdgeKind::Calls));
        let _ = g.add_edge(auth, gt, Edge::new(EdgeKind::Calls));
        g
    }

    #[test]
    fn parse_impact_question() {
        let q = parse_query("what would break if I change validate_credentials?");
        assert_eq!(
            q,
            KnowledgeQuery::Impact("validate_credentials".to_string())
        );
    }

    #[test]
    fn parse_callers_question() {
        let q = parse_query("what calls authenticate?");
        assert_eq!(q, KnowledgeQuery::Callers("authenticate".to_string()));
    }

    #[test]
    fn parse_callees_question() {
        let q = parse_query("what does authenticate depend on?");
        assert_eq!(q, KnowledgeQuery::Callees("authenticate".to_string()));
    }

    #[test]
    fn parse_explain_question() {
        let q = parse_query("explain validate_credentials");
        assert_eq!(
            q,
            KnowledgeQuery::Explain("validate_credentials".to_string())
        );
    }

    #[test]
    fn parse_neighborhood_question() {
        let q = parse_query("show subgraph around authenticate");
        assert!(
            matches!(q, KnowledgeQuery::Neighborhood { ref center, hops: 2 } if center == "authenticate"),
            "got {q:?}"
        );
    }

    #[test]
    fn parse_concept_fallback() {
        let q = parse_query("find functions related to password hashing");
        assert!(matches!(q, KnowledgeQuery::Concept(_)));
    }

    #[test]
    fn impact_answer_finds_authenticate() {
        let g = auth_graph();
        // validate_credentials is called by authenticate, so changing it affects authenticate.
        let result = g.answer_query(&KnowledgeQuery::Impact("validate_credentials".to_string()));
        assert_eq!(result.kind, "impact");
        let paths: Vec<&str> = result.nodes.iter().map(|(_, s)| s.as_str()).collect();
        assert!(
            paths.iter().any(|s| s.contains("authenticate")),
            "impact set should include authenticate; got {paths:?}"
        );
    }

    #[test]
    fn callers_answer_identifies_authenticate() {
        let g = auth_graph();
        // Nothing calls authenticate in this graph — it's the entry point.
        let result = g.answer_query(&KnowledgeQuery::Callers("authenticate".to_string()));
        assert_eq!(result.kind, "callers");
        assert!(
            result.nodes.is_empty(),
            "authenticate has no callers in test graph"
        );

        // validate_credentials is called by authenticate.
        let result2 = g.answer_query(&KnowledgeQuery::Callers("validate_credentials".to_string()));
        assert!(!result2.nodes.is_empty());
        assert!(result2
            .nodes
            .iter()
            .any(|(_, s)| s.contains("authenticate")));
    }

    #[test]
    fn callees_answer_shows_dependencies() {
        let g = auth_graph();
        let result = g.answer_query(&KnowledgeQuery::Callees("authenticate".to_string()));
        assert_eq!(result.kind, "callees");
        assert_eq!(
            result.nodes.len(),
            2,
            "authenticate calls validate_credentials + generate_token"
        );
    }

    #[test]
    fn explain_answer_includes_source_and_edges() {
        let g = auth_graph();
        let result = g.answer_query(&KnowledgeQuery::Explain("authenticate".to_string()));
        assert_eq!(result.kind, "explain");
        let combined: String = result
            .nodes
            .iter()
            .map(|(_, s)| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            combined.contains("authenticate"),
            "should mention the node path"
        );
        assert!(combined.contains("calls:"), "should list callees");
    }

    #[test]
    fn neighborhood_answer_reaches_callees() {
        let g = auth_graph();
        let result = g.answer_query(&KnowledgeQuery::Neighborhood {
            center: "authenticate".to_string(),
            hops: 1,
        });
        assert_eq!(result.kind, "neighborhood");
        // hop-1 should include validate_credentials and generate_token
        assert!(
            result.nodes.len() >= 2,
            "neighborhood should include direct callees"
        );
    }

    #[test]
    fn not_found_returns_gracefully() {
        let g = SemanticGraph::new();
        let result = g.answer_query(&KnowledgeQuery::Impact("nonexistent".to_string()));
        assert!(result.summary.contains("not found"));
        assert!(result.nodes.is_empty());
    }

    #[test]
    fn ambiguous_names_resolve_deterministically_by_path() {
        let mut g = SemanticGraph::new();
        // Insert in reverse lexical order to prove resolution is path-ordered,
        // not insertion-ordered.
        g.upsert_node(Node::new(NodeKind::Function, "run", "crate::z::run"));
        g.upsert_node(Node::new(NodeKind::Function, "run", "crate::a::run"));

        let result = g.answer_query(&KnowledgeQuery::Explain("run".to_string()));
        assert!(
            result.summary.contains("crate::a::run"),
            "got {}",
            result.summary
        );
    }
}
