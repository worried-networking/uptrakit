//! Coverage assertions producing `Violation`s.

use super::{ledgers, spec::SpecOp};
use std::collections::HashSet;
use std::fmt;

/// A single coverage failure.
#[derive(Debug)]
pub struct Violation {
    pub kind: &'static str,
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.kind, self.detail)
    }
}

fn resolve(op_id: &str) -> &str {
    ledgers::RENAME_MAP
        .iter()
        .find(|(id, _)| *id == op_id)
        .map_or(op_id, |(_, method)| *method)
}

/// Assertion 1: every spec op maps to a method (or `SPEC_ONLY`); every method
/// maps back to an op (or `CLIENT_ONLY`, or is a `list_all_` companion).
#[must_use]
pub fn check_names(ops: &[SpecOp], methods: &[String]) -> Vec<Violation> {
    let method_set: HashSet<&str> = methods.iter().map(String::as_str).collect();
    let mut violations = Vec::new();
    let mut expected: HashSet<&str> = HashSet::new();

    for op in ops {
        if ledgers::SPEC_ONLY.contains(&op.operation_id.as_str()) {
            continue;
        }
        let method = resolve(&op.operation_id);
        expected.insert(method);
        if !method_set.contains(method) {
            violations.push(Violation {
                kind: "spec-only",
                detail: format!(
                    "operationId '{}' ({} {}) has no client method '{method}'",
                    op.operation_id, op.method, op.path
                ),
            });
        }
    }

    for m in methods {
        if ledgers::CLIENT_ONLY.contains(&m.as_str())
            || ledgers::is_list_all_companion(m, methods)
            || expected.contains(m.as_str())
        {
            continue;
        }
        violations.push(Violation {
            kind: "client-only",
            detail: format!("client method '{m}' has no spec operation"),
        });
    }
    violations
}
