//! What validators report (spec §14 Phase 16, 16A: "the validators run on
//! every change"). A validator is a pure function of the document's
//! content; the [`crate::Document`] runs all of its validators after
//! every executed, undone or redone command and keeps their findings.

use serde::{Deserialize, Serialize};

/// How bad a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Warning,
    /// The document cannot be saved or built like this.
    Error,
}

/// One thing a validator found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    /// What is wrong, for people.
    pub message: String,
    /// What it is about, as the document names it (a view's name, an
    /// edge id, a node name), if it is about one thing.
    pub target: Option<String>,
}

impl Finding {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            target: None,
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            target: None,
        }
    }

    pub fn info(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Info,
            message: message.into(),
            target: None,
        }
    }

    /// The same finding about `target`.
    pub fn about(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }
}

/// A validator: findings for a document's content.
pub type Validator<D> = Box<dyn Fn(&D) -> Vec<Finding> + Send + Sync>;

/// The worst severity among `findings` (`None`: nothing found).
pub fn worst(findings: &[Finding]) -> Option<Severity> {
    findings.iter().map(|f| f.severity).max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_worst_finding_counts() {
        assert_eq!(worst(&[]), None);
        let f = [
            Finding::info("a"),
            Finding::error("b").about("x"),
            Finding::warning("c"),
        ];
        assert_eq!(worst(&f), Some(Severity::Error));
        assert_eq!(f[1].target.as_deref(), Some("x"));
    }
}
