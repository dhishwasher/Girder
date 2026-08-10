//! One module per check-kind family. Every check reduces to a
//! [`CheckOutcome`] so the executor and report layer can treat all kinds
//! uniformly regardless of what they inspect (a subprocess exit code, the
//! rebuilt graph, a test run).

pub(crate) mod command;
pub(crate) mod graph;
pub(crate) mod test_checks;

#[derive(Debug, Clone)]
pub(crate) struct CheckOutcome {
    pub(crate) kind: String,
    pub(crate) passed: bool,
    pub(crate) detail: String,
}

impl CheckOutcome {
    pub(crate) fn not_yet_implemented(kind: &str) -> Self {
        Self {
            kind: kind.to_string(),
            passed: false,
            detail: format!("check kind {kind} is not implemented yet"),
        }
    }
}
