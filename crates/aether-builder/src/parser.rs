//! Thin wrapper over tree-sitter parsers for the languages we support.

use tree_sitter::{Language, Parser, Tree};

/// Languages Girder can currently project into the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Python,
    TypeScript,
    Tsx,
    Go,
}

impl Lang {
    /// Infer language from a file path's extension.
    pub fn from_path(path: &str) -> Option<Lang> {
        match path.rsplit('.').next() {
            Some("rs") => Some(Lang::Rust),
            Some("py") => Some(Lang::Python),
            Some("ts" | "mts" | "cts") => Some(Lang::TypeScript),
            Some("tsx") => Some(Lang::Tsx),
            Some("go") => Some(Lang::Go),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Python => "python",
            Lang::TypeScript | Lang::Tsx => "typescript",
            Lang::Go => "go",
        }
    }

    pub fn is_typescript(self) -> bool {
        matches!(self, Lang::TypeScript | Lang::Tsx)
    }

    fn ts_language(self) -> Language {
        match self {
            Lang::Rust => tree_sitter_rust::language(),
            Lang::Python => tree_sitter_python::language(),
            Lang::TypeScript => tree_sitter_typescript::language_typescript(),
            Lang::Tsx => tree_sitter_typescript::language_tsx(),
            Lang::Go => tree_sitter_go::language(),
        }
    }
}

/// A reusable parser bound to one language. Holds the previous [`Tree`] so edits
/// can be re-parsed *incrementally* (tree-sitter only re-walks changed regions).
pub struct IncrementalParser {
    pub lang: Lang,
    parser: Parser,
    tree: Option<Tree>,
}

impl IncrementalParser {
    pub fn new(lang: Lang) -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&lang.ts_language())
            .expect("tree-sitter grammar/runtime ABI mismatch");
        IncrementalParser {
            lang,
            parser,
            tree: None,
        }
    }

    /// Full (re)parse from scratch.
    pub fn parse(&mut self, source: &str) -> Tree {
        let tree = self
            .parser
            .parse(source, None)
            .expect("parse should not fail for a valid grammar");
        self.tree = Some(tree.clone());
        tree
    }

    /// Incremental reparse: feed tree-sitter the prior tree (already adjusted
    /// with [`tree_sitter::InputEdit`]) so only the dirty subtree is re-walked.
    pub fn reparse(&mut self, source: &str) -> Tree {
        let old = self.tree.as_ref();
        let tree = self
            .parser
            .parse(source, old)
            .expect("incremental parse failed");
        self.tree = Some(tree.clone());
        tree
    }

    /// Record an edit on the cached tree so the next [`reparse`] is incremental.
    pub fn apply_edit(&mut self, edit: &tree_sitter::InputEdit) {
        if let Some(tree) = self.tree.as_mut() {
            tree.edit(edit);
        }
    }
}
