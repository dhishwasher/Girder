//! Tree-sitter powered syntax highlighting *spans*.
//!
//! Returns editor-agnostic `(range, kind)` spans so the UI layer (egui) can map
//! kinds to colors without depending on tree-sitter. The same parser that feeds
//! the semantic graph drives highlighting — one source of structural truth.

use crate::parser::{IncrementalParser, Lang};
use tree_sitter::Node as TsNode;

/// Coarse highlight categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HlKind {
    Keyword,
    Type,
    Function,
    Str,
    Number,
    Comment,
    Ident,
    Punct,
    Plain,
}

/// A highlighted byte range.
#[derive(Debug, Clone, Copy)]
pub struct HlSpan {
    pub start: usize,
    pub end: usize,
    pub kind: HlKind,
}

/// Compute highlight spans for `source`. Spans cover only the tokens we
/// classify; the caller fills the gaps with a default color.
pub fn spans(source: &str, lang: Lang) -> Vec<HlSpan> {
    let mut parser = IncrementalParser::new(lang);
    let tree = parser.parse(source);
    let mut out = Vec::new();
    collect(tree.root_node(), &mut out);
    out.sort_by_key(|s| s.start);
    out
}

fn collect(node: TsNode, out: &mut Vec<HlSpan>) {
    let mut cursor = node.walk();
    let children: Vec<TsNode> = node.children(&mut cursor).collect();

    if children.is_empty() {
        // Leaf token — classify it.
        if let Some(kind) = classify(node) {
            out.push(HlSpan {
                start: node.start_byte(),
                end: node.end_byte(),
                kind,
            });
        }
        return;
    }
    for child in children {
        collect(child, out);
    }
}

fn classify(node: TsNode) -> Option<HlKind> {
    let kind = node.kind();

    // Comments.
    if kind.contains("comment") {
        return Some(HlKind::Comment);
    }
    // Strings (string_literal, string, string_content, etc.).
    if kind.contains("string") || kind == "char_literal" {
        return Some(HlKind::Str);
    }
    // Numbers.
    if kind.contains("integer") || kind.contains("float") || kind == "number" {
        return Some(HlKind::Number);
    }
    // Types.
    if kind == "type_identifier" || kind == "primitive_type" {
        return Some(HlKind::Type);
    }
    // Identifiers: call targets / function names render as Function.
    if kind == "identifier" {
        let parent_kind = node.parent().map(|p| p.kind()).unwrap_or("");
        if matches!(parent_kind, "call_expression" | "call" | "function_item" | "function_definition")
        {
            return Some(HlKind::Function);
        }
        return Some(HlKind::Ident);
    }
    // Anonymous tokens: alphabetic ones are keywords, the rest punctuation.
    if !node.is_named() {
        let is_word = kind.chars().all(|c| c.is_ascii_alphabetic());
        return Some(if is_word && !kind.is_empty() {
            HlKind::Keyword
        } else {
            HlKind::Punct
        });
    }
    None
}
